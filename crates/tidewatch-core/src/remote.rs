//! Remote control of a session: the name it is given and the link the CLI prints for it.

use crate::agents::Wiring;

const REPO_MAX: usize = 24;
const TAB_MAX: usize = 32;
const FALLBACK_NAME: &str = "tidewatch";
const CLAUDE_PATH: &str = "/code/";
const WRAP_ROWS: usize = 3;

/// Control characters dropped, runs of spaces collapsed, a leading `-` removed (the CLI would read
/// it as a flag), cut to `max` characters with an ellipsis.
fn tidy(text: &str, max: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let joined: String = words.join(" ").chars().filter(|c| !c.is_control()).collect();
    let joined = joined.trim_start_matches(|c: char| c == '-' || c.is_whitespace()).trim_end();
    if joined.chars().count() <= max {
        return joined.to_owned();
    }
    let cut: String = joined.chars().take(max.saturating_sub(1)).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// The name of a remote session: the repo and the worktree, such as `acme-web · retry-stage-gates`.
/// A tab with no worktree (the Other group) is named by its folder alone.
#[must_use]
pub fn session_name(repo: &str, worktree: Option<&str>) -> String {
    let repo = tidy(repo, REPO_MAX);
    let tab = worktree.map(|tab| tidy(tab, TAB_MAX)).unwrap_or_default();
    match (repo.is_empty(), tab.is_empty()) {
        (false, false) => format!("{repo} \u{b7} {tab}"),
        (false, true) => repo,
        (true, false) => tab,
        (true, true) => FALLBACK_NAME.to_owned(),
    }
}

fn ends_link(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || ('\u{2500}'..='\u{257f}').contains(&c)
        || "<>\"'`".contains(c)
}

fn leading_token(row: &str) -> &str {
    let end = row.find(ends_link).unwrap_or(row.len());
    &row[..end]
}

/// The link starting at `at` in `rows[row]`, joined with the rows below it when it runs to the last
/// column of a full row (the terminal wrapped it).
fn link_at(rows: &[String], row: usize, at: usize, cols: usize) -> String {
    let first = &rows[row][at..];
    let mut link = leading_token(first).to_owned();
    let mut ran_to_edge = link.len() == first.len() && rows[row].chars().count() >= cols;
    for below in rows.iter().skip(row + 1).take(WRAP_ROWS) {
        if !ran_to_edge || below.starts_with(ends_link) {
            break;
        }
        let token = leading_token(below);
        link.push_str(token);
        ran_to_edge = token.len() == below.len() && below.chars().count() >= cols;
    }
    link.trim_end_matches(['.', ',', ';', ':', ')', ']', '}', '!', '?']).to_owned()
}

fn is_session_link(wiring: Wiring, link: &str) -> bool {
    let Some(rest) = link.strip_prefix("https://") else { return false };
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, path) = rest.split_at(host_end);
    let host = host.to_ascii_lowercase();
    match wiring {
        Wiring::Claude => {
            matches!(host.as_str(), "claude.ai" | "www.claude.ai")
                && path.starts_with(CLAUDE_PATH)
                && path.len() > CLAUDE_PATH.len()
        }
        Wiring::Copilot => {
            matches!(host.as_str(), "github.com" | "www.github.com") && path.len() > 1
        }
        Wiring::Codex | Wiring::Gemini | Wiring::Custom => false,
    }
}

/// The newest remote session link the CLI printed, from the terminal's rows (`cols` wide) as they
/// stand. Claude prints a `claude.ai/code/...` link and Copilot a `github.com` one. Nothing is
/// guessed: no link on screen is no link.
#[must_use]
pub fn find_link(wiring: Wiring, rows: &[String], cols: usize) -> Option<String> {
    let mut newest = None;
    for (row, text) in rows.iter().enumerate() {
        for (at, _) in text.match_indices("https://") {
            let link = link_at(rows, row, at, cols);
            if is_session_link(wiring, &link) {
                newest = Some(link);
            }
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    #[test]
    fn names_join_the_repo_and_the_worktree_and_stay_short_and_safe() {
        let long = "a".repeat(60);
        let cases: [(&str, Option<&str>, String); 9] = [
            ("acme-web", Some("retry-stage-gates"), "acme-web \u{b7} retry-stage-gates".into()),
            ("acme-web", None, "acme-web".into()),
            ("", Some("otter"), "otter".into()),
            ("", None, "tidewatch".into()),
            ("  acme   web ", Some("a\tb\nc"), "acme web \u{b7} a b c".into()),
            ("--acme", Some("--otter"), "acme \u{b7} otter".into()),
            ("ac\u{1b}[31me", Some("x"), "ac[31me \u{b7} x".into()),
            (&long, Some("x"), format!("{}\u{2026} \u{b7} x", "a".repeat(23))),
            ("web", Some(&long), format!("web \u{b7} {}\u{2026}", "a".repeat(31))),
        ];
        for (repo, tab, want) in cases {
            assert_eq!(session_name(repo, tab), want, "{repo:?} {tab:?}");
        }
    }

    #[test]
    fn a_name_never_starts_with_a_dash_or_runs_past_the_limits() {
        for repo in ["-x", "---", "-- -a"] {
            assert!(!session_name(repo, Some("-y")).starts_with('-'), "{repo}");
        }
        let name = session_name(&"w".repeat(500), Some(&"t".repeat(500)));
        assert!(name.chars().count() <= REPO_MAX + TAB_MAX + 3, "{name}");
    }

    #[test]
    fn links_are_found_per_cli_and_nothing_is_made_up() {
        let claude_link = "https://claude.ai/code/session_01AbCdEf";
        let github_link = "https://github.com/acme/web/tasks/42";
        let cases: [(Wiring, Vec<String>, Option<&str>); 17] = [
            (
                Wiring::Claude,
                rows(&[&format!("Remote Control active: {claude_link}")]),
                Some(claude_link),
            ),
            (
                Wiring::Claude,
                rows(&[&format!("\u{2502} {claude_link} \u{2502}")]),
                Some(claude_link),
            ),
            (Wiring::Claude, rows(&[&format!("open {claude_link}.")]), Some(claude_link)),
            (Wiring::Claude, rows(&[&format!("({claude_link})")]), Some(claude_link)),
            (Wiring::Claude, rows(&["see https://claude.ai/code for more"]), None),
            (Wiring::Claude, rows(&["https://claude.ai/code/"]), None),
            (Wiring::Claude, rows(&["https://claude.ai/settings/usage"]), None),
            (Wiring::Claude, rows(&["https://github.com/acme/web/tasks/42"]), None),
            (Wiring::Claude, rows(&["http://claude.ai/code/session_1"]), None),
            (Wiring::Claude, rows(&["https://evil.example/claude.ai/code/session_1"]), None),
            (Wiring::Claude, rows(&["https://claude.ai.evil.example/code/session_1"]), None),
            (Wiring::Copilot, rows(&[&format!("Remote: {github_link}")]), Some(github_link)),
            (Wiring::Copilot, rows(&["https://github.com/"]), None),
            (Wiring::Copilot, rows(&[claude_link]), None),
            (Wiring::Codex, rows(&[claude_link, github_link]), None),
            (Wiring::Gemini, rows(&[claude_link, github_link]), None),
            (Wiring::Claude, rows(&["", "no link here"]), None),
        ];
        for (wiring, text, want) in cases {
            assert_eq!(find_link(wiring, &text, 80).as_deref(), want, "{wiring:?} {text:?}");
        }
    }

    #[test]
    fn the_newest_link_wins() {
        let text = rows(&[
            "https://claude.ai/code/session_old",
            "later",
            "https://claude.ai/code/session_new",
        ]);
        assert_eq!(
            find_link(Wiring::Claude, &text, 80).as_deref(),
            Some("https://claude.ai/code/session_new")
        );
    }

    #[test]
    fn a_link_the_terminal_wrapped_is_joined_and_a_short_row_is_not() {
        let link = "https://claude.ai/code/session_01AbCdEfGhIjKl";
        let (head, tail) = link.split_at(30);
        let wrapped = rows(&[head, tail]);
        assert_eq!(find_link(Wiring::Claude, &wrapped, 30).as_deref(), Some(link));
        assert_eq!(
            find_link(Wiring::Claude, &wrapped, 80).as_deref(),
            Some(head),
            "row 0 ends before the edge, so it is the whole link"
        );
        let spaced = rows(&[head, " tail"]);
        assert_eq!(find_link(Wiring::Claude, &spaced, 30).as_deref(), Some(head));
        let three = rows(&["https://claude.ai/c", "ode/session_01AbCdE", "fGh"]);
        assert_eq!(
            find_link(Wiring::Claude, &three, 19).as_deref(),
            Some("https://claude.ai/code/session_01AbCdEfGh")
        );
    }
}
