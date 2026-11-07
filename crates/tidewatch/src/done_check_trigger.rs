//! When the done check runs (pure): once per turn that ends with a done claim, never again for the
//! same claim, and not for a tab that was already done when Tidewatch started watching it.

use std::hash::{DefaultHasher, Hash, Hasher};

use tidewatch_core::kind::Kind;
use tidewatch_core::progress::{Ledger, State};
use tidewatch_core::status::AgentStatus;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Claim(u64);

/// What the agent says about being done: only a report whose state is `done` claims it. The claim
/// is a digest of the report's content (goal, state, stages and steps, decisions, verified and not
/// verified), never of when it was saved, so a re-save of the same words is the same claim.
pub(crate) fn claim(ledger: Option<&Ledger>) -> Option<Claim> {
    let ledger = ledger.filter(|ledger| ledger.status.state == Some(State::Done))?;
    let mut hasher = DefaultHasher::new();
    format!(
        "{:?}{:?}{:?}{:?}{:?}",
        ledger.goal, ledger.checklist, ledger.decisions, ledger.verified, ledger.not_verified
    )
    .hash(&mut hasher);
    Some(Claim(hasher.finish()))
}

pub(crate) fn applies(ordinary: bool, kind: Option<Kind>) -> bool {
    ordinary && kind != Some(Kind::ReadOnly)
}

#[derive(Debug, Default)]
pub(crate) struct Trigger {
    worked: bool,
    last: Option<Claim>,
}

impl Trigger {
    pub(crate) fn observe(&mut self, status: &AgentStatus, claim: Option<Claim>) -> bool {
        match status {
            AgentStatus::Working => self.worked = true,
            AgentStatus::Done if self.worked => {
                self.worked = false;
                if let Some(claim) = claim.filter(|claim| self.last.as_ref() != Some(claim)) {
                    self.last = Some(claim);
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// The next turn that ends with the same words is a new claim (the user asked for a fix).
    pub(crate) fn forget_claim(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::status::Reason;

    fn report(state: &str, step: &str) -> Ledger {
        Ledger::parse(&format!(
            "# Goal\nShip\n\n## Status\nstate: {state}\n\n## Checklist\n### 1. Build\n- [x] {step}\n"
        ))
    }

    #[test]
    fn only_a_report_in_the_done_state_is_a_claim() {
        let cases = [
            (None, false),
            (Some(report("done", "a")), true),
            (Some(report("working", "a")), false),
            (Some(report("waiting", "a")), false),
            (Some(report("blocked", "a")), false),
            (Some(report("needs_you", "a")), false),
        ];
        for (ledger, want) in cases {
            assert_eq!(claim(ledger.as_ref()).is_some(), want, "{ledger:?}");
        }
    }

    #[test]
    fn the_same_content_is_one_claim_however_often_it_is_saved_and_a_changed_step_is_a_new_one() {
        let one = claim(Some(&report("done", "a")));
        let mut resaved = report("done", "a");
        resaved.status.note = "saved again".to_owned();
        let cases = [(report("done", "a"), true), (resaved, true), (report("done", "b"), false)];
        for (ledger, same) in cases {
            assert_eq!(claim(Some(&ledger)) == one, same, "{ledger:?}");
        }
    }

    #[test]
    fn a_read_only_report_skips_the_done_check() {
        let cases = [
            (true, None, true),
            (true, Some(Kind::Edit), true),
            (true, Some(Kind::ReadOnly), false),
            (false, None, false),
            (false, Some(Kind::Edit), false),
        ];
        for (ordinary, kind, want) in cases {
            assert_eq!(applies(ordinary, kind), want, "{ordinary} {kind:?}");
        }
    }

    fn some() -> Option<Claim> {
        claim(Some(&report("done", "a")))
    }

    #[test]
    fn it_fires_once_when_a_working_turn_ends_with_a_claim() {
        let mut trigger = Trigger::default();
        assert!(!trigger.observe(&AgentStatus::Working, None));
        assert!(trigger.observe(&AgentStatus::Done, some()));
        assert!(!trigger.observe(&AgentStatus::Done, some()));
    }

    #[test]
    fn a_tab_already_done_when_first_seen_is_not_checked() {
        let mut trigger = Trigger::default();
        assert!(!trigger.observe(&AgentStatus::SettingUp, some()));
        assert!(!trigger.observe(&AgentStatus::Done, some()));
    }

    #[test]
    fn the_same_claim_after_another_turn_is_not_checked_twice() {
        let mut trigger = Trigger::default();
        trigger.observe(&AgentStatus::Working, None);
        assert!(trigger.observe(&AgentStatus::Done, some()));
        trigger.observe(&AgentStatus::Working, None);
        assert!(!trigger.observe(&AgentStatus::Done, some()));
        trigger.observe(&AgentStatus::Working, None);
        assert!(trigger.observe(&AgentStatus::Done, claim(Some(&report("done", "b")))));
    }

    #[test]
    fn forgetting_the_claim_lets_the_same_words_fire_again() {
        let mut trigger = Trigger::default();
        trigger.observe(&AgentStatus::Working, None);
        assert!(trigger.observe(&AgentStatus::Done, some()));
        trigger.forget_claim();
        trigger.observe(&AgentStatus::Working, None);
        assert!(trigger.observe(&AgentStatus::Done, some()));
    }

    #[test]
    fn a_turn_without_a_claim_waits_for_the_next_one() {
        let mut trigger = Trigger::default();
        trigger.observe(&AgentStatus::Working, None);
        assert!(!trigger.observe(&AgentStatus::Done, None));
        trigger.observe(&AgentStatus::Working, None);
        assert!(trigger.observe(&AgentStatus::Done, some()));
    }

    #[test]
    fn a_question_in_the_middle_of_a_turn_does_not_end_it() {
        let mut trigger = Trigger::default();
        trigger.observe(&AgentStatus::Working, None);
        assert!(!trigger.observe(&AgentStatus::NeedsYou(Reason::Question), some()));
        assert!(trigger.observe(&AgentStatus::Done, some()));
    }
}
