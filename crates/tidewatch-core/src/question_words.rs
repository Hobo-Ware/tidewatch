//! What Tidewatch tells an agent when the user chose to skip its questions. Pure.

/// What the tool result says when the user chose to leave the choice to the agent.
pub const SKIPPED: &str = "The user skipped your questions. Go with your recommended answer and say which one you picked in your final reply.";
/// What a skip types into an agent that asked with its own tool, after the menu is dismissed.
pub const SKIP_REPLY: &str = "Skip your questions. Go with your recommended answer and say which one you picked in your final reply.";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_skip_hands_the_choice_to_the_agent_and_asks_it_to_say_what_it_picked() {
        for text in [SKIPPED, SKIP_REPLY] {
            assert!(text.contains("Go with your recommended answer"), "{text}");
            assert!(text.contains("say which one you picked in your final reply"), "{text}");
            assert!(!text.contains("best judgment"), "{text}");
        }
    }
}
