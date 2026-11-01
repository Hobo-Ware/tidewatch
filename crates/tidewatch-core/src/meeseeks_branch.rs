//! What a poofed Meeseeks left on its branch: commits ahead of the base and the files changed with
//! their line totals. Read from the repo's main checkout, since the worktree folder is gone; a
//! branch that was deleted since says so. Blocks on git: call it off the UI thread.

use std::path::Path;

use crate::delivery::{compare_ref, detect_base};
use crate::tools::Tools;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Branch {
    Missing,
    Found(Found),
    /// Git could not be asked, with why.
    Unreadable(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    pub files: usize,
    pub added: u64,
    pub removed: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The base the branch is measured against, without a remote name.
    pub base: String,
    pub ahead: usize,
    pub changed: Changed,
}

/// The totals of `git diff --numstat`: a binary file counts as a file with no lines.
#[must_use]
pub fn parse_numstat(text: &str) -> Changed {
    let mut changed = Changed::default();
    for line in text.lines() {
        let mut columns = line.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) =
            (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        changed.files += 1;
        changed.added += added.parse::<u64>().unwrap_or(0);
        changed.removed += removed.parse::<u64>().unwrap_or(0);
    }
    changed
}

#[must_use]
pub fn read(tools: &Tools, repo: &Path, branch: &str) -> Branch {
    let local = format!("refs/heads/{branch}");
    if tools.git_stdout(repo, &["show-ref", "--verify", "--quiet", &local]).is_err() {
        return Branch::Missing;
    }
    let Some(base) = detect_base(tools, repo) else {
        return Branch::Unreadable("no base branch to compare against".to_owned());
    };
    let start = compare_ref(tools, repo, &base);
    let counted = tools.git_stdout(repo, &["rev-list", "--count", &format!("{start}..{branch}")]);
    let diff = tools.git_stdout(repo, &["diff", "--numstat", &format!("{start}...{branch}"), "--"]);
    match (counted, diff) {
        (Ok(count), Ok(numstat)) => Branch::Found(Found {
            base,
            ahead: count.trim().parse().unwrap_or(0),
            changed: parse_numstat(&numstat),
        }),
        (Err(why), _) | (_, Err(why)) => Branch::Unreadable(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_adds_up_files_and_lines_and_counts_a_binary_file_without_lines() {
        let cases = [
            ("", Changed::default()),
            ("3\t1\ta.rs\n", Changed { files: 1, added: 3, removed: 1 }),
            (
                "3\t1\ta.rs\n10\t0\tdir/b.rs\n-\t-\timg.png\n",
                Changed { files: 3, added: 13, removed: 1 },
            ),
            ("3\t1\told => new.rs\n", Changed { files: 1, added: 3, removed: 1 }),
            ("not numstat\n\n", Changed::default()),
        ];
        for (text, want) in cases {
            assert_eq!(parse_numstat(text), want, "{text:?}");
        }
    }
}
