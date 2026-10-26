//! Whether Claude Code's question menu has left the terminal before an answer is typed into it
//! (pure). Only positive evidence refuses: a readable screen with no numbered row for the first
//! option. An empty screen or a question with no options is unreadable and the answer goes as before.
//! Test screens follow the menu as PLAN.md records it, not a raw capture.

use stdusk_core::terminal::PtyTerm;
use tidewatch_core::question::Question;

use crate::job_glue::screen_rows;

const LABEL_PREFIX: usize = 20;
const SIDES: [char; 2] = ['\u{2502}', '\u{2503}'];
const NOISE: [char; 2] = ['`', '*'];
const ELLIPSIS: char = '\u{2026}';
const MARKS: [char; 12] = [
    '[', ']', '(', ')', '\u{2610}', '\u{2611}', '\u{2612}', '\u{2714}', '\u{2713}', '\u{25cb}',
    '\u{25cf}', '\u{25ef}',
];

pub(crate) trait ShowsScreen {
    fn rows(&self) -> Vec<String>;
}

impl ShowsScreen for PtyTerm {
    fn rows(&self) -> Vec<String> {
        screen_rows(&self.grid_snapshot())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    Open,
    Gone,
    Unreadable,
}

fn squash(text: &str) -> String {
    let plain: String = text
        .chars()
        .filter(|c| !NOISE.contains(c))
        .map(|c| if SIDES.contains(&c) { ' ' } else { c })
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn label_prefix(label: &str) -> String {
    squash(label).chars().take(LABEL_PREFIX).collect::<String>().trim_end().to_owned()
}

fn cut_short(rest: &str, label: &str) -> bool {
    let at = rest.find(ELLIPSIS).or_else(|| rest.find("..."));
    at.is_some_and(|at| {
        let head = rest[..at].trim_end();
        !head.is_empty() && label.starts_with(head)
    })
}

fn has_first_option(screen: &str, label: &str) -> bool {
    screen.match_indices("1.").any(|(at, marker)| {
        let alone = !screen[..at].chars().next_back().is_some_and(|c| c.is_ascii_digit());
        let rest = screen[at + marker.len()..]
            .trim_start_matches(|c: char| c == ' ' || MARKS.contains(&c));
        alone && (rest.starts_with(label) || cut_short(rest, label))
    })
}

/// What the screen `rows` say about the menu of `questions`. The rows are joined, so a question or
/// label that wraps over rows reads as one line; the first option is found by a prefix of its label
/// after a `1.` that any cursor glyph or tick box may precede, and a label cut at the window edge
/// still counts.
pub(crate) fn look(rows: &[String], questions: &[Question]) -> Look {
    let screen = squash(&rows.join(" "));
    let label = questions
        .first()
        .and_then(|question| question.options.first())
        .map(|option| label_prefix(&option.label))
        .unwrap_or_default();
    if screen.is_empty() || label.is_empty() {
        Look::Unreadable
    } else if has_first_option(&screen, &label) {
        Look::Open
    } else {
        Look::Gone
    }
}

pub(crate) fn menu_look<T: ShowsScreen>(term: &T, questions: &[Question]) -> Look {
    look(&term.rows(), questions)
}

#[cfg(test)]
mod tests {
    use tidewatch_core::question::{QuestionOption, Select};

    use super::*;

    fn question(text: &str, select: Select, labels: &[&str]) -> Question {
        Question {
            text: text.into(),
            header: None,
            decision: None,
            select,
            options: labels.iter().map(|label| QuestionOption::new(*label)).collect(),
        }
    }

    fn rows(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    fn wrapped(text: &str, columns: usize) -> Vec<String> {
        let mut lines = vec![String::new()];
        for word in text.split_whitespace() {
            let last = lines.last_mut().unwrap();
            if !last.is_empty() && last.len() + 1 + word.len() > columns {
                lines.push(word.to_owned());
            } else {
                if !last.is_empty() {
                    last.push(' ');
                }
                last.push_str(word);
            }
        }
        lines
    }

    const LONG: &str = "Which environment should this run in once the migration has finished and the cache has been warmed up again?";

    fn asked() -> Vec<Question> {
        vec![
            question(LONG, Select::Single, &["Stage", "Run"]),
            question("Which checks?", Select::Multi, &["Unit", "Lints"]),
        ]
    }

    fn menu_at(columns: usize) -> Vec<String> {
        let mut screen =
            vec!["\u{2190} \u{2610} Env  \u{2610} Checks  \u{2714} Submit \u{2192}".to_owned()];
        screen.extend(wrapped(LONG, columns));
        screen.extend(rows(&[
            "\u{276f} 1. Stage",
            "     Deploys to the staging cluster",
            "  2. Run",
            "  3. Type something.",
            "Enter to select \u{b7} Esc to cancel",
        ]));
        screen
    }

    #[test]
    fn a_menu_that_is_on_screen_reads_open_however_it_is_drawn() {
        let long_label =
            vec![question("Where?", Select::Single, &["Deploy to the staging cluster"])];
        let cases = [
            ("the full menu", menu_at(200), asked()),
            ("the question wrapped at 60 columns", menu_at(60), asked()),
            ("the question wrapped at 100 columns", menu_at(100), asked()),
            ("a cursor row", rows(&["Where?", "\u{276f} 1. Stage", "  2. Run"]), asked()),
            ("a plain marker with spaces", rows(&["Where?", "   1.   Stage"]), asked()),
            (
                "a label cut by an ellipsis",
                rows(&["Where?", "\u{276f} 1. Deploy to the sta\u{2026}"]),
                long_label.clone(),
            ),
            (
                "a label cut by three dots",
                rows(&["Where?", "\u{276f} 1. Deploy to...", "  2. Run"]),
                long_label.clone(),
            ),
            (
                "a label wrapped over two rows",
                rows(&["Where?", "\u{276f} 1. Deploy to the", "     staging cluster"]),
                long_label,
            ),
            (
                "the menu inside side borders",
                rows(&[
                    "\u{2502} Where?               \u{2502}",
                    "\u{2502} \u{276f} 1. Stage         \u{2502}",
                ]),
                asked(),
            ),
            ("another case", rows(&["WHERE?", "1. STAGE"]), asked()),
            (
                "a tick box before the label",
                rows(&["Which checks?", "\u{276f} 1. [ ] Unit"]),
                vec![asked().remove(1)],
            ),
        ];
        for (what, screen, questions) in cases {
            assert_eq!(look(&screen, &questions), Look::Open, "{what}");
        }
    }

    #[test]
    fn a_screen_that_shows_something_else_reads_gone() {
        let echo = rows(&[
            "\u{25cf} User answered Claude's questions:",
            "  \u{23bf}  \u{b7} Which environment should this run in once the migration has finished? \u{2192} Run",
            "",
            "> ",
        ]);
        let cases = [
            ("the answered echo", echo),
            ("another question's menu", rows(&["Which colour?", "\u{276f} 1. Red", "  2. Blue"])),
            ("the second tab of the same menu", rows(&["Which checks?", "\u{276f} 1. [ ] Unit"])),
            (
                "the input box",
                rows(&["\u{2500}".repeat(20).as_str(), "> ", "\u{2500}".repeat(20).as_str()]),
            ),
            (
                "a permission prompt",
                rows(&["Do you want to run it?", "\u{276f} 1. Yes", "  2. No"]),
            ),
            ("an option number inside a longer number", rows(&["Where?", "11. Stage"])),
        ];
        for (what, screen) in cases {
            assert_eq!(look(&screen, &asked()), Look::Gone, "{what}");
        }
    }

    #[test]
    fn a_screen_with_nothing_to_judge_by_reads_unreadable_so_the_answer_goes() {
        let no_options = vec![question("Where?", Select::Single, &[])];
        let cases = [
            ("no rows", rows(&[]), asked()),
            ("blank rows", rows(&["", "   ", ""]), asked()),
            ("a question with no options", rows(&["Where?", "1. Stage"]), no_options),
            ("no questions", rows(&["Where?", "1. Stage"]), Vec::new()),
        ];
        for (what, screen, questions) in cases {
            assert_eq!(look(&screen, &questions), Look::Unreadable, "{what}");
        }
    }

    #[test]
    fn markdown_marks_in_a_label_do_not_make_it_unfindable() {
        let questions = vec![question("Where?", Select::Single, &["Use `stage` **now**"])];
        assert_eq!(look(&rows(&["\u{276f} 1. Use stage now"]), &questions), Look::Open);
    }
}
