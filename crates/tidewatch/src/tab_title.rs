//! The label of an agent tab: what the work is, not the worktree's name. `pick` holds the order.

use stdusk_egui::ui::ellipsize;
use tidewatch_core::branch_name;

use crate::pane::AgentTab;

pub(crate) const MAX_CHARS: usize = 100;
const MAX_TASK_CHARS: usize = 300;
const MIN_LETTERS: usize = 2;
const GENERIC_TERMINAL_TITLES: [&str; 1] = ["claude code"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Chosen,
    TaskSlug,
    Session,
    Terminal,
    Goal,
    FirstMessage,
    Name,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sources<'a> {
    pub(crate) chosen: Option<&'a str>,
    pub(crate) task: Option<&'a str>,
    pub(crate) session: Option<&'a str>,
    pub(crate) terminal: Option<&'a str>,
    pub(crate) goal: Option<&'a str>,
    pub(crate) first_message: Option<&'a str>,
}

pub(crate) fn pick(sources: Sources<'_>, name: &str) -> (Source, String) {
    let task_slug = sources.task.map(branch_name::slug);
    [
        (Source::Chosen, sources.chosen.map(str::trim)),
        (Source::TaskSlug, task_slug.as_deref()),
        (Source::Session, sources.session),
        (Source::Terminal, sources.terminal),
        (Source::Goal, sources.goal),
        (Source::FirstMessage, sources.first_message),
    ]
    .into_iter()
    .find_map(|(source, text)| {
        let text = text?;
        let shown = match source {
            Source::Chosen | Source::TaskSlug => (!text.is_empty()).then(|| text.to_owned()),
            _ => tidy(text),
        };
        Some((source, shown?))
    })
    .unwrap_or_else(|| (Source::Name, name.to_owned()))
}

pub(crate) fn of_agent(agent: &AgentTab, name: &str) -> (Source, String) {
    let terminal = terminal_title(&agent.run.agent_id, agent.run.terminal_title());
    let meeseeks = agent.meeseeks.as_ref();
    pick(
        Sources {
            chosen: meeseeks.and_then(|meeseeks| meeseeks.name.as_deref()),
            task: meeseeks.map(|meeseeks| meeseeks.task.as_str()),
            session: agent.reader.session_title(),
            terminal: terminal.as_deref(),
            goal: agent.run.goal(),
            first_message: agent.reader.first_message(),
        },
        name,
    )
}

pub(crate) fn label(agent: Option<&AgentTab>, name: &str) -> String {
    agent.map_or_else(|| name.to_owned(), |agent| of_agent(agent, name).1)
}

/// The second line of a tab's tooltip: the task of a Meeseeks, whose label is a short name, else the
/// worktree's name.
pub(crate) fn subtitle(agent: Option<&AgentTab>, name: &str) -> String {
    agent
        .and_then(|agent| agent.meeseeks.as_ref())
        .and_then(|meeseeks| task_line(&meeseeks.task))
        .unwrap_or_else(|| name.to_owned())
}

/// A task as one line of at most 300 characters. `None` for an empty one.
pub(crate) fn task_line(task: &str) -> Option<String> {
    let line = task.split_whitespace().collect::<Vec<_>>().join(" ");
    (!line.is_empty()).then(|| ellipsize(&line, MAX_TASK_CHARS).0)
}

/// The first line of `text` as a short title: markdown marks and extra spaces gone. `None` when
/// nothing readable is left.
pub(crate) fn tidy(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| line.chars().any(char::is_alphanumeric))?;
    let line = line.replace("**", "");
    let line =
        line.trim_start_matches(|c: char| matches!(c, '#' | '>' | '-' | '*') || c.is_whitespace());
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    (line.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_LETTERS)
        .then(|| ellipsize(&line, MAX_CHARS).0)
}

/// Claude's terminal title minus its spinner glyph. Other CLIs set a status or a folder instead.
pub(crate) fn terminal_title(agent_id: &str, raw: Option<&str>) -> Option<String> {
    if agent_id != "claude" {
        return None;
    }
    let text = raw?.trim_start_matches(|c: char| {
        c.is_whitespace() || c == '\u{b7}' || ('\u{2190}'..='\u{2bff}').contains(&c)
    });
    let title = tidy(text)?;
    (!GENERIC_TERMINAL_TITLES.contains(&title.to_lowercase().as_str())).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "brave-otter";

    fn picked(sources: Sources<'_>) -> (Source, String) {
        pick(sources, NAME)
    }

    #[test]
    fn the_first_source_with_something_wins() {
        let all = Sources {
            chosen: None,
            task: None,
            session: Some("Fix login redirect"),
            terminal: Some("Terminal title"),
            goal: Some("Ledger goal"),
            first_message: Some("First message"),
        };
        let cases = [
            (all, (Source::Session, "Fix login redirect")),
            (Sources { session: None, ..all }, (Source::Terminal, "Terminal title")),
            (Sources { session: None, terminal: None, ..all }, (Source::Goal, "Ledger goal")),
            (
                Sources { session: None, terminal: None, goal: None, ..all },
                (Source::FirstMessage, "First message"),
            ),
            (Sources::default(), (Source::Name, NAME)),
        ];
        for (sources, (source, text)) in cases {
            assert_eq!(picked(sources), (source, text.to_owned()), "{sources:?}");
        }
    }

    #[test]
    fn a_meeseeks_shows_its_chosen_name_then_a_slug_of_its_task_before_any_title() {
        let titles = Sources {
            session: Some("Fix the https validation on signup"),
            terminal: Some("Terminal title"),
            goal: Some("Ledger goal"),
            first_message: Some("Fix the https validation on signup"),
            ..Sources::default()
        };
        let task = "Fix the https validation on signup";
        let cases = [
            (
                Sources { chosen: Some("signup-https"), task: Some(task), ..titles },
                (Source::Chosen, "signup-https"),
            ),
            (
                Sources { chosen: Some("  signup-https "), task: Some(task), ..titles },
                (Source::Chosen, "signup-https"),
            ),
            (
                Sources { chosen: None, task: Some(task), ..titles },
                (Source::TaskSlug, "fix-https-validation-signup"),
            ),
            (
                Sources { chosen: Some("   "), task: Some(task), ..titles },
                (Source::TaskSlug, "fix-https-validation-signup"),
            ),
            (
                Sources { chosen: None, task: Some("?!"), ..titles },
                (Source::Session, "Fix the https validation on signup"),
            ),
            (Sources { chosen: None, task: None, ..titles }, (Source::Session, task)),
        ];
        for (sources, (source, text)) in cases {
            assert_eq!(picked(sources), (source, text.to_owned()), "{sources:?}");
        }
    }

    #[test]
    fn the_tooltip_second_line_is_the_task_of_a_meeseeks_and_the_worktree_name_of_any_other_tab() {
        use crate::agent_run::AgentRun;
        use tidewatch_core::meeseeks::{Criteria, Meeseeks};
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        assert_eq!(subtitle(Some(&agent), NAME), NAME);
        assert_eq!(subtitle(None, NAME), NAME);
        agent.meeseeks = Some(Meeseeks::summon(
            "  Open a PR to fix https\nvalidation  ",
            Criteria::default(),
            0,
        ));
        assert_eq!(subtitle(Some(&agent), NAME), "Open a PR to fix https validation");
        agent.meeseeks = Some(Meeseeks::summon("", Criteria::default(), 0));
        assert_eq!(subtitle(Some(&agent), NAME), NAME);
    }

    #[test]
    fn a_task_is_one_line_for_the_tooltip_and_cut_when_very_long() {
        let long = "word ".repeat(100);
        let cut = task_line(&long).unwrap();
        assert_eq!(cut.chars().count(), MAX_TASK_CHARS);
        assert!(cut.ends_with('\u{2026}'));
        let cases = [
            ("  Fix  the\tlogin\nredirect ", Some("Fix the login redirect")),
            ("", None),
            (" \n ", None),
        ];
        for (text, want) in cases {
            assert_eq!(task_line(text).as_deref(), want, "{text:?}");
        }
    }

    #[test]
    fn a_source_with_nothing_readable_is_passed_over() {
        let sources = Sources {
            session: Some("   "),
            terminal: Some("..."),
            goal: Some(""),
            first_message: Some("\n\n  Add dark mode\nthen ship it"),
            ..Sources::default()
        };
        assert_eq!(picked(sources), (Source::FirstMessage, "Add dark mode".to_owned()));
    }

    #[test]
    fn tidy_keeps_the_first_line_without_markdown_marks() {
        let cases = [
            ("Fix the login redirect", Some("Fix the login redirect")),
            ("  Fix   the\tlogin  redirect  ", Some("Fix the login redirect")),
            ("# Plan the migration\nwith steps", Some("Plan the migration")),
            ("> - quoted ask", Some("quoted ask")),
            ("**Ship** it", Some("Ship it")),
            ("\n\nsecond line is the first real one", Some("second line is the first real one")),
            ("", None),
            ("   \n  ", None),
            ("---", None),
            ("a", None),
        ];
        for (text, want) in cases {
            assert_eq!(tidy(text).as_deref(), want, "{text:?}");
        }
    }

    #[test]
    fn a_long_line_is_cut_with_a_mark() {
        let long = "word ".repeat(60);
        let title = tidy(&long).unwrap();
        assert_eq!(title.chars().count(), MAX_CHARS);
        assert!(title.ends_with('\u{2026}'));
    }

    #[test]
    fn claude_terminal_titles_lose_their_spinner_and_the_generic_name() {
        let cases = [
            ("claude", Some("\u{2733} Fix login redirect"), Some("Fix login redirect")),
            ("claude", Some("\u{25d0} Fix login redirect"), Some("Fix login redirect")),
            ("claude", Some("\u{2802} \u{b7} Fix login redirect"), Some("Fix login redirect")),
            ("claude", Some("Fix login redirect"), Some("Fix login redirect")),
            ("claude", Some("\u{2733} Claude Code"), None),
            ("claude", Some("Claude Code"), None),
            ("claude", Some("\u{2733}"), None),
            ("claude", None, None),
            ("codex", Some("acme-web | Working"), None),
            ("gemini", Some("\u{2726} Ready (acme-web)"), None),
            ("copilot", Some("GitHub Copilot"), None),
        ];
        for (agent, raw, want) in cases {
            assert_eq!(terminal_title(agent, raw).as_deref(), want, "{agent} {raw:?}");
        }
    }
}
