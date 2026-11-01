//! The Step 7 checks Tidewatch runs itself when an agent says it is done, as plain data and rules
//! (pure, table-tested): the test run it made, the verified items that name no command, the files
//! changed outside what the plan named, and a fix branch with no test. The done check gathers the
//! facts and shows these as lines.

use std::collections::BTreeSet;
use std::time::Duration;

use crate::done_check::Status;
use crate::limited;
use crate::meeseeks::tail_of;
use crate::test_command::TestRun;
use crate::text::squash;
use crate::workflow_profile::{Fix, Workflow};

/// What Tidewatch ran itself and what it left behind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Findings {
    pub tests: Option<TestsRun>,
    /// The verified items that name no command: claimed, not checked.
    pub claimed: Vec<String>,
    /// The changed files the plan did not name.
    pub outside: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestsRun {
    pub command: String,
    /// `-1` when the run never finished.
    pub exit_code: i32,
    pub took: Duration,
    pub failed: Option<usize>,
    pub tail: String,
}

impl TestsRun {
    #[must_use]
    pub fn of(command: &str, run: &TestRun, took: Duration) -> Self {
        Self {
            command: command.to_owned(),
            exit_code: run.exit_code,
            took,
            failed: failed_count(&run.output),
            tail: tail_of(&run.output),
        }
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.exit_code == 0
    }

    /// The line the done check shows: the command and the result.
    #[must_use]
    pub fn status(&self) -> (Status, String) {
        let command = &self.command;
        match (self.exit_code, self.failed) {
            (0, _) => {
                (Status::Pass, format!("Ran {command}: passed in {}", limited::describe(self.took)))
            }
            (-1, _) => {
                let why = self.tail.lines().last().unwrap_or("it did not finish");
                (Status::Unknown, format!("Ran {command}: did not finish ({why})"))
            }
            (_, Some(n)) => (Status::Fail, format!("Ran {command}: {n} failed")),
            (code, None) => (Status::Fail, format!("Ran {command}: failed (exit {code})")),
        }
    }

    /// What the agent hears when the run failed: the result, where the fix goes (from the repo's
    /// workflow profile) and the end of the output.
    #[must_use]
    pub fn failure_note(&self, workflow: Workflow) -> String {
        let (result, them, fix) = match self.failed {
            Some(1) => ("1 failed".to_owned(), "it", Fix::The),
            Some(n) => (format!("{n} failed"), "them", Fix::Each),
            None => (format!("failed (exit {})", self.exit_code), "it", Fix::The),
        };
        let command = &self.command;
        let fold = workflow.fold(fix);
        let note = format!(
            "Tidewatch ran {command}: {result} (output below). Fix {them}, {fold}, and say done again."
        );
        if self.tail.is_empty() { note } else { format!("{note}\n\n```\n{}\n```", self.tail) }
    }
}

fn punctuation(c: char) -> bool {
    matches!(c, ',' | ';' | ':' | '.' | '(' | ')' | '[' | ']')
}

fn number_before(line: &str, word: &str) -> Option<usize> {
    let mut previous = "";
    for token in line.split_whitespace() {
        if token.trim_matches(punctuation).eq_ignore_ascii_case(word)
            && let Ok(n) = previous.trim_matches(punctuation).parse()
        {
            return Some(n);
        }
        previous = token;
    }
    None
}

/// How many tests a runner's output says failed: the sum of cargo's `test result:` lines, else the
/// largest `N failed` or `N fail` any line carries. `None` when it says no number above zero.
#[must_use]
pub fn failed_count(output: &str) -> Option<usize> {
    let cargo: Vec<usize> = output
        .lines()
        .filter(|line| line.trim_start().starts_with("test result:"))
        .filter_map(|line| number_before(line, "failed"))
        .collect();
    let count = if cargo.is_empty() {
        output
            .lines()
            .filter_map(|line| {
                number_before(line, "failed").or_else(|| number_before(line, "fail"))
            })
            .max()
    } else {
        Some(cargo.iter().sum())
    };
    count.filter(|n| *n > 0)
}

/// One entry of the agent's Verified list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    pub text: String,
    pub why: Option<String>,
}

const RUNNERS: [&str; 34] = [
    "cargo", "npm", "npx", "pnpm", "yarn", "bun", "bunx", "deno", "dx", "node", "python",
    "python3", "pytest", "go", "make", "just", "rspec", "rake", "bundle", "mix", "gradle", "mvn",
    "dotnet", "git", "gh", "curl", "sh", "bash", "tsc", "vp", "docker", "psql", "sqlite3", "jq",
];

fn looks_like_command(text: &str) -> bool {
    let text = text.trim();
    let first = text.split_whitespace().next().unwrap_or_default();
    text.contains(' ') && RUNNERS.contains(&first)
        || first.starts_with("./")
        || first.starts_with("scripts/")
}

fn code_spans(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).map(str::trim).filter(|span| !span.is_empty()).collect()
}

/// The command a verified item names: a code span that reads as a command, else the text itself
/// when it starts with one, in the item or its reason.
#[must_use]
pub fn command_in(item: &Verified) -> Option<String> {
    let why = item.why.as_deref().unwrap_or_default();
    let spans = code_spans(&item.text).into_iter().chain(code_spans(why));
    let bare =
        [item.text.as_str(), why].into_iter().map(|text| text.split(':').next().unwrap_or(text));
    spans.chain(bare).find(|text| looks_like_command(text)).map(|text| text.trim().to_owned())
}

/// Whether a verified command is the one Tidewatch ran itself and saw pass.
#[must_use]
pub fn ran_by_tidewatch(command: &str, ran: Option<&TestsRun>) -> bool {
    let Some(run) = ran.filter(|run| run.passed()) else { return false };
    let (item, ours) = (squash(command), squash(&run.command));
    item.contains(&ours) || ours.contains(&item)
}

/// The Verified line of the done check and the items that name no command.
#[must_use]
pub fn verified_status(
    items: &[Verified],
    ran: Option<&TestsRun>,
) -> Option<(Status, String, Vec<String>)> {
    if items.is_empty() {
        return None;
    }
    let claimed: Vec<String> = items
        .iter()
        .filter(|item| command_in(item).is_none())
        .map(|item| item.text.clone())
        .collect();
    let total = items.len();
    let checked = items
        .iter()
        .filter_map(command_in)
        .filter(|command| ran_by_tidewatch(command, ran))
        .count();
    let noun = |n: usize| if n == 1 { "item" } else { "items" };
    if claimed.is_empty() {
        let seen =
            if checked > 0 { format!(", {checked} run by Tidewatch") } else { String::new() };
        let detail = format!("{total} verified {} name a command{seen}", noun(total));
        return Some((Status::Pass, detail, claimed));
    }
    let detail = format!(
        "{} of {total} verified {} name no command, so they are claimed, not checked",
        claimed.len(),
        noun(total)
    );
    Some((Status::Noted, detail, claimed))
}

const NOT_PLANNED: [&str; 4] = ["Cargo.lock", "bun.lock", "package-lock.json", "pnpm-lock.yaml"];

fn path_tokens(plan: &[String]) -> BTreeSet<String> {
    plan.iter()
        .flat_map(|text| text.split(|c: char| c.is_whitespace() || "`'\"(),;:<>[]{}".contains(c)))
        .map(|token| token.trim_matches(['.', '/']).to_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn named(file: &str, tokens: &BTreeSet<String>) -> bool {
    let file = file.to_lowercase();
    let name = file.rsplit('/').next().unwrap_or(&file);
    tokens.iter().any(|token| {
        file == *token
            || file.starts_with(&format!("{token}/"))
            || file.ends_with(&format!("/{token}"))
            || (token.contains('/') && file.contains(token.as_str()))
            || name == token
            || (stem(name).len() >= 3 && stem(name) == token)
    })
}

fn names_a_path(tokens: &BTreeSet<String>) -> bool {
    tokens.iter().any(|token| {
        token.contains('/')
            || token.rsplit_once('.').is_some_and(|(stem, extension)| {
                !stem.is_empty()
                    && (1..=5).contains(&extension.len())
                    && extension.chars().all(|c| c.is_ascii_alphanumeric())
                    && extension.chars().any(|c| c.is_ascii_alphabetic())
            })
    })
}

/// The Scope line and the changed files the plan did not name. `None` plan means the tab keeps no
/// report; a plan that names no file has nothing to compare with.
#[must_use]
pub fn scope_status(files: &[String], plan: Option<&[String]>) -> (Status, String, Vec<String>) {
    let Some(plan) = plan else {
        return (Status::Skipped, "the agent keeps no progress report".to_owned(), Vec::new());
    };
    let tokens = path_tokens(plan);
    if !names_a_path(&tokens) {
        return (Status::Skipped, "the plan names no files".to_owned(), Vec::new());
    }
    let outside: Vec<String> = files
        .iter()
        .filter(|file| !NOT_PLANNED.iter().any(|lock| file.rsplit('/').next() == Some(lock)))
        .filter(|file| !named(file, &tokens))
        .cloned()
        .collect();
    if outside.is_empty() {
        return (Status::Pass, "all changed files are in the plan".to_owned(), outside);
    }
    let count = outside.len();
    let files = if count == 1 { "file" } else { "files" };
    (Status::Noted, format!("{count} changed {files} outside what the plan named"), outside)
}

/// What a fix branch with no test hears.
pub const FIX_WITHOUT_TEST: &str = "This branch fixes something but adds no test that fails without the fix. Add one, or say why none fits.";

/// Whether the branch says it fixes something: a commit subject that starts `fix`, or a branch
/// name with `fix` as a word.
#[must_use]
pub fn is_fix(subjects: &[String], branch: &str) -> bool {
    let conventional = |subject: &String| {
        let head = subject.trim().to_lowercase();
        head.strip_prefix("fix").is_some_and(|rest| rest.starts_with([':', '(', '!', ' ']))
    };
    let word = |w: &str| matches!(w, "fix" | "fixes" | "bugfix" | "hotfix");
    subjects.iter().any(conventional) || branch.to_lowercase().split(['/', '-', '_']).any(word)
}

fn test_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let in_test_dir = lower
        .split('/')
        .rev()
        .skip(1)
        .any(|part| matches!(part, "test" | "tests" | "__tests__" | "spec" | "specs" | "e2e"));
    in_test_dir
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_test.")
        || name.contains("_spec.")
        || name.starts_with("test_")
        || stem(name).ends_with("tests")
}

const TEST_MARKERS: [&str; 9] = [
    "#[test]",
    "#[tokio::test]",
    "#[cfg(test)]",
    "def test_",
    "func Test",
    "@Test",
    "it(\"",
    "it('",
    "test(",
];

/// Whether a diff (`git diff -U0`) adds or changes a test: a test file in it, or an added line
/// that starts one.
#[must_use]
pub fn adds_test(patch: &str) -> bool {
    patch.lines().any(|line| {
        line.strip_prefix("+++ b/").is_some_and(test_path)
            || line.strip_prefix('+').is_some_and(|added| {
                !added.starts_with("++") && TEST_MARKERS.iter().any(|marker| added.contains(marker))
            })
    })
}

/// Whether any changed path is a test file.
#[must_use]
pub fn touches_test_file(files: &[String]) -> bool {
    files.iter().any(|file| test_path(file))
}

#[cfg(test)]
mod tests;
