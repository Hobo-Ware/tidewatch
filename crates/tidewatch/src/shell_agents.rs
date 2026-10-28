//! The agents found inside the shell tabs of the window: what the last scan saw in each tab, when
//! to look again, the session a tab ended on request (so the picker can switch back), and the
//! stand-in `TIDEWATCH_FAKE_SHELL_AGENT` reports for captures. Pure; the scan is `retarget_jobs`.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::agents::Level;
use tidewatch_core::elsewhere::{Activity, Cli, Session};

pub(crate) const EVERY_S: f64 = 4.0;
pub(crate) const FAKE_SESSION: &str = "5b0c6bd2-1f4e-4a37-9a5c-0d2f8e7a1c43";

/// A tab whose shell pid the scan looks under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    pub(crate) tab: u64,
    pub(crate) shell_pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Detected {
    pub(crate) cli: Cli,
    pub(crate) session_id: Option<String>,
    /// The level the session last ran at.
    pub(crate) level: Option<Level>,
    pub(crate) working: bool,
    pub(crate) cwd: PathBuf,
}

impl Detected {
    pub(crate) fn of(session: &Session) -> Option<Self> {
        let running = session.running()?;
        Some(Self {
            cli: session.cli,
            session_id: session.session_id.clone(),
            level: session.flags.level,
            working: running.activity == Activity::Working,
            cwd: session.cwd.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Remembered {
    pub(crate) cli: Cli,
    pub(crate) session_id: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Fake {
    #[default]
    Off,
    Idle,
    Working,
}

impl Fake {
    pub(crate) fn parse(text: Option<&str>) -> Self {
        match text {
            Some("working") => Self::Working,
            Some("1" | "idle" | "on") => Self::Idle,
            _ => Self::Off,
        }
    }

    pub(crate) fn from_env() -> Self {
        Self::parse(std::env::var("TIDEWATCH_FAKE_SHELL_AGENT").ok().as_deref())
    }

    fn detected(self) -> Option<Detected> {
        (self != Self::Off).then(|| Detected {
            cli: Cli::Claude,
            session_id: Some(FAKE_SESSION.to_owned()),
            level: Some(Level::Auto),
            working: self == Self::Working,
            cwd: std::env::var_os("TIDEWATCH_FAKE_SHELL_AGENT_CWD")
                .map(PathBuf::from)
                .unwrap_or_default(),
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct ShellAgents {
    found: HashMap<u64, Detected>,
    remembered: HashMap<u64, Remembered>,
    in_flight: bool,
    last: Option<f64>,
    fake: Fake,
}

impl ShellAgents {
    pub(crate) fn new(fake: Fake) -> Self {
        Self { fake, ..Self::default() }
    }

    /// Whether a scan should start now: one at a time, one every `EVERY_S`, only while the window
    /// shows and there is a shell tab to look in.
    pub(crate) fn begin_scan(&mut self, now: f64, visible: bool, shells: usize) -> bool {
        let due = self.next_in(now, visible, shells).is_some_and(|wait| wait <= 0.0);
        if due {
            self.in_flight = true;
            self.last = Some(now);
        }
        due
    }

    /// Seconds until the next scan is due, `None` when none is wanted.
    pub(crate) fn next_in(&self, now: f64, visible: bool, shells: usize) -> Option<f64> {
        if !visible || shells == 0 || self.in_flight || self.fake != Fake::Off {
            return None;
        }
        Some(self.last.map_or(0.0, |last| (last + EVERY_S - now).max(0.0)))
    }

    pub(crate) fn finish(&mut self, found: Vec<(u64, Detected)>) {
        self.in_flight = false;
        self.found = found.into_iter().collect();
    }

    pub(crate) fn failed(&mut self) {
        self.in_flight = false;
    }

    /// With the stand-in on, the first shell tab reports a found session.
    pub(crate) fn note_fake(&mut self, first_shell: Option<u64>) {
        self.found.clear();
        if let (Some(tab), Some(detected)) = (first_shell, self.fake.detected()) {
            self.found.insert(tab, detected);
        }
    }

    pub(crate) fn fake(&self) -> Fake {
        self.fake
    }

    pub(crate) fn found(&self, tab: u64) -> Option<&Detected> {
        self.found.get(&tab)
    }

    pub(crate) fn remember(&mut self, tab: u64, remembered: Remembered) {
        self.found.remove(&tab);
        self.remembered.insert(tab, remembered);
    }

    pub(crate) fn remembered(&self, tab: u64) -> Option<&Remembered> {
        self.remembered.get(&tab)
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.found.remove(&tab);
        self.remembered.remove(&tab);
    }
}

#[cfg(test)]
mod tests;
