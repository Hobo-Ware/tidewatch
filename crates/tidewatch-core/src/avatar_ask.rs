//! When a tab that has no avatar is asked to draw one: once, on its next prompt. The ask rides the
//! answer to Claude's `UserPromptSubmit` hook as extra context (see `prompt_answer.rs`), so it
//! never touches the input box. The decision is pure; the listener reads the facts.

use crate::agents::Wiring;

/// The line pasted into an idle agent when the owner clicks Draw avatar.
pub const ASK: &str = "Call set_avatar once with an icon for this task: a round icon, bright colors on a dark background, nothing near the edge.";
pub const CONTEXT: &str = "Also call set_avatar once with an icon for this task: a round icon, bright colors on a dark background, nothing near the edge.";

/// The hook agent whose `UserPromptSubmit` answer reaches the model as context.
const PROMPT_ANSWERING_AGENT: &str = "claude";

/// Whether the CLI gets the `set_avatar` tool: the two whose MCP config Tidewatch writes.
#[must_use]
pub fn can_draw(wiring: Wiring) -> bool {
    matches!(wiring, Wiring::Claude | Wiring::Copilot)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Fixed,
    Picture,
    Letter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub answers_prompts: bool,
    pub face: Face,
    pub asked: bool,
    pub on_prompt: bool,
}

#[must_use]
pub fn answers_prompts(hook_agent: &str) -> bool {
    hook_agent == PROMPT_ANSWERING_AGENT
}

#[must_use]
pub fn due(facts: &Facts) -> bool {
    facts.answers_prompts && facts.face == Face::Letter && !facts.asked && facts.on_prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: Facts =
        Facts { answers_prompts: true, face: Face::Letter, asked: false, on_prompt: true };

    #[test]
    fn a_ready_tab_is_asked() {
        assert!(due(&READY));
    }

    #[test]
    fn any_one_fact_against_it_means_no_ask() {
        let cases = [
            ("a CLI whose hook cannot add context", Facts { answers_prompts: false, ..READY }),
            ("a Meeseeks", Facts { face: Face::Fixed, ..READY }),
            ("a tab with a picture", Facts { face: Face::Picture, ..READY }),
            ("a tab asked before", Facts { asked: true, ..READY }),
            ("an event that is not a prompt", Facts { on_prompt: false, ..READY }),
        ];
        for (what, facts) in cases {
            assert!(!due(&facts), "{what}");
        }
    }

    #[test]
    fn only_claude_answers_prompts() {
        for (agent, want) in
            [("claude", true), ("copilot", false), ("codex", false), ("gemini", false), ("", false)]
        {
            assert_eq!(answers_prompts(agent), want, "{agent}");
        }
    }

    #[test]
    fn only_claude_and_copilot_get_the_tool() {
        let cases = [
            (Wiring::Claude, true),
            (Wiring::Copilot, true),
            (Wiring::Codex, false),
            (Wiring::Gemini, false),
            (Wiring::Custom, false),
        ];
        for (wiring, want) in cases {
            assert_eq!(can_draw(wiring), want, "{wiring:?}");
        }
    }

    #[test]
    fn both_lines_name_the_tool() {
        assert!(ASK.contains(crate::avatar_mcp::SET_TOOL));
        assert!(CONTEXT.contains(crate::avatar_mcp::SET_TOOL));
    }

    #[test]
    fn both_lines_ask_for_a_round_bright_icon_with_nothing_near_the_edge() {
        for line in [ASK, CONTEXT] {
            for want in
                ["round icon", "bright colors on a dark background", "nothing near the edge"]
            {
                assert!(line.contains(want), "{want} in {line}");
            }
        }
    }
}
