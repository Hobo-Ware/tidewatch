//! The keys that answer a CLI's permission prompt, so the Reader's buttons can type them into the
//! pty. Only answers verified against a real prompt are listed; for any other agent the Reader
//! offers "Switch to terminal" instead of guessing.
//!
//! Claude Code 2.1.296 at Ask level, checked 2026-10-13 in real tabs (raw grid read, then each key sent
//! from the Reader): the prompt is a numbered list, "1. Yes", "2. Yes, and ..." (seen for a file
//! write; other tools word it differently), "3. No", under "Esc to cancel". "1" allowed the write
//! (the file was created, the tab went back to Working). Esc denied it (no file, the transcript got
//! "[Request interrupted by user for tool use]"). Deny is Esc, not "3", because the options can differ per tool.
//! A denied prompt sends no hook and the status stays "needs you", so the Reader hides its buttons
//! after one answer until the next hook.

use tidewatch_core::transcript::Source;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Allow,
    Deny,
}

impl Answer {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Allow => "Allow",
            Self::Deny => "Deny",
        }
    }
}

const CLAUDE: &[(Answer, &[u8])] = &[(Answer::Allow, b"1"), (Answer::Deny, b"\x1b")];

/// The answers a CLI's permission prompt takes and the bytes that give each, or none when they are
/// not verified.
pub(crate) fn answers(source: Option<Source>) -> &'static [(Answer, &'static [u8])] {
    match source {
        Some(Source::Claude) => CLAUDE,
        _ => &[],
    }
}

pub(crate) fn bytes(source: Option<Source>, answer: Answer) -> Option<&'static [u8]> {
    answers(source).iter().find(|(a, _)| *a == answer).map(|(_, keys)| *keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_allows_with_one_and_denies_with_escape() {
        assert_eq!(bytes(Some(Source::Claude), Answer::Allow), Some(&b"1"[..]));
        assert_eq!(bytes(Some(Source::Claude), Answer::Deny), Some(&b"\x1b"[..]));
    }

    #[test]
    fn unverified_prompts_have_no_buttons() {
        for source in [Some(Source::Gemini), Some(Source::Copilot), Some(Source::Codex), None] {
            assert_eq!(answers(source).len(), 0, "{source:?}");
            assert_eq!(bytes(source, Answer::Allow), None);
        }
    }

    #[test]
    fn every_listed_answer_has_a_label_and_distinct_keys() {
        let listed = answers(Some(Source::Claude));
        assert_ne!(listed[0].1, listed[1].1);
        assert_eq!(listed[0].0.label(), "Allow");
        assert_eq!(listed[1].0.label(), "Deny");
    }
}
