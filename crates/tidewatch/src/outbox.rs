//! The one gate every message Tidewatch types into an agent goes through (pure, table-tested): whether
//! the tab may take text now, one send per tab until the agent leaves Done or `HOLD_SECS` pass, what
//! waits in line behind that, and the shared cap on automatic sends so two rules cannot loop with an
//! agent forever.

use std::collections::{HashMap, VecDeque};

use tidewatch_core::pr_cards::MAX_AUTO_SENDS;
use tidewatch_core::sent::{Cap, Reason};
use tidewatch_core::status::AgentStatus;

use crate::note_send::HOLD_SECS;

pub(crate) const NO_AGENT: &str = "There is no running agent in this tab";
pub(crate) const WAITING_ON_YOU: &str = "The agent is waiting on you. Answer it first";
const NO_BOX_PATIENCE_S: f64 = 30.0;
const QUEUE_MAX: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Auto,
    Hand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Free,
    Typed,
    NoBox,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tell {
    pub(crate) reason: Reason,
    pub(crate) because: String,
    pub(crate) cap: Option<Cap>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Send {
    pub(crate) text: String,
    pub(crate) origin: Origin,
    pub(crate) tell: Option<Tell>,
    pub(crate) mid_turn: bool,
}

impl Send {
    pub(crate) fn auto(text: &str, reason: Reason, because: &str) -> Self {
        Self {
            text: text.to_owned(),
            origin: Origin::Auto,
            tell: Some(Tell { reason, because: because.to_owned(), cap: None }),
            mid_turn: false,
        }
    }

    pub(crate) fn by_hand(text: &str, reason: Reason, because: &str) -> Self {
        Self { origin: Origin::Hand, ..Self::auto(text, reason, because) }
    }

    pub(crate) fn untold(text: &str, origin: Origin) -> Self {
        Self { text: text.to_owned(), origin, tell: None, mid_turn: false }
    }

    pub(crate) fn mid_turn(mut self) -> Self {
        self.mid_turn = true;
        self
    }

    pub(crate) fn with_cap(mut self, cap: Cap) -> Self {
        if let Some(tell) = &mut self.tell {
            tell.cap = Some(cap);
        }
        self
    }

    fn counted(&self) -> bool {
        self.origin == Origin::Auto
            && self.tell.as_ref().is_some_and(|tell| {
                tell.cap.is_none()
                    && matches!(
                        tell.reason,
                        Reason::DoneCheck
                            | Reason::CommentGuard
                            | Reason::LedgerGuard
                            | Reason::BackgroundJob
                    )
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Offer {
    Go,
    Wait,
    Paused,
    Refuse(&'static str),
}

#[derive(Debug, Default)]
struct Lane {
    hold_until: Option<f64>,
    queue: VecDeque<Send>,
    auto_sends: u32,
    no_box_since: Option<f64>,
}

#[derive(Debug, Default)]
pub(crate) struct Outbox {
    lanes: HashMap<u64, Lane>,
}

fn decide(status: Option<&AgentStatus>, origin: Origin, held: bool, mid_turn: bool) -> Offer {
    match status {
        Some(AgentStatus::Done) if held => Offer::Wait,
        Some(AgentStatus::Done) => Offer::Go,
        Some(AgentStatus::Working) if origin == Origin::Hand || mid_turn => Offer::Go,
        Some(AgentStatus::NeedsYou(_)) if origin == Origin::Hand => Offer::Refuse(WAITING_ON_YOU),
        Some(AgentStatus::Working | AgentStatus::NeedsYou(_)) => Offer::Wait,
        Some(
            AgentStatus::SettingUp
            | AgentStatus::Failed(_)
            | AgentStatus::Stopped
            | AgentStatus::Shell,
        )
        | None => Offer::Refuse(NO_AGENT),
    }
}

impl Lane {
    fn held(&self, now: f64) -> bool {
        self.hold_until.is_some_and(|until| now < until)
    }

    fn look(
        &mut self,
        status: Option<&AgentStatus>,
        screen: Screen,
        (origin, mid_turn): (Origin, bool),
        now: f64,
    ) -> Offer {
        if status != Some(&AgentStatus::Done) {
            self.hold_until = None;
        }
        let offer = decide(status, origin, self.held(now), mid_turn);
        if offer != Offer::Go || origin == Origin::Hand {
            return offer;
        }
        if status == Some(&AgentStatus::Working) {
            return if screen == Screen::Free { Offer::Go } else { Offer::Wait };
        }
        match screen {
            Screen::Free => {
                self.no_box_since = None;
                Offer::Go
            }
            Screen::Typed => Offer::Wait,
            Screen::NoBox => {
                let since = *self.no_box_since.get_or_insert(now);
                if now - since >= NO_BOX_PATIENCE_S { Offer::Go } else { Offer::Wait }
            }
        }
    }
}

impl Outbox {
    fn lane(&mut self, tab: u64) -> &mut Lane {
        self.lanes.entry(tab).or_default()
    }

    pub(crate) fn offer(
        &mut self,
        tab: u64,
        send: &Send,
        status: Option<&AgentStatus>,
        screen: Screen,
        now: f64,
    ) -> Offer {
        let lane = self.lane(tab);
        if send.counted() && lane.auto_sends >= MAX_AUTO_SENDS {
            return Offer::Paused;
        }
        lane.look(status, screen, (send.origin, send.mid_turn), now)
    }

    pub(crate) fn cap_for(&self, tab: u64, send: &Send) -> Option<Cap> {
        let own = send.tell.as_ref().and_then(|tell| tell.cap);
        own.or_else(|| {
            send.counted().then(|| Cap {
                n: self.lanes.get(&tab).map_or(0, |lane| lane.auto_sends) + 1,
                max: MAX_AUTO_SENDS,
            })
        })
    }

    pub(crate) fn sent(&mut self, tab: u64, send: &Send, now: f64) {
        let counted = send.counted();
        let lane = self.lane(tab);
        lane.hold_until = Some(now + HOLD_SECS);
        if counted {
            lane.auto_sends += 1;
        }
    }

    pub(crate) fn enqueue(&mut self, tab: u64, send: Send) {
        let lane = self.lane(tab);
        if lane.queue.len() >= QUEUE_MAX {
            lane.queue.pop_front();
        }
        lane.queue.push_back(send);
    }

    pub(crate) fn next_ready(
        &mut self,
        tab: u64,
        status: Option<&AgentStatus>,
        screen: Screen,
        now: f64,
    ) -> Option<Send> {
        let lane = self.lanes.get_mut(&tab)?;
        let front = lane.queue.front()?;
        let kind = (front.origin, front.mid_turn);
        match lane.look(status, screen, kind, now) {
            Offer::Go => lane.queue.pop_front(),
            Offer::Refuse(_) => {
                lane.queue.clear();
                None
            }
            Offer::Wait | Offer::Paused => None,
        }
    }

    pub(crate) fn held(&self, tab: u64, now: f64) -> bool {
        self.lanes.get(&tab).is_some_and(|lane| lane.held(now))
    }

    pub(crate) fn waiting_tabs(&self) -> Vec<u64> {
        self.lanes.iter().filter(|(_, lane)| !lane.queue.is_empty()).map(|(tab, _)| *tab).collect()
    }

    pub(crate) fn next_check(&self, now: f64) -> Option<f64> {
        self.lanes
            .values()
            .filter(|lane| !lane.queue.is_empty())
            .filter_map(|lane| lane.hold_until)
            .filter(|until| *until > now)
            .reduce(f64::min)
    }

    pub(crate) fn user_spoke(&mut self, tab: u64) {
        if let Some(lane) = self.lanes.get_mut(&tab) {
            lane.auto_sends = 0;
        }
    }

    pub(crate) fn paused(&self, tab: u64) -> bool {
        self.lanes.get(&tab).is_some_and(|lane| lane.auto_sends >= MAX_AUTO_SENDS)
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.lanes.remove(&tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::status::Reason as Why;

    const TAB: u64 = 4;

    fn sweep() -> Send {
        Send::auto("note", Reason::CommentGuard, "the branch has new comments")
    }

    fn offer(outbox: &mut Outbox, send: &Send, status: &AgentStatus, now: f64) -> Offer {
        outbox.offer(TAB, send, Some(status), Screen::Free, now)
    }

    #[test]
    fn the_status_decides_who_may_paste() {
        let cases = [
            (Some(AgentStatus::Done), Origin::Auto, Offer::Go),
            (Some(AgentStatus::Done), Origin::Hand, Offer::Go),
            (Some(AgentStatus::Working), Origin::Auto, Offer::Wait),
            (Some(AgentStatus::Working), Origin::Hand, Offer::Go),
            (Some(AgentStatus::NeedsYou(Why::Permission)), Origin::Auto, Offer::Wait),
            (Some(AgentStatus::NeedsYou(Why::Quiet)), Origin::Hand, Offer::Refuse(WAITING_ON_YOU)),
            (Some(AgentStatus::SettingUp), Origin::Auto, Offer::Refuse(NO_AGENT)),
            (Some(AgentStatus::Failed("x".into())), Origin::Hand, Offer::Refuse(NO_AGENT)),
            (Some(AgentStatus::Stopped), Origin::Auto, Offer::Refuse(NO_AGENT)),
            (Some(AgentStatus::Shell), Origin::Hand, Offer::Refuse(NO_AGENT)),
            (None, Origin::Auto, Offer::Refuse(NO_AGENT)),
        ];
        for (status, origin, want) in cases {
            let send = Send::untold("x", origin);
            let got = Outbox::default().offer(TAB, &send, status.as_ref(), Screen::Free, 0.0);
            assert_eq!(got, want, "{status:?} {origin:?}");
        }
    }

    #[test]
    fn a_mid_turn_send_goes_to_a_working_agent_only_while_its_input_box_is_free() {
        let send = Send::auto("x", Reason::LedgerGuard, "the report went stale").mid_turn();
        let cases = [
            (AgentStatus::Working, Screen::Free, Offer::Go),
            (AgentStatus::Working, Screen::Typed, Offer::Wait),
            (AgentStatus::Working, Screen::NoBox, Offer::Wait),
            (AgentStatus::NeedsYou(Why::Permission), Screen::Free, Offer::Wait),
            (AgentStatus::Stopped, Screen::Free, Offer::Refuse(NO_AGENT)),
        ];
        for (status, screen, want) in cases {
            let got = Outbox::default().offer(TAB, &send, Some(&status), screen, 0.0);
            assert_eq!(got, want, "{status:?} {screen:?}");
        }
        let plain = Send::auto("x", Reason::LedgerGuard, "the report went stale");
        let got =
            Outbox::default().offer(TAB, &plain, Some(&AgentStatus::Working), Screen::Free, 0.0);
        assert_eq!(got, Offer::Wait, "an automatic send without mid_turn still waits for idle");
    }

    #[test]
    fn a_mid_turn_send_with_its_own_cap_is_not_counted_or_paused_by_the_shared_cap() {
        let mut outbox = Outbox::default();
        let send = Send::auto("x", Reason::LedgerGuard, "the report went stale")
            .with_cap(Cap { n: 1, max: 4 })
            .mid_turn();
        for turn in 0..2 * MAX_AUTO_SENDS {
            outbox.sent(TAB, &send, f64::from(turn) * 100.0);
        }
        assert!(!outbox.paused(TAB));
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Working, 900.0), Offer::Go);
        assert_eq!(outbox.cap_for(TAB, &send), Some(Cap { n: 1, max: 4 }));
    }

    #[test]
    fn two_senders_in_one_frame_paste_once_and_the_other_waits() {
        let mut outbox = Outbox::default();
        let first = Send::auto("one", Reason::DueToday, "an item is due");
        let second = Send::auto("two", Reason::MeeseeksCheck, "the check failed");
        assert_eq!(offer(&mut outbox, &first, &AgentStatus::Done, 5.0), Offer::Go);
        outbox.sent(TAB, &first, 5.0);
        assert_eq!(offer(&mut outbox, &second, &AgentStatus::Done, 5.0), Offer::Wait);
        outbox.enqueue(TAB, second.clone());
        assert_eq!(outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, 5.0), None);
        assert_eq!(offer(&mut outbox, &second, &AgentStatus::Working, 6.0), Offer::Wait);
        let ready = outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, 9.0);
        assert_eq!(ready, Some(second));
    }

    #[test]
    fn the_hold_ends_after_ten_seconds_when_the_agent_never_left_done() {
        let mut outbox = Outbox::default();
        let send = Send::auto("one", Reason::DueToday, "an item is due");
        outbox.sent(TAB, &send, 0.0);
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, HOLD_SECS - 0.1), Offer::Wait);
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, HOLD_SECS), Offer::Go);
        assert_eq!(outbox.next_check(0.0), None);
    }

    #[test]
    fn leaving_done_ends_the_hold_at_once() {
        let mut outbox = Outbox::default();
        let send = Send::auto("one", Reason::DueToday, "an item is due");
        outbox.sent(TAB, &send, 0.0);
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Working, 1.0), Offer::Wait);
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, 2.0), Offer::Go);
    }

    #[test]
    fn three_automatic_sweeps_then_paused_until_the_user_speaks() {
        let mut outbox = Outbox::default();
        let send = sweep();
        let mut caps = Vec::new();
        for turn in 0..3 {
            let now = f64::from(turn) * 100.0;
            assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, now), Offer::Go);
            caps.push(outbox.cap_for(TAB, &send));
            outbox.sent(TAB, &send, now);
        }
        assert_eq!(
            caps,
            [Some(Cap { n: 1, max: 3 }), Some(Cap { n: 2, max: 3 }), Some(Cap { n: 3, max: 3 })]
        );
        assert!(outbox.paused(TAB));
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, 500.0), Offer::Paused);
        outbox.user_spoke(TAB);
        assert!(!outbox.paused(TAB));
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, 500.0), Offer::Go);
    }

    #[test]
    fn ledger_reminders_and_comment_notes_share_the_three_automatic_sends() {
        let mut outbox = Outbox::default();
        let reminder = Send::auto("ledger", Reason::LedgerGuard, "the ledger was not updated");
        let sends = [sweep(), reminder.clone(), sweep()];
        for (turn, send) in sends.iter().enumerate() {
            let now = turn as f64 * 100.0;
            assert_eq!(offer(&mut outbox, send, &AgentStatus::Done, now), Offer::Go);
            outbox.sent(TAB, send, now);
        }
        assert_eq!(
            outbox.cap_for(TAB, &reminder),
            Some(Cap { n: 4, max: 3 }),
            "the reminder is the fourth automatic send"
        );
        assert_eq!(offer(&mut outbox, &reminder, &AgentStatus::Done, 500.0), Offer::Paused);
        outbox.user_spoke(TAB);
        assert_eq!(offer(&mut outbox, &reminder, &AgentStatus::Done, 500.0), Offer::Go);
    }

    #[test]
    fn background_nudges_share_the_automatic_cap() {
        let mut outbox = Outbox::default();
        let nudge = Send::auto("x", Reason::BackgroundJob, "a background job has no progress");
        let sweep = sweep();
        outbox.sent(TAB, &nudge, 0.0);
        outbox.sent(TAB, &sweep, 100.0);
        assert_eq!(outbox.cap_for(TAB, &nudge), Some(Cap { n: 3, max: MAX_AUTO_SENDS }));
        outbox.sent(TAB, &nudge, 200.0);
        assert_eq!(offer(&mut outbox, &nudge, &AgentStatus::Done, 500.0), Offer::Paused);
        assert_eq!(offer(&mut outbox, &sweep, &AgentStatus::Done, 500.0), Offer::Paused);
    }

    #[test]
    fn hand_sends_and_sends_with_their_own_cap_are_not_counted() {
        let mut outbox = Outbox::default();
        let by_hand = Send::by_hand("x", Reason::DoneCheck, "you asked");
        let own = Send::auto("x", Reason::AutoFix, "Auto is on").with_cap(Cap { n: 1, max: 3 });
        for turn in 0..5 {
            let now = f64::from(turn) * 100.0;
            outbox.sent(TAB, &by_hand, now);
            outbox.sent(TAB, &own, now);
        }
        assert!(!outbox.paused(TAB));
        assert_eq!(outbox.cap_for(TAB, &own), Some(Cap { n: 1, max: 3 }));
        assert_eq!(outbox.cap_for(TAB, &by_hand), None);
    }

    #[test]
    fn the_queue_goes_out_oldest_first_one_per_idle_moment() {
        let mut outbox = Outbox::default();
        for text in ["a", "b", "c"] {
            outbox.enqueue(TAB, Send::untold(text, Origin::Auto));
        }
        assert_eq!(outbox.waiting_tabs(), [TAB]);
        let mut order = Vec::new();
        for turn in 0..3 {
            let now = f64::from(turn) * 100.0;
            let next = outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, now).unwrap();
            outbox.sent(TAB, &next, now);
            assert_eq!(outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, now), None);
            order.push(next.text);
        }
        assert_eq!(order, ["a", "b", "c"]);
        assert_eq!(outbox.waiting_tabs(), Vec::<u64>::new());
    }

    #[test]
    fn the_queue_is_bounded_and_dropped_when_the_agent_is_gone() {
        let mut outbox = Outbox::default();
        for n in 0..QUEUE_MAX + 3 {
            outbox.enqueue(TAB, Send::untold(&n.to_string(), Origin::Auto));
        }
        let first = outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, 0.0).unwrap();
        assert_eq!(first.text, "3");
        assert_eq!(outbox.next_ready(TAB, Some(&AgentStatus::Stopped), Screen::Free, 99.0), None);
        assert_eq!(outbox.waiting_tabs(), Vec::<u64>::new());
    }

    #[test]
    fn a_half_typed_draft_holds_automatic_sends_and_a_hand_send_goes() {
        let mut outbox = Outbox::default();
        let auto = Send::auto("x", Reason::DueToday, "an item is due");
        let by_hand = Send::by_hand("x", Reason::DueToday, "you asked");
        let done = Some(&AgentStatus::Done);
        assert_eq!(outbox.offer(TAB, &auto, done, Screen::Typed, 0.0), Offer::Wait);
        assert_eq!(outbox.offer(TAB, &by_hand, done, Screen::Typed, 0.0), Offer::Go);
        assert_eq!(outbox.offer(TAB, &auto, done, Screen::Free, 1.0), Offer::Go);
    }

    #[test]
    fn a_dialog_over_the_input_box_holds_for_a_while_and_then_gives_way() {
        let mut outbox = Outbox::default();
        let auto = Send::auto("x", Reason::DueToday, "an item is due");
        let done = Some(&AgentStatus::Done);
        assert_eq!(outbox.offer(TAB, &auto, done, Screen::NoBox, 0.0), Offer::Wait);
        assert_eq!(
            outbox.offer(TAB, &auto, done, Screen::NoBox, NO_BOX_PATIENCE_S - 1.0),
            Offer::Wait
        );
        assert_eq!(outbox.offer(TAB, &auto, done, Screen::NoBox, NO_BOX_PATIENCE_S), Offer::Go);
        assert_eq!(
            outbox.offer(TAB, &auto, done, Screen::Free, NO_BOX_PATIENCE_S + 1.0),
            Offer::Go
        );
        assert_eq!(
            outbox.offer(TAB, &auto, done, Screen::NoBox, NO_BOX_PATIENCE_S + 2.0),
            Offer::Wait
        );
    }

    #[test]
    fn a_closed_tab_forgets_its_lane() {
        let mut outbox = Outbox::default();
        outbox.enqueue(TAB, Send::untold("x", Origin::Auto));
        outbox.forget(TAB);
        assert_eq!(outbox.waiting_tabs(), Vec::<u64>::new());
    }
}
