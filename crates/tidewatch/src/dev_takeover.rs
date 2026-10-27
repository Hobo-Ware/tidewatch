//! `TIDEWATCH_DEV_TAKEOVER=<pid>` or `move:<pid>` presses Move on the running session with that pid
//! once the first scan has it (`confirm:<pid>` stops at the open dialog, for a capture); a trailing
//! `:worktree` ticks "Move it into a new worktree first". `TIDEWATCH_DEV_RECENT=open` starts with the
//! card's "Recent sessions" list open. `TIDEWATCH_DEV_ELSEWHERE=open` starts with the Running elsewhere
//! card folded out under its line. Read once.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DevTakeover {
    pub(crate) pid: u32,
    pub(crate) press: bool,
    pub(crate) worktree: bool,
}

impl DevTakeover {
    pub(crate) fn from_env() -> Option<Self> {
        Self::parse(std::env::var("TIDEWATCH_DEV_TAKEOVER").ok().as_deref())
    }

    pub(crate) fn recent_open_from_env() -> bool {
        std::env::var("TIDEWATCH_DEV_RECENT").is_ok_and(|value| value == "open")
    }

    pub(crate) fn line_open_from_env() -> bool {
        std::env::var("TIDEWATCH_DEV_ELSEWHERE").is_ok_and(|value| value == "open")
    }

    fn parse(value: Option<&str>) -> Option<Self> {
        let (value, worktree) = match value?.trim().strip_suffix(":worktree") {
            Some(rest) => (rest, true),
            None => (value?.trim(), false),
        };
        let (press, pid) = match value.split_once(':') {
            Some(("confirm", pid)) => (false, pid),
            Some(("move", pid)) => (true, pid),
            Some(_) => return None,
            None => (true, value),
        };
        Some(Self { pid: pid.parse().ok()?, press, worktree })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pid_presses_move_and_confirm_stops_at_the_dialog() {
        let cases = [
            (None, None),
            (Some("4242"), Some(DevTakeover { pid: 4242, press: true, worktree: false })),
            (Some("move:4242"), Some(DevTakeover { pid: 4242, press: true, worktree: false })),
            (Some("4242:worktree"), Some(DevTakeover { pid: 4242, press: true, worktree: true })),
            (
                Some("confirm:4242:worktree"),
                Some(DevTakeover { pid: 4242, press: false, worktree: true }),
            ),
            (Some("confirm:4242"), Some(DevTakeover { pid: 4242, press: false, worktree: false })),
            (Some("confirm:x"), None),
            (Some("other:4242"), None),
            (Some(""), None),
        ];
        for (value, want) in cases {
            assert_eq!(DevTakeover::parse(value), want, "{value:?}");
        }
    }
}
