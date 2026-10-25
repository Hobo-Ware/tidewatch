//! What the agent's last turn left for the owner, as plain data for the Progress panel: the Done /
//! Needs you / Next block of its last message and the done check. [`LastTurn::attention`] names the
//! part that wants a look, so the panel's tab can carry a dot until the section has been shown.

use std::hash::{DefaultHasher, Hash, Hasher};

use tidewatch_core::end_block::EndBlock;

use crate::done_check_glue::Look;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LastTurn {
    pub(crate) done: Vec<String>,
    pub(crate) needs: Vec<String>,
    pub(crate) next: Vec<String>,
    pub(crate) check: Option<Look>,
    pub(crate) can_ask: Result<(), &'static str>,
    pub(crate) owner_asks: bool,
}

impl LastTurn {
    /// The last turn of a tab, `None` when it has neither a summary block nor a done check.
    pub(crate) fn of(
        block: Option<&EndBlock>,
        check: Option<Look>,
        can_ask: Result<(), &'static str>,
    ) -> Option<Self> {
        if block.is_none() && check.is_none() {
            return None;
        }
        let block = block.cloned().unwrap_or_default();
        let needs = if block.needs_input() { block.needs_you } else { Vec::new() };
        Some(Self { done: block.done, needs, next: block.next, check, can_ask, owner_asks: true })
    }

    /// A number for the Needs you items and the done check's report, `None` when there are none.
    /// It changes whenever either does, which is what makes them new to the owner.
    pub(crate) fn attention(&self) -> Option<u64> {
        let report = match &self.check {
            Some(Look::Report(report)) => Some(report),
            Some(Look::Checking) | None => None,
        };
        if self.needs.is_empty() && report.is_none() {
            return None;
        }
        let mut hasher = DefaultHasher::new();
        format!("{:?}{report:?}", self.needs).hash(&mut hasher);
        Some(hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::done_check::{Kind, Line, Report, Status};

    fn block(done: &[&str], needs: &[&str], next: &[&str]) -> EndBlock {
        let own = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
        EndBlock { done: own(done), needs_you: own(needs), next: own(next) }
    }

    fn report(detail: &str) -> Look {
        Look::Report(Report {
            head: String::new(),
            lines: vec![Line {
                kind: Kind::Pushed,
                status: Status::Fail,
                detail: detail.to_owned(),
            }],
            unverified: Vec::new(),
        })
    }

    #[test]
    fn a_tab_with_no_summary_and_no_check_has_no_last_turn() {
        assert_eq!(LastTurn::of(None, None, Ok(())), None);
    }

    #[test]
    fn nothing_to_ask_reads_as_no_needs_you_items() {
        let turn = LastTurn::of(Some(&block(&["a"], &["nothing"], &["go"])), None, Ok(())).unwrap();
        assert_eq!(
            (turn.done, turn.needs, turn.next),
            (vec!["a".to_owned()], vec![], vec!["go".to_owned()])
        );
        let asking = LastTurn::of(Some(&block(&[], &["which base?"], &[])), None, Ok(())).unwrap();
        assert_eq!(asking.needs, ["which base?"]);
    }

    #[test]
    fn a_check_alone_is_a_last_turn() {
        let turn = LastTurn::of(None, Some(Look::Checking), Ok(())).unwrap();
        assert_eq!((turn.done.len(), turn.needs.len(), turn.next.len()), (0, 0, 0));
    }

    #[test]
    fn attention_is_there_for_questions_and_reports_and_moves_when_they_change() {
        let quiet = block(&["a"], &["nothing"], &["go"]);
        assert_eq!(LastTurn::of(Some(&quiet), None, Ok(())).unwrap().attention(), None);
        assert_eq!(
            LastTurn::of(Some(&quiet), Some(Look::Checking), Ok(())).unwrap().attention(),
            None
        );
        let asking = block(&["a"], &["which base?"], &["go"]);
        let question = LastTurn::of(Some(&asking), None, Ok(())).unwrap().attention();
        assert!(question.is_some());
        let checked = LastTurn::of(Some(&quiet), Some(report("one")), Ok(())).unwrap().attention();
        assert!(checked.is_some());
        let again = LastTurn::of(Some(&quiet), Some(report("one")), Ok(())).unwrap().attention();
        let other = LastTurn::of(Some(&quiet), Some(report("two")), Ok(())).unwrap().attention();
        assert_eq!(checked, again);
        assert_ne!(checked, other);
        assert_ne!(checked, question);
    }

    #[test]
    fn whether_the_agent_can_be_asked_does_not_make_the_turn_new() {
        let asking = block(&[], &["which base?"], &[]);
        let ready = LastTurn::of(Some(&asking), Some(report("x")), Ok(())).unwrap();
        let busy = LastTurn::of(Some(&asking), Some(report("x")), Err("working")).unwrap();
        assert_eq!(ready.attention(), busy.attention());
    }
}
