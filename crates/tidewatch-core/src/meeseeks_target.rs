//! Where a Meeseeks' work is meant to land, read from its task: a new draft pull request, or the
//! pull request the task names (pure, table-tested).

use crate::github_link::GithubRepo;
use crate::link_scan;

const ISSUE_WORDS: [&str; 11] = [
    "issue", "issues", "fix", "fixes", "fixed", "close", "closes", "closed", "resolve", "resolves",
    "resolved",
];
const MAX_DIGITS: usize = 9;

/// A pull request the task names. `repo` is set when the task gave its link; `certain` is false for
/// a bare `#12`, which may be an issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub repo: Option<GithubRepo>,
    pub url: Option<String>,
    pub certain: bool,
}

impl PullRequest {
    /// What `gh pr view` takes: the link when the task gave one, else the number.
    #[must_use]
    pub fn lookup(&self) -> String {
        self.url.clone().unwrap_or_else(|| self.number.to_string())
    }

    #[must_use]
    pub fn label(&self) -> String {
        format!("PR #{}", self.number)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Target {
    #[default]
    New,
    Existing(PullRequest),
}

impl Target {
    /// The first pull request link in `task`, else the first `PR 12`, `PR #12` or `pull request #12`,
    /// else the first `#12` (not after `issue`, `fixes` and the like), else a new pull request.
    #[must_use]
    pub fn of_task(task: &str) -> Self {
        if let Some(pr) = link_scan::pull_requests(task).iter().find_map(|url| from_link(url)) {
            return Self::Existing(pr);
        }
        mention(task).map_or(Self::New, Self::Existing)
    }

    #[must_use]
    pub fn pull_request(&self) -> Option<&PullRequest> {
        match self {
            Self::New => None,
            Self::Existing(pr) => Some(pr),
        }
    }
}

/// The pull request a hand-off summary points at, for a task that named none.
#[must_use]
pub fn in_summary(summary: &str) -> Option<PullRequest> {
    link_scan::pull_requests(summary).iter().find_map(|url| from_link(url))
}

fn from_link(url: &str) -> Option<PullRequest> {
    let number = url.rsplit('/').next()?.parse().ok()?;
    Some(PullRequest {
        number,
        repo: GithubRepo::parse(url),
        url: Some(url.to_owned()),
        certain: true,
    })
}

fn mention(task: &str) -> Option<PullRequest> {
    let lower = task.to_ascii_lowercase();
    let found: Vec<(u64, bool)> = (0..lower.len())
        .filter(|&at| lower.is_char_boundary(at))
        .filter_map(|at| number_at(&lower, at))
        .collect();
    let (number, certain) =
        found.iter().find(|(_, certain)| *certain).or(found.first()).copied()?;
    Some(PullRequest { number, repo: None, url: None, certain })
}

fn number_at(text: &str, at: usize) -> Option<(u64, bool)> {
    let before = text[..at].chars().next_back();
    let rest = &text[at..];
    if let Some(after) = rest.strip_prefix('#') {
        let clear = !before.is_some_and(|c| c.is_alphanumeric() || matches!(c, '/' | '&'));
        let issue = previous_word(&text[..at]).is_some_and(|word| ISSUE_WORDS.contains(&word));
        return (clear && !issue).then(|| digits(after)).flatten().map(|n| (n, false));
    }
    if before.is_some_and(char::is_alphanumeric) {
        return None;
    }
    let after = rest.strip_prefix("pull request").or_else(|| rest.strip_prefix("pr"))?;
    if !after.starts_with([' ', '#']) {
        return None;
    }
    let after = after.trim_start_matches(' ');
    digits(after.strip_prefix('#').unwrap_or(after)).map(|n| (n, true))
}

fn previous_word(head: &str) -> Option<&str> {
    let word = head.split_whitespace().next_back()?;
    Some(word.trim_matches(|c: char| !c.is_alphanumeric()))
}

fn digits(text: &str) -> Option<u64> {
    let count = text.bytes().take_while(u8::is_ascii_digit).count();
    let glued = text[count..].chars().next().is_some_and(char::is_alphanumeric);
    (count > 0 && count <= MAX_DIGITS && !glued)
        .then(|| text[..count].parse().ok())
        .flatten()
        .filter(|&number| number > 0)
}

#[cfg(test)]
mod tests;
