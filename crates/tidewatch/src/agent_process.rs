//! A backup for hooks that never came: while an agent tab's turn is believed open, ask the process
//! list every two seconds whether the agent is still there. One `ps` answers for every tab, through
//! the `System` seam the take-over scan already uses, so a fake stands in for it in tests.

use tidewatch_core::elsewhere::{ProcessTable, parse_ps};
use tidewatch_core::elsewhere_scan::System;

pub(crate) const EVERY_MS: i64 = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hosting {
    /// The pty's own process is the agent's launcher: it is gone when the agent is.
    Launcher,
    /// The agent was typed at an interactive shell: the shell stays and the agent is its child.
    Shell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) tab: u64,
    pub(crate) root: u32,
    pub(crate) hosting: Hosting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    Alive,
    Gone,
}

/// `None` when `ps` failed, so a failed read is never taken for a dead agent.
pub(crate) fn probe(system: &dyn System, targets: &[Target]) -> Option<Vec<(Target, Presence)>> {
    let table = ProcessTable::new(parse_ps(&system.ps()?));
    Some(targets.iter().map(|target| (*target, presence(&table, *target))).collect())
}

fn presence(table: &ProcessTable, target: Target) -> Presence {
    let alive = table.get(target.root).is_some()
        && match target.hosting {
            Hosting::Launcher => true,
            Hosting::Shell => table
                .iter()
                .any(|row| row.pid != target.root && table.descends_from(row.pid, target.root)),
        };
    if alive { Presence::Alive } else { Presence::Gone }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Rest,
    Probe,
    Wait(i64),
}

/// When to probe: every `EVERY_MS` while some turn is believed open, one at a time, and not at all
/// while none is, so an idle window runs no timer for it.
#[derive(Debug, Default)]
pub(crate) struct Schedule {
    last_ms: Option<i64>,
    inflight: bool,
}

impl Schedule {
    pub(crate) fn step(&mut self, now_ms: i64, wanted: bool) -> Step {
        if !wanted {
            self.last_ms = None;
            return Step::Rest;
        }
        if self.inflight {
            return Step::Rest;
        }
        match self.last_ms {
            Some(last) if now_ms - last < EVERY_MS => Step::Wait(last + EVERY_MS - now_ms),
            _ => {
                self.last_ms = Some(now_ms);
                self.inflight = true;
                Step::Probe
            }
        }
    }

    pub(crate) fn finished(&mut self) {
        self.inflight = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct FakeSystem(Option<String>);

    impl System for FakeSystem {
        fn ps(&self) -> Option<String> {
            self.0.clone()
        }
        fn cwd_of(&self, _: u32) -> Option<PathBuf> {
            None
        }
        fn git_root(&self, _: &Path) -> Option<PathBuf> {
            None
        }
        fn cwd_holders(&self, _: &Path) -> Option<Vec<u32>> {
            None
        }
        fn terminate(&self, _: u32) -> bool {
            false
        }
        fn alive(&self, _: u32) -> Option<String> {
            None
        }
    }

    fn row(pid: u32, ppid: u32, args: &str) -> String {
        format!("{pid} {ppid} Sat Oct 31 11:00:00 2026 {args}\n")
    }

    fn list(rows: &[(u32, u32, &str)]) -> FakeSystem {
        FakeSystem(Some(rows.iter().map(|(pid, ppid, args)| row(*pid, *ppid, args)).collect()))
    }

    fn target(tab: u64, root: u32, hosting: Hosting) -> Target {
        Target { tab, root, hosting }
    }

    #[test]
    fn a_launcher_is_alive_while_its_process_is_listed_and_gone_when_it_is_not() {
        let system = list(&[(1, 0, "launchd"), (40, 1, "/bin/zsh -c claude"), (41, 40, "claude")]);
        let targets = [target(1, 40, Hosting::Launcher), target(2, 99, Hosting::Launcher)];
        let want = vec![(targets[0], Presence::Alive), (targets[1], Presence::Gone)];
        assert_eq!(probe(&system, &targets), Some(want));
    }

    #[test]
    fn an_agent_typed_at_a_shell_is_alive_only_while_the_shell_has_a_child() {
        let with_agent = list(&[(40, 1, "-zsh"), (41, 40, "claude --resume x"), (42, 41, "node")]);
        let at_prompt = list(&[(40, 1, "-zsh"), (50, 1, "other")]);
        let targets = [target(7, 40, Hosting::Shell)];
        assert_eq!(probe(&with_agent, &targets), Some(vec![(targets[0], Presence::Alive)]));
        assert_eq!(probe(&at_prompt, &targets), Some(vec![(targets[0], Presence::Gone)]));
    }

    #[test]
    fn a_process_list_that_cannot_be_read_says_nothing_instead_of_gone() {
        let targets = [target(1, 40, Hosting::Launcher)];
        assert_eq!(probe(&FakeSystem(None), &targets), None);
    }

    #[test]
    fn one_list_answers_for_every_tab() {
        let system = list(&[(10, 1, "zsh"), (11, 10, "codex"), (20, 1, "zsh"), (30, 1, "zsh")]);
        let targets = [
            target(1, 10, Hosting::Shell),
            target(2, 20, Hosting::Shell),
            target(3, 30, Hosting::Launcher),
        ];
        assert_eq!(
            probe(&system, &targets),
            Some(vec![
                (targets[0], Presence::Alive),
                (targets[1], Presence::Gone),
                (targets[2], Presence::Alive)
            ])
        );
    }

    #[test]
    fn the_schedule_probes_every_two_seconds_one_at_a_time_and_only_while_a_turn_is_open() {
        let mut schedule = Schedule::default();
        assert_eq!(schedule.step(0, false), Step::Rest);
        assert_eq!(schedule.step(10, true), Step::Probe);
        assert_eq!(schedule.step(11, true), Step::Rest);
        schedule.finished();
        assert_eq!(schedule.step(500, true), Step::Wait(1_510));
        assert_eq!(schedule.step(2_010, true), Step::Probe);
        schedule.finished();
        assert_eq!(schedule.step(2_500, false), Step::Rest);
        assert_eq!(schedule.step(2_600, true), Step::Probe);
    }
}
