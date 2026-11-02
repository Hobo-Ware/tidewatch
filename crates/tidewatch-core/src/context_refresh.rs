//! The one "what changed" line an agent reads when Tidewatch resumes, moves or compacts its
//! session: why it is reading it, where it is (folder, branch), its pull request as last checked,
//! its open review notes and its saved report. [`line`] is pure; [`gather`] reads the tab's files
//! and asks git for the branch. The listener sends the line as hook context, never as typed text.

use std::path::{Path, PathBuf};

use crate::link_board::{Checks, Kind, Link, LinkState, Review};
use crate::notes::Notes;
use crate::paths::{links_file_beside, progress_file_beside};
use crate::progress_mcp::GET_TOOL;
use crate::progress_report::ProgressReport;
use crate::text::squash;
use crate::tools::Tools;
use crate::{link_store, progress_store};

const NOW_CHARS: usize = 120;

/// Why the agent's context needs refreshing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// Tidewatch resumed the conversation: after a restart, or in a tab it reopened.
    Resumed,
    /// The conversation ran outside Tidewatch, in `from`, and Tidewatch took it over.
    TakenOver { from: PathBuf },
    /// Claude compacted the conversation, so earlier turns are now a summary.
    Compacted,
}

/// What the agent's tab looks like right now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub folder: PathBuf,
    /// `None` on a detached HEAD or outside git.
    pub branch: Option<String>,
    pub pr: Option<Link>,
    pub open_notes: usize,
    pub report: Option<ProgressReport>,
}

/// The tab's facts from its state files beside `notes_file`, for an agent working in `folder`.
#[must_use]
pub fn gather(notes_file: &Path, folder: &Path) -> Facts {
    let branch = Tools::system()
        .git_stdout(folder, &["branch", "--show-current"])
        .ok()
        .map(|out| out.trim().to_owned())
        .filter(|name| !name.is_empty());
    let pr = link_store::load(&links_file_beside(notes_file))
        .and_then(|board| pull_request(&board.links).cloned());
    let open_notes = Notes::load(notes_file).threads.iter().filter(|t| !t.resolved).count();
    let report = progress_store::load_report(&progress_file_beside(notes_file));
    Facts { folder: folder.to_path_buf(), branch, pr, open_notes, report }
}

/// The pull request the tab is about: the newest one still open, else the newest one.
fn pull_request(links: &[Link]) -> Option<&Link> {
    let prs = || links.iter().filter(|link| link.kind() == Kind::Pr);
    prs()
        .filter(|link| !link.is_done())
        .max_by_key(|link| link.changed_ms())
        .or_else(|| prs().max_by_key(|link| link.changed_ms()))
}

/// The line, as plain sentences of facts and one ask: read the report before going on.
#[must_use]
pub fn line(why: &Why, facts: &Facts) -> String {
    let mut parts = vec![opening(why)];
    let branch = facts
        .branch
        .as_deref()
        .map_or_else(|| "no branch checked out".to_owned(), |branch| format!("branch {branch}"));
    parts.push(format!("You are in {} on {branch}.", facts.folder.display()));
    if let Some(pr) = &facts.pr {
        parts.push(pr_sentence(pr));
    }
    if facts.open_notes > 0 {
        let notes = if facts.open_notes == 1 { "note" } else { "notes" };
        parts.push(format!(
            "{} open review {notes} from the owner: call list_notes.",
            facts.open_notes
        ));
    }
    parts.push(match &facts.report {
        Some(report) => format!(
            "Your saved report says {}: \"{}\". Call {GET_TOOL} and carry on from it.",
            report.state.word(),
            clip(&report.now)
        ),
        None => format!("No progress report is saved for this tab yet ({GET_TOOL} returns none)."),
    });
    parts.join(" ")
}

fn opening(why: &Why) -> String {
    match why {
        Why::Resumed => "Tidewatch resumed this conversation, so time has passed and the PR or \
                         notes may have changed."
            .to_owned(),
        Why::TakenOver { from } => format!(
            "This conversation moved into Tidewatch from {}: work in the folder below from now \
             on, and the tidewatch MCP tools ({GET_TOOL}, update_progress, list_notes) now work.",
            from.display()
        ),
        Why::Compacted => "Your context was just compacted.".to_owned(),
    }
}

fn pr_sentence(pr: &Link) -> String {
    let number = pr.number().map_or_else(|| pr.url.clone(), |n| format!("#{n}"));
    let state = match pr.state {
        LinkState::Open => "open",
        LinkState::Merged => "merged",
        LinkState::Closed => "closed",
        LinkState::Unknown => "not checked yet",
    };
    let mut words = vec![state];
    if pr.status.draft {
        words.push("draft");
    }
    words.extend(pr.status.checks.map(|checks| match checks {
        Checks::Pass => "checks passing",
        Checks::Fail => "checks failing",
        Checks::Running => "checks running",
    }));
    words.extend(pr.status.review.map(|review| match review {
        Review::Approved => "approved",
        Review::ChangesRequested => "changes requested",
    }));
    if pr.status.conflicts {
        words.push("conflicts with its base");
    }
    format!("PR {number}: {} (as Tidewatch last checked it).", words.join(", "))
}

fn clip(text: &str) -> String {
    let one_line = squash(text);
    if one_line.chars().count() <= NOW_CHARS {
        return one_line;
    }
    let head: String = one_line.chars().take(NOW_CHARS - 3).collect();
    format!("{}...", head.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link_board::{LinkBoard, Status};
    use crate::progress::State;

    fn pr(number: u64, state: LinkState, status: Status, changed: i64) -> Link {
        Link {
            url: format!("https://github.com/hobo/web/pull/{number}"),
            title: String::new(),
            note: None,
            first_seen_ms: changed,
            title_from: crate::link_board::TitleFrom::Guess,
            state,
            status,
        }
    }

    fn report(state: State, now: &str) -> ProgressReport {
        ProgressReport {
            goal: "Ship".into(),
            title: String::new(),
            state,
            now: now.into(),
            kind: None,
            stages: Vec::new(),
            steps: Vec::new(),
            needs_you: None,
            waiting_on: None,
            scheduled: Vec::new(),
            verified: Vec::new(),
            not_verified: Vec::new(),
            decisions: Vec::new(),
        }
    }

    fn facts() -> Facts {
        Facts { folder: PathBuf::from("/w/brave-otter"), ..Facts::default() }
    }

    #[test]
    fn the_line_says_why_where_and_what_to_read_next() {
        let failing = Status {
            draft: true,
            checks: Some(Checks::Fail),
            review: Some(Review::ChangesRequested),
            ..Status::default()
        };
        let full = Facts {
            branch: Some("agent/brave-otter".into()),
            pr: Some(pr(12, LinkState::Open, failing, 1)),
            open_notes: 2,
            report: Some(report(State::Working, "Fixing the parser")),
            ..facts()
        };
        let cases = [
            (
                "a resume with everything known",
                Why::Resumed,
                full.clone(),
                "Tidewatch resumed this conversation, so time has passed and the PR or notes may have changed. You are in /w/brave-otter on branch agent/brave-otter. PR #12: open, draft, checks failing, changes requested (as Tidewatch last checked it). 2 open review notes from the owner: call list_notes. Your saved report says working: \"Fixing the parser\". Call get_progress and carry on from it.",
            ),
            (
                "a take-over names where it came from",
                Why::TakenOver { from: PathBuf::from("/w/web") },
                Facts { pr: None, open_notes: 0, ..full.clone() },
                "This conversation moved into Tidewatch from /w/web: work in the folder below from now on, and the tidewatch MCP tools (get_progress, update_progress, list_notes) now work. You are in /w/brave-otter on branch agent/brave-otter. Your saved report says working: \"Fixing the parser\". Call get_progress and carry on from it.",
            ),
            (
                "a compaction with nothing saved",
                Why::Compacted,
                facts(),
                "Your context was just compacted. You are in /w/brave-otter on no branch checked out. No progress report is saved for this tab yet (get_progress returns none).",
            ),
            (
                "one note, a merged PR",
                Why::Compacted,
                Facts {
                    open_notes: 1,
                    pr: Some(pr(3, LinkState::Merged, Status::default(), 1)),
                    ..facts()
                },
                "Your context was just compacted. You are in /w/brave-otter on no branch checked out. PR #3: merged (as Tidewatch last checked it). 1 open review note from the owner: call list_notes. No progress report is saved for this tab yet (get_progress returns none).",
            ),
        ];
        for (what, why, facts, want) in cases {
            assert_eq!(line(&why, &facts), want, "{what}");
        }
    }

    #[test]
    fn the_open_pr_wins_over_a_newer_merged_one() {
        let links = [
            pr(1, LinkState::Open, Status::default(), 10),
            pr(2, LinkState::Merged, Status::default(), 20),
            pr(3, LinkState::Open, Status::default(), 5),
        ];
        assert_eq!(pull_request(&links).and_then(Link::number), Some(1));
        assert_eq!(pull_request(&links[1..2]).and_then(Link::number), Some(2));
        assert_eq!(pull_request(&[]), None);
    }

    #[test]
    fn a_long_now_is_cut_to_one_short_line() {
        let long = format!("{}\nsecond line", "x".repeat(200));
        let said =
            line(&Why::Resumed, &Facts { report: Some(report(State::Done, &long)), ..facts() });
        assert_eq!(said.lines().count(), 1);
        assert!(said.contains(&format!("\"{}...\"", "x".repeat(NOW_CHARS - 3))), "{said}");
    }

    #[test]
    fn gather_reads_the_tabs_files_and_the_branch() {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            &["init", "-q", "-b", "agent/otter"][..],
            &["commit", "-q", "--allow-empty", "-m", "x"],
        ] {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .unwrap()
                .success();
            assert!(ok);
        }
        let notes_file = home.path().join("state/notes.json");
        std::fs::create_dir_all(notes_file.parent().unwrap()).unwrap();
        let board = LinkBoard { links: vec![pr(7, LinkState::Open, Status::default(), 1)] };
        link_store::save(&links_file_beside(&notes_file), &board).unwrap();
        progress_store::save_report(
            &progress_file_beside(&notes_file),
            &report(State::Working, "On it"),
        )
        .unwrap();
        let found = gather(&notes_file, &repo);
        assert_eq!(found.branch.as_deref(), Some("agent/otter"));
        assert_eq!(found.pr.as_ref().and_then(Link::number), Some(7));
        assert_eq!(found.open_notes, 0);
        assert_eq!(found.report.map(|r| r.now), Some("On it".to_owned()));
        let bare = gather(&home.path().join("none/notes.json"), home.path());
        assert_eq!((bare.branch, bare.pr, bare.open_notes, bare.report), (None, None, 0, None));
    }
}
