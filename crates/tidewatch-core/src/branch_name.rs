//! The name a new worktree can take: a slug of the first message, git's rules for one branch name
//! segment, and the next free variant of a taken name. Pure.

const MAX_WORDS: usize = 4;
const MAX_CHARS: usize = 30;

const STOP_WORDS: &[&str] = &[
    "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "do", "does", "for", "from",
    "had", "has", "have", "how", "if", "in", "into", "is", "it", "its", "just", "may", "me", "my",
    "not", "of", "on", "or", "our", "please", "should", "so", "than", "that", "the", "their",
    "then", "there", "these", "this", "those", "to", "us", "was", "we", "were", "what", "when",
    "where", "which", "while", "who",
];

const FORBIDDEN: [char; 8] = ['~', '^', ':', '?', '*', '[', '\\', '/'];

/// Lowercase ascii words of `text` joined with `-`: stop words and one-letter words dropped,
/// apostrophes removed, at most 4 words and 30 characters. Empty when no word is left.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut slug = String::new();
    let mut words = 0;
    let separators = |c: char| !(c.is_ascii_alphanumeric() || c == '\'' || c == '\u{2019}');
    for raw in text.split(separators) {
        let word: String = raw
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if word.len() < 2 || STOP_WORDS.contains(&word.as_str()) {
            continue;
        }
        if words == 0 {
            slug.extend(word.chars().take(MAX_CHARS));
        } else if words == MAX_WORDS || slug.len() + 1 + word.len() > MAX_CHARS {
            break;
        } else {
            slug.push('-');
            slug.push_str(&word);
        }
        words += 1;
    }
    slug
}

/// What is wrong with a worktree name, by git's rules for a branch name (`git check-ref-format`)
/// and Tidewatch's own: one segment, no leading dash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Empty,
    Space,
    Control,
    Forbidden(char),
    DoubleDot,
    AtBrace,
    JustAt,
    LeadingDash,
    LeadingDot,
    EndsWithLock,
    EndsWithDot,
}

impl Problem {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "Give the worktree a name.".to_owned(),
            Self::Space => "A branch name cannot have spaces.".to_owned(),
            Self::Control => "A branch name cannot have control characters.".to_owned(),
            Self::Forbidden('/') => "A branch name here is one word, without a slash.".to_owned(),
            Self::Forbidden(c) => format!("A branch name cannot have \"{c}\"."),
            Self::DoubleDot => "A branch name cannot have \"..\".".to_owned(),
            Self::AtBrace => "A branch name cannot have \"@{\".".to_owned(),
            Self::JustAt => "A branch name cannot be just \"@\".".to_owned(),
            Self::LeadingDash => "A branch name cannot start with a dash.".to_owned(),
            Self::LeadingDot => "A branch name cannot start with a dot.".to_owned(),
            Self::EndsWithLock => "A branch name cannot end with \".lock\".".to_owned(),
            Self::EndsWithDot => "A branch name cannot end with a dot.".to_owned(),
        }
    }
}

pub fn check(name: &str) -> Result<(), Problem> {
    if name.is_empty() {
        return Err(Problem::Empty);
    }
    if name.contains(' ') {
        return Err(Problem::Space);
    }
    if name.chars().any(|c| c.is_ascii_control()) {
        return Err(Problem::Control);
    }
    if let Some(c) = name.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(Problem::Forbidden(c));
    }
    if name.contains("..") {
        return Err(Problem::DoubleDot);
    }
    if name.contains("@{") {
        return Err(Problem::AtBrace);
    }
    if name == "@" {
        return Err(Problem::JustAt);
    }
    if name.starts_with('-') {
        return Err(Problem::LeadingDash);
    }
    if name.starts_with('.') {
        return Err(Problem::LeadingDot);
    }
    if name.rsplit_once('.').is_some_and(|(_, ending)| ending == "lock") {
        return Err(Problem::EndsWithLock);
    }
    if name.ends_with('.') {
        return Err(Problem::EndsWithDot);
    }
    Ok(())
}

pub fn next_free(name: &str, is_taken: impl Fn(&str) -> bool) -> String {
    (2..=u32::MAX)
        .map(|n| format!("{name}-{n}"))
        .find(|candidate| !is_taken(candidate))
        .expect("a counter that runs to u32::MAX finds a free name first")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_keeps_the_first_words_that_say_something() {
        let cases = [
            ("", ""),
            ("   ", ""),
            ("the and of", ""),
            ("Add retry to the stage gates", "add-retry-stage-gates"),
            ("When a workflow stage's tests fail, retry the stage", "workflow-stages-tests-fail"),
            ("Fix the login bug!", "fix-login-bug"),
            ("fix: handle `null` users (again)", "fix-handle-null-users"),
            ("one two three four five six", "one-two-three-four"),
            ("Ship v2 of the importer", "ship-v2-importer"),
            ("naive café résumé parser", "naive-caf-sum-parser"),
            ("a b c d e", ""),
            ("don't panic", "dont-panic"),
            ("REFACTOR   THE\tWORKTREE\nNAMES", "refactor-worktree-names"),
        ];
        for (text, want) in cases {
            assert_eq!(slug(text), want, "{text:?}");
        }
    }

    #[test]
    fn a_slug_stops_at_thirty_characters_between_words() {
        let long = "implementation authentication configuration";
        assert_eq!(slug(long), "implementation-authentication");
        assert!(slug(long).len() <= MAX_CHARS);
        let one_word = "a".repeat(80);
        assert_eq!(slug(&one_word), "a".repeat(MAX_CHARS));
    }

    #[test]
    fn a_slug_is_always_a_valid_name_or_empty() {
        for text in ["", "!!!", "x.lock", "--dash", "..", "a/b c", "😀 emoji only 😀", "@{x}"] {
            let slug = slug(text);
            assert!(slug.is_empty() || check(&slug).is_ok(), "{text:?} -> {slug:?}");
        }
    }

    #[test]
    fn git_rules_for_a_branch_name_segment() {
        let cases = [
            ("retry-stage-gates", Ok(())),
            ("fix_1.2", Ok(())),
            ("v1.0", Ok(())),
            ("a", Ok(())),
            ("CamelCase", Ok(())),
            ("naïve", Ok(())),
            ("", Err(Problem::Empty)),
            ("two words", Err(Problem::Space)),
            ("tab\there", Err(Problem::Control)),
            ("new\nline", Err(Problem::Control)),
            ("del\u{7f}", Err(Problem::Control)),
            ("a~b", Err(Problem::Forbidden('~'))),
            ("a^b", Err(Problem::Forbidden('^'))),
            ("a:b", Err(Problem::Forbidden(':'))),
            ("a?b", Err(Problem::Forbidden('?'))),
            ("a*b", Err(Problem::Forbidden('*'))),
            ("a[b", Err(Problem::Forbidden('['))),
            ("a\\b", Err(Problem::Forbidden('\\'))),
            ("a/b", Err(Problem::Forbidden('/'))),
            ("a..b", Err(Problem::DoubleDot)),
            ("..", Err(Problem::DoubleDot)),
            ("a@{b", Err(Problem::AtBrace)),
            ("@", Err(Problem::JustAt)),
            ("a@b", Ok(())),
            ("-lead", Err(Problem::LeadingDash)),
            ("--force", Err(Problem::LeadingDash)),
            (".hidden", Err(Problem::LeadingDot)),
            ("topic.lock", Err(Problem::EndsWithLock)),
            ("topic.lock.x", Ok(())),
            ("trail.", Err(Problem::EndsWithDot)),
        ];
        for (name, want) in cases {
            assert_eq!(check(name), want, "{name:?}");
        }
    }

    #[test]
    fn every_problem_has_its_own_message() {
        let problems = [
            Problem::Empty,
            Problem::Space,
            Problem::Control,
            Problem::Forbidden('~'),
            Problem::Forbidden('/'),
            Problem::DoubleDot,
            Problem::AtBrace,
            Problem::JustAt,
            Problem::LeadingDash,
            Problem::LeadingDot,
            Problem::EndsWithLock,
            Problem::EndsWithDot,
        ];
        let messages: std::collections::HashSet<String> =
            problems.iter().map(Problem::message).collect();
        assert_eq!(messages.len(), problems.len());
    }

    #[test]
    fn a_taken_name_gets_the_first_free_number_from_two() {
        let cases: [(&[&str], &str); 3] = [
            (&["fix"], "fix-2"),
            (&["fix", "fix-2", "fix-3"], "fix-4"),
            (&["fix", "fix-3"], "fix-2"),
        ];
        for (taken, want) in cases {
            assert_eq!(next_free("fix", |name| taken.contains(&name)), want, "{taken:?}");
        }
    }
}
