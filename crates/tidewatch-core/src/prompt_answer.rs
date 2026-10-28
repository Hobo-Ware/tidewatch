//! The answer to Claude's `UserPromptSubmit` hook: extra context the model reads with the prompt,
//! so what Tidewatch needs to say never touches the input box (pure).

use crate::progress::{Ledger, Mark, State, real};
use crate::progress_mcp::UPDATE_TOOL;

/// Asks the agent to refresh its ledger before its final reply once the owner has answered a question or a wait.
pub const LEDGER_CONTEXT: &str = "The user answered; call update_progress so your report shows the new state before your final reply.";

/// Asks a tab that never saved a ledger to write one, which also reaches a tab whose settings file
/// predates the Stop guard.
pub const MISSING_LEDGER_CONTEXT: &str = "Tidewatch has no progress report for this task yet. Call update_progress with the whole report (state, now, your stages and steps) this turn, before other work.";

const NAMED_ITEMS: usize = 3;
const NAME_CHARS: usize = 60;

/// The pending done checks (Not verified) and the unchecked steps of the ledger, in that order.
fn open_items(ledger: &Ledger) -> Vec<String> {
    let checks = real(&ledger.not_verified).map(|entry| entry.text.as_str());
    let steps = ledger
        .checklist
        .iter()
        .flat_map(|phase| &phase.items)
        .filter(|item| matches!(item.mark, Mark::Todo | Mark::Doing | Mark::Blocked))
        .map(|item| item.text.as_str());
    checks.chain(steps).map(short_name).collect()
}

fn short_name(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= NAME_CHARS {
        return one_line;
    }
    let head: String = one_line.chars().take(NAME_CHARS - 3).collect();
    format!("{}...", head.trim_end())
}

/// The one line that tells the agent what its ledger still holds open, so a message that settles
/// one of them reaches `update_progress`. A ledger that waits on the owner, or waits and names
/// nothing, gets `LEDGER_CONTEXT` instead; `None` when nothing is open.
#[must_use]
pub fn ledger_context(ledger: &Ledger) -> Option<String> {
    let unnamed_wait = ledger.status.state == Some(State::Waiting) && ledger.waiting_on.is_none();
    if ledger.awaits_owner() || unnamed_wait {
        return Some(LEDGER_CONTEXT.to_owned());
    }
    let open = open_items(ledger);
    if open.is_empty() {
        return None;
    }
    let named = open.iter().take(NAMED_ITEMS).map(|name| format!("\"{name}\"")).collect::<Vec<_>>();
    let more = open.len().saturating_sub(NAMED_ITEMS);
    let more = if more > 0 { format!(" (+{more} more)") } else { String::new() };
    Some(format!(
        "Open in your report: {}{more}. If this message changes any of them, call {UPDATE_TOOL}.",
        named.join("; ")
    ))
}

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

    #[test]
    fn the_ledger_line_asks_for_the_report_before_the_final_reply_not_before_continuing() {
        assert!(LEDGER_CONTEXT.contains("before your final reply"));
        assert!(!LEDGER_CONTEXT.contains("before you continue"));
    }

    fn ledger(status: &str, checklist: &str, not_verified: &str) -> Ledger {
        Ledger::parse(&format!(
            "# Goal\nShip\n\n## Status\nstate: {status}\n\n## Checklist\n### 1. Build\n{checklist}\n\n## Not verified\n{not_verified}\n"
        ))
    }

    #[test]
    fn the_line_names_what_is_open_and_nothing_is_said_when_nothing_is() {
        let long = "x".repeat(80);
        let cases = [
            ("nothing open", ledger("working", "- [x] Build", "- Nothing"), None),
            ("no ledger content", Ledger::default(), None),
            (
                "a pending check",
                ledger("working", "- [x] Build", "- Verified live"),
                Some(
                    "Open in your report: \"Verified live\". If this message changes any of them, call update_progress.",
                ),
            ),
            (
                "a pending check on a finished ledger",
                ledger("done", "- [x] Build", "- Verified live"),
                Some(
                    "Open in your report: \"Verified live\". If this message changes any of them, call update_progress.",
                ),
            ),
            (
                "an unchecked step",
                ledger("working", "- [x] Build\n- [ ] Add tests", "- Nothing"),
                Some(
                    "Open in your report: \"Add tests\". If this message changes any of them, call update_progress.",
                ),
            ),
            (
                "doing and blocked steps count, skipped ones do not",
                ledger("working", "- [~] Wire it\n- [!] Ship\n- [-] Skip me", "- Nothing"),
                Some(
                    "Open in your report: \"Wire it\"; \"Ship\". If this message changes any of them, call update_progress.",
                ),
            ),
            (
                "checks come before steps",
                ledger("working", "- [ ] Add tests", "- Verified live"),
                Some(
                    "Open in your report: \"Verified live\"; \"Add tests\". If this message changes any of them, call update_progress.",
                ),
            ),
            (
                "more than three names three",
                ledger("working", "- [ ] a\n- [ ] b\n- [ ] c", "- d\n- e"),
                Some(
                    "Open in your report: \"d\"; \"e\"; \"a\" (+2 more). If this message changes any of them, call update_progress.",
                ),
            ),
            (
                "a long name is cut",
                ledger("working", "- [x] Build", &format!("- {long}")),
                Some(&format!(
                    "Open in your report: \"{}...\". If this message changes any of them, call update_progress.",
                    "x".repeat(57)
                )),
            ),
            (
                "a ledger that waits on the owner keeps its own line",
                ledger("waiting", "- [ ] Add tests", "- Verified live"),
                Some(LEDGER_CONTEXT),
            ),
        ];
        for (what, ledger, want) in cases {
            assert_eq!(ledger_context(&ledger).as_deref(), want, "{what}");
        }
    }

    #[test]
    fn the_open_line_is_one_line_of_ascii_that_names_the_tool() {
        let line = ledger_context(&ledger("working", "- [ ] a\n- [ ] b", "- c")).unwrap();
        assert_eq!(line.lines().count(), 1);
        assert!(line.is_ascii());
        assert!(line.contains(UPDATE_TOOL));
    }

    #[test]
    fn the_missing_ledger_line_names_the_tool_and_asks_for_the_whole_report() {
        assert!(MISSING_LEDGER_CONTEXT.contains(UPDATE_TOOL));
        assert!(
            MISSING_LEDGER_CONTEXT
                .contains("whole report (state, now, your stages and steps) this turn")
        );
        assert!(MISSING_LEDGER_CONTEXT.is_ascii());
    }
}
