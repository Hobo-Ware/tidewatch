//! What the merged-PR card knows about its worktree: the safety check's state and the lines the
//! card shows for it (pure, table-tested). The check itself is `tidewatch_core::worktree_cleanup`.

use tidewatch_core::progress::Ledger;
use tidewatch_core::status::{AgentStatus, Reason};
use tidewatch_core::worktree_cleanup::Report;

const MOST_LISTED: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Cleanup {
    #[default]
    Unchecked,
    Checking,
    Checked(Report),
    Failed(String),
}

impl Cleanup {
    pub(crate) fn wants_check(&self) -> bool {
        matches!(self, Self::Unchecked)
    }
}

/// Whether the agent of a merged tab still needs its worktree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Busy {
    pub(crate) working: bool,
    pub(crate) open_ledger: bool,
    pub(crate) job: bool,
}

impl Busy {
    pub(crate) fn of(status: &AgentStatus, ledger: Option<&Ledger>, job_running: bool) -> Self {
        let alive =
            !matches!(status, AgentStatus::Failed(_) | AgentStatus::Stopped | AgentStatus::Shell);
        Self {
            working: matches!(
                status,
                AgentStatus::Working
                    | AgentStatus::NeedsYou(
                        Reason::Permission | Reason::Question | Reason::Attention | Reason::Quiet
                    )
            ),
            open_ledger: alive && ledger.is_some_and(Ledger::is_open),
            job: alive && job_running,
        }
    }

    pub(crate) fn any(self) -> bool {
        self.working || self.open_ledger || self.job
    }

    fn reasons(self) -> Vec<String> {
        [
            (self.working, "the agent is still working"),
            (self.open_ledger, "its ledger still has open steps"),
            (self.job, "a job is still running"),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, words)| words.to_owned())
        .collect()
    }
}

fn joined(mut parts: Vec<String>) -> String {
    let last = parts.pop().unwrap_or_default();
    if parts.is_empty() { last } else { format!("{} and {last}", parts.join(", ")) }
}

/// Why a merged tab's worktree stays while its agent still needs it, as one line. `Some` exactly
/// when `busy` holds something.
pub(crate) fn wait_line(busy: Busy, cleanup: &Cleanup) -> Option<String> {
    if !busy.any() {
        return None;
    }
    let mut reasons = busy.reasons();
    if let Cleanup::Checked(report) = cleanup {
        let plural = |n: usize| if n == 1 { ("", "is") } else { ("s", "are") };
        if let Some(n) = report.local_commits.filter(|n| *n > 0) {
            let (s, is) = plural(n);
            reasons.push(format!("{n} commit{s} {is} not in the PR"));
        }
        let files = report.changed.len() + report.untracked.len();
        if files > 0 {
            let (s, _) = plural(files);
            reasons.push(format!("{files} file{s} not committed"));
        }
    }
    Some(format!("PR merged. Cleanup waits: {}.", joined(reasons)))
}

fn capped(items: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = items.iter().take(MOST_LISTED).cloned().collect();
    if items.len() > MOST_LISTED {
        lines.push(format!("and {} more", items.len() - MOST_LISTED));
    }
    lines
}

/// The ignored files the removal would lose, for the card's log box.
pub(crate) fn ignored_lines(report: &Report) -> Vec<String> {
    capped(&report.ignored)
}

/// Why the worktree is kept: each blocker in words, then the files behind it.
pub(crate) fn blocker_lines(report: &Report) -> Vec<String> {
    let mut lines: Vec<String> = report.blockers().iter().map(ToString::to_string).collect();
    let files: Vec<String> = report
        .changed
        .iter()
        .map(|path| format!("  modified  {path}"))
        .chain(report.untracked.iter().map(|path| format!("  untracked  {path}")))
        .collect();
    lines.extend(capped(&files));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report { local_commits: Some(0), ..Report::default() }
    }

    fn ledger(text: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\ng\n\n{text}"))
    }

    #[test]
    fn only_an_unchecked_card_starts_a_check() {
        assert!(Cleanup::Unchecked.wants_check());
        assert!(!Cleanup::Checking.wants_check());
        assert!(!Cleanup::Checked(report()).wants_check());
        assert!(!Cleanup::Failed("x".into()).wants_check());
    }

    #[test]
    fn long_lists_are_cut_with_a_count_of_the_rest() {
        let names: Vec<String> = (0..11).map(|n| format!("f{n}.log")).collect();
        let lines = ignored_lines(&Report { ignored: names, ..report() });
        assert_eq!(lines.len(), 9);
        assert_eq!(lines[7], "f7.log");
        assert_eq!(lines[8], "and 3 more");
        let few = ignored_lines(&Report { ignored: vec!["a.log".into()], ..report() });
        assert_eq!(few, ["a.log"]);
    }

    #[test]
    fn blockers_come_with_the_files_behind_them() {
        let blocked = Report {
            changed: vec!["src/lib.rs".into()],
            untracked: vec!["notes.md".into()],
            local_commits: Some(2),
            ..Report::default()
        };
        assert_eq!(
            blocker_lines(&blocked),
            [
                "1 file with uncommitted changes",
                "1 untracked file",
                "2 commits that the pull request does not have",
                "  modified  src/lib.rs",
                "  untracked  notes.md",
            ]
        );
        assert_eq!(blocker_lines(&report()), Vec::<String>::new());
    }

    #[test]
    fn the_agent_needs_its_worktree_while_it_works_has_open_steps_or_runs_a_job() {
        let open =
            ledger("## Status\nstate: working\n\n## Checklist\n### 1. P\n- [x] a\n- [ ] b\n");
        let closed = ledger("## Status\nstate: done\n\n## Checklist\n### 1. P\n- [x] a\n");
        let working = Busy { working: true, ..Busy::default() };
        let stuck = Busy { open_ledger: true, ..Busy::default() };
        let job = Busy { job: true, ..Busy::default() };
        let cases = [
            (AgentStatus::Working, None, false, working),
            (AgentStatus::NeedsYou(Reason::Permission), None, false, working),
            (AgentStatus::NeedsYou(Reason::Quiet), None, false, working),
            (AgentStatus::Done, Some(&open), false, stuck),
            (AgentStatus::NeedsYou(Reason::Input), Some(&open), false, stuck),
            (AgentStatus::Done, None, true, job),
            (AgentStatus::Done, Some(&closed), false, Busy::default()),
            (AgentStatus::NeedsYou(Reason::Input), None, false, Busy::default()),
            (AgentStatus::Failed("x".into()), Some(&open), true, Busy::default()),
            (AgentStatus::Stopped, Some(&open), true, Busy::default()),
            (AgentStatus::Shell, Some(&open), true, Busy::default()),
        ];
        for (status, ledger, running, want) in cases {
            assert_eq!(Busy::of(&status, ledger, running), want, "{status:?} {running}");
        }
        assert!(!Busy::default().any());
        assert!(working.any() && stuck.any() && job.any());
    }

    #[test]
    fn the_wait_line_names_every_reason_in_one_plain_sentence() {
        let all = Busy { working: true, open_ledger: true, job: true };
        let two_commits = Cleanup::Checked(Report { local_commits: Some(2), ..Report::default() });
        let dirty = Cleanup::Checked(Report {
            changed: vec!["a.rs".into()],
            untracked: vec!["b.md".into()],
            local_commits: Some(1),
            ..Report::default()
        });
        let working = Busy { working: true, ..Busy::default() };
        let cases = [
            (working, &Cleanup::Unchecked, "PR merged. Cleanup waits: the agent is still working."),
            (
                working,
                &two_commits,
                "PR merged. Cleanup waits: the agent is still working and 2 commits are not in the PR.",
            ),
            (
                Busy { open_ledger: true, ..Busy::default() },
                &Cleanup::Checking,
                "PR merged. Cleanup waits: its ledger still has open steps.",
            ),
            (
                Busy { job: true, ..Busy::default() },
                &Cleanup::Failed("x".into()),
                "PR merged. Cleanup waits: a job is still running.",
            ),
            (
                all,
                &dirty,
                "PR merged. Cleanup waits: the agent is still working, its ledger still has open steps, a job is still running, 1 commit is not in the PR and 2 files not committed.",
            ),
        ];
        for (busy, cleanup, want) in cases {
            assert_eq!(wait_line(busy, cleanup).as_deref(), Some(want));
        }
        assert_eq!(wait_line(Busy::default(), &two_commits), None);
    }
}
