//! Small text helpers every screen and message builder shares.

/// `text` with every run of whitespace, line breaks included, collapsed to one space and the ends trimmed.
#[must_use]
pub fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `count` and the noun that fits it: `1 file`, `2 files`.
#[must_use]
pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// `text` cut to `max` characters with `...` after the cut, or unchanged when it fits.
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max).collect();
    clipped.push_str("...");
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squash_collapses_every_whitespace_run() {
        let cases = [
            ("", ""),
            ("   ", ""),
            ("one", "one"),
            ("  a   b  ", "a b"),
            ("a\nb\t\tc\r\nd", "a b c d"),
            ("Thu Oct  8 21:57:15 2026", "Thu Oct 8 21:57:15 2026"),
            ("tab\u{a0}nbsp", "tab nbsp"),
        ];
        for (input, want) in cases {
            assert_eq!(squash(input), want, "input {input:?}");
        }
    }

    #[test]
    fn plural_picks_the_noun_by_count() {
        let cases = [(0, "0 files"), (1, "1 file"), (2, "2 files"), (11, "11 files")];
        for (count, want) in cases {
            assert_eq!(plural(count, "file", "files"), want, "count {count}");
        }
        assert_eq!(plural(3, "is", "are"), "3 are");
    }

    #[test]
    fn clip_cuts_by_characters_and_marks_the_cut() {
        let cases = [
            ("", 3, ""),
            ("abc", 3, "abc"),
            ("abcd", 3, "abc..."),
            ("caf\u{e9}s", 4, "caf\u{e9}..."),
            ("a\u{1F600}b", 2, "a\u{1F600}..."),
            ("abc", 0, "..."),
        ];
        for (input, max, want) in cases {
            assert_eq!(clip(input, max), want, "input {input:?} max {max}");
        }
    }
}
