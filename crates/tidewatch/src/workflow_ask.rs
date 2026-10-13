//! What a workflow tab asks the user and which answers it offers: the wording and the buttons for
//! each reason the runner stops to ask, and the event each button turns into. No drawing.

use tidewatch_core::runner::{AskReason, RunEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Approve,
    Retry,
    Reject,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Button {
    pub(crate) choice: Choice,
    pub(crate) label: &'static str,
    pub(crate) hint: &'static str,
    pub(crate) primary: bool,
}

const fn button(choice: Choice, label: &'static str, hint: &'static str, primary: bool) -> Button {
    Button { choice, label, hint, primary }
}

const STOP: Button = button(Choice::Stop, "Stop", "End the workflow here", false);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ask {
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) buttons: Vec<Button>,
    /// Shows a field for the test command next to the buttons.
    pub(crate) test_command: bool,
}

fn non_empty(text: &str) -> Option<String> {
    Some(text.trim()).filter(|text| !text.is_empty()).map(str::to_owned)
}

/// The question for `reason` on the stage called `stage` (for example `Review 2/4`). `summary` is
/// what the stage reported, shown with an approval.
pub(crate) fn ask_of(reason: &AskReason, stage: &str, summary: Option<&str>) -> Ask {
    let pass = |label, hint| button(Choice::Approve, label, hint, false);
    let retry = |label, hint, primary| button(Choice::Retry, label, hint, primary);
    match reason {
        AskReason::Approve => Ask {
            title: format!("{stage} is waiting for your approval"),
            detail: summary.and_then(non_empty),
            buttons: vec![
                button(Choice::Approve, "Approve", "Accept the stage and go on", true),
                button(
                    Choice::Reject,
                    "Reject with feedback",
                    "Send the work back with your reason",
                    false,
                ),
                STOP,
            ],
            test_command: false,
        },
        AskReason::NoStageDone { last_message } => Ask {
            title: format!("{stage}: the agent stopped without saying it was done"),
            detail: last_message.as_deref().and_then(non_empty),
            buttons: vec![
                pass("Pass", "Accept what is there and check the gate"),
                retry("Retry", "Run the stage again", true),
                STOP,
            ],
            test_command: false,
        },
        AskReason::GateFailed { gate, detail, loops_used } => Ask {
            title: format!("{stage}: the gate failed, {detail}"),
            detail: Some(format!(
                "Gate: {}. The work went back {loops_used} time{}.",
                gate.label(),
                if *loops_used == 1 { "" } else { "s" }
            )),
            buttons: vec![
                retry("Retry", "Another round of loops", true),
                pass("Pass anyway", "Go on although the gate failed"),
                STOP,
            ],
            test_command: false,
        },
        AskReason::AgentFailed { reason } => Ask {
            title: format!("{stage}: the agent failed"),
            detail: non_empty(reason),
            buttons: vec![
                retry("Retry", "Start the stage again", true),
                pass("Pass anyway", "Go on without this stage's result"),
                STOP,
            ],
            test_command: false,
        },
        AskReason::NoTestCommand => Ask {
            title: format!("{stage}: there is no test command to run"),
            detail: Some(
                "Type one below, or put test = \"...\" in .tidewatch.toml in the repo.".to_owned(),
            ),
            buttons: vec![
                retry("Run the tests", "Use the command typed here", true),
                pass("Skip the gate", "Go on without running tests"),
                STOP,
            ],
            test_command: true,
        },
    }
}

/// The event a click on `choice` means. `None` for a rejection without a reason.
pub(crate) fn event_of(choice: Choice, feedback: &str) -> Option<RunEvent> {
    match choice {
        Choice::Approve => Some(RunEvent::UserApprove),
        Choice::Retry => Some(RunEvent::UserRetry),
        Choice::Stop => Some(RunEvent::UserStop),
        Choice::Reject => non_empty(feedback).map(|feedback| RunEvent::UserReject { feedback }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::workflows::Gate;

    fn choices(ask: &Ask) -> Vec<Choice> {
        ask.buttons.iter().map(|b| b.choice).collect()
    }

    fn every_reason() -> Vec<AskReason> {
        vec![
            AskReason::Approve,
            AskReason::NoStageDone { last_message: Some("I think I am done".into()) },
            AskReason::GateFailed {
                gate: Gate::NoBlocking,
                detail: "2 blocking findings".into(),
                loops_used: 2,
            },
            AskReason::AgentFailed { reason: "exit 1".into() },
            AskReason::NoTestCommand,
        ]
    }

    #[test]
    fn every_reason_offers_a_way_on_a_way_back_and_a_way_out() {
        for reason in every_reason() {
            let ask = ask_of(&reason, "Review 2/4", None);
            let offered = choices(&ask);
            assert!(offered.contains(&Choice::Approve), "{reason:?}");
            assert!(offered.contains(&Choice::Stop), "{reason:?}");
            assert_eq!(ask.buttons.iter().filter(|b| b.primary).count(), 1, "{reason:?}");
            assert!(ask.title.starts_with("Review 2/4"), "{}", ask.title);
        }
    }

    #[test]
    fn only_an_approval_can_be_rejected_and_only_a_failure_can_be_retried() {
        for reason in every_reason() {
            let offered = choices(&ask_of(&reason, "s", None));
            assert_eq!(
                offered.contains(&Choice::Reject),
                reason == AskReason::Approve,
                "{reason:?}"
            );
            assert_eq!(
                offered.contains(&Choice::Retry),
                reason != AskReason::Approve,
                "{reason:?}"
            );
        }
    }

    #[test]
    fn an_approval_shows_what_the_stage_reported() {
        let ask = ask_of(&AskReason::Approve, "Hand off 4/4", Some("  feat: retry the call \n"));
        assert_eq!(ask.detail.as_deref(), Some("feat: retry the call"));
        assert_eq!(ask_of(&AskReason::Approve, "s", Some("  ")).detail, None);
    }

    #[test]
    fn a_silent_stop_shows_the_agents_last_message() {
        let reason = AskReason::NoStageDone { last_message: Some("Tests added".into()) };
        assert_eq!(ask_of(&reason, "s", None).detail.as_deref(), Some("Tests added"));
        let none = AskReason::NoStageDone { last_message: None };
        assert_eq!(ask_of(&none, "s", None).detail, None);
    }

    #[test]
    fn a_failed_gate_says_how_often_the_work_went_back() {
        let reason = AskReason::GateFailed {
            gate: Gate::Tests,
            detail: "tests failed (exit 1)".into(),
            loops_used: 1,
        };
        let ask = ask_of(&reason, "Work 1/3", None);
        assert_eq!(ask.title, "Work 1/3: the gate failed, tests failed (exit 1)");
        assert_eq!(ask.detail.as_deref(), Some("Gate: Tests pass. The work went back 1 time."));
    }

    #[test]
    fn only_a_missing_test_command_asks_for_one() {
        for reason in every_reason() {
            assert_eq!(
                ask_of(&reason, "s", None).test_command,
                reason == AskReason::NoTestCommand,
                "{reason:?}"
            );
        }
    }

    #[test]
    fn buttons_turn_into_the_runners_events() {
        assert_eq!(event_of(Choice::Approve, ""), Some(RunEvent::UserApprove));
        assert_eq!(event_of(Choice::Retry, ""), Some(RunEvent::UserRetry));
        assert_eq!(event_of(Choice::Stop, ""), Some(RunEvent::UserStop));
        assert_eq!(
            event_of(Choice::Reject, "  clear the timer "),
            Some(RunEvent::UserReject { feedback: "clear the timer".into() })
        );
    }

    #[test]
    fn a_rejection_needs_a_reason() {
        assert_eq!(event_of(Choice::Reject, ""), None);
        assert_eq!(event_of(Choice::Reject, "  \n "), None);
    }
}
