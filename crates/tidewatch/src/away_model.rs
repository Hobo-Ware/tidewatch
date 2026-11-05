//! What flipping the Away toggle does, as plain data the app host and the tests share.

use std::collections::HashMap;

use tidewatch_core::progress::{Ledger, State};
use tidewatch_core::sent::because;
use tidewatch_core::status::AgentStatus;

use crate::comment_model::Turn;

/// What a click on the Away pill changes: the new Away value, and whether keep-awake has to be
/// started with it. Turning Away off never touches keep-awake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Flip {
    pub(crate) away: bool,
    pub(crate) start_keep_awake: bool,
}

/// Whether the owner is at the keyboard. An enum keeps the app struct free of one more loose bool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Presence {
    #[default]
    Here,
    Away,
}

impl Presence {
    pub(crate) const fn of(away: bool) -> Self {
        if away { Self::Away } else { Self::Here }
    }

    pub(crate) const fn is_away(self) -> bool {
        matches!(self, Self::Away)
    }
}

pub(crate) const fn flip(away: bool, keep_awake_on: bool) -> Flip {
    let away = !away;
    Flip { away, start_keep_awake: away && !keep_awake_on }
}

pub(crate) const TOLD_ON: &str = "The user is away now. Make your own decisions from here instead of asking, and write each one with its reason under Decisions in your progress report. This Mac stays awake, also with the lid closed.";
pub(crate) const TOLD_OFF: &str =
    "The user is back. Ask with your question tool when a choice needs them.";

/// What a flip tells a running agent, and the `because` its row in the Reader shows.
pub(crate) fn told(away: bool) -> (&'static str, &'static str) {
    if away { (TOLD_ON, because::AWAY_ON) } else { (TOLD_OFF, because::AWAY_OFF) }
}

/// The tabs that hear about a flip: an agent that runs. A shell tab, a stopped agent, one that
/// failed or is still setting up has nobody to read it.
pub(crate) fn hearers<'a>(
    tabs: impl IntoIterator<Item = (u64, Option<&'a AgentStatus>)>,
) -> Vec<u64> {
    tabs.into_iter()
        .filter(|(_, status)| {
            matches!(
                status,
                Some(AgentStatus::Working | AgentStatus::NeedsYou(_) | AgentStatus::Done)
            )
        })
        .map(|(id, _)| id)
        .collect()
}

pub(crate) const KEEP_GOING: &str = "The user is away. Keep going with the next open step in your progress report; decide yourself and record why.";
pub(crate) const MAX_KEEP_GOING: u32 = 3;

/// What the progress report says about work left, as far as keep-going cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Open {
    state: Option<State>,
    steps_todo: bool,
}

impl Open {
    pub(crate) fn of(ledger: &Ledger) -> Self {
        let counts = ledger.counts();
        Self { state: ledger.status.state, steps_todo: counts.done < counts.total }
    }
}

/// What a turn end means for the keep-going chain: send the message or not, and the new count of
/// messages in a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Next {
    pub(crate) send: bool,
    pub(crate) in_a_row: u32,
}

/// A turn ended with Away on or off. Done, blocked, or nothing left to do ends the chain; a chain
/// that reached the cap stays quiet until the owner speaks.
pub(crate) fn keep_going(away: bool, report: Option<Open>, in_a_row: u32) -> Next {
    let end = Next { send: false, in_a_row: 0 };
    let Some(report) = report.filter(|_| away) else { return end };
    if matches!(report.state, Some(State::Done | State::Blocked)) || !report.steps_todo {
        return end;
    }
    if in_a_row >= MAX_KEEP_GOING {
        return Next { send: false, in_a_row };
    }
    Next { send: true, in_a_row: in_a_row + 1 }
}

/// Sees a tab's status each frame and says once when a working agent has just stopped.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    turns: HashMap<u64, Turn>,
}

impl Watch {
    pub(crate) fn ended(&mut self, tab: u64, status: &AgentStatus) -> bool {
        self.turns.entry(tab).or_default().ended(status)
    }

    pub(crate) fn keep_only(&mut self, live: &[u64]) {
        self.turns.retain(|tab, _| live.contains(tab));
    }
}

pub(crate) const TIP: &str =
    "Away: agents decide on their own and keep working. Also keeps the Mac awake.";

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_wraps)]
    fn open(state: Option<State>, steps_todo: bool) -> Option<Open> {
        Some(Open { state, steps_todo })
    }

    #[test]
    fn keep_going_follows_away_the_report_and_the_cap() {
        let cases = [
            ("away off", false, open(Some(State::Working), true), 1, false, 0),
            ("no report", true, None, 1, false, 0),
            ("done", true, open(Some(State::Done), true), 2, false, 0),
            ("blocked", true, open(Some(State::Blocked), true), 2, false, 0),
            ("no steps left", true, open(Some(State::Working), false), 2, false, 0),
            ("open and working", true, open(Some(State::Working), true), 0, true, 1),
            ("open and needs you", true, open(Some(State::NeedsYou), true), 1, true, 2),
            ("open and no state", true, open(None, true), 2, true, 3),
            ("cap reached", true, open(Some(State::Working), true), 3, false, 3),
            ("cap passed", true, open(Some(State::Waiting), true), 7, false, 7),
        ];
        for (what, away, report, in_a_row, send, now) in cases {
            assert_eq!(keep_going(away, report, in_a_row), Next { send, in_a_row: now }, "{what}");
        }
    }

    #[test]
    fn a_chain_runs_three_messages_then_stops_and_the_owner_speaking_starts_it_again() {
        let report = open(Some(State::Working), true);
        let mut count = 0;
        let mut sent = 0;
        for _ in 0..6 {
            let next = keep_going(true, report, count);
            sent += u32::from(next.send);
            count = next.in_a_row;
        }
        assert_eq!(sent, MAX_KEEP_GOING);
        count = 0;
        assert!(keep_going(true, report, count).send);
    }

    #[test]
    fn a_turn_end_is_seen_once_per_turn_and_per_tab() {
        let mut watch = Watch::default();
        let (working, done) = (AgentStatus::Working, AgentStatus::Done);
        assert!(!watch.ended(1, &done));
        assert!(!watch.ended(1, &working));
        assert!(!watch.ended(2, &done), "another tab has its own turn");
        assert!(watch.ended(1, &done));
        assert!(!watch.ended(1, &done), "not again while it stays done");
        watch.keep_only(&[2]);
        assert!(!watch.ended(1, &done));
    }

    #[test]
    fn the_keep_going_text_asks_for_the_next_step_and_never_names_an_option() {
        assert!(KEEP_GOING.contains("next open step in your progress report"));
        assert!(!KEEP_GOING.contains('\u{2014}') && !KEEP_GOING.contains('\u{2013}'));
        assert!(!KEEP_GOING.to_lowercase().contains("recommend"));
    }

    #[test]
    fn only_a_running_agent_hears_about_a_flip() {
        use tidewatch_core::status::Reason;
        let working = AgentStatus::Working;
        let asking = AgentStatus::NeedsYou(Reason::Question);
        let done = AgentStatus::Done;
        let failed = AgentStatus::Failed("x".into());
        let tabs = [
            (1, Some(&working)),
            (2, None),
            (3, Some(&AgentStatus::Shell)),
            (4, Some(&AgentStatus::Stopped)),
            (5, Some(&done)),
            (6, Some(&failed)),
            (7, Some(&asking)),
            (8, Some(&AgentStatus::SettingUp)),
        ];
        assert_eq!(hearers(tabs), [1, 5, 7]);
    }

    #[test]
    fn the_messages_say_who_decides_and_never_name_an_option() {
        let (on, why_on) = told(true);
        let (off, why_off) = told(false);
        assert!(on.contains("Make your own decisions") && on.contains("lid closed"));
        assert!(off.contains("Ask with your question tool"));
        assert_eq!((why_on, why_off), ("you turned Away on", "you turned Away off"));
        for text in [on, off] {
            assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
            assert!(!text.to_lowercase().contains("recommend"));
        }
    }

    #[test]
    fn turning_away_on_starts_keep_awake_only_when_it_is_off() {
        let cases = [
            (false, false, Flip { away: true, start_keep_awake: true }),
            (false, true, Flip { away: true, start_keep_awake: false }),
            (true, false, Flip { away: false, start_keep_awake: false }),
            (true, true, Flip { away: false, start_keep_awake: false }),
        ];
        for (away, keep_awake_on, want) in cases {
            assert_eq!(flip(away, keep_awake_on), want, "away {away}, awake {keep_awake_on}");
        }
    }
}
