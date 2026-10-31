//! Is a merged tab's worktree safe to remove? Blocks on git and disk: run it off the UI thread.

use std::fmt;
use std::path::Path;
use std::process::Command;

use crate::limited;
use crate::worktree::Worktree;

/// Ignored folders that are only build or dependency output: losing them costs a rebuild.
pub const BUILD_OUTPUT: [&str; 5] = ["node_modules", "target", "dist", "build", ".svelte-kit"];
const IDENTICAL_COPY_LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Tracked files with staged or unstaged changes.
    pub changed: Vec<String>,
    pub untracked: Vec<String>,
    /// Commits on the branch that the pull request head does not hold. `None` when unknown.
    pub local_commits: Option<usize>,
    /// Ignored files and folders that are not build output and not a copy of the main checkout's.
    pub ignored: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    Changes(usize),
    Untracked(usize),
    LocalCommits(usize),
    CannotTell,
}

impl fmt::Display for Blocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        match self {
            Self::Changes(n) => write!(f, "{n} file{} with uncommitted changes", plural(*n)),
            Self::Untracked(n) => write!(f, "{n} untracked file{}", plural(*n)),
            Self::LocalCommits(n) => {
                write!(f, "{n} commit{} that the pull request does not have", plural(*n))
            }
            Self::CannotTell => {
                f.write_str("could not tell whether every commit is in the pull request")
            }
        }
    }
}

impl Report {
    #[must_use]
    pub fn blockers(&self) -> Vec<Blocker> {
        let mut found = Vec::new();
        if !self.changed.is_empty() {
            found.push(Blocker::Changes(self.changed.len()));
        }
        if !self.untracked.is_empty() {
            found.push(Blocker::Untracked(self.untracked.len()));
        }
        match self.local_commits {
            Some(0) => {}
            Some(n) => found.push(Blocker::LocalCommits(n)),
            None => found.push(Blocker::CannotTell),
        }
        found
    }

    #[must_use]
    pub fn is_safe(&self) -> bool {
        self.blockers().is_empty()
    }
}

/// A squash merge leaves the branch's commits off the default branch, so `pr_head` (the head GitHub
/// last reported) is the yardstick; with no head in this clone, only commits on a remote count.
pub fn check(
    worktree: &Worktree,
    pr_head: Option<&str>,
    copied: &[String],
) -> Result<Report, String> {
    let status = git_out(&worktree.path, &["status", "--porcelain", "-z", "--ignored"])?;
    let mut report =
        Report { local_commits: local_commits(worktree, pr_head), ..Report::default() };
    let mut fields = status.split('\0').filter(|field| !field.is_empty());
    while let Some(field) = fields.next() {
        let (Some(code), Some(path)) = (field.get(..2), field.get(3..)) else { continue };
        if code.starts_with(['R', 'C']) || code.ends_with(['R', 'C']) {
            fields.next();
        }
        match code {
            "??" => report.untracked.push(path.to_owned()),
            "!!" => {
                if !is_expendable(worktree, path, copied) {
                    report.ignored.push(path.to_owned());
                }
            }
            _ => report.changed.push(path.to_owned()),
        }
    }
    Ok(report)
}

fn local_commits(worktree: &Worktree, pr_head: Option<&str>) -> Option<usize> {
    let head_known = pr_head.filter(|head| {
        let spec = format!("{head}^{{commit}}");
        git_out(&worktree.path, &["cat-file", "-e", &spec]).is_ok()
    });
    let counted = match head_known {
        Some(head) => git_out(&worktree.path, &["rev-list", "--count", &format!("{head}..HEAD")]),
        None => git_out(&worktree.path, &["rev-list", "--count", "HEAD", "--not", "--remotes"]),
    };
    counted.ok()?.trim().parse().ok()
}

fn is_expendable(worktree: &Worktree, path: &str, copied: &[String]) -> bool {
    let trimmed = path.trim_end_matches('/');
    let build_output = trimmed.split('/').any(|part| BUILD_OUTPUT.contains(&part));
    let here = worktree.path.join(trimmed);
    let is_link = here.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink());
    build_output
        || is_link
        || (copied.iter().any(|c| c == trimmed) && matches_main(&worktree.repo, &here, trimmed))
}

fn matches_main(main: &Path, here: &Path, rel: &str) -> bool {
    let (Ok(ours), Ok(theirs)) = (std::fs::metadata(here), std::fs::metadata(main.join(rel)))
    else {
        return false;
    };
    ours.is_file()
        && ours.len() == theirs.len()
        && ours.len() <= IDENTICAL_COPY_LIMIT
        && matches!((std::fs::read(here), std::fs::read(main.join(rel))), (Ok(a), Ok(b)) if a == b)
}

fn git_out(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = limited::output(Command::new("git").arg("-C").arg(dir).args(args), limited::DEFAULT)
        .map_err(|e| limited::failure("git", &e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(changed: usize, untracked: usize, commits: Option<usize>) -> Report {
        Report {
            changed: vec!["a".into(); changed],
            untracked: vec!["b".into(); untracked],
            local_commits: commits,
            ignored: Vec::new(),
        }
    }

    #[test]
    fn blockers_name_every_reason_in_words() {
        let cases = [
            (report(0, 0, Some(0)), vec![]),
            (report(1, 0, Some(0)), vec![Blocker::Changes(1)]),
            (report(0, 2, Some(0)), vec![Blocker::Untracked(2)]),
            (report(0, 0, Some(3)), vec![Blocker::LocalCommits(3)]),
            (report(0, 0, None), vec![Blocker::CannotTell]),
            (
                report(2, 1, Some(1)),
                vec![Blocker::Changes(2), Blocker::Untracked(1), Blocker::LocalCommits(1)],
            ),
        ];
        for (report, want) in cases {
            assert_eq!(report.blockers(), want);
            assert_eq!(report.is_safe(), want.is_empty());
        }
        assert_eq!(Blocker::Changes(1).to_string(), "1 file with uncommitted changes");
        assert_eq!(Blocker::Untracked(2).to_string(), "2 untracked files");
        assert_eq!(
            Blocker::LocalCommits(1).to_string(),
            "1 commit that the pull request does not have"
        );
    }

    #[test]
    fn ignored_build_output_is_not_listed_at_any_depth() {
        let worktree =
            Worktree { repo: "/r".into(), path: "/nowhere".into(), branch: "agent/x".into() };
        let cases = [
            ("node_modules/", true),
            ("apps/web/node_modules/", true),
            ("target/", true),
            ("dist/", true),
            ("build/", true),
            (".svelte-kit/", true),
            ("scratch.log", false),
            (".env.local", false),
            ("notes/", false),
            ("my-target/", false),
        ];
        for (path, want) in cases {
            assert_eq!(is_expendable(&worktree, path, &[]), want, "path {path:?}");
        }
    }
}
