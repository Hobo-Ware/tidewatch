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
pub(crate) const HALF_TYPED: &str = "The agent's input box held a half-typed message for too long";
pub(crate) const LINE_FULL: &str = "Too many messages are already waiting for this agent";
const NO_BOX_PATIENCE_S: f64 = 30.0;
const TYPED_PATIENCE_S: f64 = 120.0;
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
    pub(crate) reply: bool,
}

#[derive(Clone, Copy, Debug)]
struct Kind {
    origin: Origin,
    mid_turn: bool,
    reply: bool,
    command: bool,
}

impl Kind {
    fn holds(self) -> bool {
        self.reply && !self.command && !self.mid_turn
    }

    fn reads_screen_mid_turn(self) -> bool {
        self.mid_turn && !self.command
    }
}

impl Send {
    pub(crate) fn auto(text: &str, reason: Reason, because: &str) -> Self {
        Self {
            text: text.to_owned(),
            origin: Origin::Auto,
            tell: Some(Tell { reason, because: because.to_owned(), cap: None }),
            mid_turn: false,
            reply: false,
        }
    }

    pub(crate) fn by_hand(text: &str, reason: Reason, because: &str) -> Self {
        Self { origin: Origin::Hand, ..Self::auto(text, reason, because) }
    }

    pub(crate) fn untold(text: &str, origin: Origin) -> Self {
        Self { text: text.to_owned(), origin, tell: None, mid_turn: false, reply: false }
    }

    /// A message the owner typed: it waits while the agent works and goes when the turn is over,
    /// except a slash command, which runs at once. For an agent that takes typed text mid-turn,
    /// add `mid_turn`: the reply then goes while it works, as soon as its input box is free.
    pub(crate) fn reply(text: &str) -> Self {
        Self { reply: true, ..Self::untold(text, Origin::Hand) }
    }

    fn kind(&self) -> Kind {
        Kind {
            origin: self.origin,
            mid_turn: self.mid_turn,
            reply: self.reply,
            command: crate::send_line::is_command(&self.text),
        }
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

    /// A message Tidewatch chose to send on its own, which save mode holds back. An answer to a
    /// question the agent waits on is not one: it spends nothing the agent had not already begun.
    fn held_by_save_mode(&self) -> bool {
        self.origin == Origin::Auto
            && self.tell.as_ref().is_some_and(|tell| tell.reason != Reason::AutoPick)
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
    /// Save mode is holding back what Tidewatch sends on its own.
    Saving,
    Refuse(&'static str),
}

/// Messages that left a tab's line without being typed, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dropped {
    pub(crate) count: usize,
    pub(crate) why: &'static str,
}

impl Dropped {
    /// The line the owner sees when messages left a tab's line without being typed.
    pub(crate) fn line(self) -> String {
        let what = if self.count == 1 {
            "A queued message was".to_owned()
        } else {
            format!("{} queued messages were", self.count)
        };
        format!("{what} not sent: {}", self.why)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ready {
    Nothing,
    Send(Send),
    Dropped(Dropped),
}

#[derive(Debug, Default)]
struct Lane {
    hold_until: Option<f64>,
    queue: VecDeque<Send>,
    auto_sends: u32,
    no_box_since: Option<f64>,
    typed_since: Option<f64>,
}

#[derive(Debug, Default)]
pub(crate) struct Outbox {
    lanes: HashMap<u64, Lane>,
    saving: bool,
}

fn decide(status: Option<&AgentStatus>, kind: Kind, held: bool) -> Offer {
    match status {
        Some(AgentStatus::Done) if held && !kind.command => Offer::Wait,
        Some(AgentStatus::Done) => Offer::Go,
        Some(AgentStatus::Working) if kind.holds() => Offer::Wait,
        Some(AgentStatus::Working) if kind.origin == Origin::Hand || kind.mid_turn => Offer::Go,
        Some(AgentStatus::NeedsYou(_)) if kind.reply => Offer::Go,
        Some(AgentStatus::NeedsYou(_)) if kind.origin == Origin::Hand => {
            Offer::Refuse(WAITING_ON_YOU)
        }
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
        kind: Kind,
        now: f64,
    ) -> Offer {
        if status != Some(&AgentStatus::Done) {
            self.hold_until = None;
        }
        let offer = decide(status, kind, self.held(now));
        if offer != Offer::Go {
            return offer;
        }
        if status == Some(&AgentStatus::Working) && kind.reads_screen_mid_turn() {
            return if screen == Screen::Free { Offer::Go } else { Offer::Wait };
        }
        if kind.origin == Origin::Hand {
            return offer;
        }
        match screen {
            Screen::Free => {
                self.no_box_since = None;
                self.typed_since = None;
                Offer::Go
            }
            Screen::Typed => {
                let since = *self.typed_since.get_or_insert(now);
                if now - since >= TYPED_PATIENCE_S {
                    self.typed_since = None;
                    Offer::Refuse(HALF_TYPED)
                } else {
                    Offer::Wait
                }
            }
            Screen::NoBox => {
                self.typed_since = None;
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

    pub(crate) fn set_saving(&mut self, saving: bool) {
        self.saving = saving;
    }

    pub(crate) fn offer(
        &mut self,
        tab: u64,
        send: &Send,
        status: Option<&AgentStatus>,
        screen: Screen,
        now: f64,
    ) -> Offer {
        if self.saving && send.held_by_save_mode() {
            return Offer::Saving;
        }
        let lane = self.lane(tab);
        if send.counted() && lane.auto_sends >= MAX_AUTO_SENDS {
            return Offer::Paused;
        }
        lane.look(status, screen, send.kind(), now)
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
        let command = send.kind().command && send.reply;
        let lane = self.lane(tab);
        if !command {
            lane.hold_until = Some(now + HOLD_SECS);
        }
        if counted {
            lane.auto_sends += 1;
        }
    }

    /// Adds `send` to the tab's line. A full line gives the send back: the oldest is never
    /// dropped to make room.
    pub(crate) fn enqueue(&mut self, tab: u64, send: Send) -> Result<(), Send> {
        let lane = self.lane(tab);
        if lane.queue.len() >= QUEUE_MAX {
            return Err(send);
        }
        lane.queue.push_back(send);
        Ok(())
    }

    pub(crate) fn next_ready(
        &mut self,
        tab: u64,
        status: Option<&AgentStatus>,
        screen: Screen,
        now: f64,
    ) -> Ready {
        let Some(lane) = self.lanes.get_mut(&tab) else { return Ready::Nothing };
        if self.saving {
            lane.queue.retain(|send| !send.held_by_save_mode());
        }
        let Some(front) = lane.queue.front() else { return Ready::Nothing };
        let kind = front.kind();
        match lane.look(status, screen, kind, now) {
            Offer::Go => lane.queue.pop_front().map_or(Ready::Nothing, Ready::Send),
            Offer::Refuse(why) => {
                let count = if why == HALF_TYPED { 1 } else { lane.queue.len() };
                lane.queue.drain(..count);
                Ready::Dropped(Dropped { count, why })
            }
            Offer::Wait | Offer::Paused | Offer::Saving => Ready::Nothing,
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
        assert_eq!(outbox.enqueue(TAB, second.clone()), Ok(()));
        assert_eq!(
            outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, 5.0),
            Ready::Nothing
        );
        assert_eq!(offer(&mut outbox, &second, &AgentStatus::Working, 6.0), Offer::Wait);
        let ready = outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, 9.0);
        assert_eq!(ready, Ready::Send(second));
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
            assert_eq!(outbox.enqueue(TAB, Send::untold(text, Origin::Auto)), Ok(()));
        }
        assert_eq!(outbox.waiting_tabs(), [TAB]);
        let mut order = Vec::new();
        for turn in 0..3 {
            let now = f64::from(turn) * 100.0;
            let Ready::Send(next) =
                outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, now)
            else {
                panic!("a message was due");
            };
            outbox.sent(TAB, &next, now);
            assert_eq!(
                outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, now),
                Ready::Nothing
            );
            order.push(next.text);
        }
        assert_eq!(order, ["a", "b", "c"]);
        assert_eq!(outbox.waiting_tabs(), Vec::<u64>::new());
    }

    #[test]
    fn a_full_line_refuses_the_new_send_and_keeps_every_older_one() {
        let mut outbox = Outbox::default();
        for n in 0..QUEUE_MAX {
            assert_eq!(outbox.enqueue(TAB, Send::untold(&n.to_string(), Origin::Auto)), Ok(()));
        }
        let extra = Send::untold("one too many", Origin::Auto);
        assert_eq!(outbox.enqueue(TAB, extra.clone()), Err(extra));
        let done = Some(&AgentStatus::Done);
        let Ready::Send(first) = outbox.next_ready(TAB, done, Screen::Free, 0.0) else {
            panic!("the oldest was due");
        };
        assert_eq!(first.text, "0");
    }

    #[test]
    fn a_line_that_cannot_be_typed_says_how_many_left_it_and_why() {
        let mut outbox = Outbox::default();
        for n in 0..3 {
            assert_eq!(outbox.enqueue(TAB, Send::untold(&n.to_string(), Origin::Auto)), Ok(()));
        }
        let gone = outbox.next_ready(TAB, Some(&AgentStatus::Stopped), Screen::Free, 99.0);
        assert_eq!(gone, Ready::Dropped(Dropped { count: 3, why: NO_AGENT }));
        assert_eq!(outbox.waiting_tabs(), Vec::<u64>::new());
    }

    #[test]
    fn the_line_for_a_drop_names_how_many_and_why() {
        let one = Dropped { count: 1, why: NO_AGENT }.line();
        assert_eq!(one, "A queued message was not sent: There is no running agent in this tab");
        let many = Dropped { count: 3, why: HALF_TYPED }.line();
        assert!(many.starts_with("3 queued messages were not sent: "), "{many}");
    }

    #[test]
    fn a_wait_on_a_half_typed_draft_ends_with_a_line_that_says_so_and_only_for_that_send() {
        let mut outbox = Outbox::default();
        let auto = Send::auto("x", Reason::DueToday, "an item is due");
        let done = Some(&AgentStatus::Done);
        assert_eq!(outbox.offer(TAB, &auto, done, Screen::Typed, 0.0), Offer::Wait);
        assert_eq!(
            outbox.offer(TAB, &auto, done, Screen::Typed, TYPED_PATIENCE_S - 1.0),
            Offer::Wait
        );
        assert_eq!(
            outbox.offer(TAB, &auto, done, Screen::Typed, TYPED_PATIENCE_S),
            Offer::Refuse(HALF_TYPED)
        );
        assert_eq!(outbox.offer(TAB, &auto, done, Screen::Free, TYPED_PATIENCE_S), Offer::Go);
    }

    #[test]
    fn a_queued_send_that_waited_too_long_on_a_draft_is_dropped_alone() {
        let mut outbox = Outbox::default();
        for text in ["a", "b"] {
            assert_eq!(outbox.enqueue(TAB, Send::untold(text, Origin::Auto)), Ok(()));
        }
        let done = Some(&AgentStatus::Done);
        assert_eq!(outbox.next_ready(TAB, done, Screen::Typed, 0.0), Ready::Nothing);
        let late = outbox.next_ready(TAB, done, Screen::Typed, TYPED_PATIENCE_S);
        assert_eq!(late, Ready::Dropped(Dropped { count: 1, why: HALF_TYPED }));
        assert_eq!(outbox.waiting_tabs(), [TAB]);
        let Ready::Send(next) = outbox.next_ready(TAB, done, Screen::Free, TYPED_PATIENCE_S) else {
            panic!("the second send was still waiting");
        };
        assert_eq!(next.text, "b");
    }

    #[test]
    fn a_reply_waits_while_the_agent_works_and_goes_when_it_is_done() {
        let reply = Send::reply("also check the Android build");
        let cases = [
            (AgentStatus::Working, Offer::Wait),
            (AgentStatus::Done, Offer::Go),
            (AgentStatus::NeedsYou(Why::Quiet), Offer::Go),
            (AgentStatus::Stopped, Offer::Refuse(NO_AGENT)),
            (AgentStatus::Shell, Offer::Refuse(NO_AGENT)),
        ];
        for (status, want) in cases {
            let got = Outbox::default().offer(TAB, &reply, Some(&status), Screen::Free, 0.0);
            assert_eq!(got, want, "{status:?}");
        }
    }

    #[test]
    fn a_reply_to_a_working_claude_goes_at_once_only_while_its_input_box_is_free() {
        let reply = Send::reply("also check the Android build").mid_turn();
        let cases = [
            (AgentStatus::Working, Screen::Free, Offer::Go),
            (AgentStatus::Working, Screen::Typed, Offer::Wait),
            (AgentStatus::Working, Screen::NoBox, Offer::Wait),
            (AgentStatus::Done, Screen::Free, Offer::Go),
            (AgentStatus::Done, Screen::Typed, Offer::Go),
            (AgentStatus::NeedsYou(Why::Quiet), Screen::Free, Offer::Go),
            (AgentStatus::Stopped, Screen::Free, Offer::Refuse(NO_AGENT)),
        ];
        for (status, screen, want) in cases {
            let got = Outbox::default().offer(TAB, &reply, Some(&status), screen, 0.0);
            assert_eq!(got, want, "{status:?} {screen:?}");
        }
    }

    #[test]
    fn a_reply_to_a_working_cli_that_cannot_take_it_mid_turn_still_waits_for_the_turn_to_end() {
        let reply = Send::reply("also check the Android build");
        for screen in [Screen::Free, Screen::Typed, Screen::NoBox] {
            let got =
                Outbox::default().offer(TAB, &reply, Some(&AgentStatus::Working), screen, 0.0);
            assert_eq!(got, Offer::Wait, "{screen:?}");
        }
        let done =
            Outbox::default().offer(TAB, &reply, Some(&AgentStatus::Done), Screen::Free, 0.0);
        assert_eq!(done, Offer::Go);
    }

    #[test]
    fn a_slash_command_to_a_working_claude_goes_whatever_its_input_box_holds() {
        let command = Send::reply("/model opus").mid_turn();
        for screen in [Screen::Free, Screen::Typed, Screen::NoBox] {
            let got =
                Outbox::default().offer(TAB, &command, Some(&AgentStatus::Working), screen, 0.0);
            assert_eq!(got, Offer::Go, "{screen:?}");
        }
    }

    #[test]
    fn a_reply_never_merges_with_the_one_before_it() {
        let mut outbox = Outbox::default();
        let (first, second) = (Send::reply("one"), Send::reply("two"));
        assert_eq!(offer(&mut outbox, &first, &AgentStatus::Done, 0.0), Offer::Go);
        outbox.sent(TAB, &first, 0.0);
        assert_eq!(offer(&mut outbox, &second, &AgentStatus::Done, 1.0), Offer::Wait);
        assert_eq!(offer(&mut outbox, &second, &AgentStatus::Working, 2.0), Offer::Wait);
        assert_eq!(offer(&mut outbox, &second, &AgentStatus::Done, 3.0), Offer::Go);
    }

    #[test]
    fn a_slash_command_runs_at_once_and_does_not_hold_the_next_reply() {
        let mut outbox = Outbox::default();
        let command = Send::reply("/model opus");
        assert_eq!(offer(&mut outbox, &command, &AgentStatus::Working, 0.0), Offer::Go);
        outbox.sent(TAB, &command, 0.0);
        let next = Send::reply("go on");
        assert_eq!(offer(&mut outbox, &next, &AgentStatus::Done, 1.0), Offer::Go);
    }

    #[test]
    fn save_mode_holds_what_tidewatch_chose_to_send_and_nothing_the_owner_sent() {
        let mut outbox = Outbox::default();
        outbox.set_saving(true);
        let done = Some(&AgentStatus::Done);
        let held = [
            Reason::AutoFix,
            Reason::AutoPush,
            Reason::DueToday,
            Reason::LedgerGuard,
            Reason::CommentGuard,
            Reason::DoneCheck,
            Reason::StuckJob,
            Reason::BackgroundJob,
            Reason::Avatar,
        ];
        for reason in held {
            let send = Send::auto("x", reason, "a rule fired");
            assert_eq!(
                outbox.offer(TAB, &send, done, Screen::Free, 0.0),
                Offer::Saving,
                "{reason:?}"
            );
        }
        let goes = [
            ("a hand send", Send::by_hand("x", Reason::DoneCheck, "you asked")),
            ("an untold review", Send::untold("x", Origin::Auto)),
            ("a pick for a waiting agent", Send::auto("x", Reason::AutoPick, "nobody answered")),
        ];
        for (what, send) in goes {
            assert_eq!(outbox.offer(TAB, &send, done, Screen::Free, 0.0), Offer::Go, "{what}");
        }
        outbox.set_saving(false);
        let send = Send::auto("x", Reason::AutoFix, "a rule fired");
        assert_eq!(outbox.offer(TAB, &send, done, Screen::Free, 0.0), Offer::Go);
    }

    #[test]
    fn save_mode_empties_the_line_of_automatic_sends_but_keeps_the_owners() {
        let mut outbox = Outbox::default();
        assert_eq!(
            outbox.enqueue(TAB, Send::auto("auto", Reason::CommentGuard, "new comments")),
            Ok(())
        );
        assert_eq!(outbox.enqueue(TAB, Send::untold("review", Origin::Auto)), Ok(()));
        outbox.set_saving(true);
        let Ready::Send(next) = outbox.next_ready(TAB, Some(&AgentStatus::Done), Screen::Free, 0.0)
        else {
            panic!("the review was not ready");
        };
        assert_eq!(next.text, "review");
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
        assert_eq!(outbox.enqueue(TAB, Send::untold("x", Origin::Auto)), Ok(()));
        outbox.forget(TAB);
        assert_eq!(outbox.waiting_tabs(), Vec::<u64>::new());
    }
}
