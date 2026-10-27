//! What a worktree branch has ahead of its base (D7). Tidewatch only reads; the agent opens the pull request.

use std::fmt;
use std::path::Path;

use crate::tools::{Finished, Tools, finish};

const REMOTE: &str = "origin";
const FIELD_SEPARATOR: char = '\u{1f}';
const RECORD_SEPARATOR: char = '\u{1e}';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitLine {
    pub sha: String,
    pub subject: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ahead {
    pub base: String,
    pub commits: Vec<CommitLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryError {
    NoBase,
    BadBranch(String),
    Git(String),
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBase => {
                f.write_str("cannot tell which branch to open the pull request against")
            }
            Self::BadBranch(branch) => write!(f, "{branch:?} is not a usable branch name"),
            Self::Git(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DeliveryError {}

impl Ahead {
    pub fn build(tools: &Tools, dir: &Path, branch: &str) -> Result<Self, DeliveryError> {
        Self::build_after(tools, dir, branch, None)
    }

    /// Only the commits made after `merged_head`, the head its pull request merged with.
    pub fn build_after(
        tools: &Tools,
        dir: &Path,
        branch: &str,
        merged_head: Option<&str>,
    ) -> Result<Self, DeliveryError> {
        check_branch(branch)?;
        let base = detect_base(tools, dir).ok_or(DeliveryError::NoBase)?;
        let onto = compare_ref(tools, dir, &base);
        let commits = match merged_head {
            Some(sha) => {
                check_branch(sha)?;
                let start = if is_commit(tools, dir, sha) { sha } else { &onto };
                let after = commits_between(tools, dir, start, branch)?;
                not_in_base(tools, dir, &onto, branch, after)
            }
            None => commits_between(tools, dir, &onto, branch)?,
        };
        Ok(Self { base, commits })
    }
}

/// The name a branch that was moved to a fresh remote branch is pushed as: the one it tracks, when
/// that is `<branch>-<number>`. Anything else tracked (a branch started from `origin/main`) is not
/// a place Tidewatch looks for a pull request.
#[must_use]
pub fn pushed_as(tools: &Tools, dir: &Path, branch: &str) -> String {
    let key = format!("branch.{branch}.merge");
    let tracked = git_output(tools, dir, &["config", "--get", &key])
        .ok()
        .filter(|out| out.success)
        .and_then(|out| out.stdout.trim().strip_prefix("refs/heads/").map(str::to_owned));
    tracked
        .filter(|name| {
            name.strip_prefix(branch).and_then(|rest| rest.strip_prefix('-')).is_some_and(
                |digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
            )
        })
        .unwrap_or_else(|| branch.to_owned())
}

fn is_commit(tools: &Tools, dir: &Path, sha: &str) -> bool {
    git_succeeds(tools, dir, &["cat-file", "-e", &format!("{sha}^{{commit}}")])
}

/// `commits` without those whose patch `base_ref` already has (a rebase or cherry-pick merge gave
/// them other hashes). Left as it is when `git cherry` cannot say.
fn not_in_base(
    tools: &Tools,
    dir: &Path,
    base_ref: &str,
    branch: &str,
    commits: Vec<CommitLine>,
) -> Vec<CommitLine> {
    let Ok(out) = git_output(tools, dir, &["cherry", base_ref, branch]) else { return commits };
    if !out.success {
        return commits;
    }
    let applied: Vec<&str> =
        out.stdout.lines().filter_map(|line| line.strip_prefix("- ")).map(str::trim).collect();
    commits.into_iter().filter(|c| !applied.iter().any(|full| full.starts_with(&c.sha))).collect()
}

fn check_branch(branch: &str) -> Result<(), DeliveryError> {
    if branch.is_empty() || branch.starts_with('-') || branch.contains(char::is_whitespace) {
        return Err(DeliveryError::BadBranch(branch.to_owned()));
    }
    Ok(())
}

fn git_output(tools: &Tools, dir: &Path, args: &[&str]) -> Result<Finished, DeliveryError> {
    let mut command = tools.git(dir);
    command.args(args);
    finish(command).map_err(|e| DeliveryError::Git(format!("cannot run git: {e}")))
}

fn git_succeeds(tools: &Tools, dir: &Path, args: &[&str]) -> bool {
    git_output(tools, dir, args).is_ok_and(|out| out.success)
}

/// The branch pull requests target: where `origin/HEAD` points, else `main` or `master` as a remote
/// branch, else as a local one. `None` when none of them exist. Blocks on git.
#[must_use]
pub fn detect_base(tools: &Tools, dir: &Path) -> Option<String> {
    let from_remote_head =
        git_output(tools, dir, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
            .ok()
            .filter(|out| out.success)
            .and_then(|out| out.stdout.trim().strip_prefix("origin/").map(str::to_owned))
            .filter(|name| !name.is_empty());
    let exists =
        |full: String| git_succeeds(tools, dir, &["show-ref", "--verify", "--quiet", &full]);
    from_remote_head.into_iter().chain(["main", "master"].map(str::to_owned)).find(|name| {
        exists(format!("refs/remotes/{REMOTE}/{name}")) || exists(format!("refs/heads/{name}"))
    })
}

/// What to compare the branch against: the remote copy of the base when there is one, since the
/// local base may be stale and the pull request is measured against the remote.
pub(crate) fn compare_ref(tools: &Tools, dir: &Path, base: &str) -> String {
    let remote = format!("{REMOTE}/{base}");
    if git_succeeds(
        tools,
        dir,
        &["show-ref", "--verify", "--quiet", &format!("refs/remotes/{remote}")],
    ) {
        remote
    } else {
        base.to_owned()
    }
}

fn commits_between(
    tools: &Tools,
    dir: &Path,
    base_ref: &str,
    branch: &str,
) -> Result<Vec<CommitLine>, DeliveryError> {
    let format = format!("--format=%h{FIELD_SEPARATOR}%s{FIELD_SEPARATOR}%b{RECORD_SEPARATOR}");
    let range = format!("{base_ref}..{branch}");
    let out = git_output(tools, dir, &["log", "--reverse", &format, &range, "--"])?;
    if out.success {
        Ok(parse_log(&out.stdout))
    } else {
        Err(DeliveryError::Git(out.stderr.trim().to_owned()))
    }
}

fn parse_log(text: &str) -> Vec<CommitLine> {
    text.split(RECORD_SEPARATOR)
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(3, FIELD_SEPARATOR);
            let sha = fields.next()?.trim();
            let subject = fields.next()?.trim();
            let body = fields.next().unwrap_or_default().trim();
            (!sha.is_empty()).then(|| CommitLine {
                sha: sha.into(),
                subject: subject.into(),
                body: body.into(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_records_split_into_subject_and_body() {
        let text = format!(
            "a1{FIELD_SEPARATOR}feat: one{FIELD_SEPARATOR}Body line 1\n\nBody line 2\n{RECORD_SEPARATOR}\na2{FIELD_SEPARATOR}fix: two{FIELD_SEPARATOR}{RECORD_SEPARATOR}\n"
        );
        assert_eq!(
            parse_log(&text),
            [
                CommitLine {
                    sha: "a1".into(),
                    subject: "feat: one".into(),
                    body: "Body line 1\n\nBody line 2".into()
                },
                CommitLine { sha: "a2".into(), subject: "fix: two".into(), body: String::new() },
            ]
        );
        assert_eq!(parse_log(""), []);
    }

    #[test]
    fn branch_names_that_look_like_options_are_refused() {
        for bad in ["", "-f", "--force", "has space"] {
            assert_eq!(check_branch(bad), Err(DeliveryError::BadBranch(bad.into())), "{bad:?}");
        }
        assert_eq!(check_branch("agent/brave-otter"), Ok(()));
    }
}
