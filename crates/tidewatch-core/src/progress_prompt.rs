//! The instruction that puts the progress report in front of an agent: when to call
//! `update_progress` and `get_progress`, and the user's must-do steps when there are any (pure). The
//! tool's own schema and its error replies teach the report format. Every CLI gets the same text;
//! only the sentence on a turn that ends with no report differs.

use crate::agents::Wiring;

const REPORT_FORMAT: &str = "\
## Report

Call `update_progress` once real work starts and again before your final reply. The tool tells you what it needs. {turn_end}
After a context reset, call `get_progress` first. If `update_progress` is missing, look for `mcp__tidewatch__update_progress` with tool search once. If it is still missing, tell the user that Tidewatch reporting is down.
";

const MUST_DOS_INTRO: &str = "\
Must-do steps from the user or this repo. Add each as a step under the stage where it fits. They are a flat list, not stages:
";

/// Who answers a turn that ends with no report saved: Claude's Stop hook, or a pasted reminder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnEnd {
    SentBack,
    Reminded,
}

impl TurnEnd {
    #[must_use]
    pub fn of(wiring: Wiring) -> Self {
        if wiring == Wiring::Claude { Self::SentBack } else { Self::Reminded }
    }

    fn sentence(self) -> &'static str {
        match self {
            Self::SentBack => {
                "A turn that ends with no report saved in it is sent back to write one."
            }
            Self::Reminded => {
                "A turn that ends with no report saved in it gets a reminder pasted to you, to write one."
            }
        }
    }
}

/// The report section of an agent's house style. `must_dos` is the user's or the repo's list of
/// steps every task includes, when there is one.
#[must_use]
pub fn section(must_dos: Option<&str>, turn_end: TurnEnd) -> String {
    let must_dos = must_dos.map_or_else(String::new, |text| {
        format!("\n{MUST_DOS_INTRO}\n{}\n", flatten_headings(text.trim_end_matches('\n')))
    });
    format!("{}{must_dos}", REPORT_FORMAT.replace("{turn_end}", turn_end.sentence()))
}

/// A heading in a must-do list (`### 1. Understand`) becomes a plain bullet, so an agent that
/// copies the list never grows a stage of its own. Fenced code is left alone.
fn flatten_headings(text: &str) -> String {
    let mut fenced = false;
    let lines: Vec<String> = text
        .lines()
        .filter_map(|line| {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
            }
            match line.trim_start().strip_prefix('#') {
                Some(heading) if !fenced => {
                    let title = without_number(heading.trim_start_matches('#').trim());
                    (!title.is_empty()).then(|| format!("- {title}"))
                }
                _ => Some(line.to_owned()),
            }
        })
        .collect();
    lines.join("\n")
}

fn without_number(title: &str) -> &str {
    title
        .split_once(['.', ')'])
        .filter(|(number, _)| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
        .map_or(title, |(_, rest)| rest.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_section_is_two_short_paragraphs_that_name_both_tools() {
        let text = section(None, TurnEnd::SentBack);
        assert!(text.starts_with("## Report\n\n"));
        assert!(text.contains(
            "Call `update_progress` once real work starts and again before your final reply."
        ));
        assert!(text.contains("The tool tells you what it needs."));
        assert!(text.contains("After a context reset, call `get_progress` first."));
        assert!(text.split_whitespace().count() <= 90, "{text}");
        assert!(!text.contains("```"), "no format example: the tool teaches the format");
    }

    #[test]
    fn a_missing_tool_is_looked_for_once_and_then_reported_to_the_user() {
        let text = section(None, TurnEnd::SentBack);
        assert!(text.contains("`mcp__tidewatch__update_progress` with tool search once"));
        assert!(text.contains("Tidewatch reporting is down"));
    }

    #[test]
    fn a_list_of_must_dos_comes_after_the_report_paragraphs() {
        let text = section(Some("- [ ] only this\n"), TurnEnd::SentBack);
        let at = text.find("Must-do steps from the user or this repo").unwrap();
        assert!(at > text.find("tell the user that Tidewatch reporting is down").unwrap());
        assert!(text.ends_with("- [ ] only this\n"), "{text}");
    }

    #[test]
    fn must_do_headings_become_plain_bullets_and_never_look_like_stages() {
        let cases = [
            (
                "a numbered heading",
                "### 1. Understand\n- [ ] read it",
                "- Understand\n- [ ] read it",
            ),
            (
                "a bare heading",
                "## Before merge\n- [ ] run tests",
                "- Before merge\n- [ ] run tests",
            ),
            ("a paren number", "# 2) Ship", "- Ship"),
            ("an empty heading", "###\n- [ ] a", "- [ ] a"),
            ("a title with a dot inside", "### Fix v1.2 notes", "- Fix v1.2 notes"),
            ("code is left alone", "```sh\n# a comment\n```", "```sh\n# a comment\n```"),
            ("plain bullets", "- [ ] a\n- [ ] b", "- [ ] a\n- [ ] b"),
        ];
        for (name, input, want) in cases {
            assert_eq!(flatten_headings(input), want, "{name}");
        }
        let ten_phases = "### 1. Phase\n- [ ] x\n".repeat(10);
        let text = section(Some(&ten_phases), TurnEnd::SentBack);
        let after = &text[text.find("Must-do steps").unwrap()..];
        assert!(!after.contains('#'), "{after}");
    }

    #[test]
    fn what_happens_to_a_turn_with_no_report_is_told_per_cli_and_is_true_of_it() {
        let sent_back = section(None, TurnEnd::of(Wiring::Claude));
        assert!(sent_back.contains("is sent back to write one."));
        assert!(!sent_back.contains("reminder"));
        for wiring in [Wiring::Codex, Wiring::Gemini, Wiring::Copilot] {
            let text = section(None, TurnEnd::of(wiring));
            assert!(text.contains("gets a reminder pasted to you, to write one."), "{wiring:?}");
            assert!(!text.contains("sent back"), "{wiring:?}");
        }
        for turn_end in [TurnEnd::SentBack, TurnEnd::Reminded] {
            assert!(!section(None, turn_end).contains("{turn_end}"));
        }
    }

    #[test]
    fn every_cli_gets_the_same_text_but_for_the_one_sentence_on_a_turn_with_no_report() {
        let claude = section(None, TurnEnd::of(Wiring::Claude));
        for wiring in [Wiring::Codex, Wiring::Gemini, Wiring::Copilot] {
            let other = section(None, TurnEnd::of(wiring));
            assert_eq!(
                other.replace(TurnEnd::Reminded.sentence(), TurnEnd::SentBack.sentence()),
                claude,
                "{wiring:?}"
            );
        }
    }

    #[test]
    fn it_is_plain_ascii_and_names_both_tools() {
        for text in
            [section(None, TurnEnd::SentBack), section(Some("- [ ] a\n"), TurnEnd::Reminded)]
        {
            assert!(text.is_ascii());
            assert!(text.contains("update_progress"));
            assert!(text.contains("get_progress"));
        }
    }
}
