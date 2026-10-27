//! The last lines of what a service printed, with terminal colors and control codes removed, so
//! an agent (or a test) reads text. Pure.

use std::collections::VecDeque;

pub const KEPT_LINES: usize = 2000;
pub const MAX_LINE_CHARS: usize = 400;

/// `raw` as plain text: escape sequences, control characters and text a carriage return
/// overwrote are gone, and a tab is one space.
#[must_use]
pub fn clean_line(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut overwrite = false;
    while let Some(c) = chars.next() {
        let printed = match c {
            '\u{1b}' => {
                skip_sequence(&mut chars);
                None
            }
            '\r' => {
                overwrite = true;
                None
            }
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        };
        if let Some(c) = printed {
            if std::mem::take(&mut overwrite) {
                out.clear();
            }
            out.push(c);
        }
    }
    let trimmed = out.trim_end();
    trimmed.chars().take(MAX_LINE_CHARS).collect()
}

fn skip_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        Some('[') => {
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
        Some(']') => {
            while let Some(c) = chars.next() {
                if c == '\u{7}' {
                    break;
                }
                if c == '\u{1b}' {
                    chars.next();
                    break;
                }
            }
        }
        Some('(' | ')' | '#') => {
            chars.next();
        }
        _ => {}
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    lines: VecDeque<String>,
}

impl Log {
    /// Every line cleaned, a run of empty lines at the end dropped, the newest `KEPT_LINES` kept.
    #[must_use]
    pub fn from_lines<I, S>(lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut kept: VecDeque<String> =
            lines.into_iter().flat_map(|raw| split_lines(raw.as_ref())).collect();
        while kept.back().is_some_and(String::is_empty) {
            kept.pop_back();
        }
        while kept.len() > KEPT_LINES {
            kept.pop_front();
        }
        Self { lines: kept }
    }

    /// The last `n` lines, oldest first.
    #[must_use]
    pub fn tail(&self, n: usize) -> Vec<String> {
        let skip = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(skip).cloned().collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

fn split_lines(raw: &str) -> Vec<String> {
    raw.split('\n').map(clean_line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_control_codes_are_removed() {
        let cases = [
            ("plain", "plain"),
            ("\u{1b}[32mgreen\u{1b}[0m text", "green text"),
            ("\u{1b}[1;38;5;208mbold\u{1b}[m", "bold"),
            ("\u{1b}]0;title\u{7}after title", "after title"),
            ("\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\", "link"),
            ("a\u{1b}(Bb", "ab"),
            ("a\tb", "a b"),
            ("bell\u{7}here", "bellhere"),
            ("progress 10%\rprogress 100%", "progress 100%"),
            ("windows line\r", "windows line"),
            ("  trailing space   ", "  trailing space"),
            ("\u{1b}[", ""),
            ("\u{1b}", ""),
        ];
        for (raw, want) in cases {
            assert_eq!(clean_line(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn a_very_long_line_is_cut() {
        let long = "x".repeat(MAX_LINE_CHARS + 50);
        assert_eq!(clean_line(&long).chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn the_tail_is_the_newest_lines_oldest_first() {
        let log = Log::from_lines(["one", "two", "three", "four"]);
        assert_eq!(log.tail(2), ["three", "four"]);
        assert_eq!(log.tail(10), ["one", "two", "three", "four"]);
        assert_eq!(log.tail(0), Vec::<String>::new());
    }

    #[test]
    fn blank_lines_at_the_end_of_a_screen_are_dropped_and_those_between_stay() {
        let log = Log::from_lines(["a", "", "b", "", "   ", ""]);
        assert_eq!(log.tail(10), ["a", "", "b"]);
    }

    #[test]
    fn a_chunk_with_newlines_becomes_lines() {
        let log = Log::from_lines(["\u{1b}[31mone\u{1b}[0m\ntwo\n"]);
        assert_eq!(log.tail(5), ["one", "two"]);
    }

    #[test]
    fn only_the_newest_lines_are_kept() {
        let log = Log::from_lines((0..KEPT_LINES + 5).map(|n| format!("line {n}")));
        assert_eq!(log.len(), KEPT_LINES);
        assert_eq!(log.tail(1), [format!("line {}", KEPT_LINES + 4)]);
        assert_eq!(log.tail(KEPT_LINES)[0], "line 5");
    }

    #[test]
    fn no_output_is_an_empty_log() {
        let log = Log::from_lines(Vec::<String>::new());
        assert!(log.is_empty());
        assert_eq!(log.tail(5), Vec::<String>::new());
    }
}
