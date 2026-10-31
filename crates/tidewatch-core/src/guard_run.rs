//! Runs a guarded `git` or `gh` call: applies the rules to the text and flags in the command line,
//! then either refuses with the reasons or runs the real tool.
//!
//! This is the half of the guard that runs inside the agent's `git` and `gh` shims (see
//! `guard_shim`). It is reached only when the agent's environment names a settings file
//! (`TIDEWATCH_GUARD_CONFIG`); Tidewatch's own `git` and `gh` calls never have it.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::guard_args::{GhAction, GhCall, GitCommit, Part, Source, TextArg, parse_gh, parse_git};
use crate::guards::{Rule, Rules, find_dash, find_private_name, find_trailer};
use crate::limited;
use crate::prefs::Prefs;

pub const CONFIG_ENV: &str = "TIDEWATCH_GUARD_CONFIG";
pub const BIN_ENV: &str = "TIDEWATCH_GUARD_BIN";
pub const REPO_ENV: &str = "TIDEWATCH_GUARD_REPO";
pub const CACHE_ENV: &str = "TIDEWATCH_GUARD_CACHE";
pub const DIR_ENV: &str = "TIDEWATCH_GUARD_DIR";
pub const TAB_ENV: &str = "TIDEWATCH_GUARD_TAB";

const VISIBILITY_FRESH: Duration = Duration::from_hours(6);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Git,
    Gh,
}

impl Tool {
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "git" => Some(Self::Git),
            "gh" => Some(Self::Gh),
            _ => None,
        }
    }

    #[must_use]
    pub fn program(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Gh => "gh",
        }
    }
}

/// One rule that blocked a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub rule: Rule,
    pub detail: String,
}

impl Violation {
    /// The line the agent sees.
    #[must_use]
    pub fn message(&self) -> String {
        let detail =
            if self.detail.is_empty() { String::new() } else { format!(" ({})", self.detail) };
        format!(
            "tidewatch guard [{}]: {}{detail} Fix it and run the command again; Tidewatch Settings, Guards turns this rule off.",
            self.rule.key(),
            self.rule.reason(),
        )
    }
}

/// What a check needs from the machine, so the rules can be tested without one.
pub trait Machine {
    /// The text behind a source, or `None` when it cannot be read (the real tool reports that).
    fn read(&mut self, dir: Option<&Path>, source: &Source) -> Option<String>;
    /// Whether the repo the command works on is public. `None` when that is not known.
    fn is_public(&mut self, dir: Option<&Path>, repo: Option<&str>) -> Option<bool>;
}

struct Texts<'a> {
    parts: Vec<(Part, String)>,
    machine: &'a mut dyn Machine,
}

impl<'a> Texts<'a> {
    fn new(texts: &[TextArg], dir: Option<&Path>, machine: &'a mut dyn Machine) -> Self {
        let parts = texts
            .iter()
            .filter_map(|arg| Some((arg.part, machine.read(dir, &arg.source)?)))
            .collect();
        Self { parts, machine }
    }
}

fn scan(
    rules: Rules,
    private_repos: &[String],
    texts: &mut Texts,
    dir: Option<&Path>,
    repo: Option<&str>,
) -> Vec<Violation> {
    let mut found = Vec::new();
    let first = |rule: Rule, finder: &dyn Fn(&str) -> Option<crate::guards::Hit>| {
        if !rules.is_on(rule) {
            return None;
        }
        texts.parts.iter().find_map(|(part, text)| {
            let hit = finder(text)?;
            Some((hit, *part))
        })
    };
    let describe = |hit: &crate::guards::Hit, part: Part| {
        format!("{} has {} at line {}, column {}", part.label(), hit.found, hit.line, hit.column)
    };
    if let Some((hit, part)) = first(Rule::Dashes, &find_dash) {
        found.push(Violation { rule: Rule::Dashes, detail: describe(&hit, part) });
    }
    if let Some((hit, part)) = first(Rule::Trailers, &find_trailer) {
        found.push(Violation { rule: Rule::Trailers, detail: describe(&hit, part) });
    }
    if !private_repos.is_empty()
        && let Some((hit, part)) =
            first(Rule::PrivateNames, &|text| find_private_name(text, private_repos))
        && texts.machine.is_public(dir, repo) == Some(true)
    {
        found.push(Violation { rule: Rule::PrivateNames, detail: describe(&hit, part) });
    }
    found
}

#[must_use]
pub fn check_git(
    commit: &GitCommit,
    rules: Rules,
    private_repos: &[String],
    machine: &mut dyn Machine,
) -> Vec<Violation> {
    let dir = commit.dir.as_deref();
    let mut texts = Texts::new(&commit.texts, dir, machine);
    scan(rules, private_repos, &mut texts, dir, None)
}

#[must_use]
pub fn check_gh(
    call: &GhCall,
    rules: Rules,
    private_repos: &[String],
    machine: &mut dyn Machine,
) -> Vec<Violation> {
    let mut texts = Texts::new(&call.texts, None, machine);
    let mut found = scan(rules, private_repos, &mut texts, None, call.repo.as_deref());
    if rules.draft_prs && call.action == GhAction::PrCreate && !call.draft {
        found.push(Violation { rule: Rule::DraftPrs, detail: String::new() });
    }
    if rules.rerun_failed && call.action == GhAction::RunRerun && !call.narrow_rerun {
        found.push(Violation { rule: Rule::RerunFailed, detail: String::new() });
    }
    found
}

/// `owner/name` from a git remote url, for GitHub only.
#[must_use]
pub fn slug_from_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let path = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.split_once("://").and_then(|(_, rest)| host_path(rest, "github.com")))?;
    slug_from_path(path)
}

fn host_path<'a>(rest: &'a str, host: &str) -> Option<&'a str> {
    let (authority, path) = rest.split_once('/')?;
    (authority.rsplit('@').next() == Some(host)).then_some(path)
}

fn slug_from_path(path: &str) -> Option<String> {
    let path = path.trim_matches('/').trim_end_matches(".git");
    let mut parts = path.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !name.is_empty() && parts.next().is_none())
        .then(|| format!("{owner}/{name}"))
}

/// `owner/name` from a `-R` value: `owner/name`, `HOST/owner/name` or a url.
#[must_use]
pub fn slug_from_repo_flag(value: &str) -> Option<String> {
    let value = value.trim();
    if value.contains("://") || value.starts_with("git@") {
        return slug_from_remote(value);
    }
    let parts: Vec<&str> = value.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [owner, name] | [_, owner, name] => Some(format!("{owner}/{name}")),
        _ => None,
    }
}

/// `PUBLIC` is public; `PRIVATE` and `INTERNAL` are not.
#[must_use]
pub fn parse_visibility(output: &str) -> Option<bool> {
    match output.trim().to_ascii_uppercase().as_str() {
        "PUBLIC" => Some(true),
        "PRIVATE" | "INTERNAL" => Some(false),
        _ => None,
    }
}

/// The first executable `name` on `path` that is not in `skip`.
#[must_use]
pub fn resolve_on_path(name: &str, path: &OsStr, skip: &Path) -> Option<PathBuf> {
    let skip = skip.canonicalize().unwrap_or_else(|_| skip.to_path_buf());
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .filter(|dir| dir.canonicalize().unwrap_or_else(|_| dir.clone()) != skip)
        .map(|dir| dir.join(name))
        .find(|candidate| {
            candidate.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// What the guard is told through its environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardEnv {
    pub config: PathBuf,
    pub repo: PathBuf,
    pub cache: Option<PathBuf>,
    pub shim_dir: PathBuf,
    pub tab: Option<PathBuf>,
    pub path: OsString,
}

impl GuardEnv {
    /// `None` when the process is not an agent's: no settings file is named.
    #[must_use]
    pub fn from_env(get: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        Some(Self {
            config: get(CONFIG_ENV).filter(|v| !v.is_empty())?.into(),
            repo: get(REPO_ENV).map(PathBuf::from).unwrap_or_default(),
            cache: get(CACHE_ENV).map(PathBuf::from),
            shim_dir: get(DIR_ENV).map(PathBuf::from).unwrap_or_default(),
            tab: get(TAB_ENV).filter(|v| !v.is_empty()).map(PathBuf::from),
            path: get("PATH").unwrap_or_default(),
        })
    }

    fn real(&self, tool: Tool) -> Option<PathBuf> {
        resolve_on_path(tool.program(), &self.path, &self.shim_dir)
    }
}

struct RealMachine<'a> {
    env: &'a GuardEnv,
    stdin: Option<Vec<u8>>,
}

impl RealMachine<'_> {
    fn run(&self, tool: Tool, dir: Option<&Path>, args: &[&str]) -> Option<String> {
        let mut command = Command::new(self.env.real(tool)?);
        command
            .args(args)
            .env_remove(CONFIG_ENV)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        let output = limited::output(&mut command, limited::DEFAULT).ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn cached(&self, slug: &str) -> Option<bool> {
        let file = self.cache_file(slug)?;
        let age = file
            .metadata()
            .ok()?
            .modified()
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())?;
        (age < VISIBILITY_FRESH)
            .then(|| std::fs::read_to_string(file).ok().and_then(|text| parse_visibility(&text)))
            .flatten()
    }

    fn cache_file(&self, slug: &str) -> Option<PathBuf> {
        Some(self.env.cache.as_ref()?.join(format!("visibility-{}", slug.replace('/', "__"))))
    }

    fn remember(&self, slug: &str, public: bool) {
        let Some(file) = self.cache_file(slug) else { return };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(file, if public { "PUBLIC" } else { "PRIVATE" });
    }
}

impl Machine for RealMachine<'_> {
    fn read(&mut self, dir: Option<&Path>, source: &Source) -> Option<String> {
        match source {
            Source::Inline(text) => Some(text.clone()),
            Source::File(path) => {
                let path = dir.map_or_else(|| path.clone(), |dir| dir.join(path));
                std::fs::read(path).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            }
            Source::Stdin => {
                if self.stdin.is_none() {
                    let mut bytes = Vec::new();
                    let _ = std::io::stdin().read_to_end(&mut bytes);
                    self.stdin = Some(bytes);
                }
                self.stdin.as_ref().map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            }
        }
    }

    fn is_public(&mut self, dir: Option<&Path>, repo: Option<&str>) -> Option<bool> {
        let slug = match repo {
            Some(flag) => slug_from_repo_flag(flag)?,
            None => {
                slug_from_remote(&self.run(Tool::Git, dir, &["remote", "get-url", "origin"])?)?
            }
        };
        if let Some(public) = self.cached(&slug) {
            return Some(public);
        }
        let output = self.run(
            Tool::Gh,
            None,
            &["repo", "view", &slug, "--json", "visibility", "-q", ".visibility"],
        )?;
        let public = parse_visibility(&output)?;
        self.remember(&slug, public);
        Some(public)
    }
}

/// Runs `tool` with `args` as the agent's shim does. Returns only when the command was refused or
/// could not start: a command that passes the guard replaces this process.
#[must_use]
pub fn run(tool: Tool, args: &[OsString], env: &GuardEnv) -> i32 {
    let Some(real) = env.real(tool) else {
        eprintln!("tidewatch guard: no real {} found on PATH", tool.program());
        return 127;
    };
    let words: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let prefs = Prefs::load_file(&env.config).guards;
    let rules = prefs.rules_for(&env.repo);
    let mut machine = RealMachine { env, stdin: None };
    let violations = match tool {
        Tool::Git => parse_git(&words)
            .map(|commit| check_git(&commit, rules, &prefs.private_repos, &mut machine)),
        Tool::Gh => {
            parse_gh(&words).map(|call| check_gh(&call, rules, &prefs.private_repos, &mut machine))
        }
    };
    if let Some(violations) = violations.filter(|v| !v.is_empty()) {
        for violation in violations {
            eprintln!("{}", violation.message());
        }
        return 1;
    }
    let mut command = Command::new(&real);
    command.args(args);
    match machine.stdin {
        None => {
            let error = command.exec();
            eprintln!("tidewatch guard: cannot run {}: {error}", real.display());
            127
        }
        Some(bytes) => feed(command, &bytes),
    }
}

fn feed(mut command: Command, bytes: &[u8]) -> i32 {
    command.stdin(Stdio::piped());
    let Ok(mut child) = command.spawn() else { return 127 };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(bytes);
    }
    child.wait().ok().and_then(|status| status.code()).unwrap_or(1)
}

#[cfg(test)]
mod tests;
