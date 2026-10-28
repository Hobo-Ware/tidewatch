//! When one repo's `gh webhook forward` runs, as a pure state machine: events in, effects out. A repo
//! that cannot have a forwarder falls back to polling and tries again later; a crash retries after
//! 1 s, 2 s, 4 s ... and five within ten minutes fall back. The caller owns the clock and the process.

use serde_json::Value;

use crate::service_supervisor::backoff_secs;

pub const RETRY_TAKEN_SECS: f64 = 300.0;
pub const RETRY_CRASHED_SECS: f64 = 300.0;
pub const RETRY_LATER_SECS: f64 = 1800.0;
pub const GIVE_UP_EXITS: usize = 5;
pub const GIVE_UP_WINDOW_SECS: f64 = 600.0;
pub const STABLE_RUN_SECS: f64 = 120.0;
const FORWARDER_HOST: &str = "webhook-forwarder.github.com";

/// Why a repo is polled instead of fed live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    NotInstalled,
    InstallFailed,
    NoAdmin,
    HookTaken,
    NoGh,
    Crashed,
}

impl Why {
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::NotInstalled => "the gh webhook extension is not installed",
            Self::InstallFailed => "the gh webhook extension could not be installed",
            Self::NoAdmin => "it needs admin rights on the repository",
            Self::HookTaken => "another forwarder holds the repository's webhook",
            Self::NoGh => "gh is not installed",
            Self::Crashed => "the forwarder keeps stopping",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Stopped,
    Installing,
    Clearing,
    Starting,
    Live { since: f64 },
    Backoff { until: f64 },
    Fallback { why: Why, retry_at: f64 },
}

/// What a finished forwarder's error output says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitKind {
    NotInstalled,
    NoAdmin,
    HookTaken,
    NoGh,
    Other,
}

/// What looking for our own stale webhook came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cleared {
    Deleted,
    NotOurs,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Open { at: f64 },
    Close,
    Ping { hook_id: u64, at: f64 },
    Exited { kind: ExitKind, at: f64 },
    Installed { ok: bool, at: f64 },
    Cleared { outcome: Cleared, at: f64 },
    Tick { at: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Spawn,
    Kill,
    Install,
    ClearStale,
    Record { hook_id: u64 },
}

#[must_use]
pub fn classify(stderr: &str) -> ExitKind {
    let text = stderr.to_ascii_lowercase();
    if text.contains("unknown command \"webhook\"") {
        ExitKind::NotInstalled
    } else if text.contains("hook already exists") {
        ExitKind::HookTaken
    } else if text.contains("http 403")
        || text.contains("http 404")
        || text.contains("admin rights")
        || text.contains("not found")
    {
        ExitKind::NoAdmin
    } else {
        ExitKind::Other
    }
}

#[must_use]
pub fn forwarder_hook_ids(json: &str) -> Vec<u64> {
    let Ok(Value::Array(hooks)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    hooks
        .iter()
        .filter(|hook| {
            hook["name"].as_str() == Some("cli")
                && hook["config"]["url"].as_str().is_some_and(|url| url.contains(FORWARDER_HOST))
        })
        .filter_map(|hook| hook["id"].as_u64())
        .collect()
}

/// The hook to delete: the one we recorded, and only while it is still there.
#[must_use]
pub fn stale_to_delete(recorded: Option<u64>, existing: &[u64]) -> Option<u64> {
    recorded.filter(|id| existing.contains(id))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Forwarder {
    state: State,
    install_tried: bool,
    cleared: bool,
    exits: Vec<f64>,
    streak: u32,
}

impl Default for Forwarder {
    fn default() -> Self {
        Self::new()
    }
}

impl Forwarder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Stopped,
            install_tried: false,
            cleared: false,
            exits: Vec::new(),
            streak: 0,
        }
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// Whether events arrive from GitHub now.
    #[must_use]
    pub fn live(&self) -> bool {
        matches!(self.state, State::Live { .. })
    }

    #[must_use]
    pub fn why(&self) -> Option<Why> {
        match self.state {
            State::Fallback { why, .. } => Some(why),
            _ => None,
        }
    }

    /// Whether a process should exist.
    #[must_use]
    pub fn wants_process(&self) -> bool {
        matches!(self.state, State::Starting | State::Live { .. })
    }

    /// When a `Tick` next does something, if one is pending.
    #[must_use]
    pub fn next_wake(&self) -> Option<f64> {
        match self.state {
            State::Backoff { until, .. } => Some(until),
            State::Fallback { retry_at, .. } => Some(retry_at),
            _ => None,
        }
    }

    fn begin(&mut self) -> Vec<Effect> {
        self.state = State::Starting;
        vec![Effect::Spawn]
    }

    fn fall_back(&mut self, why: Why, at: f64, after: f64) -> Vec<Effect> {
        self.state = State::Fallback { why, retry_at: at + after };
        Vec::new()
    }

    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        match (event, self.state) {
            (Event::Open { .. }, State::Stopped) => {
                self.cleared = false;
                self.exits.clear();
                self.streak = 0;
                self.begin()
            }
            (Event::Close, State::Starting | State::Live { .. }) => {
                self.state = State::Stopped;
                vec![Effect::Kill]
            }
            (Event::Close, State::Stopped) => Vec::new(),
            (Event::Close, _) => {
                self.state = State::Stopped;
                Vec::new()
            }
            (Event::Ping { hook_id, at }, State::Starting) => {
                self.state = State::Live { since: at };
                vec![Effect::Record { hook_id }]
            }
            (Event::Ping { hook_id, .. }, State::Live { .. }) => vec![Effect::Record { hook_id }],
            (Event::Exited { kind, at }, State::Starting) => self.exited(kind, at, 0.0),
            (Event::Exited { kind, at }, State::Live { since }) => {
                self.exited(kind, at, at - since)
            }
            (Event::Installed { ok: false, at }, State::Installing) => {
                self.fall_back(Why::InstallFailed, at, RETRY_LATER_SECS)
            }
            (Event::Installed { ok: true, .. }, State::Installing)
            | (Event::Cleared { outcome: Cleared::Deleted, .. }, State::Clearing) => self.begin(),
            (Event::Cleared { at, .. }, State::Clearing) => {
                self.fall_back(Why::HookTaken, at, RETRY_TAKEN_SECS)
            }
            (Event::Tick { at }, State::Backoff { until }) if at >= until => self.begin(),
            (Event::Tick { at }, State::Fallback { retry_at, .. }) if at >= retry_at => {
                self.cleared = false;
                self.begin()
            }
            _ => Vec::new(),
        }
    }

    fn exited(&mut self, kind: ExitKind, at: f64, ran: f64) -> Vec<Effect> {
        match kind {
            ExitKind::NotInstalled if !self.install_tried => {
                self.install_tried = true;
                self.state = State::Installing;
                vec![Effect::Install]
            }
            ExitKind::NotInstalled => self.fall_back(Why::NotInstalled, at, RETRY_LATER_SECS),
            ExitKind::NoGh => self.fall_back(Why::NoGh, at, RETRY_LATER_SECS),
            ExitKind::NoAdmin => self.fall_back(Why::NoAdmin, at, RETRY_LATER_SECS),
            ExitKind::HookTaken if !self.cleared => {
                self.cleared = true;
                self.state = State::Clearing;
                vec![Effect::ClearStale]
            }
            ExitKind::HookTaken => self.fall_back(Why::HookTaken, at, RETRY_TAKEN_SECS),
            ExitKind::Other => self.crashed(at, ran),
        }
    }

    fn crashed(&mut self, at: f64, ran: f64) -> Vec<Effect> {
        if ran >= STABLE_RUN_SECS {
            self.exits.clear();
            self.streak = 0;
        }
        self.exits.push(at);
        self.exits.retain(|earlier| at - earlier <= GIVE_UP_WINDOW_SECS);
        self.streak += 1;
        if self.exits.len() >= GIVE_UP_EXITS {
            self.exits.clear();
            self.streak = 0;
            return self.fall_back(Why::Crashed, at, RETRY_CRASHED_SECS);
        }
        self.state = State::Backoff { until: at + backoff_secs(self.streak) };
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> Forwarder {
        let mut forwarder = Forwarder::new();
        forwarder.step(Event::Open { at: 0.0 });
        forwarder
    }

    fn live() -> Forwarder {
        let mut forwarder = started();
        forwarder.step(Event::Ping { hook_id: 7, at: 1.0 });
        forwarder
    }

    #[test]
    fn opening_spawns_and_the_first_ping_makes_it_live_and_records_the_hook() {
        let mut forwarder = Forwarder::new();
        assert_eq!(forwarder.step(Event::Open { at: 0.0 }), [Effect::Spawn]);
        assert_eq!(forwarder.state(), State::Starting);
        assert!(!forwarder.live());
        assert_eq!(
            forwarder.step(Event::Ping { hook_id: 630, at: 2.0 }),
            [Effect::Record { hook_id: 630 }]
        );
        assert_eq!(forwarder.state(), State::Live { since: 2.0 });
        assert!(forwarder.live() && forwarder.wants_process());
    }

    #[test]
    fn closing_kills_a_running_forwarder_and_only_that() {
        let cases: [(Forwarder, Vec<Effect>); 4] = [
            (started(), vec![Effect::Kill]),
            (live(), vec![Effect::Kill]),
            (Forwarder::new(), vec![]),
            (
                {
                    let mut f = started();
                    f.step(Event::Exited { kind: ExitKind::NoAdmin, at: 1.0 });
                    f
                },
                vec![],
            ),
        ];
        for (mut forwarder, want) in cases {
            assert_eq!(forwarder.step(Event::Close), want);
            assert_eq!(forwarder.state(), State::Stopped);
        }
    }

    #[test]
    fn a_forwarder_that_cannot_work_falls_back_with_its_reason_and_retries_later() {
        let cases = [
            (ExitKind::NoAdmin, Why::NoAdmin, RETRY_LATER_SECS),
            (ExitKind::NoGh, Why::NoGh, RETRY_LATER_SECS),
        ];
        for (kind, why, after) in cases {
            let mut forwarder = started();
            assert_eq!(forwarder.step(Event::Exited { kind, at: 10.0 }), []);
            assert_eq!(forwarder.state(), State::Fallback { why, retry_at: 10.0 + after });
            assert!(!forwarder.live() && !forwarder.wants_process());
            assert_eq!(forwarder.why(), Some(why));
            assert_eq!(forwarder.next_wake(), Some(10.0 + after));
            assert_eq!(forwarder.step(Event::Tick { at: 9.0 + after }), []);
            assert_eq!(forwarder.step(Event::Tick { at: 10.0 + after }), [Effect::Spawn]);
            assert_eq!(forwarder.state(), State::Starting);
        }
    }

    #[test]
    fn a_missing_extension_is_installed_once_and_then_started_again() {
        let mut forwarder = started();
        assert_eq!(
            forwarder.step(Event::Exited { kind: ExitKind::NotInstalled, at: 1.0 }),
            [Effect::Install]
        );
        assert_eq!(forwarder.state(), State::Installing);
        assert_eq!(forwarder.step(Event::Installed { ok: true, at: 9.0 }), [Effect::Spawn]);
        assert_eq!(forwarder.state(), State::Starting);
        assert_eq!(forwarder.step(Event::Exited { kind: ExitKind::NotInstalled, at: 10.0 }), []);
        assert_eq!(forwarder.why(), Some(Why::NotInstalled));
        forwarder.step(Event::Tick { at: 10.0 + RETRY_LATER_SECS });
        assert_eq!(
            forwarder.step(Event::Exited { kind: ExitKind::NotInstalled, at: 4000.0 }),
            [],
            "the install is never tried a second time"
        );
        assert_eq!(forwarder.why(), Some(Why::NotInstalled));
    }

    #[test]
    fn a_failed_install_falls_back_and_install_events_after_a_close_are_ignored() {
        let mut forwarder = started();
        forwarder.step(Event::Exited { kind: ExitKind::NotInstalled, at: 1.0 });
        assert_eq!(forwarder.step(Event::Installed { ok: false, at: 5.0 }), []);
        assert_eq!(
            forwarder.state(),
            State::Fallback { why: Why::InstallFailed, retry_at: 5.0 + RETRY_LATER_SECS }
        );
        let mut closed = started();
        closed.step(Event::Exited { kind: ExitKind::NotInstalled, at: 1.0 });
        closed.step(Event::Close);
        assert_eq!(closed.step(Event::Installed { ok: true, at: 5.0 }), []);
        assert_eq!(closed.state(), State::Stopped);
    }

    #[test]
    fn a_taken_hook_is_cleared_once_and_started_again_only_when_it_was_ours() {
        let mut forwarder = started();
        assert_eq!(
            forwarder.step(Event::Exited { kind: ExitKind::HookTaken, at: 1.0 }),
            [Effect::ClearStale]
        );
        assert_eq!(forwarder.state(), State::Clearing);
        assert_eq!(
            forwarder.step(Event::Cleared { outcome: Cleared::Deleted, at: 2.0 }),
            [Effect::Spawn]
        );
        assert_eq!(forwarder.step(Event::Exited { kind: ExitKind::HookTaken, at: 3.0 }), []);
        assert_eq!(
            forwarder.state(),
            State::Fallback { why: Why::HookTaken, retry_at: 3.0 + RETRY_TAKEN_SECS }
        );
    }

    #[test]
    fn a_hook_that_is_not_ours_or_cannot_be_deleted_falls_back_and_retries_in_minutes() {
        for outcome in [Cleared::NotOurs, Cleared::Failed] {
            let mut forwarder = started();
            forwarder.step(Event::Exited { kind: ExitKind::HookTaken, at: 1.0 });
            assert_eq!(forwarder.step(Event::Cleared { outcome, at: 2.0 }), []);
            assert_eq!(
                forwarder.state(),
                State::Fallback { why: Why::HookTaken, retry_at: 2.0 + RETRY_TAKEN_SECS }
            );
            assert_eq!(forwarder.step(Event::Tick { at: 2.0 + RETRY_TAKEN_SECS }), [Effect::Spawn]);
            assert_eq!(
                forwarder.step(Event::Exited { kind: ExitKind::HookTaken, at: 400.0 }),
                [Effect::ClearStale],
                "each retry may clear once more"
            );
        }
    }

    #[test]
    fn a_crash_retries_after_one_two_four_seconds_and_five_in_ten_minutes_fall_back() {
        let mut forwarder = started();
        for (n, wait) in [1.0, 2.0, 4.0, 8.0].into_iter().enumerate() {
            let at = 100.0 + n as f64 * 50.0;
            assert_eq!(forwarder.step(Event::Exited { kind: ExitKind::Other, at }), []);
            assert_eq!(forwarder.state(), State::Backoff { until: at + wait });
            assert_eq!(forwarder.step(Event::Tick { at: at + wait - 0.1 }), []);
            assert_eq!(forwarder.step(Event::Tick { at: at + wait }), [Effect::Spawn]);
        }
        assert_eq!(forwarder.step(Event::Exited { kind: ExitKind::Other, at: 300.0 }), []);
        assert_eq!(
            forwarder.state(),
            State::Fallback { why: Why::Crashed, retry_at: 300.0 + RETRY_CRASHED_SECS }
        );
    }

    #[test]
    fn a_long_live_run_forgives_the_crash_count() {
        let mut forwarder = started();
        for n in 0..4 {
            forwarder.step(Event::Exited { kind: ExitKind::Other, at: 10.0 + f64::from(n) });
            forwarder.step(Event::Tick { at: 1000.0 });
        }
        forwarder.step(Event::Ping { hook_id: 1, at: 20.0 });
        forwarder.step(Event::Exited { kind: ExitKind::Other, at: 20.0 + STABLE_RUN_SECS });
        assert_eq!(forwarder.state(), State::Backoff { until: 20.0 + STABLE_RUN_SECS + 1.0 });
    }

    #[test]
    fn events_that_do_not_fit_the_state_change_nothing() {
        let mut stopped = Forwarder::new();
        for event in [
            Event::Ping { hook_id: 1, at: 0.0 },
            Event::Exited { kind: ExitKind::Other, at: 0.0 },
            Event::Installed { ok: true, at: 0.0 },
            Event::Cleared { outcome: Cleared::Deleted, at: 0.0 },
            Event::Tick { at: 9e9 },
        ] {
            assert_eq!(stopped.step(event), []);
            assert_eq!(stopped.state(), State::Stopped);
        }
        let mut running = live();
        assert_eq!(running.step(Event::Open { at: 5.0 }), []);
        assert!(running.live());
    }

    #[test]
    fn stderr_is_sorted_by_what_gh_and_github_say() {
        let cases = [
            (
                "unknown command \"webhook\" for \"gh\"\n\nDid you mean this?",
                ExitKind::NotInstalled,
            ),
            (
                "error creating webhook: HTTP 422: Validation Failed (https://api.github.com/repos/o/n/hooks)\nHook already exists on this repository",
                ExitKind::HookTaken,
            ),
            ("HTTP 403: Must have admin rights to Repository.", ExitKind::NoAdmin),
            ("HTTP 404: Not Found (https://api.github.com/repos/o/n/hooks)", ExitKind::NoAdmin),
            ("websocket: close 1006 (abnormal closure)", ExitKind::Other),
            ("", ExitKind::Other),
        ];
        for (text, want) in cases {
            assert_eq!(classify(text), want, "{text}");
        }
    }

    #[test]
    fn only_the_forwarders_own_hooks_are_listed_and_only_a_recorded_one_is_deleted() {
        let json = r#"[
            {"id": 11, "name": "web", "config": {"url": "https://ci.example/hook"}},
            {"id": 12, "name": "cli", "config": {"url": "https://webhook-forwarder.github.com/hook"}},
            {"id": 13, "name": "cli", "config": {"url": "https://other.example/hook"}},
            {"name": "cli", "config": {"url": "https://webhook-forwarder.github.com/hook"}}
        ]"#;
        assert_eq!(forwarder_hook_ids(json), [12]);
        assert_eq!(forwarder_hook_ids("not json"), Vec::<u64>::new());
        assert_eq!(forwarder_hook_ids("{}"), Vec::<u64>::new());
        let cases = [
            (Some(12), &[12][..], Some(12)),
            (Some(12), &[14][..], None),
            (None, &[12][..], None),
            (Some(12), &[][..], None),
        ];
        for (recorded, existing, want) in cases {
            assert_eq!(stale_to_delete(recorded, existing), want);
        }
    }
}
