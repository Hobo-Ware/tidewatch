//! The rules for handing notes to an agent (PLAN.md, D4), as plain data: whether a send may go out
//! now, the per-tab "deliver when idle" switch, and the short pause between a paste and its Enter.
//! Nothing here touches a terminal or a thread, so every rule is a unit test.

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
        Some(AgentStatus::SettingUp | AgentStatus::Failed(_) | AgentStatus::Shell) | None => {
            Gate::NoAgent
        }
    }
}

/// Why a send is unavailable, for a tooltip or a toast. `None` when it may go.
pub(crate) fn gate_reason(gate: Gate) -> Option<&'static str> {
    match gate {
        Gate::Ready => None,
        Gate::WaitingOnYou => Some("The agent is waiting on you. Answer it first"),
        Gate::NoAgent => Some("There is no running agent in this tab"),
    }
}

/// The per-tab "deliver when idle" switch. Queued notes go out once the agent is Done, never while
/// it works or waits on you, and never twice for one idle moment.
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

/// Enters still to send, one per paste.
#[derive(Debug, Default)]
pub(crate) struct EnterQueue {
    due: Vec<(u64, f64)>,
}

impl EnterQueue {
    pub(crate) fn push(&mut self, tab: u64, now: f64) {
        self.due.push((tab, now + ENTER_DELAY_SECS));
    }

    /// The tabs whose Enter is due, removed from the queue.
    pub(crate) fn take_due(&mut self, now: f64) -> Vec<u64> {
        let (due, later): (Vec<_>, Vec<_>) = self.due.drain(..).partition(|(_, at)| *at <= now);
        self.due = later;
        due.into_iter().map(|(tab, _)| tab).collect()
    }

    pub(crate) fn next_due(&self) -> Option<f64> {
        self.due.iter().map(|(_, at)| *at).reduce(f64::min)
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.due.retain(|(id, _)| *id != tab);
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
    fn an_enter_comes_due_after_the_pause_and_only_once() {
        let mut queue = EnterQueue::default();
        queue.push(3, 10.0);
        queue.push(4, 10.2);
        assert_eq!(queue.take_due(10.1), Vec::<u64>::new());
        assert_eq!(queue.next_due(), Some(10.0 + ENTER_DELAY_SECS));
        assert_eq!(queue.take_due(10.35), [3]);
        assert_eq!(queue.take_due(10.35), Vec::<u64>::new());
        assert_eq!(queue.take_due(11.0), [4]);
        assert_eq!(queue.next_due(), None);
    }

    #[test]
    fn a_closed_tab_loses_its_pending_enter() {
        let mut queue = EnterQueue::default();
        queue.push(3, 0.0);
        queue.forget(3);
        assert_eq!(queue.take_due(5.0), Vec::<u64>::new());
    }
}
