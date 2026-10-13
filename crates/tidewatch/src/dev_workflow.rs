//! Dev-only switches that answer a workflow run the way a click would, once, for captures and
//! runnable checks.
//!
//! - `TIDEWATCH_DEV_ASK=approve|retry|stop|reject:<reason>`: answer the next question a run asks.
//! - `TIDEWATCH_DEV_OPEN_STAGE=<n>`: open stage `n` (from 1) under the stepper, as a click would.

use crate::pane::Pane;
use crate::tabs::Tabs;
use crate::workflow_ask::Choice;
use crate::workflow_run::{Exec, TabRun};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Answer {
    Choose(Choice),
    Reject(String),
}

#[derive(Debug, Default)]
pub(crate) struct DevWorkflow {
    answer: Option<Answer>,
    open_stage: Option<usize>,
}

fn parse_answer(text: &str) -> Option<Answer> {
    match text {
        "approve" => Some(Answer::Choose(Choice::Approve)),
        "retry" => Some(Answer::Choose(Choice::Retry)),
        "stop" => Some(Answer::Choose(Choice::Stop)),
        other => other.strip_prefix("reject:").map(|reason| Answer::Reject(reason.to_owned())),
    }
}

impl DevWorkflow {
    pub(crate) fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self::parse(var("TIDEWATCH_DEV_ASK").as_deref(), var("TIDEWATCH_DEV_OPEN_STAGE").as_deref())
    }

    fn parse(ask: Option<&str>, open_stage: Option<&str>) -> Self {
        Self {
            answer: ask.and_then(parse_answer),
            open_stage: open_stage
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| n.checked_sub(1)),
        }
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.answer.is_none() && self.open_stage.is_none()
    }

    /// Does what the switches ask to the run of one tab, as soon as it can be done.
    fn apply(&mut self, run: &mut TabRun) -> Vec<Exec> {
        if let Some(index) = self.open_stage.take() {
            run.ui.open_stage = Some(index);
        }
        if run.ask().is_none() {
            return Vec::new();
        }
        match self.answer.take() {
            Some(Answer::Choose(choice)) => run.choose(choice),
            Some(Answer::Reject(reason)) => {
                run.choose(Choice::Reject);
                run.ui.feedback = reason;
                run.choose(Choice::Reject)
            }
            None => Vec::new(),
        }
    }

    /// Every workflow tab, in order. The answer goes to the first tab that asks.
    pub(crate) fn drive<T>(&mut self, tabs: &mut Tabs<Pane<T>>) -> Vec<(u64, Vec<Exec>)> {
        let mut asked = Vec::new();
        for tab in tabs.items_mut() {
            let Some(run) = tab.pane.agent.as_mut().and_then(|agent| agent.workflow.as_mut())
            else {
                continue;
            };
            let execs = self.apply(run);
            if !execs.is_empty() {
                asked.push((tab.id, execs));
            }
        }
        asked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::time::Duration;
    use tidewatch_core::events::{TabEvent, TabMessage};
    use tidewatch_core::runner::{AskReason, Phase, RunContext, Runner, StopReason};
    use tidewatch_core::workflows::Workflows;

    fn asking_approval() -> TabRun {
        let ship = Workflows::defaults().workflow.remove("ship").unwrap();
        let ctx = RunContext {
            task: "t".into(),
            base: "main".into(),
            branch: "b".into(),
            test_command: None,
        };
        let mut run = TabRun::new(
            Runner::new(&ship, &BTreeMap::new(), ctx).unwrap(),
            None,
            Duration::from_secs(60),
        );
        run.start();
        for (generation, summary) in [(1, "a"), (2, "b"), (3, "c"), (4, "feat: x")] {
            let message = TabMessage {
                tab_id: 1,
                generation,
                event: TabEvent::StageDone { summary: summary.into() },
            };
            run.message(&message);
            if generation == 1 || generation == 3 {
                run.choose(Choice::Approve);
            }
        }
        run
    }

    #[test]
    fn the_switches_parse() {
        let dev = DevWorkflow::parse(Some("reject:too vague"), Some("2"));
        assert_eq!(dev.answer, Some(Answer::Reject("too vague".into())));
        assert_eq!(dev.open_stage, Some(1));
        assert_eq!(
            DevWorkflow::parse(Some("approve"), None).answer,
            Some(Answer::Choose(Choice::Approve))
        );
        assert!(DevWorkflow::parse(Some("dance"), Some("0")).is_idle());
        assert!(DevWorkflow::parse(None, None).is_idle());
    }

    #[test]
    fn an_answer_waits_for_a_question_and_is_given_once() {
        let mut run = asking_approval();
        assert!(matches!(run.runner().phase(), Phase::Asking(AskReason::Approve)));
        let mut dev = DevWorkflow::parse(Some("approve"), None);
        assert_eq!(dev.apply(&mut run), Vec::new());
        assert!(run.finished());
        assert!(dev.is_idle());
    }

    #[test]
    fn a_rejection_sends_the_reason() {
        let mut run = asking_approval();
        let mut dev = DevWorkflow::parse(Some("reject:say more"), None);
        dev.apply(&mut run);
        assert_eq!(run.runner().phase(), &Phase::Stopped(StopReason::Rejected { stage: 3 }));
    }

    #[test]
    fn nothing_is_answered_while_the_run_has_no_question() {
        let ship = Workflows::defaults().workflow.remove("ship").unwrap();
        let ctx = RunContext {
            task: "t".into(),
            base: "main".into(),
            branch: "b".into(),
            test_command: None,
        };
        let mut run = TabRun::new(
            Runner::new(&ship, &BTreeMap::new(), ctx).unwrap(),
            None,
            Duration::from_secs(60),
        );
        run.start();
        let mut dev = DevWorkflow::parse(Some("stop"), Some("2"));
        assert_eq!(dev.apply(&mut run), Vec::new());
        assert_eq!(run.ui.open_stage, Some(1), "the stage opens at once");
        assert!(!dev.is_idle(), "the answer still waits");
    }
}
