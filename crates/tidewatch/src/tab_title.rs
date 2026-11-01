//! The label of an agent tab: what the work is, not the worktree's name. `pick` holds the order.

use stdusk_egui::ui::ellipsize;
use tidewatch_core::branch_name;
use tidewatch_core::text::squash;
use tidewatch_core::user_markup::without_image_markers;

use crate::pane::AgentTab;

pub(crate) const MAX_CHARS: usize = 100;
const MAX_TASK_CHARS: usize = 300;
const MIN_LETTERS: usize = 2;
const GENERIC_TERMINAL_TITLES: [&str; 1] = ["claude code"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Own,
    Chosen,
    TaskSlug,
    GoalSlug,
    Session,
    Terminal,
    FirstMessage,
    Name,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sources<'a> {
    pub(crate) own: Option<&'a str>,
    pub(crate) chosen: Option<&'a str>,
    pub(crate) task: Option<&'a str>,
    pub(crate) session: Option<&'a str>,
    pub(crate) terminal: Option<&'a str>,
    pub(crate) goal: Option<&'a str>,
    pub(crate) first_message: Option<&'a str>,
}

pub(crate) fn pick(sources: Sources<'_>, name: &str) -> (Source, String) {
    let task_slug = sources.task.map(branch_name::slug);
    let goal_slug = sources.goal.map(branch_name::slug);
    [
        (Source::Own, sources.own.map(str::trim)),
        (Source::Chosen, sources.chosen.map(str::trim)),
        (Source::TaskSlug, task_slug.as_deref()),
        (Source::GoalSlug, goal_slug.as_deref()),
        (Source::Session, sources.session),
        (Source::Terminal, sources.terminal),
        (Source::FirstMessage, sources.first_message),
    ]
    .into_iter()
    .find_map(|(source, text)| {
        let text = text?;
        let shown = match source {
            Source::Own | Source::Chosen | Source::TaskSlug | Source::GoalSlug => {
                (!text.is_empty()).then(|| text.to_owned())
            }
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
            own: agent.reader.own_title(),
            chosen: agent
                .chosen_name
                .as_deref()
                .or_else(|| meeseeks.and_then(|meeseeks| meeseeks.name.as_deref())),
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
    let line = squash(task);
    (!line.is_empty()).then(|| ellipsize(&line, MAX_TASK_CHARS).0)
}

/// The first line of `text` as a short title: picture markers, markdown marks and extra spaces gone. `None` when
/// nothing readable is left.
pub(crate) fn tidy(text: &str) -> Option<String> {
    let text = without_image_markers(text);
    let line = text.lines().map(str::trim).find(|line| line.chars().any(char::is_alphanumeric))?;
    let line = line.replace("**", "");
    let line =
        line.trim_start_matches(|c: char| matches!(c, '#' | '>' | '-' | '*') || c.is_whitespace());
    let line = squash(line);
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
            own: Some("Login work"),
            chosen: Some("fix-login"),
            task: Some("Fix the login redirect on signup"),
            session: Some("Fix login redirect"),
            terminal: Some("Terminal title"),
            goal: Some("Soundtrack backfill blind spots"),
            first_message: Some("First message"),
        };
        let cases = [
            (all, (Source::Own, "Login work")),
            (Sources { own: None, ..all }, (Source::Chosen, "fix-login")),
            (
                Sources { own: None, chosen: None, ..all },
                (Source::TaskSlug, "fix-login-redirect-signup"),
            ),
            (
                Sources { own: None, chosen: None, task: None, ..all },
                (Source::GoalSlug, "soundtrack-backfill-blind"),
            ),
            (
                Sources { own: None, chosen: None, task: None, goal: None, ..all },
                (Source::Session, "Fix login redirect"),
            ),
            (
                Sources { own: None, chosen: None, task: None, goal: None, session: None, ..all },
                (Source::Terminal, "Terminal title"),
            ),
            (
                Sources {
                    own: None,
                    chosen: None,
                    task: None,
                    goal: None,
                    session: None,
                    terminal: None,
                    ..all
                },
                (Source::FirstMessage, "First message"),
            ),
            (Sources::default(), (Source::Name, NAME)),
        ];
        for (sources, (source, text)) in cases {
            assert_eq!(picked(sources), (source, text.to_owned()), "{sources:?}");
        }
    }

    #[test]
    fn the_owners_rename_stays_as_written_and_beats_a_name_the_agent_chose() {
        let cases = [
            ("Sync retries", "Sync retries"),
            ("  Sync   Retries  ", "Sync   Retries"),
            ("MixedCase_name v2", "MixedCase_name v2"),
        ];
        for (own, want) in cases {
            let sources = Sources {
                own: Some(own),
                chosen: Some("retry-sync"),
                goal: Some("Add retry to the sync job"),
                ..Sources::default()
            };
            assert_eq!(picked(sources), (Source::Own, want.to_owned()), "{own:?}");
        }
        let blank = Sources { own: Some("   "), chosen: Some("retry-sync"), ..Sources::default() };
        assert_eq!(picked(blank), (Source::Chosen, "retry-sync".to_owned()));
    }

    #[test]
    fn a_chosen_name_beats_a_slug_of_the_goal_and_the_goal_is_a_kebab_slug() {
        let cases = [
            (Some("retry-sync"), "Soundtrack backfill blind spots", Source::Chosen, "retry-sync"),
            (
                None,
                "Soundtrack backfill blind spots",
                Source::GoalSlug,
                "soundtrack-backfill-blind",
            ),
            (None, "Fix the login redirect", Source::GoalSlug, "fix-login-redirect"),
            (
                None,
                "Open a PR to fix https validation for issue 2333",
                Source::GoalSlug,
                "fix-https-validation-issue",
            ),
        ];
        for (chosen, goal, source, want) in cases {
            let sources = Sources {
                chosen,
                goal: Some(goal),
                session: Some("Session title"),
                ..Sources::default()
            };
            assert_eq!(picked(sources), (source, want.to_owned()), "{goal:?}");
        }
    }

    #[test]
    fn a_goal_with_no_slug_falls_through_to_the_next_source() {
        let sources = Sources {
            goal: Some("?!"),
            session: Some("Session title"),
            first_message: Some("First message"),
            ..Sources::default()
        };
        assert_eq!(picked(sources), (Source::Session, "Session title".to_owned()));
        let nothing_else = Sources { session: None, ..sources };
        assert_eq!(picked(nothing_else), (Source::FirstMessage, "First message".to_owned()));
        let wordless = Sources {
            goal: Some("\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}"),
            ..Sources::default()
        };
        assert_eq!(picked(wordless), (Source::Name, NAME.to_owned()));
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
                (Source::GoalSlug, "ledger-goal"),
            ),
            (Sources { chosen: None, task: None, goal: None, ..titles }, (Source::Session, task)),
        ];
        for (sources, (source, text)) in cases {
            assert_eq!(picked(sources), (source, text.to_owned()), "{sources:?}");
        }
    }

    #[test]
    fn an_agent_tab_reads_the_owners_rename_then_the_agents_name_then_a_slug_of_its_goal() {
        use crate::agent_run::AgentRun;
        use tidewatch_core::meeseeks::{Criteria, Meeseeks};
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        agent.run.set_goal(Some("Soundtrack backfill blind spots"));
        let label = |agent: &AgentTab| of_agent(agent, NAME);
        assert_eq!(label(&agent), (Source::GoalSlug, "soundtrack-backfill-blind".to_owned()));
        agent.chosen_name = Some("backfill-soundtrack".into());
        assert_eq!(label(&agent), (Source::Chosen, "backfill-soundtrack".to_owned()));
        agent.reader.fake_own_title(Some("Soundtrack work"));
        assert_eq!(label(&agent), (Source::Own, "Soundtrack work".to_owned()));
        agent.reader.fake_own_title(None);
        agent.chosen_name = None;
        agent.meeseeks = Some(Meeseeks {
            name: Some("older-saved-name".into()),
            ..Meeseeks::summon("Fix the login", Criteria::default(), 0)
        });
        assert_eq!(label(&agent), (Source::Chosen, "older-saved-name".to_owned()));
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

    #[test]
    fn a_first_message_loses_its_picture_markers_and_one_of_only_pictures_falls_through() {
        let cases = [
            (
                "[Image #2]Also this should just be costs, it makes no sense",
                (Source::FirstMessage, "Also this should just be costs, it makes no sense"),
            ),
            ("[Image #1] [Image #2] compare these", (Source::FirstMessage, "compare these")),
            ("see [Image #3] now", (Source::FirstMessage, "see now")),
            ("[Image #1]\n\nfix the header", (Source::FirstMessage, "fix the header")),
            ("[Image #1]", (Source::Name, NAME)),
            ("[Image #1][Image #2]", (Source::Name, NAME)),
        ];
        for (first, (source, text)) in cases {
            let sources = Sources { first_message: Some(first), ..Sources::default() };
            assert_eq!(picked(sources), (source, text.to_owned()), "{first:?}");
        }
        let with_goal = Sources {
            goal: Some("Ledger goal"),
            first_message: Some("[Image #1]"),
            ..Sources::default()
        };
        assert_eq!(picked(with_goal), (Source::GoalSlug, "ledger-goal".to_owned()));
        let after = Sources { first_message: Some("[Image #1]"), ..Sources::default() };
        assert_eq!(picked(after).0, Source::Name);
    }
}
