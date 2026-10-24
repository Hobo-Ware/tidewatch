//! What the History and Diff panes show for one checkout: the commit graph scoped to the
//! checkout's branch (or the whole repo), how far the branch is ahead of or behind the default
//! branch, and the files and diffs of a commit or of the working tree.
//!
//! Every function reads the repository and blocks until it is done: call them from a background
//! thread, never the UI thread. Graph layout, diffs and the file watcher come from `kelp-core`.

use std::path::Path;

use crate::changes::Groups;
use kelp_core::ObjectId;
use kelp_core::commit::{self, FileChange, Summary};
use kelp_core::diff::{self, FileDiff};
use kelp_core::git_cli;
use kelp_core::history::History;
use kelp_core::review;
use kelp_core::status::{self, WorkingStatus};
use kelp_core::view::ViewFilter;
use kelp_core::watch::{Change, Layout, Watcher};

/// Which commits the graph draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Only what the checkout's current branch can reach: its commits and the ones under them.
    Current,
    /// Every branch and tag.
    Whole,
}

/// Commits one branch has that the base lacks, and the other way round.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseCompare {
    pub base: String,
    pub ahead: usize,
    pub behind: usize,
}

impl BaseCompare {
    /// `same as main`, `2 ahead main`, `2 ahead · 1 behind main`.
    #[must_use]
    pub fn describe(&self) -> String {
        let parts: Vec<String> = [(self.ahead, "ahead"), (self.behind, "behind")]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, word)| format!("{count} {word}"))
            .collect();
        if parts.is_empty() {
            format!("same as {}", self.base)
        } else {
            format!("{} {}", parts.join(" · "), self.base)
        }
    }
}

/// Reads commit summaries for the rows the graph is about to draw. Cheap object reads, done on
/// demand for the visible rows only.
pub struct Summaries {
    repo: gix::Repository,
}

impl Summaries {
    /// A commit that cannot be read still gets a row, titled by its short id.
    #[must_use]
    pub fn get(&self, id: ObjectId) -> Summary {
        commit::summary(&self.repo, id).unwrap_or_else(|_| Summary {
            title: id.to_hex_with_len(7).to_string(),
            body_preview: String::new(),
            author: String::new(),
            email: String::new(),
            time: 0,
        })
    }
}

pub struct Loaded {
    pub history: History,
    pub summaries: Summaries,
    pub head_row: Option<usize>,
    /// `None` while HEAD is detached.
    pub branch: Option<String>,
    /// `None` on the default branch itself, or when the repo has no default branch to compare to.
    pub base: Option<BaseCompare>,
    /// The git `user.name`, the author of notes written in the diff.
    pub author: String,
}

/// What a diff is taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Commit(ObjectId),
    /// Everything uncommitted (staged, unstaged and untracked) against HEAD.
    WorkTree,
}

fn view_for(scope: Scope, branch: Option<&str>) -> ViewFilter {
    let solo = match scope {
        Scope::Whole => None,
        Scope::Current => {
            Some(branch.map_or_else(|| "HEAD".to_owned(), |b| format!("refs/heads/{b}")))
        }
    };
    ViewFilter { solo, ..ViewFilter::default() }
}

/// Kelp's own `view.json` filter is not read: the scope is the only filter.
pub fn load(dir: &Path, scope: Scope) -> anyhow::Result<Loaded> {
    let repo = gix::discover(dir)?;
    let branch = repo.head_name()?.map(|name| name.shorten().to_string());
    let history = History::load_filtered(&repo, &view_for(scope, branch.as_deref()))?;
    let head_row = repo.head_id().ok().and_then(|id| history.row(&id.detach()));
    let base = branch.as_deref().and_then(|branch| compare_to_default(dir, branch));
    let author = review::author_name(&repo);
    Ok(Loaded { history, summaries: Summaries { repo }, head_row, branch, base, author })
}

pub fn working_status(dir: &Path) -> anyhow::Result<WorkingStatus> {
    status::working_status(dir)
}

pub fn search(dir: &Path, ids: &[ObjectId], query: &str) -> anyhow::Result<Vec<usize>> {
    kelp_core::search::matching_rows(dir, ids, query)
}

pub fn changed_files(dir: &Path, source: Source) -> anyhow::Result<Vec<FileChange>> {
    match source {
        Source::Commit(id) => Ok(commit::details(&gix::discover(dir)?, id)?.changes),
        Source::WorkTree => Ok(Groups::of(&status::working_status(dir)?).ordered()),
    }
}

pub fn file_diff(dir: &Path, source: Source, path: &str) -> anyhow::Result<FileDiff> {
    let repo = gix::discover(dir)?;
    match source {
        Source::Commit(id) => diff::commit_file(&repo, id, path),
        Source::WorkTree => work_tree_file(&repo, path),
    }
}

fn work_tree_file(repo: &gix::Repository, path: &str) -> anyhow::Result<FileDiff> {
    if let Ok(head) = repo.head_id() {
        return diff::range_file(repo, repo.workdir(), head.detach(), None, path, None);
    }
    let new = repo.workdir().and_then(|workdir| std::fs::read(workdir.join(path)).ok());
    Ok(diff::build(path, None, new.as_deref()))
}

/// Calls `on_change` (from a watcher thread) once a burst of file events settles. The watcher
/// stops when the returned value is dropped.
pub fn watch(dir: &Path, on_change: impl Fn(Change) + Send + 'static) -> anyhow::Result<Watcher> {
    Watcher::start(Layout::of(&gix::discover(dir)?), on_change)
}

/// True when nothing is staged, modified or untracked.
pub fn is_clean(dir: &Path) -> anyhow::Result<bool> {
    Ok(status::working_status(dir)?.all().is_empty())
}

/// The branch new work is measured against: where `origin/HEAD` points if that branch exists
/// locally, else `main`, else `master`.
#[must_use]
pub fn default_branch(dir: &Path) -> Option<String> {
    let remote_head = git_cli::run(dir, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .and_then(|out| parse_remote_head(&out));
    remote_head
        .into_iter()
        .chain(["main", "master"].map(str::to_owned))
        .find(|name| local_branch_exists(dir, name))
}

fn local_branch_exists(dir: &Path, name: &str) -> bool {
    let full = format!("refs/heads/{name}");
    git_cli::run(dir, &["rev-parse", "--verify", "--quiet", &full]).is_ok()
}

fn parse_remote_head(out: &str) -> Option<String> {
    let (_, branch) = out.trim().split_once('/')?;
    (!branch.is_empty()).then(|| branch.to_owned())
}

fn compare_to_default(dir: &Path, branch: &str) -> Option<BaseCompare> {
    let base = default_branch(dir).filter(|base| base != branch)?;
    let range = format!("{base}...HEAD");
    let out = git_cli::run(dir, &["rev-list", "--left-right", "--count", &range]).ok()?;
    let (behind, ahead) = parse_left_right(&out)?;
    Some(BaseCompare { base, ahead, behind })
}

/// `git rev-list --left-right --count base...HEAD` prints `<only in base>\t<only in HEAD>`.
fn parse_left_right(out: &str) -> Option<(usize, usize)> {
    let mut counts = out.split_whitespace().map(|n| n.parse::<usize>().ok());
    Some((counts.next()??, counts.next()??))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compare(ahead: usize, behind: usize) -> BaseCompare {
        BaseCompare { base: "main".into(), ahead, behind }
    }

    #[test]
    fn left_right_counts_are_behind_then_ahead() {
        assert_eq!(parse_left_right("3\t2\n"), Some((3, 2)));
        assert_eq!(parse_left_right("0\t0"), Some((0, 0)));
        assert_eq!(parse_left_right("12 7"), Some((12, 7)));
    }

    #[test]
    fn bad_count_output_is_none() {
        for out in ["", "\n", "4", "a\tb", "1\tx", "-1\t2"] {
            assert_eq!(parse_left_right(out), None, "output {out:?}");
        }
    }

    #[test]
    fn remote_head_loses_the_remote_name() {
        assert_eq!(parse_remote_head("origin/main\n").as_deref(), Some("main"));
        assert_eq!(parse_remote_head("upstream/release/1.x").as_deref(), Some("release/1.x"));
        assert_eq!(parse_remote_head("origin/"), None);
        assert_eq!(parse_remote_head("main"), None);
        assert_eq!(parse_remote_head(""), None);
    }

    #[test]
    fn describes_each_ahead_behind_shape() {
        assert_eq!(compare(0, 0).describe(), "same as main");
        assert_eq!(compare(2, 0).describe(), "2 ahead main");
        assert_eq!(compare(0, 5).describe(), "5 behind main");
        assert_eq!(compare(2, 1).describe(), "2 ahead · 1 behind main");
    }

    #[test]
    fn current_scope_shows_only_the_checked_out_branch() {
        let view = view_for(Scope::Current, Some("agent/brave-otter"));
        assert_eq!(view.solo.as_deref(), Some("refs/heads/agent/brave-otter"));
        assert!(view.shows("refs/heads/agent/brave-otter"));
        assert!(!view.shows("refs/heads/main"));
    }

    #[test]
    fn current_scope_on_a_detached_head_shows_no_branch_labels() {
        let view = view_for(Scope::Current, None);
        assert!(!view.shows("refs/heads/main"));
        assert!(!view.shows("refs/tags/v1"));
    }

    #[test]
    fn whole_scope_filters_nothing() {
        let view = view_for(Scope::Whole, Some("agent/brave-otter"));
        assert!(!view.is_filtering());
        assert!(view.shows("refs/heads/main"));
    }
}
