//! What the window keeps for changing a tab: the job channel, the agents found in shell tabs, the
//! open popover (the tab and what is picked), the changes waiting for a turn to end, the tabs with
//! a change in flight and the dev switches.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::change_tab::{Change, Pick};
use crate::jobs::Jobs;
use crate::retarget_jobs::{Done, Looked};
use crate::shell_agents::{Fake, ShellAgents};

pub(crate) struct Open {
    pub(crate) tab: u64,
    pub(crate) pick: Pick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevOpen {
    Agent,
    Shell,
    /// A shell tab whose found session runs in another folder: open the popover.
    Elsewhere,
    /// The same, then press Change.
    ElsewhereChange,
    /// The first shell tab, the pinned main tab when it is the only one, with the stand-in session
    /// (`TIDEWATCH_FAKE_SHELL_AGENT`): open the popover.
    Home,
    /// The same, with the stand-in session as one the tab ended, then press Change.
    HomeChange,
    /// The first tab that is not the main tab, with a change to a shell held for its turn to end,
    /// so the status bar shows the wait and its Cancel. Nothing is ever started.
    Waiting,
}

impl DevOpen {
    pub(crate) fn parse(text: Option<&str>) -> Option<Self> {
        match text {
            Some("agent") => Some(Self::Agent),
            Some("shell") => Some(Self::Shell),
            Some("elsewhere") => Some(Self::Elsewhere),
            Some("elsewhere:change") => Some(Self::ElsewhereChange),
            Some("home") => Some(Self::Home),
            Some("home:change") => Some(Self::HomeChange),
            Some("waiting") => Some(Self::Waiting),
            _ => None,
        }
    }
}

/// How long a change may wait for the agent's turn to end before it is dropped.
pub(crate) const MAX_WAIT_S: f64 = Duration::from_mins(60).as_secs_f64();

struct Held {
    change: Change,
    retry_at: f64,
    since: f64,
}

/// The changes waiting for a turn to end, each with when to try again and since when it waits.
#[derive(Default)]
pub(crate) struct Waits(HashMap<u64, Held>);

impl Waits {
    /// Notes a change as under way or held. A change already waiting keeps its first time.
    pub(crate) fn hold(&mut self, change: Change, retry_at: f64, now: f64) {
        let since = self.0.get(&change.tab).map_or(now, |held| held.since);
        self.0.insert(change.tab, Held { change, retry_at, since });
    }

    pub(crate) fn retry_at(&mut self, tab: u64, at: f64) {
        if let Some(held) = self.0.get_mut(&tab) {
            held.retry_at = at;
        }
    }

    pub(crate) fn remove(&mut self, tab: u64) -> bool {
        self.0.remove(&tab).is_some()
    }

    pub(crate) fn due(&self, now: f64, busy: &HashSet<u64>) -> Vec<Change> {
        self.0
            .values()
            .filter(|held| held.retry_at <= now && !busy.contains(&held.change.tab))
            .map(|held| held.change.clone())
            .collect()
    }

    pub(crate) fn next_retry(&self) -> Option<f64> {
        self.0.values().map(|held| held.retry_at).reduce(f64::min)
    }

    /// Drops the waits that went past `MAX_WAIT_S` and returns their tabs. A change whose job is
    /// running is left to that job.
    pub(crate) fn take_expired(&mut self, now: f64, busy: &HashSet<u64>) -> Vec<u64> {
        let mut gone: Vec<u64> = self
            .0
            .values()
            .filter(|held| now - held.since >= MAX_WAIT_S && !busy.contains(&held.change.tab))
            .map(|held| held.change.tab)
            .collect();
        gone.sort_unstable();
        for tab in &gone {
            self.0.remove(tab);
        }
        gone
    }

    /// The tabs whose change waits for a turn to end, oldest wait first.
    pub(crate) fn held(&self, busy: &HashSet<u64>) -> Vec<u64> {
        let mut held: Vec<&Held> =
            self.0.values().filter(|held| !busy.contains(&held.change.tab)).collect();
        held.sort_by(|a, b| a.since.total_cmp(&b.since).then(a.change.tab.cmp(&b.change.tab)));
        held.into_iter().map(|held| held.change.tab).collect()
    }
}

pub(crate) struct Host {
    pub(crate) jobs: Jobs<Done>,
    pub(crate) agents: ShellAgents,
    pub(crate) open: Option<Open>,
    pub(crate) busy: HashSet<u64>,
    pub(crate) waiting: Waits,
    pub(crate) dev_open: Option<DevOpen>,
    /// Where a change would put each tab, looked up while its popover is open.
    pub(crate) looks: HashMap<u64, Looked>,
    pub(crate) looking: HashSet<u64>,
}

impl Host {
    pub(crate) fn from_env() -> Self {
        Self {
            jobs: Jobs::new(),
            agents: ShellAgents::new(Fake::from_env()),
            open: None,
            busy: HashSet::new(),
            waiting: Waits::default(),
            looks: HashMap::new(),
            looking: HashSet::new(),
            dev_open: DevOpen::parse(std::env::var("TIDEWATCH_DEV_CHANGE_TAB").ok().as_deref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retarget::Target;

    fn change(tab: u64) -> Change {
        Change { tab, target: Target::Shell, interrupt: false }
    }

    fn none() -> HashSet<u64> {
        HashSet::new()
    }

    #[test]
    fn a_wait_expires_after_an_hour_and_not_before() {
        let cases = [
            (0.0, false),
            (MAX_WAIT_S - 1.0, false),
            (MAX_WAIT_S, true),
            (MAX_WAIT_S + 500.0, true),
        ];
        for (now, gone) in cases {
            let mut waits = Waits::default();
            waits.hold(change(1), 5.0, 0.0);
            assert_eq!(
                waits.take_expired(now, &none()),
                if gone { vec![1] } else { vec![] },
                "{now}"
            );
            assert_eq!(waits.held(&none()).contains(&1), !gone, "{now}");
        }
    }

    #[test]
    fn holding_again_keeps_the_first_time_so_retries_do_not_extend_the_wait() {
        let mut waits = Waits::default();
        waits.hold(change(1), 5.0, 100.0);
        waits.hold(change(1), 9.0, 3000.0);
        assert_eq!(waits.take_expired(100.0 + MAX_WAIT_S, &none()), vec![1]);
    }

    #[test]
    fn a_change_whose_job_runs_is_neither_expired_nor_shown_as_waiting() {
        let mut waits = Waits::default();
        waits.hold(change(1), 5.0, 0.0);
        waits.hold(change(2), 5.0, 1.0);
        let busy = HashSet::from([1]);
        assert_eq!(waits.take_expired(MAX_WAIT_S + 10.0, &busy), vec![2]);
        assert_eq!(waits.held(&busy), Vec::<u64>::new());
        assert_eq!(waits.held(&none()), vec![1]);
    }

    #[test]
    fn the_oldest_wait_comes_first_and_cancel_drops_it() {
        let mut waits = Waits::default();
        waits.hold(change(3), 5.0, 20.0);
        waits.hold(change(1), 5.0, 10.0);
        assert_eq!(waits.held(&none()), vec![1, 3]);
        assert!(waits.remove(1));
        assert!(!waits.remove(1));
        assert_eq!(waits.held(&none()), vec![3]);
    }

    #[test]
    fn retries_come_due_by_their_own_time() {
        let mut waits = Waits::default();
        waits.hold(change(1), 5.0, 0.0);
        waits.hold(change(2), 9.0, 0.0);
        assert_eq!(waits.due(6.0, &none()).len(), 1);
        assert_eq!(waits.next_retry(), Some(5.0));
        waits.retry_at(1, 20.0);
        assert_eq!(waits.next_retry(), Some(9.0));
    }
}
