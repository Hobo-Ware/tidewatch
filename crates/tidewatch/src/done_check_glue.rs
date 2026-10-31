//! The app's side of the done check: one trigger and one card state per agent tab, the check as a
//! `Jobs` thread, and `TIDEWATCH_FAKE_DONE_CHECK=done|blocked|unverified|waiting|rechecked|rechecking|checking` for captures.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::civil_date::Date;
use tidewatch_core::done_check::{
    self, Input, Kind, Line, Outcome, PrFact, Report, Status, Unverified,
};
use tidewatch_core::status::AgentStatus;
use tidewatch_core::tools::Tools;

use crate::done_check_trigger::{Claim, Trigger};
use crate::jobs::Jobs;
use crate::progress_glue::today;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fake {
    Off,
    Done,
    Blocked,
    Unverified,
    Waiting,
    Rechecked,
    Rechecking,
    Checking,
}

impl Fake {
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("done") => Self::Done,
            Some("blocked") => Self::Blocked,
            Some("unverified") => Self::Unverified,
            Some("waiting") => Self::Waiting,
            Some("rechecked") => Self::Rechecked,
            Some("rechecking") => Self::Rechecking,
            Some("checking") => Self::Checking,
            _ => Self::Off,
        }
    }

    fn look(self) -> Option<Look> {
        let line = |kind, status, detail: &str| Line { kind, status, detail: detail.to_owned() };
        let report = |lines: Vec<Line>, unverified: &[&str]| Report {
            head: "3f9a1c2".to_owned(),
            lines,
            unverified: unverified.iter().map(|item| Unverified::new(*item)).collect(),
        };
        let committed = line(Kind::Committed, Status::Pass, "clean tree, 2 commits ahead of main");
        let pushed = line(Kind::Pushed, Status::Pass, "origin has 3f9a1c2");
        let same = self == Self::Rechecked;
        let checked = Checked { at: now_secs(), same };
        let found = match self {
            Self::Off => return None,
            Self::Checking => return Some(Look::Checking(None)),
            Self::Done => report(
                vec![
                    committed,
                    pushed,
                    line(Kind::PullRequest, Status::Pass, "PR #212 open, checks passing"),
                    line(Kind::Unverified, Status::Pass, "nothing left unverified"),
                ],
                &[],
            ),
            Self::Blocked => report(
                vec![
                    committed,
                    line(Kind::Pushed, Status::Fail, "2 commits not pushed"),
                    line(Kind::PullRequest, Status::Fail, "PR #212 checks failing: build, lint"),
                    line(Kind::Unverified, Status::Pass, "nothing left unverified"),
                ],
                &[],
            ),
            Self::Waiting => {
                let tomorrow = today().plus_days(1);
                let mut waiting = report(
                    vec![
                        line(Kind::Committed, Status::Pass, "nothing left uncommitted"),
                        line(Kind::Pushed, Status::Pass, "the pull request is merged"),
                        line(Kind::PullRequest, Status::Pass, "PR #2359 merged"),
                        line(Kind::Unverified, Status::Waiting, "1 item waiting"),
                    ],
                    &[],
                );
                waiting.unverified =
                    vec![Unverified::until("Live list across a day boundary", tomorrow)];
                waiting
            }
            Self::Unverified | Self::Rechecked | Self::Rechecking => report(
                vec![
                    committed,
                    pushed,
                    line(
                        Kind::PullRequest,
                        Status::Unknown,
                        "the GitHub CLI (gh) is not installed",
                    ),
                    line(Kind::Unverified, Status::Unknown, "2 items the agent did not verify"),
                ],
                &["the migration on staging", "the Safari layout"],
            ),
        };
        Some(if self == Self::Rechecking {
            Look::Checking(Some(found))
        } else {
            Look::Report(found, checked)
        })
    }
}

/// When a report was made, and whether it says what the one before it said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Checked {
    pub(crate) at: i64,
    pub(crate) same: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    /// A check is running; the report it replaces, if there was one, stays on the card.
    Checking(Option<Report>),
    Report(Report, Checked),
}

#[derive(Debug)]
enum Entry {
    Checking { job: u64, before: Option<Box<Report>> },
    Report { report: Box<Report>, checked: Checked },
}

fn before_of(entry: Option<Entry>) -> Option<Box<Report>> {
    match entry? {
        Entry::Report { report, .. } => Some(report),
        Entry::Checking { before, .. } => before,
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[derive(Debug, Default)]
struct Tab {
    trigger: Trigger,
    entry: Option<Entry>,
    /// The fix note a tab that runs on its own sent without a click, so the same failure is sent once.
    auto_sent: Option<String>,
}

pub(crate) struct Done {
    tab: u64,
    job: u64,
    report: Box<Report>,
    at: i64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub(crate) id: u64,
    pub(crate) status: &'a AgentStatus,
    pub(crate) eligible: bool,
    pub(crate) claim: Option<&'a Claim>,
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) dir: PathBuf,
    pub(crate) branch: String,
    pub(crate) pr: PrFact,
    pub(crate) unverified: Option<Vec<Unverified>>,
    pub(crate) today: Date,
}

pub(crate) struct Host {
    enabled: bool,
    fake: Fake,
    jobs: Jobs<Done>,
    tabs: HashMap<u64, Tab>,
    next_job: u64,
    finished: Vec<(u64, bool)>,
}

impl Host {
    pub(crate) fn new(enabled: bool, fake: Fake) -> Self {
        Self {
            enabled,
            fake,
            jobs: Jobs::new(),
            tabs: HashMap::new(),
            next_job: 0,
            finished: Vec::new(),
        }
    }

    pub(crate) fn take_finished(&mut self) -> Vec<(u64, bool)> {
        std::mem::take(&mut self.finished)
    }

    pub(crate) fn from_env(enabled: bool) -> Self {
        Self::new(enabled, Fake::parse(std::env::var("TIDEWATCH_FAKE_DONE_CHECK").ok().as_deref()))
    }

    pub(crate) fn is_fake(&self) -> bool {
        self.fake != Fake::Off
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            for tab in self.tabs.values_mut() {
                tab.entry = None;
            }
        }
    }

    pub(crate) fn tick(&mut self, facts: &[Facts<'_>]) -> Vec<u64> {
        for done in self.jobs.drain() {
            let Some(tab) = self.tabs.get_mut(&done.tab) else { continue };
            let Some(Entry::Checking { job, before }) = &tab.entry else { continue };
            if *job != done.job {
                continue;
            }
            let same = before.as_deref() == Some(&*done.report);
            self.finished.push((done.tab, done.report.outcome() == Outcome::Done));
            tab.entry =
                Some(Entry::Report { report: done.report, checked: Checked { at: done.at, same } });
        }
        let mut fire = Vec::new();
        for fact in facts {
            let tab = self.tabs.entry(fact.id).or_default();
            if !self.enabled || !fact.eligible {
                tab.entry = None;
                continue;
            }
            if *fact.status == AgentStatus::Working {
                tab.entry = None;
            }
            if tab.trigger.observe(fact.status, fact.claim.cloned()) {
                fire.push(fact.id);
            }
        }
        fire
    }

    pub(crate) fn start(&mut self, ctx: &egui::Context, tools: &Tools, tab: u64, request: Request) {
        self.next_job += 1;
        let job = self.next_job;
        let state = self.tabs.entry(tab).or_default();
        let before = before_of(state.entry.take());
        state.entry = Some(Entry::Checking { job, before });
        let tools = tools.clone();
        self.jobs.spawn(ctx, move || {
            let input = Input {
                tools: &tools,
                dir: &request.dir,
                branch: &request.branch,
                pr: request.pr,
                unverified: request.unverified,
                today: request.today,
            };
            let report = Box::new(done_check::run(&input));
            Done { tab, job, report, at: now_secs() }
        });
    }

    pub(crate) fn look(&self, tab: u64) -> Option<Look> {
        if self.fake != Fake::Off {
            return self.fake.look();
        }
        match self.tabs.get(&tab)?.entry.as_ref()? {
            Entry::Checking { before, .. } => Some(Look::Checking(before.as_deref().cloned())),
            Entry::Report { report, checked } => Some(Look::Report((**report).clone(), *checked)),
        }
    }

    /// The fix note a tab that runs on its own sends now: its check found something to fix, the
    /// agent may be asked (`blocked` is `ask_blocked`), and that same failure was not sent before.
    pub(crate) fn auto_fix(&mut self, tab: u64, blocked: Option<&str>) -> Option<String> {
        let tab = self.tabs.get_mut(&tab)?;
        let Some(Entry::Report { report, .. }) = &tab.entry else { return None };
        let Some(note) = report.fix_note() else {
            tab.auto_sent = None;
            return None;
        };
        if blocked.is_some() || tab.auto_sent.as_ref() == Some(&note) {
            return None;
        }
        tab.auto_sent = Some(note.clone());
        Some(note)
    }

    pub(crate) fn asked(&mut self, tab: u64) {
        if let Some(tab) = self.tabs.get_mut(&tab) {
            tab.trigger.forget_claim();
        }
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.tabs.remove(&tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::done_check_trigger::claim;
    use tidewatch_core::done_check::Outcome;

    const STAMP: Checked = Checked { at: 0, same: false };

    fn facts<'a>(status: &'a AgentStatus, claim: Option<&'a Claim>, eligible: bool) -> Facts<'a> {
        Facts { id: 1, status, eligible, claim }
    }

    #[test]
    fn a_turn_that_ends_with_a_claim_fires_once_and_a_new_turn_clears_the_card() {
        let mut host = Host::new(true, Fake::Off);
        let claim = claim(true, None);
        host.tick(&[facts(&AgentStatus::Working, None, true)]);
        assert_eq!(host.tick(&[facts(&AgentStatus::Done, claim.as_ref(), true)]), [1]);
        assert_eq!(
            host.tick(&[facts(&AgentStatus::Done, claim.as_ref(), true)]),
            Vec::<u64>::new()
        );
        host.tabs.get_mut(&1).unwrap().entry = Some(Entry::Checking { job: 1, before: None });
        assert_eq!(host.look(1), Some(Look::Checking(None)));
        host.tick(&[facts(&AgentStatus::Working, None, true)]);
        assert_eq!(host.look(1), None);
    }

    #[test]
    fn nothing_fires_while_off_or_for_a_tab_that_is_not_an_ordinary_agent() {
        let claim = claim(true, None);
        for (enabled, eligible) in [(false, true), (true, false)] {
            let mut host = Host::new(enabled, Fake::Off);
            host.tick(&[facts(&AgentStatus::Working, None, eligible)]);
            assert_eq!(
                host.tick(&[facts(&AgentStatus::Done, claim.as_ref(), eligible)]),
                Vec::<u64>::new()
            );
        }
    }

    fn blocked_report(detail: &str) -> Entry {
        let line = Line { kind: Kind::Pushed, status: Status::Fail, detail: detail.into() };
        Entry::Report {
            report: Box::new(Report {
                head: "abc".into(),
                lines: vec![line],
                unverified: Vec::new(),
            }),
            checked: STAMP,
        }
    }

    #[test]
    fn a_fixable_problem_is_sent_once_when_the_agent_may_be_asked_and_a_new_one_is_sent_again() {
        let mut host = Host::new(true, Fake::Off);
        assert_eq!(host.auto_fix(1, None), None, "no tab, nothing to send");
        host.tabs.entry(1).or_default().entry = Some(blocked_report("2 commits not pushed"));
        let busy = Some("The agent is busy. Ask when it is idle");
        assert_eq!(host.auto_fix(1, busy), None, "busy: wait, and do not spend the one send");
        let note = host.auto_fix(1, None).unwrap();
        assert_eq!(
            note,
            "The done check found: 2 commits not pushed. Please fix that and tell me when it is done."
        );
        assert_eq!(host.auto_fix(1, None), None, "the same failure goes once");
        host.tabs.get_mut(&1).unwrap().entry = Some(blocked_report("3 commits not pushed"));
        assert!(host.auto_fix(1, None).unwrap().contains("3 commits not pushed"));
    }

    #[test]
    fn nothing_is_sent_for_a_passing_or_unfinished_check() {
        let mut host = Host::new(true, Fake::Off);
        let tab = host.tabs.entry(1).or_default();
        tab.entry = Some(Entry::Checking { job: 1, before: None });
        assert_eq!(host.auto_fix(1, None), None);
        let passing = Report {
            head: "abc".into(),
            lines: vec![Line { kind: Kind::Pushed, status: Status::Pass, detail: "ok".into() }],
            unverified: Vec::new(),
        };
        host.tabs.get_mut(&1).unwrap().entry =
            Some(Entry::Report { report: Box::new(passing), checked: STAMP });
        assert_eq!(host.auto_fix(1, None), None);
        host.tabs.get_mut(&1).unwrap().entry = Some(blocked_report("2 commits not pushed"));
        assert!(host.auto_fix(1, None).is_some());
        host.tabs.get_mut(&1).unwrap().entry = Some(Entry::Report {
            report: Box::new(Report {
                head: "abc".into(),
                lines: Vec::new(),
                unverified: Vec::new(),
            }),
            checked: STAMP,
        });
        assert_eq!(host.auto_fix(1, None), None);
        host.tabs.get_mut(&1).unwrap().entry = Some(blocked_report("2 commits not pushed"));
        assert!(host.auto_fix(1, None).is_some(), "fixed in between, so a return is new");
    }

    #[test]
    fn switching_it_off_drops_every_card() {
        let mut host = Host::new(true, Fake::Off);
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking { job: 1, before: None });
        host.set_enabled(false);
        assert_eq!(host.look(1), None);
    }

    #[test]
    fn a_late_answer_for_an_old_check_is_dropped() {
        let mut host = Host::new(true, Fake::Off);
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking { job: 2, before: None });
        let report =
            Box::new(Report { head: String::new(), lines: Vec::new(), unverified: Vec::new() });
        host.jobs.spawn(&egui::Context::default(), move || Done { tab: 1, job: 1, report, at: 0 });
        std::thread::sleep(std::time::Duration::from_millis(100));
        host.tick(&[]);
        assert_eq!(host.look(1), Some(Look::Checking(None)));
    }

    #[test]
    fn the_fake_shows_each_card_state_on_any_tab() {
        let outcomes: Vec<_> =
            [Fake::Done, Fake::Blocked, Fake::Unverified, Fake::Waiting, Fake::Rechecked]
                .into_iter()
                .map(|fake| match Host::new(true, fake).look(9) {
                    Some(Look::Report(report, _)) => report.outcome(),
                    other => panic!("{other:?}"),
                })
                .collect();
        let (done, blocked, unverified) = (Outcome::Done, Outcome::Blocked, Outcome::NotVerified);
        assert_eq!(outcomes, [done, blocked, unverified, Outcome::Waiting, unverified]);
        assert_eq!(Host::new(true, Fake::Checking).look(9), Some(Look::Checking(None)));
        assert_eq!(Host::new(true, Fake::Off).look(9), None);
    }

    fn answer(host: &mut Host, job: u64, report: Report, at: i64) {
        host.jobs.spawn(&egui::Context::default(), move || Done {
            tab: 1,
            job,
            report: Box::new(report),
            at,
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        host.tick(&[]);
    }

    fn report_saying(detail: &str) -> Report {
        let line = Line { kind: Kind::Unverified, status: Status::Unknown, detail: detail.into() };
        Report { head: "abc".into(), lines: vec![line], unverified: Vec::new() }
    }

    #[test]
    fn a_check_again_keeps_the_old_card_while_it_runs_and_stamps_the_answer() {
        let mut host = Host::new(true, Fake::Off);
        let first = report_saying("1 item the agent did not verify");
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking { job: 1, before: None });
        answer(&mut host, 1, first.clone(), 100);
        assert_eq!(
            host.look(1),
            Some(Look::Report(first.clone(), Checked { at: 100, same: false })),
            "the first answer has nothing to be the same as"
        );
        let again = host.tabs.get_mut(&1).unwrap();
        let before = before_of(again.entry.take());
        again.entry = Some(Entry::Checking { job: 2, before });
        assert_eq!(host.look(1), Some(Look::Checking(Some(first.clone()))));
        answer(&mut host, 2, first.clone(), 160);
        assert_eq!(host.look(1), Some(Look::Report(first, Checked { at: 160, same: true })));
    }

    #[test]
    fn a_changed_answer_is_not_the_same_result_and_a_new_turn_forgets_both() {
        let mut host = Host::new(true, Fake::Off);
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking {
            job: 3,
            before: Some(Box::new(report_saying("2 items the agent did not verify"))),
        });
        let next = report_saying("1 item the agent did not verify");
        answer(&mut host, 3, next.clone(), 200);
        assert_eq!(host.look(1), Some(Look::Report(next, Checked { at: 200, same: false })));
        host.tick(&[facts(&AgentStatus::Working, None, true)]);
        assert_eq!(host.look(1), None);
    }

    #[test]
    fn a_check_that_ended_is_told_with_whether_it_found_nothing_wrong_and_a_dropped_one_is_not() {
        let mut host = Host::new(true, Fake::Off);
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking { job: 1, before: None });
        answer(&mut host, 1, report_saying("1 item the agent did not verify"), 100);
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking { job: 2, before: None });
        let clean = Report { head: "abc".into(), lines: Vec::new(), unverified: Vec::new() };
        answer(&mut host, 2, clean, 200);
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking { job: 9, before: None });
        answer(&mut host, 3, report_saying("late"), 300);
        assert_eq!(host.take_finished(), [(1, false), (1, true)]);
        assert_eq!(host.take_finished(), Vec::<(u64, bool)>::new());
    }
}
