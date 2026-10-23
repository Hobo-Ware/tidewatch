//! The app's side of keeping awake: each frame the number of working agents goes through the state
//! machine in `tidewatch_core::keep_awake` and a change takes or releases the OS hold. A background
//! run (a screenshot, `TIDEWATCH_BACKGROUND`) never holds unless `TIDEWATCH_DEV_KEEP_AWAKE=1` asks.

use tidewatch_core::keep_awake::{State, step};

use crate::commands::Platform;
use crate::keep_awake_hold::{Hold, REASON};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Took,
    Released,
}

pub(crate) struct Host<H: Hold> {
    hold: H,
    enabled: bool,
    allowed: bool,
    unavailable: bool,
    state: State,
    working: usize,
}

pub(crate) fn dev_asks(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty() && v != "0")
}

impl<H: Hold> Host<H> {
    pub(crate) fn new(hold: H, enabled: bool, allowed: bool) -> Self {
        Self { hold, enabled, allowed, unavailable: false, state: State::Released, working: 0 }
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled {
            self.unavailable = false;
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn held(&self) -> bool {
        self.state.holds()
    }

    /// Moves the hold toward what `working` agents call for. Returns the change made, if any, and
    /// when to look again with no new input (the end of the quiet window).
    pub(crate) fn tick(&mut self, working: usize, now: f64) -> (Option<Change>, Option<f64>) {
        self.working = working;
        if !self.allowed || self.unavailable {
            return (None, None);
        }
        if self.state.holds() && self.hold.lost() {
            eprintln!("tidewatch: the sleep hold ended on its own; not keeping awake");
            self.unavailable = true;
            self.state = State::Released;
            return (Some(Change::Released), None);
        }
        let next = step(self.state, working, self.enabled, now);
        let change = match (self.state.holds(), next.holds()) {
            (false, true) => match self.hold.take(REASON) {
                Ok(()) => Some(Change::Took),
                Err(e) => {
                    eprintln!("tidewatch: cannot keep the computer awake ({e})");
                    self.unavailable = true;
                    return (None, None);
                }
            },
            (true, false) => {
                self.hold.release();
                Some(Change::Released)
            }
            _ => None,
        };
        self.state = next;
        (change, next.wake_at())
    }

    pub(crate) fn tip(&self) -> Option<String> {
        self.held().then(|| tip(self.working, Platform::CURRENT))
    }
}

/// The status bar mark's tooltip.
pub(crate) fn tip(working: usize, platform: Platform) -> String {
    let machine = match platform {
        Platform::Mac => "Mac",
        Platform::Linux => "computer",
    };
    match working {
        0 => format!("Keeping the {machine} awake: agents idle, letting go soon"),
        1 => format!("Keeping the {machine} awake: 1 agent working"),
        n => format!("Keeping the {machine} awake: {n} agents working"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::keep_awake::RELEASE_AFTER_SECS;

    #[derive(Default)]
    struct Recorder {
        held: bool,
        takes: usize,
        releases: usize,
        fail: bool,
        die: bool,
    }

    impl Hold for Recorder {
        fn take(&mut self, _reason: &str) -> Result<(), String> {
            if self.fail {
                return Err("no helper".to_owned());
            }
            self.held = true;
            self.takes += 1;
            Ok(())
        }
        fn release(&mut self) {
            self.held = false;
            self.releases += 1;
        }
        fn lost(&mut self) -> bool {
            self.die
        }
    }

    fn host(enabled: bool, allowed: bool) -> Host<Recorder> {
        Host::new(Recorder::default(), enabled, allowed)
    }

    #[test]
    fn the_hold_is_taken_at_once_and_let_go_after_the_quiet_window() {
        let mut host = host(true, true);
        assert_eq!(host.tick(0, 0.0), (None, None));
        assert_eq!(host.tick(2, 1.0), (Some(Change::Took), None));
        assert_eq!(host.tick(3, 2.0), (None, None));
        assert_eq!(host.tick(0, 10.0), (None, Some(10.0 + RELEASE_AFTER_SECS)));
        assert_eq!(host.tick(0, 100.0), (None, Some(10.0 + RELEASE_AFTER_SECS)));
        assert!(host.held() && host.hold.held);
        assert_eq!(host.tick(0, 10.0 + RELEASE_AFTER_SECS), (Some(Change::Released), None));
        assert!(!host.held() && !host.hold.held);
        assert_eq!((host.hold.takes, host.hold.releases), (1, 1));
    }

    #[test]
    fn a_pause_inside_the_window_does_not_flap() {
        let mut host = host(true, true);
        host.tick(1, 0.0);
        host.tick(0, 5.0);
        assert_eq!(host.tick(1, 60.0), (None, None));
        assert_eq!((host.hold.takes, host.hold.releases), (1, 0));
    }

    #[test]
    fn switching_off_lets_go_at_once_and_on_takes_again_when_an_agent_works() {
        let mut host = host(true, true);
        host.tick(1, 0.0);
        host.set_enabled(false);
        assert_eq!(host.tick(1, 1.0), (Some(Change::Released), None));
        assert_eq!(host.tick(1, 2.0), (None, None));
        host.set_enabled(true);
        assert_eq!(host.tick(1, 3.0), (Some(Change::Took), None));
        assert_eq!((host.hold.takes, host.hold.releases), (2, 1));
    }

    #[test]
    fn a_background_run_never_holds() {
        let mut host = host(true, false);
        assert_eq!(host.tick(5, 0.0), (None, None));
        assert!(!host.held());
        assert_eq!(host.hold.takes, 0);
    }

    #[test]
    fn a_hold_that_cannot_be_taken_is_not_retried_every_frame() {
        let mut host = host(true, true);
        host.hold.fail = true;
        assert_eq!(host.tick(1, 0.0), (None, None));
        host.hold.fail = false;
        assert_eq!(host.tick(1, 1.0), (None, None));
        assert!(!host.held());
        host.set_enabled(true);
        assert_eq!(host.tick(1, 2.0), (Some(Change::Took), None));
    }

    #[test]
    fn a_hold_the_os_ended_is_reported_once_and_left_alone() {
        let mut host = host(true, true);
        host.tick(1, 0.0);
        host.hold.die = true;
        assert_eq!(host.tick(1, 1.0), (Some(Change::Released), None));
        assert_eq!(host.tick(1, 2.0), (None, None));
        assert!(!host.held());
    }

    #[test]
    fn the_mark_shows_only_while_held_and_counts_what_the_last_tick_saw() {
        let mut host = host(true, true);
        assert_eq!(host.tip(), None);
        host.tick(2, 0.0);
        assert_eq!(host.tip(), Some(tip(2, Platform::CURRENT)));
        host.tick(0, 5.0);
        assert_eq!(host.tip(), Some(tip(0, Platform::CURRENT)));
        host.tick(0, 5.0 + RELEASE_AFTER_SECS);
        assert_eq!(host.tip(), None);
    }

    #[test]
    fn the_dev_switch_is_any_value_but_empty_or_zero() {
        assert_eq!(
            [None, Some(""), Some("0"), Some("1"), Some("yes")].map(dev_asks),
            [false, false, false, true, true]
        );
    }

    #[test]
    fn the_tip_counts_the_agents_and_names_the_machine() {
        assert_eq!(tip(2, Platform::Mac), "Keeping the Mac awake: 2 agents working");
        assert_eq!(tip(1, Platform::Mac), "Keeping the Mac awake: 1 agent working");
        assert_eq!(tip(1, Platform::Linux), "Keeping the computer awake: 1 agent working");
        assert_eq!(tip(0, Platform::Mac), "Keeping the Mac awake: agents idle, letting go soon");
    }
}
