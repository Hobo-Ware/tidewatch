//! What the window keeps for changing a tab: the job channel, the agents found in shell tabs, the
//! open popover (the tab and what is picked), the changes waiting for a turn to end, the tabs with
//! a change in flight and the dev switches.

use std::collections::{HashMap, HashSet};

use crate::change_tab::{Change, Pick};
use crate::jobs::Jobs;
use crate::retarget_jobs::Done;
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
}

impl DevOpen {
    pub(crate) fn parse(text: Option<&str>) -> Option<Self> {
        match text {
            Some("agent") => Some(Self::Agent),
            Some("shell") => Some(Self::Shell),
            Some("elsewhere") => Some(Self::Elsewhere),
            Some("elsewhere:change") => Some(Self::ElsewhereChange),
            _ => None,
        }
    }
}

pub(crate) struct Host {
    pub(crate) jobs: Jobs<Done>,
    pub(crate) agents: ShellAgents,
    pub(crate) open: Option<Open>,
    pub(crate) busy: HashSet<u64>,
    /// A change that waits for the agent's turn to end, with when to try again.
    pub(crate) waiting: HashMap<u64, (Change, f64)>,
    pub(crate) dev_open: Option<DevOpen>,
}

impl Host {
    pub(crate) fn from_env() -> Self {
        Self {
            jobs: Jobs::new(),
            agents: ShellAgents::new(Fake::from_env()),
            open: None,
            busy: HashSet::new(),
            waiting: HashMap::new(),
            dev_open: DevOpen::parse(std::env::var("TIDEWATCH_DEV_CHANGE_TAB").ok().as_deref()),
        }
    }
}
