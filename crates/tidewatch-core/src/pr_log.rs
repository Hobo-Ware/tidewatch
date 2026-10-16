//! The few lines of a failed CI job worth showing and pasting (PLAN.md, D4 follow-up): the tail of
//! `gh run view <run> --job <job> --log-failed`, cleaned. Pure.

const ERROR_MARK: &str = "##[error]";
const MARKS: [&str; 5] = ["##[group]", "##[endgroup]", "##[warning]", "##[debug]", "##[notice]"];
pub const TAIL_LINES: usize = 12;
const LINE_CHARS: usize = 200;

/// The log of a job that failed ends in a `##[error]` line followed by the runner's cleanup, so the
/// tail that matters ends at the last error line, not at the end of the file.
#[must_use]
pub fn tail(raw: &str) -> Vec<String> {
    let lines: Vec<String> = raw.lines().filter_map(clean_line).collect();
    let end =
        lines.iter().rposition(|line| line.starts_with(ERROR_MARK)).map_or(lines.len(), |i| i + 1);
    let shown: Vec<String> = lines[..end]
        .iter()
        .map(|line| line.strip_prefix(ERROR_MARK).unwrap_or(line).trim_end().to_owned())
        .collect();
    let start = shown.len().saturating_sub(TAIL_LINES);
    let mut tail = shown[start..].to_vec();
    while tail.first().is_some_and(String::is_empty) {
        tail.remove(0);
    }
    while tail.last().is_some_and(String::is_empty) {
        tail.pop();
    }
    tail
}

/// `gh` 2.98.0 prints one line per log line as `<job>\t<step>\t<timestamp> <text>` (the step is
/// `UNKNOWN STEP` when GitHub did not say), with the runner's colors in it.
fn clean_line(line: &str) -> Option<String> {
    let text = strip_columns(line);
    let text = strip_escapes(text);
    let text: String = text.chars().filter(|c| *c == '\t' || !c.is_control()).collect();
    let text = text.trim_start_matches('\u{feff}').to_owned();
    if MARKS.iter().any(|mark| text.starts_with(mark)) {
        return None;
    }
    Some(cut(&text))
}

fn strip_columns(line: &str) -> &str {
    let mut parts = line.splitn(3, '\t');
    let (Some(_job), Some(_step), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return line;
    };
    let rest = rest.trim_start_matches('\u{feff}');
    match rest.split_once(' ') {
        Some((stamp, text)) if looks_like_timestamp(stamp) => text,
        _ => rest,
    }
}

fn looks_like_timestamp(text: &str) -> bool {
    text.len() >= 20 && text.as_bytes()[4] == b'-' && text.contains('T') && text.ends_with('Z')
}

fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(next) = chars.next() {
                    if next == '\u{7}' || (next == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn cut(text: &str) -> String {
    if text.chars().count() <= LINE_CHARS {
        return text.to_owned();
    }
    let head: String = text.chars().take(LINE_CHARS - 3).collect();
    format!("{head}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh_line(text: &str) -> String {
        format!("check (ubuntu-latest)\tUNKNOWN STEP\t2026-10-14T23:23:49.7754280Z {text}")
    }

    #[test]
    fn the_tail_ends_at_the_last_error_line_not_at_the_cleanup() {
        let log = [
            gh_line("compiling"),
            gh_line("\u{1b}[1m\u{1b}[91merror\u{1b}[0m: unused import: `Sense`"),
            gh_line(" --> crates/tidewatch/src/leftover_view.rs:6:77"),
            gh_line("##[error]Process completed with exit code 101."),
            gh_line("Post job cleanup."),
            gh_line("Cleaning up orphan processes"),
        ]
        .join("\n");
        assert_eq!(
            tail(&log),
            [
                "compiling",
                "error: unused import: `Sense`",
                " --> crates/tidewatch/src/leftover_view.rs:6:77",
                "Process completed with exit code 101.",
            ]
        );
    }

    #[test]
    fn a_log_with_no_error_line_gives_its_last_lines() {
        let log: String =
            (1..=30).map(|n| gh_line(&format!("line {n}"))).collect::<Vec<_>>().join("\n");
        let got = tail(&log);
        assert_eq!(got.len(), TAIL_LINES);
        assert_eq!(got.first().map(String::as_str), Some("line 19"));
        assert_eq!(got.last().map(String::as_str), Some("line 30"));
    }

    #[test]
    fn markup_columns_escapes_and_control_characters_are_removed() {
        let cases = [
            ("a\tb\t2026-10-14T23:23:49.1Z \u{feff}plain", "plain"),
            ("a\tb\t\u{feff}2026-10-14T23:23:49.1234567Z \u{1b}[31mred\u{1b}[0m", "red"),
            ("a\tb\t2026-10-14T23:23:49.1234567Z title\u{7}\u{8}x", "titlex"),
            ("a\tb\t2026-10-14T23:23:49.1234567Z \u{1b}]0;set title\u{7}after", "after"),
            ("no columns here", "no columns here"),
            ("a\tb\tnot a stamp text", "not a stamp text"),
        ];
        for (raw, want) in cases {
            assert_eq!(tail(raw), [want], "{raw:?}");
        }
    }

    #[test]
    fn the_runners_group_and_warning_lines_are_dropped() {
        let log = [
            gh_line("##[group]Run cargo clippy"),
            gh_line("real output"),
            gh_line("##[endgroup]"),
            gh_line("##[warning]Node.js 20 is deprecated"),
        ]
        .join("\n");
        assert_eq!(tail(&log), ["real output"]);
    }

    #[test]
    fn long_lines_are_cut_with_an_ellipsis_and_blank_edges_dropped() {
        let log = format!("\n{}\n\n", gh_line(&"x".repeat(500)));
        let got = tail(&log);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].chars().count(), LINE_CHARS);
        assert!(got[0].ends_with("..."));
    }

    #[test]
    fn nothing_in_nothing_out() {
        assert_eq!(tail(""), Vec::<String>::new());
        assert_eq!(tail("\n\n"), Vec::<String>::new());
    }
}
