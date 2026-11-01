//! The name a new worktree can take: a slug of the first message, git's rules for one branch name
//! segment, and the next free variant of a taken name. Pure.

const MAX_WORDS: usize = 4;
const MAX_CHARS: usize = 30;

const STOP_WORDS: &[&str] = &[
    "about", "all", "also", "an", "and", "any", "are", "as", "at", "be", "but", "by", "can",
    "could", "do", "does", "for", "from", "get", "had", "has", "have", "help", "how", "if", "in",
    "into", "is", "it", "its", "just", "let", "lets", "like", "make", "may", "me", "my", "need",
    "needs", "not", "now", "of", "on", "open", "or", "our", "please", "should", "so", "some",
    "sure", "than", "thanks", "that", "the", "their", "then", "there", "these", "this", "those",
    "to", "us", "want", "wants", "was", "we", "were", "what", "when", "where", "which", "while",
    "who", "will", "with", "would", "you", "your",
];

const NOUNS_NEEDING_A_NUMBER: &[&str] = &["issue", "issues", "pr", "prs"];
const MIN_BARE_NUMBER_DIGITS: usize = 3;

const FORBIDDEN: [char; 8] = ['~', '^', ':', '?', '*', '[', '\\', '/'];

/// Lowercase ascii words of `text` joined with `-`: stop words and one-letter words dropped,
/// apostrophes removed, at most 4 words and 30 characters. A number is kept when it has 3 digits or
/// follows `issue` or `PR` (a version such as `6.3` is not), and `issue` and `PR` only when a number
/// follows. A pull request link, an issue link or `#12` becomes `pr 12` or `issue 12`, any other
/// link is dropped. Empty when no word is left.
#[must_use]
pub fn slug(text: &str) -> String {
    let words = words_of(text);
    let mut slug = String::new();
    let mut count = 0;
    for (at, word) in words.iter().enumerate() {
        let before = at.checked_sub(1).map(|before| words[before].as_str());
        if skipped(word, before, words.get(at + 1)) {
            continue;
        }
        if count == 0 {
            slug.extend(word.chars().take(MAX_CHARS));
        } else if count == MAX_WORDS || slug.len() + 1 + word.len() > MAX_CHARS {
            break;
        } else {
            slug.push('-');
            slug.push_str(word);
        }
        count += 1;
    }
    slug
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn skipped(word: &str, before: Option<&str>, next: Option<&String>) -> bool {
    if NOUNS_NEEDING_A_NUMBER.contains(&word) {
        return !next.is_some_and(|next| is_number(next));
    }
    if is_number(word) {
        return word.len() < MIN_BARE_NUMBER_DIGITS
            && !before.is_some_and(|before| NOUNS_NEEDING_A_NUMBER.contains(&before));
    }
    word.len() < 2 || STOP_WORDS.contains(&word)
}

fn words_of(text: &str) -> Vec<String> {
    let separators = |c: char| !(c.is_ascii_alphanumeric() || c == '\'' || c == '\u{2019}');
    let mut words = Vec::new();
    for chunk in text.split_whitespace() {
        if chunk.contains("://") {
            words.extend(link_words(chunk));
            continue;
        }
        if let Some(number) = chunk.strip_prefix('#').filter(|rest| is_number(rest)) {
            if !words.last().is_some_and(|last| NOUNS_NEEDING_A_NUMBER.contains(&last.as_str())) {
                words.push("issue".to_owned());
            }
            words.push(number.to_owned());
            continue;
        }
        words.extend(chunk.split(separators).map(|raw| {
            raw.chars()
                .filter(char::is_ascii_alphanumeric)
                .map(|c| c.to_ascii_lowercase())
                .collect::<String>()
        }));
    }
    words.retain(|word| !word.is_empty());
    words
}

fn link_words(link: &str) -> Vec<String> {
    let segments: Vec<&str> = link.split(['/', '?', '#']).collect();
    segments
        .windows(2)
        .find_map(|pair| {
            let named = match pair[0] {
                "issues" => "issue",
                "pull" => "pr",
                _ => return None,
            };
            is_number(pair[1]).then(|| vec![named.to_owned(), pair[1].to_owned()])
        })
        .unwrap_or_default()
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

/// Whether `name` is safe to hand to git as a branch or commit argument: not empty and not read as
/// an option.
pub(crate) fn is_usable_ref(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-')
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
    fn a_usable_ref_is_not_empty_and_not_an_option() {
        let cases = [
            ("main", true),
            ("feat/x", true),
            ("a-b", true),
            ("", false),
            ("-x", false),
            ("--force", false),
        ];
        for (name, want) in cases {
            assert_eq!(is_usable_ref(name), want, "{name:?}");
        }
    }

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
    fn a_slug_of_a_real_task_keeps_its_issue_number_and_its_nouns() {
        let cases = [
            ("billing-workers issue 2333 validation", "billing-workers-issue-2333"),
            ("Open a PR to fix https validation", "fix-https-validation"),
            (
                "Open a PR to fix https://github.com/acme/web/issues/2333 validation",
                "fix-issue-2333-validation",
            ),
            ("Review https://github.com/acme/web/pull/213/files", "review-pr-213"),
            ("Look at https://example.com/docs and fix the typo", "look-fix-typo"),
            ("Fix PR 213 review comments", "fix-pr-213-review"),
            ("Investigate the issue with login redirects", "investigate-login-redirects"),
            ("Make sure the importer handles empty files", "importer-handles-empty-files"),
            ("I need you to add retry to the sync job, please", "add-retry-sync-job"),
            ("Fix #2333 validation", "fix-issue-2333-validation"),
            ("Fix PR #213 review comments", "fix-pr-213-review"),
            ("Look at issue 7 and issue", "look-issue-7"),
            ("Bump vite to 6.3", "bump-vite"),
            ("Rename getUserData to loadProfile", "rename-getuserdata-loadprofile"),
            ("Upgrade to node 22 and update the CI image", "upgrade-node-update-ci"),
            ("Fix the 2026 tax report", "fix-2026-tax-report"),
            ("Step 2 of the migration", "step-migration"),
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
