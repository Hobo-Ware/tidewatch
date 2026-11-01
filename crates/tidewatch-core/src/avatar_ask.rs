//! The asks for an avatar. One is pasted into an idle agent when the owner clicks Draw avatar. The
//! other rides the answer to Claude's `UserPromptSubmit` hook as extra context (see
//! `prompt_answer.rs`), once, on the first prompt after the tab's report has a goal, so it never
//! touches the input box and the icon is picked for a task that is known. The decision is pure;
//! the listener reads the facts.

use crate::agents::Wiring;

/// The line pasted into an idle agent when the owner clicks Draw avatar.
pub const ASK: &str = "Call set_avatar once for this task: pick the Lucide icon of the one object the goal is about, send its name as icon with a color, and name the object in reason. Draw an svg only when no icon fits.";

pub const CONTEXT: &str = "Also call set_avatar once with an icon that fits your goal: the Lucide icon of the one object it is about, as icon with a color, and name the object in reason.";

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
    pub face: Face,
    pub asked: bool,
    pub has_goal: bool,
}

/// Whether the prompt that just arrived asks the tab to pick its icon: a letter face, never asked,
/// and a goal in the saved report, so the icon is chosen for the task and not before it is known.
#[must_use]
pub fn due(facts: &Facts) -> bool {
    facts.face == Face::Letter && !facts.asked && facts.has_goal
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: Facts = Facts { face: Face::Letter, asked: false, has_goal: true };

    #[test]
    fn a_tab_with_a_goal_a_letter_and_no_earlier_ask_is_asked() {
        assert!(due(&READY));
    }

    #[test]
    fn any_one_fact_against_it_means_no_ask() {
        let cases = [
            ("a Meeseeks", Facts { face: Face::Fixed, ..READY }),
            ("a tab with a picture", Facts { face: Face::Picture, ..READY }),
            ("a tab asked before", Facts { asked: true, ..READY }),
            ("a tab whose report has no goal yet", Facts { has_goal: false, ..READY }),
        ];
        for (what, facts) in cases {
            assert!(!due(&facts), "{what}");
        }
    }

    #[test]
    fn the_hook_line_names_the_tool_and_asks_for_an_icon_that_fits_the_goal() {
        assert!(CONTEXT.contains(crate::avatar_mcp::SET_TOOL));
        for want in ["fits your goal", "Lucide icon", "as icon with a color", "name the object"] {
            assert!(CONTEXT.contains(want), "{want} in {CONTEXT}");
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
    fn the_line_names_the_tool_and_asks_for_an_icon_picked_from_the_goal() {
        assert!(ASK.contains(crate::avatar_mcp::SET_TOOL));
        for want in [
            "Lucide icon",
            "the one object the goal is about",
            "as icon with a color",
            "name the object in reason",
            "svg only when no icon fits",
        ] {
            assert!(ASK.contains(want), "{want} in {ASK}");
        }
    }
}
