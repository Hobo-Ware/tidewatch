//! The one gate every message Tidewatch types into an agent goes through (pure, table-tested): whether
//! the tab may take text now, one send per tab until the agent leaves Done or `HOLD_SECS` pass, what
//! waits in line behind that, and the tab's one budget of automatic sends, so no rules can loop with
//! an agent forever. The owner typing resets the budget.

use std::collections::{HashMap, VecDeque};

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
/// Automatic sends a supervised tab takes before the owner types again.
pub(crate) const SUPERVISED_BUDGET: u32 = 5;
/// Automatic sends a tab that runs on its own takes before the owner types again.
pub(crate) const AUTONOMOUS_BUDGET: u32 = 10;

pub(crate) fn budget(autonomous: bool) -> u32 {
    if autonomous { AUTONOMOUS_BUDGET } else { SUPERVISED_BUDGET }
}

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
            tell: Some(Tell { reason, because: because.to_owned() }),
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

    /// A message Tidewatch chose to send on its own: save mode holds it back and it spends the
    /// tab's budget.
    fn automatic(&self) -> bool {
        self.origin == Origin::Auto && self.tell.is_some()
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
    autonomous: bool,
    /// How many times the owner has written to this agent.
    spoke: u32,
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
    fn spent(&self) -> bool {
        self.auto_sends >= budget(self.autonomous)
    }

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

    /// Whether the tab runs on its own, which gives it the larger budget.
    pub(crate) fn set_autonomous(&mut self, tab: u64, autonomous: bool) {
        if autonomous || self.lanes.contains_key(&tab) {
            self.lane(tab).autonomous = autonomous;
        }
    }

    pub(crate) fn offer(
        &mut self,
        tab: u64,
        send: &Send,
        status: Option<&AgentStatus>,
        screen: Screen,
        now: f64,
    ) -> Offer {
        if self.saving && send.automatic() {
            return Offer::Saving;
        }
        let lane = self.lane(tab);
        if send.automatic() && lane.spent() {
            return Offer::Paused;
        }
        lane.look(status, screen, send.kind(), now)
    }

    /// The send's place in the tab's budget, for an automatic send.
    pub(crate) fn cap_for(&self, tab: u64, send: &Send) -> Option<Cap> {
        send.automatic().then(|| {
            let lane = self.lanes.get(&tab);
            Cap {
                n: lane.map_or(0, |lane| lane.auto_sends) + 1,
                max: budget(lane.is_some_and(|lane| lane.autonomous)),
            }
        })
    }

    pub(crate) fn sent(&mut self, tab: u64, send: &Send, now: f64) {
        let counted = send.automatic();
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
            lane.queue.retain(|send| !send.automatic());
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
        let lane = self.lane(tab);
        lane.auto_sends = 0;
        lane.spoke = lane.spoke.wrapping_add(1);
    }

    pub(crate) fn spoke(&self, tab: u64) -> u32 {
        self.lanes.get(&tab).map_or(0, |lane| lane.spoke)
    }

    pub(crate) fn paused(&self, tab: u64) -> bool {
        self.lanes.get(&tab).is_some_and(Lane::spent)
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
    fn five_automatic_sends_then_paused_until_the_user_speaks() {
        let mut outbox = Outbox::default();
        let send = sweep();
        let mut caps = Vec::new();
        for turn in 0..SUPERVISED_BUDGET {
            let now = f64::from(turn) * 100.0;
            assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, now), Offer::Go);
            caps.push(outbox.cap_for(TAB, &send));
            outbox.sent(TAB, &send, now);
        }
        let want: Vec<Option<Cap>> =
            (1..=5).map(|n| Some(Cap { n, max: SUPERVISED_BUDGET })).collect();
        assert_eq!(caps, want);
        assert!(outbox.paused(TAB));
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, 500.0), Offer::Paused);
        outbox.user_spoke(TAB);
        assert!(!outbox.paused(TAB));
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, 500.0), Offer::Go);
    }

    #[test]
    fn the_owner_speaking_is_counted_per_tab() {
        let mut outbox = Outbox::default();
        assert_eq!(outbox.spoke(TAB), 0);
        outbox.user_spoke(TAB);
        outbox.user_spoke(TAB);
        outbox.user_spoke(TAB + 1);
        assert_eq!((outbox.spoke(TAB), outbox.spoke(TAB + 1)), (2, 1));
    }

    #[test]
    fn every_automatic_send_counts_in_one_budget_whatever_its_reason() {
        let reasons = [
            Reason::AutoFix,
            Reason::PullRequest,
            Reason::MeeseeksCheck,
            Reason::DueToday,
            Reason::LedgerGuard,
            Reason::CommentGuard,
            Reason::DoneCheck,
            Reason::BackgroundJob,
            Reason::KeepGoing,
        ];
        let mut outbox = Outbox::default();
        for (turn, reason) in reasons.iter().take(SUPERVISED_BUDGET as usize).enumerate() {
            let send = Send::auto("x", *reason, "a rule fired");
            let now = turn as f64 * 100.0;
            assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, now), Offer::Go, "{reason:?}");
            assert_eq!(
                outbox.cap_for(TAB, &send),
                Some(Cap { n: turn as u32 + 1, max: SUPERVISED_BUDGET }),
                "{reason:?}"
            );
            outbox.sent(TAB, &send, now);
        }
        for reason in reasons {
            let send = Send::auto("x", reason, "a rule fired");
            assert_eq!(
                offer(&mut outbox, &send, &AgentStatus::Done, 900.0),
                Offer::Paused,
                "{reason:?} waits once the budget is spent"
            );
        }
        outbox.user_spoke(TAB);
        assert!(!outbox.paused(TAB));
        let send = Send::auto("x", Reason::AutoFix, "a rule fired");
        assert_eq!(offer(&mut outbox, &send, &AgentStatus::Done, 900.0), Offer::Go);
    }

    #[test]
    fn a_tab_that_runs_on_its_own_gets_ten_sends_and_a_supervised_one_five() {
        assert_eq!((budget(false), budget(true)), (5, 10));
        let mut outbox = Outbox::default();
        outbox.set_autonomous(TAB, true);
        let send = sweep();
        for turn in 0..AUTONOMOUS_BUDGET {
            assert!(!outbox.paused(TAB), "{turn}");
            outbox.sent(TAB, &send, f64::from(turn) * 100.0);
        }
        assert!(outbox.paused(TAB));
        assert_eq!(outbox.cap_for(TAB, &send), Some(Cap { n: 11, max: AUTONOMOUS_BUDGET }));
        outbox.user_spoke(TAB);
        assert!(!outbox.paused(TAB));
        outbox.set_autonomous(TAB, false);
        for turn in 0..SUPERVISED_BUDGET {
            outbox.sent(TAB, &send, f64::from(turn) * 100.0);
        }
        assert!(outbox.paused(TAB), "turning autonomy off brings the smaller budget back");
        outbox.set_autonomous(TAB + 1, false);
        assert_eq!(outbox.waiting_tabs(), Vec::<u64>::new());
        assert!(!outbox.paused(TAB + 1), "a supervised tab seen for the first time has no lane");
    }

    #[test]
    fn hand_sends_and_untold_sends_spend_nothing() {
        let mut outbox = Outbox::default();
        let by_hand = Send::by_hand("x", Reason::DoneCheck, "you asked");
        let untold = Send::untold("x", Origin::Auto);
        let reply = Send::reply("x");
        for turn in 0..2 * SUPERVISED_BUDGET {
            let now = f64::from(turn) * 100.0;
            for send in [&by_hand, &untold, &reply] {
                outbox.sent(TAB, send, now);
            }
        }
        assert!(!outbox.paused(TAB));
        for send in [&by_hand, &untold, &reply] {
            assert_eq!(outbox.cap_for(TAB, send), None);
        }
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
