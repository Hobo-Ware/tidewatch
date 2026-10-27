//! When one repo service runs, as a pure state machine: events in, effects out. A service that
//! exits is started again after 1 s, 2 s, 4 s ... up to 60 s; five exits within ten minutes make
//! it give up (`Crashed`), and a run of two minutes forgives the count. A stop by hand never
//! restarts. The caller owns the clock (seconds on any steady scale) and the process.

use serde::{Deserialize, Serialize};

pub const BACKOFF_CAP_SECS: f64 = 60.0;
pub const GIVE_UP_EXITS: usize = 5;
pub const GIVE_UP_WINDOW_SECS: f64 = 600.0;
pub const STABLE_RUN_SECS: f64 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Stopped,
    Starting,
    Running { since: f64 },
    Backoff { attempt: u32, until: f64 },
    Crashed { exit: i32, at: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Start,
    Restart,
    Stop,
    Spawned { at: f64 },
    Exited { code: i32, at: f64 },
    Tick { at: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    Spawn,
    Kill,
    Wake { at: f64 },
}

/// The one word the owner and the agents read for a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    Running,
    Starting,
    Restarting,
    Crashed,
    Stopped,
}

impl Word {
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Starting => "starting",
            Self::Restarting => "restarting",
            Self::Crashed => "crashed",
            Self::Stopped => "stopped",
        }
    }
}

/// Seconds to wait after the `attempt`th exit in a row: 1, 2, 4, 8 ... never past the cap.
#[must_use]
pub fn backoff_secs(attempt: u32) -> f64 {
    let doublings = attempt.saturating_sub(1).min(16);
    (f64::from(1u32 << doublings)).min(BACKOFF_CAP_SECS)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Supervisor {
    state: State,
    exits: Vec<f64>,
    streak: u32,
    last_exit: Option<i32>,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    #[must_use]
    pub fn new() -> Self {
        Self { state: State::Stopped, exits: Vec::new(), streak: 0, last_exit: None }
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    #[must_use]
    pub fn last_exit(&self) -> Option<i32> {
        self.last_exit
    }

    #[must_use]
    pub fn word(&self) -> Word {
        match self.state {
            State::Running { .. } => Word::Running,
            State::Starting => Word::Starting,
            State::Backoff { .. } => Word::Restarting,
            State::Crashed { .. } => Word::Crashed,
            State::Stopped => Word::Stopped,
        }
    }

    /// Whether a process should exist.
    #[must_use]
    pub fn wants_process(&self) -> bool {
        matches!(self.state, State::Starting | State::Running { .. })
    }

    /// When a `Tick` next does something, if one is pending.
    #[must_use]
    pub fn next_wake(&self) -> Option<f64> {
        match self.state {
            State::Backoff { until, .. } => Some(until),
            _ => None,
        }
    }

    fn begin(&mut self) -> Vec<Effect> {
        self.exits.clear();
        self.streak = 0;
        self.state = State::Starting;
        vec![Effect::Spawn]
    }

    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        match (event, self.state) {
            (
                Event::Start | Event::Restart,
                State::Stopped | State::Crashed { .. } | State::Backoff { .. },
            ) => self.begin(),
            (Event::Restart, State::Starting | State::Running { .. }) => {
                self.begin();
                vec![Effect::Kill, Effect::Spawn]
            }
            (Event::Stop, State::Starting | State::Running { .. }) => {
                self.state = State::Stopped;
                vec![Effect::Kill]
            }
            (Event::Stop, State::Backoff { .. } | State::Crashed { .. }) => {
                self.state = State::Stopped;
                Vec::new()
            }
            (Event::Spawned { at }, State::Starting) => {
                self.state = State::Running { since: at };
                Vec::new()
            }
            (Event::Exited { code, .. }, State::Stopped | State::Crashed { .. }) => {
                self.last_exit = Some(code);
                Vec::new()
            }
            (Event::Exited { code, at }, State::Starting) => self.exited(code, at, 0.0),
            (Event::Exited { code, at }, State::Running { since }) => {
                self.exited(code, at, at - since)
            }
            (Event::Tick { at }, State::Backoff { until, .. }) if at >= until => self.begin_again(),
            _ => Vec::new(),
        }
    }

    fn begin_again(&mut self) -> Vec<Effect> {
        self.state = State::Starting;
        vec![Effect::Spawn]
    }

    fn exited(&mut self, code: i32, at: f64, ran: f64) -> Vec<Effect> {
        self.last_exit = Some(code);
        if ran >= STABLE_RUN_SECS {
            self.exits.clear();
            self.streak = 0;
        }
        self.exits.push(at);
        self.exits.retain(|earlier| at - earlier <= GIVE_UP_WINDOW_SECS);
        self.streak += 1;
        if self.exits.len() >= GIVE_UP_EXITS {
            self.state = State::Crashed { exit: code, at };
            return Vec::new();
        }
        let until = at + backoff_secs(self.streak);
        self.state = State::Backoff { attempt: self.streak, until };
        vec![Effect::Wake { at: until }]
    }

    /// The owner-facing line under the name: `up 2h 14m · :3309`, `restarting in 4s`,
    /// `crashed, exit 1`, `stopped, exit 1`.
    #[must_use]
    pub fn line(&self, now: f64, port: Option<u16>) -> String {
        match self.state {
            State::Running { since } => {
                let up = format!("up {}", duration_text((now - since).max(0.0)));
                port.map_or(up.clone(), |port| format!("{up} \u{b7} :{port}"))
            }
            State::Starting => "starting".to_owned(),
            State::Backoff { until, .. } => {
                format!("restarting in {}", duration_text((until - now).max(0.0).ceil()))
            }
            State::Crashed { exit, .. } => format!("crashed, exit {exit}"),
            State::Stopped => self
                .last_exit
                .map_or_else(|| "stopped".to_owned(), |code| format!("stopped, exit {code}")),
        }
    }

    #[must_use]
    pub fn uptime(&self, now: f64) -> Option<f64> {
        match self.state {
            State::Running { since } => Some((now - since).max(0.0)),
            _ => None,
        }
    }

    #[must_use]
    pub fn carry(&self, now: f64) -> Carry {
        let phase = match self.state {
            State::Stopped => Phase::Stopped,
            State::Starting => Phase::Starting,
            State::Running { since } => Phase::Running { up: (now - since).max(0.0) },
            State::Backoff { attempt, until } => {
                Phase::Backoff { attempt, wait: (until - now).max(0.0) }
            }
            State::Crashed { exit, at } => Phase::Crashed { exit, ago: (now - at).max(0.0) },
        };
        Carry {
            phase,
            streak: self.streak,
            exits_ago: self.exits.iter().map(|at| (now - at).max(0.0)).collect(),
            last_exit: self.last_exit,
        }
    }

    #[must_use]
    pub fn from_carry(carry: &Carry, now: f64) -> Self {
        let state = match carry.phase {
            Phase::Stopped => State::Stopped,
            Phase::Starting => State::Starting,
            Phase::Running { up } => State::Running { since: now - up },
            Phase::Backoff { attempt, wait } => State::Backoff { attempt, until: now + wait },
            Phase::Crashed { exit, ago } => State::Crashed { exit, at: now - ago },
        };
        Self {
            state,
            exits: carry.exits_ago.iter().map(|ago| now - ago).collect(),
            streak: carry.streak,
            last_exit: carry.last_exit,
        }
    }
}

/// What a supervisor is, relative to now, so it survives a restart of the app on another clock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Carry {
    pub phase: Phase,
    #[serde(default)]
    pub streak: u32,
    #[serde(default)]
    pub exits_ago: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_exit: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Phase {
    Stopped,
    Starting,
    Running { up: f64 },
    Backoff { attempt: u32, wait: f64 },
    Crashed { exit: i32, ago: f64 },
}

/// `45s`, `12m`, `2h 14m`, `3d 4h`: the two largest units that are not zero.
#[must_use]
pub fn duration_text(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let (days, hours, minutes, seconds) =
        (total / 86_400, total % 86_400 / 3600, total % 3600 / 60, total % 60);
    match (days, hours, minutes) {
        (0, 0, 0) => format!("{seconds}s"),
        (0, 0, _) => format!("{minutes}m"),
        (0, _, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(at: f64) -> Supervisor {
        let mut sup = Supervisor::new();
        sup.step(Event::Start);
        sup.step(Event::Spawned { at });
        sup
    }

    #[test]
    fn backoff_doubles_from_one_second_and_stops_at_the_cap() {
        let cases =
            [(1, 1.0), (2, 2.0), (3, 4.0), (4, 8.0), (5, 16.0), (6, 32.0), (7, 60.0), (40, 60.0)];
        for (attempt, want) in cases {
            assert_eq!(backoff_secs(attempt), want, "attempt {attempt}");
        }
    }

    #[test]
    fn a_start_spawns_and_the_spawn_makes_it_run() {
        let mut sup = Supervisor::new();
        assert_eq!(sup.step(Event::Start), [Effect::Spawn]);
        assert_eq!(sup.state(), State::Starting);
        assert_eq!(sup.step(Event::Spawned { at: 3.0 }), []);
        assert_eq!(sup.state(), State::Running { since: 3.0 });
        assert_eq!(sup.step(Event::Start), []);
    }

    #[test]
    fn an_exit_backs_off_one_two_four_eight_seconds_then_gives_up_on_the_fifth() {
        let mut sup = running(0.0);
        let mut at = 1.0;
        for (attempt, wait) in [(1, 1.0), (2, 2.0), (3, 4.0), (4, 8.0)] {
            let effects = sup.step(Event::Exited { code: 1, at });
            assert_eq!(effects, [Effect::Wake { at: at + wait }], "attempt {attempt}");
            assert_eq!(sup.state(), State::Backoff { attempt, until: at + wait });
            assert_eq!(sup.step(Event::Tick { at: at + wait - 0.5 }), []);
            assert_eq!(sup.step(Event::Tick { at: at + wait }), [Effect::Spawn]);
            sup.step(Event::Spawned { at: at + wait });
            at += wait + 1.0;
        }
        assert_eq!(sup.step(Event::Exited { code: 7, at }), []);
        assert_eq!(sup.state(), State::Crashed { exit: 7, at });
        assert_eq!(sup.word(), Word::Crashed);
        assert_eq!(sup.step(Event::Tick { at: at + 1000.0 }), []);
    }

    #[test]
    fn exits_older_than_ten_minutes_do_not_count_toward_giving_up() {
        let mut sup = running(0.0);
        for n in 0..8 {
            let at = 700.0 * f64::from(n) + 10.0;
            sup.step(Event::Exited { code: 1, at });
            assert!(matches!(sup.state(), State::Backoff { .. }), "exit {n}");
            sup.step(Event::Tick { at: at + 100.0 });
            sup.step(Event::Spawned { at: at + 100.0 });
        }
    }

    #[test]
    fn a_run_of_two_minutes_resets_the_count() {
        let mut sup = running(0.0);
        sup.step(Event::Exited { code: 1, at: 1.0 });
        sup.step(Event::Tick { at: 2.0 });
        sup.step(Event::Spawned { at: 2.0 });
        sup.step(Event::Exited { code: 1, at: 3.0 });
        assert_eq!(sup.state(), State::Backoff { attempt: 2, until: 5.0 });
        sup.step(Event::Tick { at: 5.0 });
        sup.step(Event::Spawned { at: 5.0 });
        sup.step(Event::Exited { code: 1, at: 5.0 + STABLE_RUN_SECS });
        assert_eq!(sup.state(), State::Backoff { attempt: 1, until: 5.0 + STABLE_RUN_SECS + 1.0 });
    }

    #[test]
    fn a_run_just_short_of_two_minutes_keeps_the_count() {
        let mut sup = running(0.0);
        sup.step(Event::Exited { code: 1, at: STABLE_RUN_SECS - 1.0 });
        assert_eq!(sup.state(), State::Backoff { attempt: 1, until: STABLE_RUN_SECS });
        sup.step(Event::Tick { at: STABLE_RUN_SECS });
        sup.step(Event::Spawned { at: STABLE_RUN_SECS });
        sup.step(Event::Exited { code: 1, at: 2.0 * STABLE_RUN_SECS - 1.0 });
        assert!(matches!(sup.state(), State::Backoff { attempt: 2, .. }));
    }

    #[test]
    fn a_stop_by_hand_kills_and_never_restarts() {
        let mut sup = running(0.0);
        assert_eq!(sup.step(Event::Stop), [Effect::Kill]);
        assert_eq!(sup.state(), State::Stopped);
        assert_eq!(sup.step(Event::Exited { code: 143, at: 1.0 }), []);
        assert_eq!(sup.state(), State::Stopped);
        assert_eq!(sup.line(5.0, None), "stopped, exit 143");
        assert_eq!(sup.step(Event::Tick { at: 99.0 }), []);
        assert_eq!(sup.step(Event::Stop), []);
    }

    #[test]
    fn stopping_while_waiting_or_after_giving_up_just_stops() {
        let mut waiting = running(0.0);
        waiting.step(Event::Exited { code: 1, at: 1.0 });
        assert_eq!(waiting.step(Event::Stop), []);
        assert_eq!(waiting.state(), State::Stopped);
        assert_eq!(waiting.step(Event::Tick { at: 50.0 }), []);
        let mut gave_up = Supervisor { state: State::Crashed { exit: 1, at: 9.0 }, ..waiting };
        assert_eq!(gave_up.step(Event::Stop), []);
        assert_eq!(gave_up.state(), State::Stopped);
    }

    #[test]
    fn restart_kills_and_spawns_at_once_and_forgives_the_count() {
        let mut sup = running(0.0);
        sup.step(Event::Exited { code: 1, at: 1.0 });
        sup.step(Event::Tick { at: 2.0 });
        sup.step(Event::Spawned { at: 2.0 });
        assert_eq!(sup.step(Event::Restart), [Effect::Kill, Effect::Spawn]);
        assert_eq!(sup.state(), State::Starting);
        sup.step(Event::Spawned { at: 3.0 });
        sup.step(Event::Exited { code: 1, at: 4.0 });
        assert_eq!(sup.state(), State::Backoff { attempt: 1, until: 5.0 });
    }

    #[test]
    fn restart_and_start_bring_back_a_stopped_waiting_or_crashed_service() {
        let waiting = {
            let mut sup = running(0.0);
            sup.step(Event::Exited { code: 1, at: 1.0 });
            sup
        };
        let crashed =
            Supervisor { state: State::Crashed { exit: 1, at: 9.0 }, ..Supervisor::new() };
        for event in [Event::Start, Event::Restart] {
            for from in [Supervisor::new(), waiting.clone(), crashed.clone()] {
                let mut sup = from;
                assert_eq!(sup.step(event), [Effect::Spawn], "{event:?}");
                assert_eq!(sup.state(), State::Starting);
            }
        }
    }

    #[test]
    fn an_exit_while_starting_counts_like_a_run_of_no_time() {
        let mut sup = Supervisor::new();
        sup.step(Event::Start);
        sup.step(Event::Exited { code: 127, at: 0.5 });
        assert_eq!(sup.state(), State::Backoff { attempt: 1, until: 1.5 });
        assert_eq!(sup.last_exit(), Some(127));
    }

    #[test]
    fn nothing_happens_for_an_event_that_means_nothing_in_the_state() {
        let mut sup = Supervisor::new();
        for event in [
            Event::Spawned { at: 1.0 },
            Event::Exited { code: 0, at: 1.0 },
            Event::Tick { at: 1.0 },
            Event::Stop,
        ] {
            assert_eq!(sup.step(event), []);
        }
        assert_eq!(sup.state(), State::Stopped);
    }

    #[test]
    fn the_line_under_the_name_in_every_state() {
        let mut sup = Supervisor::new();
        assert_eq!(sup.line(0.0, None), "stopped");
        sup.step(Event::Start);
        assert_eq!(sup.line(0.0, None), "starting");
        sup.step(Event::Spawned { at: 100.0 });
        assert_eq!(sup.line(100.0 + 8040.0, Some(3309)), "up 2h 14m \u{b7} :3309");
        assert_eq!(sup.line(130.0, None), "up 30s");
        sup.step(Event::Exited { code: 1, at: 200.0 });
        assert_eq!(sup.line(197.0, None), "restarting in 4s");
        assert_eq!(sup.line(200.2, None), "restarting in 1s");
        let crashed = Supervisor { state: State::Crashed { exit: 2, at: 1.0 }, ..sup };
        assert_eq!(crashed.line(5.0, None), "crashed, exit 2");
    }

    #[test]
    fn durations_show_the_two_largest_units() {
        let cases = [
            (0.0, "0s"),
            (59.9, "59s"),
            (60.0, "1m"),
            (3599.0, "59m"),
            (3600.0, "1h 0m"),
            (8040.0, "2h 14m"),
            (86_400.0 + 4.0 * 3600.0, "1d 4h"),
        ];
        for (secs, want) in cases {
            assert_eq!(duration_text(secs), want, "{secs}");
        }
    }

    #[test]
    fn a_carried_supervisor_comes_back_on_another_clock_and_through_toml() {
        let mut sup = running(100.0);
        sup.step(Event::Exited { code: 3, at: 110.0 });
        sup.step(Event::Tick { at: 111.0 });
        sup.step(Event::Spawned { at: 111.0 });
        let carry = sup.carry(150.0);
        let text = toml::to_string(&carry).unwrap();
        let read: Carry = toml::from_str(&text).unwrap();
        assert_eq!(read, carry);
        let back = Supervisor::from_carry(&read, 5.0);
        assert_eq!(back.state(), State::Running { since: 5.0 - 39.0 });
        assert_eq!(back.last_exit(), Some(3));
        assert_eq!(back.carry(5.0), read);
    }

    #[test]
    fn every_phase_round_trips_through_toml() {
        let phases = [
            Phase::Stopped,
            Phase::Starting,
            Phase::Running { up: 12.5 },
            Phase::Backoff { attempt: 3, wait: 2.0 },
            Phase::Crashed { exit: 9, ago: 4.0 },
        ];
        for phase in phases {
            let carry = Carry { phase, streak: 2, exits_ago: vec![1.0, 2.0], last_exit: Some(1) };
            let text = toml::to_string(&carry).unwrap();
            assert_eq!(toml::from_str::<Carry>(&text).unwrap(), carry, "{text}");
        }
    }

    #[test]
    fn a_waiting_service_still_waits_the_same_time_after_a_carry() {
        let mut sup = running(0.0);
        sup.step(Event::Exited { code: 1, at: 10.0 });
        let back = Supervisor::from_carry(&sup.carry(10.5), 100.0);
        assert_eq!(back.next_wake(), Some(100.5));
    }
}
