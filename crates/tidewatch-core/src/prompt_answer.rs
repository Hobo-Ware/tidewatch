//! The answer to Claude's `UserPromptSubmit` hook: extra context the model reads with the prompt,
//! so what Tidewatch needs to say never touches the input box (pure).

/// Asks the agent to refresh its ledger after the owner replied to a question or a wait.
pub const LEDGER_CONTEXT: &str = "The user answered; call update_progress so your ledger shows the new status before you continue.";

/// The hook's JSON answer carrying every line as one `additionalContext`, `None` for no lines.
#[must_use]
pub fn answer(contexts: &[&str]) -> Option<String> {
    if contexts.is_empty() {
        return None;
    }
    Some(
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": contexts.join("\n"),
            }
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avatar_ask;

    fn context_of(answer: &str) -> String {
        let answer: serde_json::Value = serde_json::from_str(answer).unwrap();
        assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert_eq!(answer.as_object().unwrap().len(), 1);
        answer["hookSpecificOutput"]["additionalContext"].as_str().unwrap().to_owned()
    }

    #[test]
    fn nothing_to_say_is_no_answer() {
        assert_eq!(answer(&[]), None);
    }

    #[test]
    fn one_line_is_the_shape_claude_reads_for_a_prompt() {
        assert_eq!(context_of(&answer(&[avatar_ask::CONTEXT]).unwrap()), avatar_ask::CONTEXT);
    }

    #[test]
    fn two_lines_are_joined_and_neither_is_dropped() {
        let both = context_of(&answer(&[avatar_ask::CONTEXT, LEDGER_CONTEXT]).unwrap());
        assert!(both.contains(avatar_ask::CONTEXT));
        assert!(both.contains(LEDGER_CONTEXT));
        assert_eq!(both.lines().count(), 2);
    }

    #[test]
    fn the_ledger_line_names_the_tool() {
        assert!(LEDGER_CONTEXT.contains(crate::progress_mcp::UPDATE_TOOL));
    }
}
