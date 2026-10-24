//! When the computer is kept from idle-sleeping: held at once when an agent works, released only
//! after every agent has been non-working for [`RELEASE_AFTER_SECS`], so a gap between two turns or
//! two workflow stages does not flap the hold. Pure: time is passed in, never read.
//!
//! What counts as work: an agent that is `Working`, one that went quiet (`NeedsYou(Quiet)`: a long
//! build or migration prints nothing), and any live tab with a running, not stalled, job on its
//! board. An agent that waits for you for a real reason (permission, question, your next message,
//! the bell), is done, failed or is only a shell does not.

use crate::status::{AgentStatus, Reason};

/// How long every agent must stay non-working before the hold is released.
pub const RELEASE_AFTER_SECS: f64 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Released,
    /// Held; `idle_since` is when the last working agent stopped, `None` while one works.
    Held {
        idle_since: Option<f64>,
    },
}

impl State {
    #[must_use]
    pub fn holds(self) -> bool {
        matches!(self, Self::Held { .. })
    }

    /// When the state changes with no new input: the end of the debounce, if one is running.
    #[must_use]
    pub fn wake_at(self) -> Option<f64> {
        match self {
            Self::Held { idle_since: Some(since) } => Some(since + RELEASE_AFTER_SECS),
            Self::Held { idle_since: None } | Self::Released => None,
        }
    }
}

/// Whether one tab keeps the computer awake. A tab that exited (done or failed) never does, whatever
/// its job board still says.
#[must_use]
pub fn keeps_awake(status: &AgentStatus, job_running: bool) -> bool {
    match status {
        AgentStatus::Working | AgentStatus::NeedsYou(Reason::Quiet) => true,
        AgentStatus::NeedsYou(_) | AgentStatus::SettingUp => job_running,
        AgentStatus::Done | AgentStatus::Failed(_) | AgentStatus::Stopped | AgentStatus::Shell => {
            false
        }
    }
}

/// The next state given how many agents work now. Switching it off releases at once.
#[must_use]
pub fn step(state: State, working: usize, enabled: bool, now: f64) -> State {
    if !enabled {
        return State::Released;
    }
    match (working > 0, state) {
        (true, _) => State::Held { idle_since: None },
        (false, State::Released) => State::Released,
        (false, State::Held { idle_since: None }) => State::Held { idle_since: Some(now) },
        (false, State::Held { idle_since: Some(since) }) => {
            if now - since >= RELEASE_AFTER_SECS {
                State::Released
            } else {
                state
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELD: State = State::Held { idle_since: None };

    fn held_since(at: f64) -> State {
        State::Held { idle_since: Some(at) }
    }

    #[test]
    fn each_step_follows_the_table() {
        let cases = [
            ("first agent starts", State::Released, 1, true, 5.0, HELD),
            ("many agents start", State::Released, 4, true, 5.0, HELD),
            ("nobody works and nothing held", State::Released, 0, true, 5.0, State::Released),
            ("still working", HELD, 2, true, 9.0, HELD),
            ("last agent stops", HELD, 0, true, 9.0, held_since(9.0)),
            ("quiet, inside the window", held_since(9.0), 0, true, 128.9, held_since(9.0)),
            ("quiet, window over", held_since(9.0), 0, true, 129.0, State::Released),
            ("quiet, long past the window", held_since(9.0), 0, true, 900.0, State::Released),
            ("an agent resumes in the window", held_since(9.0), 1, true, 60.0, HELD),
            ("switched off while working", HELD, 3, false, 9.0, State::Released),
            ("switched off while quiet", held_since(9.0), 0, false, 20.0, State::Released),
            ("switched off, nothing held", State::Released, 2, false, 9.0, State::Released),
        ];
        for (name, state, working, enabled, now, expected) in cases {
            assert_eq!(step(state, working, enabled, now), expected, "{name}");
        }
    }

    #[test]
    fn a_tab_keeps_the_computer_awake_while_it_works_goes_quiet_or_runs_a_job() {
        let needs = |reason| AgentStatus::NeedsYou(reason);
        let cases = [
            ("working", AgentStatus::Working, false, true),
            ("working with a job", AgentStatus::Working, true, true),
            ("quiet: a long build prints nothing", needs(Reason::Quiet), false, true),
            ("quiet with a job", needs(Reason::Quiet), true, true),
            ("waiting for a permission", needs(Reason::Permission), false, false),
            ("asked a question", needs(Reason::Question), false, false),
            ("waiting for your message", needs(Reason::Input), false, false),
            ("rang the bell", needs(Reason::Attention), false, false),
            ("idle at the prompt while a job runs", needs(Reason::Input), true, true),
            ("a question while a job runs", needs(Reason::Question), true, true),
            ("setting up", AgentStatus::SettingUp, false, false),
            ("done", AgentStatus::Done, false, false),
            ("done, board still says running", AgentStatus::Done, true, false),
            ("failed", AgentStatus::Failed("exit 1".to_owned()), false, false),
            (
                "failed, board still says running",
                AgentStatus::Failed("exit 1".to_owned()),
                true,
                false,
            ),
            ("a shell", AgentStatus::Shell, true, false),
            ("a stopped tab", AgentStatus::Stopped, true, false),
        ];
        for (name, status, job, expected) in cases {
            assert_eq!(keeps_awake(&status, job), expected, "{name}");
        }
    }

    #[test]
    fn a_pause_shorter_than_the_window_never_lets_go() {
        let mut state = step(State::Released, 1, true, 0.0);
        for (t, working) in [(10.0, 0), (60.0, 0), (100.0, 1), (130.0, 0), (240.0, 0)] {
            state = step(state, working, true, t);
            assert!(state.holds(), "at {t}");
        }
        assert_eq!(step(state, 0, true, 250.0), State::Released);
    }

    #[test]
    fn the_wake_is_the_end_of_the_window_while_one_is_running() {
        assert_eq!(held_since(10.0).wake_at(), Some(10.0 + RELEASE_AFTER_SECS));
        assert_eq!(HELD.wake_at(), None);
        assert_eq!(State::Released.wake_at(), None);
        assert!(HELD.holds() && held_since(1.0).holds() && !State::Released.holds());
    }
}
