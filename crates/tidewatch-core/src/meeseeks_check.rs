//! The checks a Meeseeks has to pass, run off the UI thread: the commit check, the tests, the clean
//! tree, the pull request and the poof. Each says what it found when it passes, or what went wrong
//! in words for the agent.

use std::path::Path;

use crate::delivery::{self, Ahead};
use crate::github_link::GithubRepo;
use crate::history;
use crate::limited;
use crate::meeseeks::Meeseeks;
use crate::meeseeks_log::pr_label_of;
use crate::meeseeks_target::{self, PullRequest, Target};
use crate::paths::TidewatchPaths;
use crate::pr_ask;
use crate::test_command::{self, Gate};
use crate::tools::{GhError, Tools, run_gh};
use crate::worktree::{self, BranchChoice, RemoveMode, Terms, Why, Worktree};
use crate::worktree_safety::Owner;

fn changed_files(tools: &Tools, dir: &Path) -> Result<usize, String> {
    let mut command = tools.git(dir);
    command.args(["status", "--porcelain"]);
    let out = tools.finish(command).map_err(|e| limited::failure("git", &e))?;
    if !out.success {
        return Err(out.stderr.trim().to_owned());
    }
    Ok(out.stdout.lines().filter(|line| !line.trim().is_empty()).count())
}

fn clean(tools: &Tools, dir: &Path) -> Result<(), String> {
    match changed_files(tools, dir)? {
        0 => Ok(()),
        1 => Err("1 file with uncommitted changes".to_owned()),
        n => Err(format!("{n} files with uncommitted changes")),
    }
}

/// What the checks need to know about the task: where its work is meant to land, what the agent
/// said it did, and when it started.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub target: Target,
    pub summary: String,
    pub started_ms: i64,
}

impl Work {
    #[must_use]
    pub fn of(meeseeks: &Meeseeks) -> Self {
        Self {
            target: meeseeks.target(),
            summary: meeseeks.summary.clone().unwrap_or_default(),
            started_ms: meeseeks.started_ms,
        }
    }
}

/// A check that passed: the one line it says, and the pull request it found when it was the hand off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub detail: String,
    pub pr_url: Option<String>,
}

#[must_use]
pub fn head(tools: &Tools, dir: &Path) -> Option<String> {
    head_or_stopped(tools, dir).ok().flatten()
}

fn head_or_stopped(tools: &Tools, dir: &Path) -> Result<Option<String>, String> {
    let mut command = tools.git(dir);
    command.args(["rev-parse", "HEAD"]);
    match tools.finish(command) {
        Ok(out) if out.success => {
            Ok(Some(out.stdout.trim().to_owned()).filter(|sha| !sha.is_empty()))
        }
        Err(e) if limited::is_stopped(&e) => Err(e.to_string()),
        _ => Ok(None),
    }
}

/// Whether the worktree differs from where the task started: files changed, or a commit made.
pub fn edited(tools: &Tools, dir: &Path, started_at: Option<&str>) -> Result<bool, String> {
    let status = history::working_status(dir).map_err(|e| format!("{e:#}"))?;
    if !status.all().is_empty() {
        return Ok(true);
    }
    Ok(started_at.is_some_and(|start| head(tools, dir).is_some_and(|now| now != start)))
}

fn said(detail: impl Into<String>) -> Checked {
    Checked { detail: detail.into(), pr_url: None }
}

pub fn change(tools: &Tools, dir: &Path, branch: &str, work: &Work) -> Result<Checked, String> {
    clean(tools, dir)?;
    let plan = Ahead::build(tools, dir, branch).map_err(|e| e.to_string())?;
    let ahead = match plan.commits.len() {
        0 => Ahead::build(tools, dir, "HEAD").map_err(|e| e.to_string())?.commits.len(),
        n => n,
    };
    match ahead {
        0 => {
            if let Target::Existing(pr) = &work.target
                && let Some(checked) = work_on_pull_request(tools, dir, pr, work.started_ms)?
            {
                return Ok(checked);
            }
            Err(format!("no commit ahead of {}", plan.base))
        }
        1 => Ok(said(format!("1 commit ahead of {}", plan.base))),
        n => Ok(said(format!("{n} commits ahead of {}", plan.base))),
    }
}

pub fn tests(repo: &Path, dir: &Path) -> Result<Checked, String> {
    let command = match test_command::resolve(dir, repo) {
        Gate::Run(detected) => detected.command,
        Gate::Skip(why) => return Ok(said(why)),
        Gate::None => return Ok(said("No test command for this repo")),
    };
    let run = test_command::run_in_timeout(dir, &command, test_command::timeout_in(dir, repo));
    if run.exit_code == 0 {
        return Ok(said(format!("{command} passed")));
    }
    Err(format!("exit {} from `{command}`\n{}", run.exit_code, run.output))
}

pub fn proof(tools: &Tools, dir: &Path) -> Result<Checked, String> {
    clean(tools, dir).map(|()| said("Worktree clean"))
}

/// The open pull request whose head is the commit this worktree is on: the agent pushed its work
/// and opened it.
fn pull_request_at_head(tools: &Tools, dir: &Path) -> Result<Option<String>, String> {
    let Some(sha) = head_or_stopped(tools, dir)? else { return Ok(None) };
    let json = match run_gh(
        tools,
        dir,
        &["pr", "list", "--state", "open", "--json", "url,headRefOid", "--limit", "100"],
    ) {
        Ok(json) => json,
        Err(GhError::Stopped(why)) => return Err(why),
        Err(_) => return Ok(None),
    };
    let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&json) else { return Ok(None) };
    Ok(list
        .iter()
        .find(|pr| pr["headRefOid"].as_str() == Some(sha.as_str()))
        .and_then(|pr| pr["url"].as_str())
        .map(str::to_owned))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Open,
    Merged,
    Closed,
}

struct Seen {
    state: State,
    url: String,
    head: String,
    same_repo: bool,
}

fn is_not_found(message: &str) -> bool {
    let message = message.to_lowercase();
    message.contains("could not resolve to a pullrequest")
        || message.contains("no pull requests found")
}

fn origin_repo(tools: &Tools, dir: &Path) -> Option<GithubRepo> {
    let mut command = tools.git(dir);
    command.args(["config", "--get", "remote.origin.url"]);
    let out = tools.finish(command).ok().filter(|out| out.success)?;
    GithubRepo::parse(out.stdout.trim())
}

/// Whether the pull request lives in the worktree's repo. An origin that is not on GitHub cannot
/// say otherwise.
fn in_this_repo(pr: &PullRequest, origin: Option<&GithubRepo>) -> bool {
    match (&pr.repo, origin) {
        (Some(named), Some(origin)) => named.same_repo(origin),
        _ => true,
    }
}

/// The pull request as GitHub has it now. `None` when gh does not know the number and the task
/// only hinted at it (`#12` may be an issue).
fn view(tools: &Tools, dir: &Path, pr: &PullRequest) -> Result<Option<Seen>, String> {
    let lookup = pr.lookup();
    let json = match run_gh(
        tools,
        dir,
        &["pr", "view", &lookup, "--json", "state,url,headRefName,headRefOid"],
    ) {
        Ok(json) => json,
        Err(GhError::Stopped(why)) => return Err(why),
        Err(GhError::Failed(message)) if is_not_found(&message) => {
            return if pr.certain {
                Err(format!("{} was not found: {}", pr.label(), message.trim()))
            } else {
                Ok(None)
            };
        }
        Err(e) => return Err(format!("could not look up {}: {e}", pr.label())),
    };
    let found: serde_json::Value = serde_json::from_str(&json)
        .map_err(|_| format!("could not read what gh said about {}", pr.label()))?;
    let state = match found["state"].as_str() {
        Some("MERGED") => State::Merged,
        Some("OPEN") => State::Open,
        Some("CLOSED") => State::Closed,
        other => return Err(format!("{} is in state {other:?}", pr.label())),
    };
    let text = |key: &str| found[key].as_str().unwrap_or_default().to_owned();
    Ok(Some(Seen {
        state,
        url: text("url"),
        head: text("headRefName"),
        same_repo: in_this_repo(pr, origin_repo(tools, dir).as_ref()),
    }))
}

fn fetch_head(tools: &Tools, dir: &Path, head: &str) -> Result<(), String> {
    if head.is_empty() || head.starts_with('-') {
        return Err(format!("the pull request has no usable branch ({head:?})"));
    }
    let mut command = tools.git(dir);
    command.args(["fetch", "--quiet", "origin", head]);
    let out = tools.finish(command).map_err(|e| limited::failure("git", &e))?;
    if out.success {
        Ok(())
    } else {
        Err(format!("could not fetch `{head}` from origin: {}", out.stderr.trim()))
    }
}

fn lines_of(tools: &Tools, dir: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let mut command = tools.git(dir);
    command.args(args);
    let out = tools.finish(command).map_err(|e| limited::failure("git", &e))?;
    if !out.success {
        return Err(out.stderr.trim().to_owned());
    }
    Ok(out.stdout.lines().map(str::to_owned).collect())
}

fn base_ref(tools: &Tools, dir: &Path) -> Option<String> {
    delivery::detect_base(tools, dir).map(|base| delivery::compare_ref(tools, dir, &base))
}

/// How many commits of the worktree the fetched pull request head lacks, by patch.
fn missing_from_fetched(tools: &Tools, dir: &Path) -> Result<usize, String> {
    let base = base_ref(tools, dir);
    let mut args = vec!["cherry", "FETCH_HEAD", "HEAD"];
    args.extend(base.as_deref());
    let lines = lines_of(tools, dir, &args)?;
    Ok(lines.iter().filter(|line| line.starts_with("+ ")).count())
}

/// The commits on the fetched pull request head that are not on the base, made since `started_ms`.
fn new_on_fetched(tools: &Tools, dir: &Path, started_ms: i64) -> Result<usize, String> {
    let since = format!("--since={}", started_ms / 1000);
    let base = base_ref(tools, dir).map(|base| format!("^{base}"));
    let mut args = vec!["log", "--format=%H"];
    if started_ms > 0 {
        args.push(&since);
    }
    args.push("FETCH_HEAD");
    args.extend(base.as_deref());
    Ok(lines_of(tools, dir, &args)?.len())
}

fn on_pull_request(tools: &Tools, dir: &Path, pr: &PullRequest) -> Result<Option<Checked>, String> {
    let Some(seen) = view(tools, dir, pr)? else { return Ok(None) };
    let label = pr.label();
    let passed = |detail: String| Ok(Some(Checked { detail, pr_url: Some(seen.url.clone()) }));
    match seen.state {
        State::Merged => passed(format!("{label} merged")),
        State::Closed => Err(pr_ask::meeseeks_pr_closed(pr.number)),
        State::Open if !seen.same_repo => {
            passed(format!("{label} is in another repo, commits not compared"))
        }
        State::Open => {
            fetch_head(tools, dir, &seen.head)?;
            match missing_from_fetched(tools, dir)? {
                0 => passed(format!("Everything is on {label}")),
                missing => Err(pr_ask::meeseeks_push_to_pr(pr.number, &seen.head, missing)),
            }
        }
    }
}

/// The change check for work that went onto the named pull request's branch instead of this one.
fn work_on_pull_request(
    tools: &Tools,
    dir: &Path,
    pr: &PullRequest,
    started_ms: i64,
) -> Result<Option<Checked>, String> {
    let Some(seen) = view(tools, dir, pr)? else { return Ok(None) };
    let label = pr.label();
    let passed = |detail: String| Ok(Some(Checked { detail, pr_url: Some(seen.url.clone()) }));
    match seen.state {
        State::Merged => passed(format!("Work is on {label}, merged")),
        State::Closed => Err(pr_ask::meeseeks_pr_closed(pr.number)),
        State::Open if !seen.same_repo => {
            passed(format!("{label} is in another repo, commits not compared"))
        }
        State::Open => {
            fetch_head(tools, dir, &seen.head)?;
            match new_on_fetched(tools, dir, started_ms)? {
                0 => Err(format!("no commit on this branch, and none new on {label}")),
                1 => passed(format!("1 new commit on {label}")),
                n => passed(format!("{n} new commits on {label}")),
            }
        }
    }
}

fn at_head(url: String) -> Checked {
    let detail = pr_label_of(&url)
        .map_or_else(|| "Found the pull request".to_owned(), |label| format!("Found {label}"));
    Checked { detail, pr_url: Some(url) }
}

/// The pull request the task names when it names one, else the open pull request at this
/// worktree's head. A task that names none can still point at one in its summary, in this repo.
pub fn hand_off(tools: &Tools, dir: &Path, branch: &str, work: &Work) -> Result<Checked, String> {
    if let Target::Existing(pr) = &work.target
        && let Some(checked) = on_pull_request(tools, dir, pr)?
    {
        return Ok(checked);
    }
    if let Some(url) = pull_request_at_head(tools, dir)? {
        return Ok(at_head(url));
    }
    if work.target == Target::New
        && let Some(pr) = meeseeks_target::in_summary(&work.summary)
        && in_this_repo(&pr, origin_repo(tools, dir).as_ref())
        && let Ok(Some(checked)) = on_pull_request(tools, dir, &pr)
    {
        return Ok(checked);
    }
    Err(pr_ask::meeseeks_fix(branch, delivery::detect_base(tools, dir).as_deref()))
}

pub fn poof(paths: &TidewatchPaths, worktree: &Worktree) -> Result<(), String> {
    let terms =
        Terms::new(BranchChoice::Keep, RemoveMode::Safe).why(Why::Poofed).owner(Owner::ThisWindow);
    worktree::remove_with(paths, worktree, &terms).map(|_| ()).map_err(|e| e.to_string())
}
