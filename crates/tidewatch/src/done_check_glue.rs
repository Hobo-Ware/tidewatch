//! The app's side of the done check: one trigger and one card state per agent tab, the check as a
//! `Jobs` thread, and `TIDEWATCH_FAKE_DONE_CHECK=done|blocked|unverified|checking` for captures.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::done_check::{self, Input, Kind, Line, PrFact, Report, Status};
use tidewatch_core::status::AgentStatus;
use tidewatch_core::tools::Tools;

use crate::done_check_trigger::{Claim, Trigger};
use crate::jobs::Jobs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fake {
    Off,
    Done,
    Blocked,
    Unverified,
    Checking,
}

impl Fake {
    pub(crate) fn parse(value: Option<&str>) -> Self {
        match value {
            Some("done") => Self::Done,
            Some("blocked") => Self::Blocked,
            Some("unverified") => Self::Unverified,
            Some("checking") => Self::Checking,
            _ => Self::Off,
        }
    }

    fn look(self) -> Option<Look> {
        let line = |kind, status, detail: &str| Line { kind, status, detail: detail.to_owned() };
        let report = |lines: Vec<Line>, unverified: &[&str]| Report {
            head: "3f9a1c2".to_owned(),
            lines,
            unverified: unverified.iter().map(|item| (*item).to_owned()).collect(),
        };
        let committed = line(Kind::Committed, Status::Pass, "clean tree, 2 commits ahead of main");
        let pushed = line(Kind::Pushed, Status::Pass, "origin has 3f9a1c2");
        Some(Look::Report(match self {
            Self::Off => return None,
            Self::Checking => return Some(Look::Checking),
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
            Self::Unverified => report(
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
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    Checking,
    Report(Report),
}

#[derive(Debug)]
enum Entry {
    Checking { job: u64 },
    Report(Box<Report>),
}

#[derive(Debug, Default)]
struct Tab {
    trigger: Trigger,
    entry: Option<Entry>,
}

pub(crate) struct Done {
    tab: u64,
    job: u64,
    report: Box<Report>,
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
    pub(crate) unverified: Option<Vec<String>>,
}

pub(crate) struct Host {
    enabled: bool,
    fake: Fake,
    jobs: Jobs<Done>,
    tabs: HashMap<u64, Tab>,
    next_job: u64,
}

impl Host {
    pub(crate) fn new(enabled: bool, fake: Fake) -> Self {
        Self { enabled, fake, jobs: Jobs::new(), tabs: HashMap::new(), next_job: 0 }
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
            let waiting = self.tabs.get_mut(&done.tab).filter(
                |tab| matches!(tab.entry, Some(Entry::Checking { job }) if job == done.job),
            );
            if let Some(tab) = waiting {
                tab.entry = Some(Entry::Report(done.report));
            }
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
        self.tabs.entry(tab).or_default().entry = Some(Entry::Checking { job });
        let tools = tools.clone();
        self.jobs.spawn(ctx, move || {
            let input = Input {
                tools: &tools,
                dir: &request.dir,
                branch: &request.branch,
                pr: request.pr,
                unverified: request.unverified,
            };
            Done { tab, job, report: Box::new(done_check::run(&input)) }
        });
    }

    pub(crate) fn look(&self, tab: u64) -> Option<Look> {
        if self.fake != Fake::Off {
            return self.fake.look();
        }
        match self.tabs.get(&tab)?.entry.as_ref()? {
            Entry::Checking { .. } => Some(Look::Checking),
            Entry::Report(report) => Some(Look::Report((**report).clone())),
        }
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
        host.tabs.get_mut(&1).unwrap().entry = Some(Entry::Checking { job: 1 });
        assert_eq!(host.look(1), Some(Look::Checking));
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

    #[test]
    fn switching_it_off_drops_every_card() {
        let mut host = Host::new(true, Fake::Off);
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking { job: 1 });
        host.set_enabled(false);
        assert_eq!(host.look(1), None);
    }

    #[test]
    fn a_late_answer_for_an_old_check_is_dropped() {
        let mut host = Host::new(true, Fake::Off);
        host.tabs.entry(1).or_default().entry = Some(Entry::Checking { job: 2 });
        let report =
            Box::new(Report { head: String::new(), lines: Vec::new(), unverified: Vec::new() });
        host.jobs.spawn(&egui::Context::default(), move || Done { tab: 1, job: 1, report });
        std::thread::sleep(std::time::Duration::from_millis(100));
        host.tick(&[]);
        assert_eq!(host.look(1), Some(Look::Checking));
    }

    #[test]
    fn the_fake_shows_each_card_state_on_any_tab() {
        let outcomes: Vec<_> = [Fake::Done, Fake::Blocked, Fake::Unverified]
            .into_iter()
            .map(|fake| match Host::new(true, fake).look(9) {
                Some(Look::Report(report)) => report.outcome(),
                other => panic!("{other:?}"),
            })
            .collect();
        let (done, blocked, unverified) = (Outcome::Done, Outcome::Blocked, Outcome::NotVerified);
        assert_eq!(outcomes, [done, blocked, unverified]);
        assert_eq!(Host::new(true, Fake::Checking).look(9), Some(Look::Checking));
        assert_eq!(Host::new(true, Fake::Off).look(9), None);
    }
}
