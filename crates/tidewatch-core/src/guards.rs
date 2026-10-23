//! The outbound guard pack: the five rules, their toggles in `settings.toml` (`[guards]`, with
//! per-repo overrides) and the pure scanners that find a violation in a piece of text.
//!
//! The rules guard what an agent sends out through `git commit` and `gh`. `guard_args` finds the
//! text in a command line, `guard_run` applies the rules and runs the real tool, `guard_shim` puts the
//! shims on the agent's `PATH`. Tidewatch's own delivery never runs through them.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    Dashes,
    Trailers,
    PrivateNames,
    DraftPrs,
    RerunFailed,
}

impl Rule {
    pub const ALL: [Self; 5] =
        [Self::Dashes, Self::Trailers, Self::PrivateNames, Self::DraftPrs, Self::RerunFailed];

    /// The key in `settings.toml`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Dashes => "dashes",
            Self::Trailers => "trailers",
            Self::PrivateNames => "private_names",
            Self::DraftPrs => "draft_prs",
            Self::RerunFailed => "rerun_failed",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Dashes => "No em or en dashes",
            Self::Trailers => "No Co-Authored-By or Generated with lines",
            Self::PrivateNames => "No private repo names in public repos",
            Self::DraftPrs => "Pull requests open as drafts",
            Self::RerunFailed => "Rerun only failed jobs",
        }
    }

    /// The one line an agent sees when the rule blocks, and the Settings text under the toggle.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Dashes => {
                "Commit messages and GitHub text use a plain hyphen, never an em or en dash."
            }
            Self::Trailers => {
                "Commit messages and GitHub text carry no Co-Authored-By trailer and no Generated with footer."
            }
            Self::PrivateNames => {
                "This repo is public, so the text must not name a repo from the private list."
            }
            Self::DraftPrs => "Open the pull request as a draft: add --draft to gh pr create.",
            Self::RerunFailed => {
                "Rerun only the failed jobs: add --failed (or --job <id>) to gh run rerun."
            }
        }
    }

    #[must_use]
    pub fn default_on(self) -> bool {
        !matches!(self, Self::DraftPrs)
    }
}

/// Which rules are on.
#[allow(clippy::struct_excessive_bools)] // one switch per rule, each its own settings.toml key
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    #[serde(default = "on")]
    pub dashes: bool,
    #[serde(default = "on")]
    pub trailers: bool,
    #[serde(default = "on")]
    pub private_names: bool,
    #[serde(default)]
    pub draft_prs: bool,
    #[serde(default = "on")]
    pub rerun_failed: bool,
}

fn on() -> bool {
    true
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            dashes: true,
            trailers: true,
            private_names: true,
            draft_prs: false,
            rerun_failed: true,
        }
    }
}

impl Rules {
    #[must_use]
    pub fn is_on(&self, rule: Rule) -> bool {
        match rule {
            Rule::Dashes => self.dashes,
            Rule::Trailers => self.trailers,
            Rule::PrivateNames => self.private_names,
            Rule::DraftPrs => self.draft_prs,
            Rule::RerunFailed => self.rerun_failed,
        }
    }

    pub fn set(&mut self, rule: Rule, on: bool) {
        match rule {
            Rule::Dashes => self.dashes = on,
            Rule::Trailers => self.trailers = on,
            Rule::PrivateNames => self.private_names = on,
            Rule::DraftPrs => self.draft_prs = on,
            Rule::RerunFailed => self.rerun_failed = on,
        }
    }
}

/// The `[guards]` table: the rules for every repo, the private repo names, and the rules a repo
/// answers differently, keyed by the repo's path.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardPrefs {
    #[serde(flatten)]
    pub rules: Rules,
    /// `owner/name` or a bare `name`. Empty means the private-names rule has nothing to look for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub private_repos: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub repos: BTreeMap<String, BTreeMap<String, bool>>,
}

impl GuardPrefs {
    /// The rules in force for `repo`: the defaults, then the repo's own answers.
    #[must_use]
    pub fn rules_for(&self, repo: &Path) -> Rules {
        let mut rules = self.rules;
        if let Some(own) = self.repos.get(&repo.display().to_string()) {
            for rule in Rule::ALL {
                if let Some(on) = own.get(rule.key()) {
                    rules.set(rule, *on);
                }
            }
        }
        rules
    }

    /// Sets a rule for every repo (`repo` is `None`) or for one. A repo answer equal to the default
    /// is dropped, so the repo follows the default again.
    pub fn set_rule(&mut self, repo: Option<&Path>, rule: Rule, on: bool) {
        let Some(repo) = repo else {
            self.rules.set(rule, on);
            return;
        };
        let key = repo.display().to_string();
        let own = self.repos.entry(key.clone()).or_default();
        if on == self.rules.is_on(rule) {
            own.remove(rule.key());
        } else {
            own.insert(rule.key().to_owned(), on);
        }
        if own.is_empty() {
            self.repos.remove(&key);
        }
    }
}

/// Splits what a person typed into names: separated by lines, commas or spaces, no repeats.
#[must_use]
pub fn parse_private_repos(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in text.split(|c: char| c == ',' || c.is_whitespace()).filter(|n| !n.is_empty()) {
        if !names.iter().any(|seen| seen.eq_ignore_ascii_case(name)) {
            names.push(name.to_owned());
        }
    }
    names
}

/// Where in a text a rule found something. Line and column count from 1; the column counts chars.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub line: usize,
    pub column: usize,
    pub found: String,
}

impl Hit {
    fn at(line: usize, column: usize, found: impl Into<String>) -> Self {
        Self { line, column, found: found.into() }
    }
}

/// The first em dash (U+2014) or en dash (U+2013).
#[must_use]
pub fn find_dash(text: &str) -> Option<Hit> {
    text.lines().enumerate().find_map(|(line, content)| {
        content.chars().enumerate().find_map(|(column, c)| match c {
            '\u{2014}' => Some(Hit::at(line + 1, column + 1, "an em dash")),
            '\u{2013}' => Some(Hit::at(line + 1, column + 1, "an en dash")),
            _ => None,
        })
    })
}

/// The first line that is a `Co-Authored-By:` trailer, or a "Generated with" footer (optionally led
/// by an emoji, a bullet, a quote mark or markdown emphasis).
#[must_use]
pub fn find_trailer(text: &str) -> Option<Hit> {
    text.lines().enumerate().find_map(|(line, content)| {
        let lead = content.trim_start();
        let lowered = lead.to_lowercase();
        let column = content.chars().count() - lead.chars().count() + 1;
        if lowered.starts_with("co-authored-by:") {
            return Some(Hit::at(line + 1, column, "a Co-Authored-By trailer"));
        }
        let words = lowered.trim_start_matches(|c: char| !c.is_alphanumeric());
        words
            .starts_with("generated with")
            .then(|| Hit::at(line + 1, column, "a Generated with footer"))
    })
}

fn name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// The first of `names` the text mentions, matched without regard to case and only as a whole name:
/// `acme-web` is not found in `acme-web-extras`, but is in `acme/acme-web#12`.
#[must_use]
pub fn find_private_name(text: &str, names: &[String]) -> Option<Hit> {
    let haystack = text.to_ascii_lowercase();
    let bytes = haystack.as_bytes();
    let mut best: Option<(usize, &String)> = None;
    for name in names.iter().filter(|name| !name.is_empty()) {
        let needle = name.to_ascii_lowercase();
        let found = haystack.match_indices(&needle).map(|(at, _)| at).find(|at| {
            let before_ok = *at == 0 || !name_char(bytes[at - 1]);
            let after = at + needle.len();
            before_ok && (after >= bytes.len() || !name_char(bytes[after]))
        });
        if let Some(at) = found
            && best.is_none_or(|(first, _)| at < first)
        {
            best = Some((at, name));
        }
    }
    let (at, name) = best?;
    let line = text[..at].matches('\n').count() + 1;
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    Some(Hit::at(
        line,
        text[line_start..at].chars().count() + 1,
        format!("the private repo {name}"),
    ))
}

#[cfg(test)]
mod tests;
