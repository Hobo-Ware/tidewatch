//! The pull request card's state: Tidewatch asks the agent to open the pull request and only looks for it.

pub(crate) const RECHECK_SECS: f64 = 120.0;
pub(crate) const TYPED_GRACE_SECS: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    First,
    Between,
    Second,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Wait {
    Queued,
    Typed { at: f64 },
    Working,
    Idle { since: f64, look: Look },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Delivery {
    #[default]
    Hidden,
    Probing,
    Ready {
        commits: usize,
    },
    Asked {
        commits: usize,
        wait: Wait,
    },
    Missing {
        commits: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dot {
    Ready,
    Asked,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardView {
    pub(crate) text: String,
    pub(crate) dot: Dot,
    pub(crate) button: Option<String>,
}

impl Delivery {
    pub(crate) fn probe_started(&mut self) {
        if matches!(self, Self::Hidden | Self::Ready { .. }) {
            *self = Self::Probing;
        }
    }

    pub(crate) fn probe_done(&mut self, commits: Option<usize>) {
        if !matches!(self, Self::Probing) {
            return;
        }
        *self = match commits {
            Some(commits) if commits > 0 => Self::Ready { commits },
            _ => Self::Hidden,
        };
    }

    pub(crate) fn leave_done(&mut self) {
        if matches!(self, Self::Probing | Self::Ready { .. } | Self::Missing { .. }) {
            *self = Self::Hidden;
        }
    }

    pub(crate) fn hide(&mut self) {
        *self = Self::Hidden;
    }

    pub(crate) fn is_asked(&self) -> bool {
        matches!(self, Self::Asked { .. })
    }

    pub(crate) fn ask_started(&mut self) -> bool {
        let (Self::Ready { commits } | Self::Missing { commits }) = *self else { return false };
        *self = Self::Asked { commits, wait: Wait::Queued };
        true
    }

    pub(crate) fn asked(&mut self, now: f64) {
        if let Self::Asked { commits, wait: Wait::Queued } = *self {
            *self = Self::Asked { commits, wait: Wait::Typed { at: now } };
        }
    }

    pub(crate) fn not_asked(&mut self) {
        if let Self::Asked { commits, wait: Wait::Queued } = *self {
            *self = Self::Ready { commits };
        }
    }

    /// True when the pull request should be looked up now: the agent went idle after the ask, and
    /// again `RECHECK_SECS` later.
    pub(crate) fn watch(&mut self, idle: bool, now: f64) -> bool {
        let Self::Asked { commits, wait } = *self else { return false };
        let (next, look) = match wait {
            Wait::Typed { .. } | Wait::Idle { .. } if !idle => (Wait::Working, false),
            Wait::Working if idle => (Wait::Idle { since: now, look: Look::First }, true),
            Wait::Typed { at } if now - at >= TYPED_GRACE_SECS => {
                (Wait::Idle { since: now, look: Look::First }, true)
            }
            Wait::Idle { since, look: Look::Between } if now - since >= RECHECK_SECS => {
                (Wait::Idle { since, look: Look::Second }, true)
            }
            Wait::Queued | Wait::Working | Wait::Typed { .. } | Wait::Idle { .. } => return false,
        };
        *self = Self::Asked { commits, wait: next };
        look
    }

    pub(crate) fn answered(&mut self, found: bool) {
        let Self::Asked { commits, wait: Wait::Idle { since, look } } = *self else { return };
        *self = match (found, look) {
            (true, _) => Self::Hidden,
            (false, Look::First) => {
                Self::Asked { commits, wait: Wait::Idle { since, look: Look::Between } }
            }
            (false, Look::Second) => Self::Missing { commits },
            (false, Look::Between) => return,
        };
    }

    pub(crate) fn next_wake(&self) -> Option<f64> {
        match self {
            Self::Asked { wait: Wait::Typed { at }, .. } => Some(at + TYPED_GRACE_SECS),
            Self::Asked { wait: Wait::Idle { since, look: Look::Between }, .. } => {
                Some(since + RECHECK_SECS)
            }
            _ => None,
        }
    }

    pub(crate) fn card(&self, after_merge: bool, profile: &str) -> Option<CardView> {
        match self {
            Self::Ready { commits } => Some(CardView {
                text: card_text(*commits, after_merge),
                dot: Dot::Ready,
                button: Some(if after_merge {
                    format!("Ask {profile} to open a new PR")
                } else {
                    format!("Ask {profile} to open a PR")
                }),
            }),
            Self::Asked { .. } => Some(CardView {
                text: format!("Asked {profile} to open the pull request"),
                dot: Dot::Asked,
                button: None,
            }),
            Self::Missing { .. } => Some(CardView {
                text: "No pull request yet".to_owned(),
                dot: Dot::Missing,
                button: Some("Ask again".to_owned()),
            }),
            Self::Hidden | Self::Probing => None,
        }
    }
}

pub(crate) fn card_text(commits: usize, after_merge: bool) -> String {
    let noun = if commits == 1 { "commit" } else { "commits" };
    if after_merge {
        format!("{commits} {noun} after the merge")
    } else {
        format!("{commits} {noun} ready")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(commits: usize) -> Delivery {
        let mut card = Delivery::Hidden;
        card.probe_started();
        card.probe_done(Some(commits));
        card
    }

    fn asked_and_typed(at: f64) -> Delivery {
        let mut card = ready(2);
        assert!(card.ask_started());
        card.asked(at);
        card
    }

    #[test]
    fn the_card_appears_only_when_the_probe_found_commits() {
        assert_eq!(ready(2), Delivery::Ready { commits: 2 });
        assert_eq!(ready(0), Delivery::Hidden);
        let mut failed = Delivery::Hidden;
        failed.probe_started();
        failed.probe_done(None);
        assert_eq!(failed, Delivery::Hidden);
        let mut stray = Delivery::Hidden;
        stray.probe_done(Some(3));
        assert_eq!(stray, Delivery::Hidden);
    }

    #[test]
    fn an_ask_starts_only_from_a_ready_or_missing_card_and_a_refused_one_offers_it_again() {
        let mut hidden = Delivery::Hidden;
        assert!(!hidden.ask_started());
        let mut card = ready(2);
        assert!(card.ask_started());
        assert_eq!(card, Delivery::Asked { commits: 2, wait: Wait::Queued });
        assert!(!card.ask_started(), "one ask at a time");
        card.not_asked();
        assert_eq!(card, Delivery::Ready { commits: 2 });
        let mut missing = Delivery::Missing { commits: 3 };
        assert!(missing.ask_started());
        assert_eq!(missing, Delivery::Asked { commits: 3, wait: Wait::Queued });
    }

    #[test]
    fn typing_the_ask_starts_the_wait_and_nothing_is_looked_up_before_the_agent_is_idle_again() {
        let mut card = ready(2);
        card.ask_started();
        assert!(!card.watch(true, 1.0), "queued: the agent has not been told yet");
        card.asked(10.0);
        assert_eq!(card, Delivery::Asked { commits: 2, wait: Wait::Typed { at: 10.0 } });
        assert!(!card.watch(true, 11.0), "still idle one second after typing");
        assert!(!card.watch(false, 12.0));
        assert_eq!(card, Delivery::Asked { commits: 2, wait: Wait::Working });
        assert!(!card.watch(false, 40.0), "working: no lookup");
    }

    #[test]
    fn the_first_lookup_is_when_the_agent_goes_idle_and_the_second_two_minutes_later() {
        let mut card = asked_and_typed(0.0);
        card.watch(false, 1.0);
        assert!(card.watch(true, 50.0), "idle after work: look now");
        assert_eq!(
            card,
            Delivery::Asked { commits: 2, wait: Wait::Idle { since: 50.0, look: Look::First } }
        );
        assert!(!card.watch(true, 51.0), "the answer is awaited, not asked for again");
        card.answered(false);
        assert_eq!(
            card,
            Delivery::Asked { commits: 2, wait: Wait::Idle { since: 50.0, look: Look::Between } }
        );
        assert_eq!(card.next_wake(), Some(170.0));
        assert!(!card.watch(true, 169.9));
        assert!(card.watch(true, 170.0), "two minutes after going idle");
        card.answered(false);
        assert_eq!(card, Delivery::Missing { commits: 2 });
    }

    #[test]
    fn a_pull_request_found_by_either_lookup_ends_the_wait_with_no_card() {
        for second in [false, true] {
            let mut card = asked_and_typed(0.0);
            card.watch(false, 1.0);
            card.watch(true, 5.0);
            if second {
                card.answered(false);
                card.watch(true, 200.0);
            }
            card.answered(true);
            assert_eq!(card, Delivery::Hidden, "second lookup: {second}");
        }
    }

    #[test]
    fn an_agent_that_goes_back_to_work_restarts_the_count_from_its_next_idle_moment() {
        let mut card = asked_and_typed(0.0);
        card.watch(false, 1.0);
        card.watch(true, 5.0);
        card.answered(false);
        assert!(!card.watch(false, 60.0));
        assert_eq!(card, Delivery::Asked { commits: 2, wait: Wait::Working });
        assert!(card.watch(true, 100.0));
        assert_eq!(
            card,
            Delivery::Asked { commits: 2, wait: Wait::Idle { since: 100.0, look: Look::First } }
        );
    }

    #[test]
    fn an_agent_that_never_seemed_to_start_is_looked_at_once_the_grace_passes() {
        let mut card = asked_and_typed(10.0);
        assert_eq!(card.next_wake(), Some(10.0 + TYPED_GRACE_SECS));
        assert!(!card.watch(true, 10.0 + TYPED_GRACE_SECS - 0.1));
        assert!(card.watch(true, 10.0 + TYPED_GRACE_SECS));
    }

    #[test]
    fn nothing_waits_on_a_card_that_is_not_asked_and_answers_for_it_change_nothing() {
        let cards =
            [Delivery::Hidden, Delivery::Probing, ready(1), Delivery::Missing { commits: 1 }];
        for mut card in cards {
            let before = card.clone();
            assert!(!card.watch(true, 100.0));
            card.answered(false);
            card.asked(5.0);
            card.not_asked();
            assert_eq!(card, before);
            assert_eq!(card.next_wake(), None);
        }
    }

    #[test]
    fn work_clears_a_card_that_only_offered_the_last_stretch_but_never_an_ask_under_way() {
        let table = [
            (Delivery::Probing, false),
            (ready(2), false),
            (Delivery::Missing { commits: 2 }, false),
            (asked_and_typed(0.0), true),
        ];
        for (mut card, kept) in table {
            card.leave_done();
            assert_eq!(card != Delivery::Hidden, kept);
        }
    }

    #[test]
    fn the_card_says_what_it_is_waiting_for_and_offers_the_button_for_that_state() {
        let table = [
            (
                ready(2),
                false,
                Some(("2 commits ready", Dot::Ready, Some("Ask Builder to open a PR"))),
            ),
            (
                ready(1),
                true,
                Some((
                    "1 commit after the merge",
                    Dot::Ready,
                    Some("Ask Builder to open a new PR"),
                )),
            ),
            (
                asked_and_typed(0.0),
                false,
                Some(("Asked Builder to open the pull request", Dot::Asked, None)),
            ),
            (
                Delivery::Missing { commits: 2 },
                false,
                Some(("No pull request yet", Dot::Missing, Some("Ask again"))),
            ),
            (Delivery::Hidden, false, None),
            (Delivery::Probing, false, None),
        ];
        for (card, after_merge, want) in table {
            let got = card.card(after_merge, "Builder");
            let want = want.map(|(text, dot, button)| CardView {
                text: text.to_owned(),
                dot,
                button: button.map(str::to_owned),
            });
            assert_eq!(got, want, "{card:?}");
        }
    }

    #[test]
    fn the_card_text_counts_commits() {
        assert_eq!(card_text(1, false), "1 commit ready");
        assert_eq!(card_text(3, false), "3 commits ready");
        assert_eq!(card_text(1, true), "1 commit after the merge");
        assert_eq!(card_text(2, true), "2 commits after the merge");
    }
}
