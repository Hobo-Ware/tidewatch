//! What flipping the Away toggle does, as plain data the app host and the tests share.

use tidewatch_core::sent::because;
use tidewatch_core::status::AgentStatus;

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

pub(crate) const TIP: &str =
    "Away: agents decide on their own and keep working. Also keeps the Mac awake.";

#[cfg(test)]
mod tests {
    use super::*;

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
