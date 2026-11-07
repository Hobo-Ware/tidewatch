//! The answer to Claude's hook events: extra context the model reads before its next message, so
//! what Tidewatch needs to say after a resume or when the repo's services change never touches the
//! input box (pure).

/// The `UserPromptSubmit` answer carrying every line as one `additionalContext`, `None` for no lines.
#[must_use]
pub fn answer(contexts: &[&str]) -> Option<String> {
    answer_to("UserPromptSubmit", contexts)
}

/// The answer to the hook `event` (`UserPromptSubmit` or `SessionStart`, the two whose
/// `additionalContext` Claude reads), `None` for no lines.
#[must_use]
pub fn answer_to(event: &str, contexts: &[&str]) -> Option<String> {
    if contexts.is_empty() {
        return None;
    }
    Some(
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": event,
                "additionalContext": contexts.join("\n"),
            }
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let line = "Services changed.";
        assert_eq!(context_of(&answer(&[line]).unwrap()), line);
    }

    #[test]
    fn two_lines_are_joined_and_neither_is_dropped() {
        let both = context_of(&answer(&["one", "two"]).unwrap());
        assert_eq!(both, "one\ntwo");
    }

    #[test]
    fn a_session_start_answer_names_its_own_event() {
        let answer: serde_json::Value =
            serde_json::from_str(&answer_to("SessionStart", &["here"]).unwrap()).unwrap();
        assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "SessionStart");
        assert_eq!(answer["hookSpecificOutput"]["additionalContext"], "here");
    }
}
