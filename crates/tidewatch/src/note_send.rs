//! The rules for handing a review to an agent (PLAN.md, D4): send now, hold until idle or refuse,
//! the hold itself, and the pause between a paste and its Enter (pure).

use tidewatch_core::status::AgentStatus;

/// How long after a send the switch stays quiet when the status has not left Done yet: the hook that
/// says the agent started its turn can arrive a moment after the paste.
pub(crate) const HOLD_SECS: f64 = 10.0;
/// The pause between the paste and the Enter that submits it, so the agent's input box has taken
/// the paste before Enter arrives.
pub(crate) const ENTER_DELAY_SECS: f64 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    Ready,
    WaitingOnYou,
    NoAgent,
}

/// Whether a manual send may paste into the tab now. A tab that waits on a permission prompt or a
/// question would take the paste as its answer, so it is refused; a working agent queues the message.
pub(crate) fn gate(status: Option<&AgentStatus>) -> Gate {
    match status {
        Some(AgentStatus::Working | AgentStatus::Done) => Gate::Ready,
        Some(AgentStatus::NeedsYou(_)) => Gate::WaitingOnYou,
        Some(
            AgentStatus::SettingUp
            | AgentStatus::Failed(_)
            | AgentStatus::Stopped
            | AgentStatus::Shell,
        )
        | None => Gate::NoAgent,
    }
}

const NO_AGENT: &str = "There is no running agent in this tab";

/// Why a send is unavailable, for a tooltip or a toast. `None` when it may go.
pub(crate) fn gate_reason(gate: Gate) -> Option<&'static str> {
    match gate {
        Gate::Ready => None,
        Gate::WaitingOnYou => Some("The agent is waiting on you. Answer it first"),
        Gate::NoAgent => Some(NO_AGENT),
    }
}

/// Where a review goes: a review is never pasted into a Working or NeedsYou agent, which would take
/// the paste as an answer or queue it behind its own turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReviewGate {
    Now,
    WhenIdle,
    Refused(&'static str),
}

pub(crate) fn review_gate(status: Option<&AgentStatus>) -> ReviewGate {
    match status {
        Some(AgentStatus::Done) => ReviewGate::Now,
        Some(AgentStatus::Working | AgentStatus::NeedsYou(_)) => ReviewGate::WhenIdle,
        Some(
            AgentStatus::SettingUp
            | AgentStatus::Failed(_)
            | AgentStatus::Stopped
            | AgentStatus::Shell,
        )
        | None => ReviewGate::Refused(NO_AGENT),
    }
}

/// The per-tab hold on a review the user sent while the agent was busy (`enabled`). It goes out
/// once the agent is Done, never while it works or waits on you, and never twice for one idle
/// moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AutoDelivery {
    pub(crate) enabled: bool,
    in_flight: bool,
    hold_until: Option<f64>,
}

impl AutoDelivery {
    /// Call every frame with the tab's status: leaving Done ends the hold.
    pub(crate) fn observe(&mut self, status: &AgentStatus) {
        if *status != AgentStatus::Done {
            self.hold_until = None;
        }
    }

    pub(crate) fn should_send(&self, status: &AgentStatus, pending: usize, now: f64) -> bool {
        self.enabled
            && !self.in_flight
            && pending > 0
            && *status == AgentStatus::Done
            && self.hold_until.is_none_or(|until| now >= until)
    }

    pub(crate) fn begin(&mut self) {
        self.in_flight = true;
    }

    /// The send job ended. A send that went out holds the switch until the agent leaves Done.
    pub(crate) fn finished(&mut self, sent: bool, now: f64) {
        self.in_flight = false;
        if sent {
            self.hold_until = Some(now + HOLD_SECS);
        }
    }

    /// A manual send counts as a delivery for the hold.
    pub(crate) fn sent_by_hand(&mut self, now: f64) {
        self.hold_until = Some(now + HOLD_SECS);
    }

    /// The moment the hold ends, when a frame is needed to look again.
    pub(crate) fn next_check(&self, now: f64) -> Option<f64> {
        self.hold_until.filter(|until| *until > now && self.enabled)
    }
}

/// How long the terminal must stay silent after a paste before its Enter goes: a big paste is
/// still being echoed when the first pause is over.
pub(crate) const ENTER_QUIET_SECS: f64 = 0.12;
/// The longest an Enter waits for the terminal to go quiet.
pub(crate) const ENTER_MAX_WAIT_SECS: f64 = 3.0;

#[derive(Debug)]
struct Waiting {
    tab: u64,
    due: f64,
    deadline: f64,
    seen: Option<(u64, f64)>,
}

/// Enters still to send, one per paste.
#[derive(Debug, Default)]
pub(crate) struct EnterQueue {
    waiting: Vec<Waiting>,
}

impl EnterQueue {
    pub(crate) fn push(&mut self, tab: u64, now: f64) {
        self.waiting.push(Waiting {
            tab,
            due: now + ENTER_DELAY_SECS,
            deadline: now + ENTER_MAX_WAIT_SECS,
            seen: None,
        });
    }

    /// The tabs whose Enter may go, removed from the queue: the pause is over and the terminal's
    /// output count (`output`, none when it is not known) has stood still for `ENTER_QUIET_SECS`,
    /// or the longest wait ran out.
    pub(crate) fn take_due(&mut self, now: f64, output: impl Fn(u64) -> Option<u64>) -> Vec<u64> {
        let mut due = Vec::new();
        self.waiting.retain_mut(|waiting| {
            if now < waiting.due {
                return true;
            }
            let fire = match (now >= waiting.deadline, output(waiting.tab), waiting.seen) {
                (true, _, _) | (_, None, _) => true,
                (false, Some(count), Some((seen, since))) if seen == count => {
                    now - since >= ENTER_QUIET_SECS
                }
                (false, Some(count), _) => {
                    waiting.seen = Some((count, now));
                    false
                }
            };
            if fire {
                due.push(waiting.tab);
            }
            !fire
        });
        due
    }

    pub(crate) fn next_due(&self) -> Option<f64> {
        self.waiting
            .iter()
            .map(|waiting| match waiting.seen {
                Some((_, since)) => (since + ENTER_QUIET_SECS).min(waiting.deadline),
                None => waiting.due,
            })
            .reduce(f64::min)
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.waiting.retain(|waiting| waiting.tab != tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::status::Reason;

    fn auto(enabled: bool) -> AutoDelivery {
        AutoDelivery { enabled, ..AutoDelivery::default() }
    }

    #[test]
    fn a_hand_send_goes_to_a_working_or_done_agent_and_never_to_one_waiting_on_you() {
        let cases = [
            (Some(AgentStatus::Working), Gate::Ready),
            (Some(AgentStatus::Done), Gate::Ready),
            (Some(AgentStatus::NeedsYou(Reason::Permission)), Gate::WaitingOnYou),
            (Some(AgentStatus::NeedsYou(Reason::Quiet)), Gate::WaitingOnYou),
            (Some(AgentStatus::SettingUp), Gate::NoAgent),
            (Some(AgentStatus::Failed("x".into())), Gate::NoAgent),
            (Some(AgentStatus::Stopped), Gate::NoAgent),
            (Some(AgentStatus::Shell), Gate::NoAgent),
            (None, Gate::NoAgent),
        ];
        for (status, want) in cases {
            assert_eq!(gate(status.as_ref()), want, "{status:?}");
        }
        assert_eq!(gate_reason(Gate::Ready), None);
        assert!(gate_reason(Gate::WaitingOnYou).is_some());
    }

    #[test]
    fn a_review_goes_to_an_idle_agent_waits_for_a_busy_one_and_is_refused_without_one() {
        let cases = [
            (Some(AgentStatus::Done), ReviewGate::Now),
            (Some(AgentStatus::Working), ReviewGate::WhenIdle),
            (Some(AgentStatus::NeedsYou(Reason::Permission)), ReviewGate::WhenIdle),
            (Some(AgentStatus::NeedsYou(Reason::Question)), ReviewGate::WhenIdle),
            (Some(AgentStatus::SettingUp), ReviewGate::Refused(NO_AGENT)),
            (Some(AgentStatus::Failed("x".into())), ReviewGate::Refused(NO_AGENT)),
            (Some(AgentStatus::Shell), ReviewGate::Refused(NO_AGENT)),
            (None, ReviewGate::Refused(NO_AGENT)),
        ];
        for (status, want) in cases {
            assert_eq!(review_gate(status.as_ref()), want, "{status:?}");
        }
    }

    #[test]
    fn queued_notes_go_out_only_when_the_agent_is_done() {
        let switch = auto(true);
        let cases = [
            (AgentStatus::Done, true),
            (AgentStatus::Working, false),
            (AgentStatus::NeedsYou(Reason::Permission), false),
            (AgentStatus::NeedsYou(Reason::Input), false),
            (AgentStatus::NeedsYou(Reason::Quiet), false),
            (AgentStatus::SettingUp, false),
            (AgentStatus::Failed("x".into()), false),
        ];
        for (status, want) in cases {
            assert_eq!(switch.should_send(&status, 2, 0.0), want, "{status:?}");
        }
    }

    #[test]
    fn nothing_queued_or_the_switch_off_sends_nothing() {
        assert!(!auto(true).should_send(&AgentStatus::Done, 0, 0.0));
        assert!(!auto(false).should_send(&AgentStatus::Done, 3, 0.0));
    }

    #[test]
    fn a_running_send_blocks_a_second_one_and_a_finished_send_holds_until_the_agent_leaves_done() {
        let mut switch = auto(true);
        assert!(switch.should_send(&AgentStatus::Done, 1, 0.0));
        switch.begin();
        assert!(!switch.should_send(&AgentStatus::Done, 1, 0.1));
        switch.finished(true, 1.0);
        assert!(!switch.should_send(&AgentStatus::Done, 1, 2.0));
        assert_eq!(switch.next_check(2.0), Some(1.0 + HOLD_SECS));
        switch.observe(&AgentStatus::Working);
        switch.observe(&AgentStatus::Done);
        assert!(switch.should_send(&AgentStatus::Done, 1, 3.0));
    }

    #[test]
    fn a_hold_that_the_agent_never_ends_expires_on_its_own() {
        let mut switch = auto(true);
        switch.begin();
        switch.finished(true, 0.0);
        switch.observe(&AgentStatus::Done);
        assert!(!switch.should_send(&AgentStatus::Done, 1, HOLD_SECS - 0.1));
        assert!(switch.should_send(&AgentStatus::Done, 1, HOLD_SECS));
        assert_eq!(switch.next_check(HOLD_SECS), None);
    }

    #[test]
    fn a_send_job_that_sent_nothing_does_not_hold_and_a_hand_send_does() {
        let mut switch = auto(true);
        switch.begin();
        switch.finished(false, 0.0);
        assert!(switch.should_send(&AgentStatus::Done, 1, 0.0));
        switch.sent_by_hand(0.0);
        assert!(!switch.should_send(&AgentStatus::Done, 1, 1.0));
    }

    #[test]
    fn an_enter_comes_due_after_the_pause_a_quiet_terminal_and_only_once() {
        let mut queue = EnterQueue::default();
        queue.push(3, 10.0);
        queue.push(4, 10.2);
        let quiet = |_: u64| Some(7);
        assert_eq!(queue.take_due(10.1, quiet), Vec::<u64>::new());
        assert_eq!(queue.next_due(), Some(10.0 + ENTER_DELAY_SECS));
        assert_eq!(queue.take_due(10.35, quiet), Vec::<u64>::new());
        assert_eq!(queue.next_due(), Some(10.35 + ENTER_QUIET_SECS));
        assert_eq!(queue.take_due(10.5, quiet), [3]);
        assert_eq!(queue.take_due(10.5, quiet), Vec::<u64>::new());
        assert_eq!(queue.take_due(11.0, quiet), [4]);
        assert_eq!(queue.next_due(), None);
    }

    #[test]
    fn a_terminal_still_printing_the_paste_holds_the_enter_back() {
        let mut queue = EnterQueue::default();
        queue.push(3, 0.0);
        let count = std::cell::Cell::new(1_u64);
        let output = |_: u64| Some(count.get());
        assert_eq!(queue.take_due(0.4, output), Vec::<u64>::new());
        count.set(2);
        assert_eq!(queue.take_due(0.4 + ENTER_QUIET_SECS, output), Vec::<u64>::new());
        count.set(3);
        assert_eq!(queue.take_due(0.4 + 2.0 * ENTER_QUIET_SECS, output), Vec::<u64>::new());
        assert_eq!(queue.take_due(0.4 + 3.0 * ENTER_QUIET_SECS, output), [3]);
    }

    #[test]
    fn a_terminal_that_never_goes_quiet_or_is_unknown_does_not_block_the_enter() {
        let mut queue = EnterQueue::default();
        queue.push(3, 0.0);
        queue.push(4, 0.0);
        let count = std::cell::Cell::new(0_u64);
        let output = |tab: u64| (tab == 3).then(|| count.replace(count.get() + 1));
        assert_eq!(queue.take_due(0.4, output), [4]);
        assert_eq!(queue.take_due(1.0, output), Vec::<u64>::new());
        assert_eq!(queue.take_due(ENTER_MAX_WAIT_SECS, output), [3]);
    }

    #[test]
    fn a_closed_tab_loses_its_pending_enter() {
        let mut queue = EnterQueue::default();
        queue.push(3, 0.0);
        queue.forget(3);
        assert_eq!(queue.take_due(5.0, |_| Some(1)), Vec::<u64>::new());
    }
}
