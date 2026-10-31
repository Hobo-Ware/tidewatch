//! Proposes a repo's workflow profile from the text of the owner's and the repo's rule files.

use std::io::Read as _;
use std::path::Path;

use crate::folder_repos::tilde;
use crate::paths::TidewatchPaths;
use crate::prefs::Prefs;
use crate::workflow_profile::{CatchUp, Fixes, PullRequests, Workflow};

const MAX_FILE_BYTES: u64 = 128 * 1024;
const MAX_RULE_FILES: usize = 40;
const QUOTE_LEN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Owner,
    Repo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub label: String,
    pub scope: Scope,
    pub text: String,
}

/// Why a value was proposed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Found { quote: String, file: String },
    Setting { what: &'static str },
    Default { why: &'static str },
}

impl Source {
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Found { quote, file } => format!("found: \"{quote}\" in {file}"),
            Self::Setting { what } => format!("found: {what}"),
            Self::Default { why } => format!("nothing found: {why}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick<T> {
    pub value: T,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub catch_up: Pick<CatchUp>,
    pub fixes: Pick<Fixes>,
    pub pull_requests: Pick<PullRequests>,
}

impl Detected {
    #[must_use]
    pub fn workflow(&self) -> Workflow {
        Workflow {
            catch_up: self.catch_up.value,
            fixes: self.fixes.value,
            pull_requests: self.pull_requests.value,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Strength {
    Explicit,
    Hint,
}

struct Hit<T> {
    value: T,
    strength: Strength,
    at: usize,
}

impl<T> Hit<T> {
    fn explicit(value: T, at: usize) -> Self {
        Self { value, strength: Strength::Explicit, at }
    }

    fn hint(value: T, at: usize) -> Self {
        Self { value, strength: Strength::Hint, at }
    }
}

/// Repo files beat the owner's file only on a line that names the method; a hint such as a
/// force-push rule never beats a line that names it.
#[must_use]
pub fn detect(documents: &[Document], draft_guard: bool) -> Detected {
    let catch_up = choose(documents, scan_catch_up).unwrap_or(Pick {
        value: CatchUp::Rebase,
        source: Source::Default {
            why: "a rebase keeps the branch's history linear, which fits folding fixes into their commits",
        },
    });
    let fixes = choose(documents, scan_fixes).unwrap_or(Pick {
        value: Fixes::Fixup,
        source: Source::Default {
            why: "a fixup keeps each commit whole, so a reviewer reads one change per commit",
        },
    });
    let pull_requests = if draft_guard {
        Pick {
            value: PullRequests::Draft,
            source: Source::Setting { what: "the Draft PRs guard is on for this repo" },
        }
    } else {
        choose(documents, scan_pull_requests).unwrap_or(Pick {
            value: PullRequests::Draft,
            source: Source::Default {
                why: "Tidewatch has always asked for drafts, and the Draft PRs guard (off by default) only refuses a pull request that is not one",
            },
        })
    };
    Detected { catch_up, fixes, pull_requests }
}

/// The documents Tidewatch reads for `repo`: the repo's CLAUDE.md, AGENTS.md and
/// `.agents/rules/*.md`, then the owner's CLAUDE.md. Blocks on disk.
#[must_use]
pub fn gather(paths: &TidewatchPaths, repo: &Path) -> Vec<Document> {
    let mut documents = Vec::new();
    let mut add = |file: &Path, label: String, scope: Scope| {
        if let Some(text) = read_capped(file) {
            documents.push(Document { label, scope, text });
        }
    };
    for name in ["CLAUDE.md", "AGENTS.md"] {
        add(&repo.join(name), name.to_owned(), Scope::Repo);
    }
    let rules = repo.join(".agents").join("rules");
    let mut names: Vec<String> = std::fs::read_dir(&rules)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            Path::new(name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        })
        .collect();
    names.sort();
    names.truncate(MAX_RULE_FILES);
    for name in names {
        add(&rules.join(&name), format!(".agents/rules/{name}"), Scope::Repo);
    }
    let owner = paths.claude_dir().join("CLAUDE.md");
    let home = paths.config_dir().parent().and_then(Path::parent);
    add(&owner, tilde(&owner, home), Scope::Owner);
    documents
}

/// [`gather`] and [`detect`] for `repo`, with the repo's Draft PRs guard answer. Blocks on disk.
#[must_use]
pub fn detect_repo(paths: &TidewatchPaths, repo: &Path) -> Detected {
    let draft_guard = Prefs::load(paths).guards.rules_for(repo).draft_prs;
    detect(&gather(paths, repo), draft_guard)
}

fn read_capped(file: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(file).ok()?.take(MAX_FILE_BYTES).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn choose<T: Copy>(
    documents: &[Document],
    scan: impl Fn(&str) -> Option<Hit<T>>,
) -> Option<Pick<T>> {
    let mut best: Option<(u8, Pick<T>)> = None;
    for document in documents {
        for line in document.text.lines() {
            let Some(hit) = scan(line) else { continue };
            let rank = match (document.scope, hit.strength) {
                (Scope::Repo, Strength::Explicit) => 0,
                (Scope::Owner, Strength::Explicit) => 1,
                (Scope::Repo, Strength::Hint) => 2,
                (Scope::Owner, Strength::Hint) => 3,
            };
            if best.as_ref().is_none_or(|(have, _)| rank < *have) {
                let source = Source::Found {
                    quote: quote_around(line, hit.at),
                    file: document.label.clone(),
                };
                best = Some((rank, Pick { value: hit.value, source }));
            }
        }
    }
    best.map(|(_, pick)| pick)
}

fn scan_catch_up(line: &str) -> Option<Hit<CatchUp>> {
    let lower = line.to_ascii_lowercase();
    let interactive = lower.contains("interactive") || lower.contains("rebase -i");
    if !interactive && let Some(at) = find_word(&lower, "rebase") {
        let (value, from) = polar(&lower, at, CatchUp::Rebase, CatchUp::Merge);
        return Some(Hit::explicit(value, from));
    }
    if let Some(at) = merge_of_base(&lower) {
        let (value, from) = polar(&lower, at, CatchUp::Merge, CatchUp::Rebase);
        return Some(Hit::explicit(value, from));
    }
    let at = first_of(&lower, &["force-with-lease", "force-push", "force push", "linear history"])?;
    let (value, from) = polar(&lower, at, CatchUp::Rebase, CatchUp::Merge);
    Some(Hit::hint(value, from))
}

fn scan_fixes(line: &str) -> Option<Hit<Fixes>> {
    let lower = line.to_ascii_lowercase();
    if let Some(at) = first_of(&lower, &["--fixup", "fixup", "autosquash"]) {
        let (value, from) = polar(&lower, at, Fixes::Fixup, Fixes::NewCommits);
        return Some(Hit::explicit(value, from));
    }
    if let Some(at) = find_start_of_word(&lower, "fold")
        && (lower.contains("commit") || lower.contains("fix"))
    {
        let (value, from) = polar(&lower, at, Fixes::Fixup, Fixes::NewCommits);
        return Some(Hit::explicit(value, from));
    }
    if let Some(at) = lower.find("standalone")
        && lower.contains("commit")
    {
        let (value, from) = polar(&lower, at, Fixes::NewCommits, Fixes::Fixup);
        return Some(Hit::explicit(value, from));
    }
    let about_fixes = ["fix", "review", "feedback", "address"].iter().any(|k| lower.contains(k));
    if about_fixes
        && let Some(at) = first_of(&lower, &["new commit", "separate commit", "follow-up commit"])
    {
        let (value, from) = polar(&lower, at, Fixes::NewCommits, Fixes::Fixup);
        return Some(Hit::explicit(value, from));
    }
    if let Some(at) = find_word(&lower, "amend") {
        let (value, from) = polar(&lower, at, Fixes::Fixup, Fixes::NewCommits);
        return Some(Hit::hint(value, from));
    }
    let at = first_of(&lower, &["force-with-lease", "force-push", "force push"])?;
    let (value, from) = polar(&lower, at, Fixes::Fixup, Fixes::NewCommits);
    Some(Hit::hint(value, from))
}

fn scan_pull_requests(line: &str) -> Option<Hit<PullRequests>> {
    let lower = line.to_ascii_lowercase();
    if let Some(at) = lower.find("non-draft") {
        return Some(Hit::explicit(PullRequests::Ready, at));
    }
    let about_prs =
        lower.contains("pull request") || lower.contains("pr ") || lower.contains("prs");
    let keys: &[&str] = if about_prs {
        &["--draft", "draft pr", "draft pull", "as a draft", "as draft", "a draft", "drafts"]
    } else {
        &["--draft"]
    };
    let at = first_of(&lower, keys)?;
    let (value, from) = polar(&lower, at, PullRequests::Draft, PullRequests::Ready);
    Some(Hit::explicit(value, from))
}

fn first_of(lower: &str, keys: &[&str]) -> Option<usize> {
    keys.iter().filter_map(|key| lower.find(key)).min()
}

/// `yes` when the match at `at` stands alone, else `no` and where its clause begins.
fn polar<T>(lower: &str, at: usize, yes: T, no: T) -> (T, usize) {
    let clause = lower[..at].rfind(['.', ';', ',', ':', '!', '?', '(', ')']).map_or(0, |i| i + 1);
    let denied =
        lower[clause..at].split(|c: char| !(c.is_alphanumeric() || c == '\'')).any(|word| {
            matches!(word, "not" | "never" | "no" | "don't" | "dont" | "avoid" | "without")
        });
    if denied {
        let start = clause + (lower[clause..].len() - lower[clause..].trim_start().len());
        (no, start)
    } else {
        (yes, at)
    }
}

/// A `merge` that names the branch to catch up with: `merge origin/main`, `merge main into`.
fn merge_of_base(lower: &str) -> Option<usize> {
    const BASES: [&str; 8] =
        ["origin", "main", "master", "develop", "trunk", "upstream", "base", "default branch"];
    let mut from = 0;
    while let Some(found) = find_word(&lower[from..], "merge") {
        let at = from + found;
        let mut rest = lower[at + "merge".len()..].trim_start();
        while rest.starts_with('-') {
            rest = rest.split_once(' ').map_or("", |(_, tail)| tail.trim_start());
        }
        let rest = rest.strip_prefix("the ").unwrap_or(rest);
        if BASES.iter().any(|base| starts_with_word(rest, base)) {
            return Some(at);
        }
        from = at + "merge".len();
    }
    None
}

fn starts_with_word(text: &str, word: &str) -> bool {
    text.strip_prefix(word).is_some_and(|tail| !tail.starts_with(|c: char| c.is_ascii_alphabetic()))
}

fn find_start_of_word(lower: &str, word: &str) -> Option<usize> {
    lower.match_indices(word).map(|(at, _)| at).find(|at| {
        !lower[..*at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn find_word(lower: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = lower[from..].find(word) {
        let at = from + found;
        let before = lower[..at].chars().next_back();
        let after = lower[at + word.len()..].chars().next();
        let bounded = !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_ascii_alphabetic());
        if bounded {
            return Some(at);
        }
        from = at + word.len();
    }
    None
}

fn quote_around(line: &str, from: usize) -> String {
    let tail = &line[from..];
    let stop = tail.find(['`', '.', ';']).unwrap_or(tail.len());
    let capped = tail[..stop].chars().count() > QUOTE_LEN;
    let text: String =
        tail[..stop].chars().take(QUOTE_LEN).filter(|c| !matches!(c, '`' | '*')).collect();
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{words}{}", if capped { "..." } else { "" })
}

#[cfg(test)]
mod tests;
