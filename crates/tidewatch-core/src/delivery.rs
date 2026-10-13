//! Delivery (PLAN.md, D7): Tidewatch pushes the branch and opens the pull request, never the agent.
//!
//! [`DeliveryPlan::build`] reads the commits a worktree branch has on top of its base and proposes a
//! title and text for the "push and open a PR?" card. [`deliver`] runs `git push -u origin <branch>`
//! and then `gh pr create`, after the user approved the (possibly edited) title and text.
//!
//! [`DeliveryPlan::build`] and [`deliver`] run `git` and `gh` and block: call them off the UI thread.
//! [`propose`] is pure.

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::tools::{Finished, GhError, Tools, classify_gh, finish, run_gh};

const REMOTE: &str = "origin";
const FIELD_SEPARATOR: char = '\u{1f}';
const RECORD_SEPARATOR: char = '\u{1e}';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitLine {
    pub sha: String,
    pub subject: String,
    pub body: String,
}

/// What would be delivered: the commits and the proposed pull request text. The title and body are
/// the proposal; what [`deliver`] sends is whatever the user approved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryPlan {
    pub dir: PathBuf,
    pub branch: String,
    /// The branch the pull request targets, without a remote name (`main`).
    pub base: String,
    /// Commits on `branch` that `base` does not have, oldest first.
    pub commits: Vec<CommitLine>,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    pub url: String,
    /// False when the branch already had an open pull request: the push updated it.
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryError {
    /// The repository has no `origin` remote to push to.
    NoRemote,
    /// No `origin` remote points at GitHub.
    NoGithubRemote,
    /// Neither `origin/HEAD`, `main` nor `master` could be found to compare against.
    NoBase,
    NothingToDeliver,
    EmptyTitle,
    /// The branch name could be read as an option by git.
    BadBranch(String),
    /// The remote refused the push (rejected, non-fast-forward, a server hook).
    PushRejected(String),
    /// The push failed for another reason (network, credentials).
    PushFailed(String),
    GhMissing,
    GhNotAuthenticated,
    PrFailed(String),
    /// A git command that reads the repository failed.
    Git(String),
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRemote => f.write_str("this repository has no \"origin\" remote to push to"),
            Self::NoGithubRemote => f.write_str("the \"origin\" remote does not point at GitHub"),
            Self::NoBase => {
                f.write_str("cannot tell which branch to open the pull request against")
            }
            Self::NothingToDeliver => f.write_str("the branch has no commits on top of its base"),
            Self::EmptyTitle => f.write_str("the pull request needs a title"),
            Self::BadBranch(branch) => write!(f, "{branch:?} is not a usable branch name"),
            Self::PushRejected(message) => write!(f, "the push was rejected: {message}"),
            Self::PushFailed(message) => write!(f, "the push failed: {message}"),
            Self::GhMissing => fmt::Display::fmt(&GhError::Missing, f),
            Self::GhNotAuthenticated => fmt::Display::fmt(&GhError::NotAuthenticated, f),
            Self::PrFailed(message) | Self::Git(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DeliveryError {}

impl From<GhError> for DeliveryError {
    fn from(error: GhError) -> Self {
        match error {
            GhError::Missing => Self::GhMissing,
            GhError::NotAuthenticated => Self::GhNotAuthenticated,
            GhError::NoGithubRemote => Self::NoGithubRemote,
            GhError::Failed(message) => Self::PrFailed(message),
        }
    }
}

impl DeliveryPlan {
    /// Reads `git log <base>..<branch>` in `dir` and proposes the pull request text. `stage_summary`
    /// is the last stage's `stage_done` summary (the Hand off stage asks for a title and text).
    /// Blocks on git.
    pub fn build(
        tools: &Tools,
        dir: &Path,
        branch: &str,
        stage_summary: Option<&str>,
    ) -> Result<Self, DeliveryError> {
        check_branch(branch)?;
        let base = detect_base(tools, dir).ok_or(DeliveryError::NoBase)?;
        let range_start = compare_ref(tools, dir, &base);
        let commits = commits_between(tools, dir, &range_start, branch)?;
        let (title, body) = propose(branch, &commits, stage_summary);
        Ok(Self { dir: dir.to_path_buf(), branch: branch.to_owned(), base, commits, title, body })
    }
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

/// The proposed title and body. A stage summary wins: its first line is the title, the rest the body
/// (the Hand off stage prompt asks for exactly that). Without one, a single commit gives its own
/// subject and body, and several give the oldest subject over a list of all of them.
#[must_use]
pub fn propose(
    branch: &str,
    commits: &[CommitLine],
    stage_summary: Option<&str>,
) -> (String, String) {
    let from_summary = stage_summary.and_then(split_summary);
    let from_commits = || commit_text(branch, commits);
    match from_summary {
        Some((title, body)) if !body.is_empty() => (title, body),
        Some((title, _)) => (title, from_commits().1),
        None => from_commits(),
    }
}

fn split_summary(summary: &str) -> Option<(String, String)> {
    let mut lines = summary.trim().lines();
    let title = lines.by_ref().map(clean_title).find(|line| !line.is_empty())?;
    let body = lines.collect::<Vec<_>>().join("\n");
    Some((title, body.trim().to_owned()))
}

fn clean_title(line: &str) -> String {
    let line = line.trim().trim_start_matches('#').trim();
    let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line);
    let line = strip_label(line.trim(), "title:");
    let line = line.trim().trim_matches(|c| matches!(c, '*' | '`' | '"' | '\''));
    line.trim().to_owned()
}

fn strip_label<'a>(line: &'a str, label: &str) -> &'a str {
    match line.get(..label.len()) {
        Some(head) if head.eq_ignore_ascii_case(label) => &line[label.len()..],
        _ => line,
    }
}

fn commit_text(branch: &str, commits: &[CommitLine]) -> (String, String) {
    match commits {
        [] => (humanize_branch(branch), String::new()),
        [only] => (only.subject.clone(), only.body.clone()),
        [first, ..] => {
            let list: Vec<String> = commits.iter().map(|c| format!("- {}", c.subject)).collect();
            (first.subject.clone(), format!("## Changes\n\n{}", list.join("\n")))
        }
    }
}

fn humanize_branch(branch: &str) -> String {
    let name = branch.rsplit('/').next().unwrap_or(branch).replace(['-', '_'], " ");
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// Pushes `plan.branch` with `git push -u origin <branch>`, then opens the pull request with
/// `gh pr create --base <base> --head <branch> --title <title> --body-file <tmp>`. Blocks on the
/// network. `title` and `body` are what the user approved.
///
/// A branch that already has an open pull request is pushed and answers `created: false` with that
/// pull request's URL.
pub fn deliver(
    tools: &Tools,
    plan: &DeliveryPlan,
    title: &str,
    body: &str,
) -> Result<Delivered, DeliveryError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(DeliveryError::EmptyTitle);
    }
    if plan.commits.is_empty() {
        return Err(DeliveryError::NothingToDeliver);
    }
    check_branch(&plan.branch)?;
    run_gh(tools, &plan.dir, &["--version"])?;
    if !git_succeeds(tools, &plan.dir, &["remote", "get-url", REMOTE]) {
        return Err(DeliveryError::NoRemote);
    }
    push(tools, &plan.dir, &plan.branch)?;
    create_pull_request(tools, plan, title, body)
}

fn push(tools: &Tools, dir: &Path, branch: &str) -> Result<(), DeliveryError> {
    let out = git_output(tools, dir, &["push", "-u", REMOTE, branch])?;
    if out.success {
        return Ok(());
    }
    let message = out.stderr.trim().to_owned();
    let rejected = ["[rejected]", "[remote rejected]", "non-fast-forward", "declined"];
    if rejected.iter().any(|marker| message.contains(marker)) {
        Err(DeliveryError::PushRejected(message))
    } else {
        Err(DeliveryError::PushFailed(message))
    }
}

fn create_pull_request(
    tools: &Tools,
    plan: &DeliveryPlan,
    title: &str,
    body: &str,
) -> Result<Delivered, DeliveryError> {
    let mut body_file =
        tempfile::Builder::new().prefix("tidewatch-pr-").suffix(".md").tempfile().map_err(|e| {
            DeliveryError::PrFailed(format!("cannot write the pull request text: {e}"))
        })?;
    body_file
        .write_all(body.as_bytes())
        .and_then(|()| body_file.flush())
        .map_err(|e| DeliveryError::PrFailed(format!("cannot write the pull request text: {e}")))?;
    let mut command = tools.gh(&plan.dir);
    command
        .args(["pr", "create", "--base", &plan.base, "--head", &plan.branch, "--title", title])
        .arg("--body-file")
        .arg(body_file.path());
    let finished = finish(command).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DeliveryError::GhMissing,
        _ => DeliveryError::PrFailed(format!("cannot run gh: {e}")),
    })?;
    if finished.success {
        return last_url(&finished.stdout).map(|url| Delivered { url, created: true }).ok_or_else(
            || DeliveryError::PrFailed("gh did not print the pull request URL".into()),
        );
    }
    if finished.stderr.contains("already exists")
        && let Some(url) = first_url(&finished.stderr)
    {
        return Ok(Delivered { url, created: false });
    }
    Err(classify_gh(&finished).into())
}

fn last_url(text: &str) -> Option<String> {
    text.lines().rev().map(str::trim).find(|line| line.starts_with("http")).map(str::to_owned)
}

fn first_url(text: &str) -> Option<String> {
    text.split_whitespace().find(|word| word.starts_with("http")).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(subject: &str, body: &str) -> CommitLine {
        CommitLine { sha: "abc1234".into(), subject: subject.into(), body: body.into() }
    }

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
    fn a_single_commit_gives_its_own_text() {
        let commits = [commit("feat: add refresh", "Refresh tokens five minutes early.")];
        assert_eq!(
            propose("agent/brave-otter", &commits, None),
            ("feat: add refresh".into(), "Refresh tokens five minutes early.".into())
        );
    }

    #[test]
    fn several_commits_give_the_oldest_subject_over_a_list() {
        let commits = [commit("feat: add refresh", ""), commit("test: cover expiry", "")];
        let (title, body) = propose("agent/brave-otter", &commits, None);
        assert_eq!(title, "feat: add refresh");
        assert_eq!(body, "## Changes\n\n- feat: add refresh\n- test: cover expiry");
    }

    #[test]
    fn no_commits_falls_back_to_the_branch_name() {
        assert_eq!(propose("agent/brave-otter", &[], None), ("Brave otter".into(), String::new()));
    }

    #[test]
    fn the_stage_summary_wins_first_line_is_the_title() {
        let commits = [commit("wip", "")];
        let summary = "## Title: **Refresh tokens early**\n\nTokens refresh 5 minutes before expiry.\n\n- tested";
        let (title, body) = propose("agent/x", &commits, Some(summary));
        assert_eq!(title, "Refresh tokens early");
        assert_eq!(body, "Tokens refresh 5 minutes before expiry.\n\n- tested");
    }

    #[test]
    fn a_one_line_summary_keeps_the_commit_body() {
        let commits = [commit("wip", "details"), commit("more", "")];
        let (title, body) = propose("agent/x", &commits, Some("Add token refresh"));
        assert_eq!(title, "Add token refresh");
        assert_eq!(body, "## Changes\n\n- wip\n- more");
    }

    #[test]
    fn a_blank_summary_is_ignored() {
        let commits = [commit("feat: a", "")];
        assert_eq!(propose("agent/x", &commits, Some("  \n ")), propose("agent/x", &commits, None));
    }

    #[test]
    fn urls_are_found_in_gh_output() {
        assert_eq!(
            last_url("Warning: 1 uncommitted change\nhttps://github.com/o/r/pull/12\n"),
            Some("https://github.com/o/r/pull/12".into())
        );
        assert_eq!(last_url("nothing here"), None);
        assert_eq!(
            first_url(
                "a pull request for branch \"x\" into branch \"main\" already exists:\nhttps://github.com/o/r/pull/7\n"
            ),
            Some("https://github.com/o/r/pull/7".into())
        );
    }

    #[test]
    fn branch_names_that_look_like_options_are_refused() {
        for bad in ["", "-f", "--force", "has space"] {
            assert_eq!(check_branch(bad), Err(DeliveryError::BadBranch(bad.into())), "{bad:?}");
        }
        assert_eq!(check_branch("agent/brave-otter"), Ok(()));
    }
}
