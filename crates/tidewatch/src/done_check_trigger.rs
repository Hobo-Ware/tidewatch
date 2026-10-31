//! When the done check runs (pure): once per turn that ends with a done claim, never again for the
//! same claim, and not for a tab that was already done when Tidewatch started watching it.

use tidewatch_core::end_block::EndBlock;
use tidewatch_core::kind::Kind;
use tidewatch_core::status::AgentStatus;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Claim(String);

/// What the agent says about being done. Its ledger state is `done`, or its last message ends with
/// a Done / Needs you / Next block that lists something done and asks nothing of the user. A block
/// that asks for the user is no claim, whatever the ledger says.
pub(crate) fn claim(ledger_done: bool, block: Option<&EndBlock>) -> Option<Claim> {
    if block.is_some_and(EndBlock::needs_input) {
        return None;
    }
    let done: Vec<&str> = block
        .map(|block| block.done.iter().map(|item| item.trim()).filter(|i| !i.is_empty()).collect())
        .unwrap_or_default();
    (ledger_done || !done.is_empty()).then(|| Claim(format!("{ledger_done}\n{}", done.join("\n"))))
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

    fn block(done: &[&str], needs: &[&str]) -> EndBlock {
        EndBlock {
            done: done.iter().map(|s| (*s).to_owned()).collect(),
            needs_you: needs.iter().map(|s| (*s).to_owned()).collect(),
            next: Vec::new(),
        }
    }

    #[test]
    fn a_claim_is_a_done_ledger_or_a_done_section_that_asks_nothing() {
        let cases = [
            (false, None, false),
            (true, None, true),
            (false, Some(block(&["added the helper"], &["nothing"])), true),
            (false, Some(block(&[], &["nothing"])), false),
            (false, Some(block(&["  "], &["none"])), false),
            (true, Some(block(&["added the helper"], &["which name?"])), false),
            (false, Some(block(&["added the helper"], &["which name?"])), false),
            (true, Some(block(&[], &[])), true),
        ];
        for (ledger_done, block, want) in cases {
            assert_eq!(
                claim(ledger_done, block.as_ref()).is_some(),
                want,
                "{ledger_done} {block:?}"
            );
        }
    }

    #[test]
    fn different_words_or_ledger_state_make_a_different_claim() {
        let one = claim(false, Some(&block(&["a"], &[])));
        assert_eq!(one, claim(false, Some(&block(&["a"], &[]))));
        assert_ne!(one, claim(false, Some(&block(&["b"], &[]))));
        assert_ne!(one, claim(true, Some(&block(&["a"], &[]))));
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
        claim(true, None)
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
        assert!(trigger.observe(&AgentStatus::Done, claim(false, Some(&block(&["x"], &[])))));
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
