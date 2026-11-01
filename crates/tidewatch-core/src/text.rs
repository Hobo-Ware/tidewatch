//! Small text helpers every screen and message builder shares.

/// `text` with every run of whitespace, line breaks included, collapsed to one space and the ends trimmed.
#[must_use]
pub fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
}
