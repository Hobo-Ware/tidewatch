//! The ask Tidewatch pastes into an idle agent when the owner clicks Draw avatar. Nothing asks an
//! agent to draw on its own.

use crate::agents::Wiring;

/// The line pasted into an idle agent when the owner clicks Draw avatar.
pub const ASK: &str = "Call set_avatar once with an icon for this task: a round icon, bright colors, no ring, frame or background (Tidewatch draws the dark plate), nothing near the edge.";

/// Whether the CLI gets the `set_avatar` tool: the two whose MCP config Tidewatch writes.
#[must_use]
pub fn can_draw(wiring: Wiring) -> bool {
    matches!(wiring, Wiring::Claude | Wiring::Copilot)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_line_names_the_tool_and_asks_for_a_round_bright_icon_with_nothing_near_the_edge() {
        assert!(ASK.contains(crate::avatar_mcp::SET_TOOL));
        for want in
            ["round icon", "bright colors, no ring, frame or background", "nothing near the edge"]
        {
            assert!(ASK.contains(want), "{want} in {ASK}");
        }
    }
}
