//! Per-repo worktree setup recipes in `~/.config/tidewatch/worktree-setup.toml`, never in the repo.
//! [`apply`] blocks on disk and git: run it off the UI thread. A missing source is a note.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::launch::shell_quote;
use crate::limited;
use crate::paths::TidewatchPaths;
use crate::prepare::exclude_from_git;

pub const BUILD_DIR_VAR: &str = "CARGO_TARGET_DIR";
pub const JOB_NAME: &str = "Worktree setup";
const TEMPLATE_SUFFIXES: [&str; 5] = ["example", "sample", "template", "dist", "defaults"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipe {
    /// Files copied from the main checkout, relative to the repo root.
    #[serde(default)]
    pub copy: Vec<String>,
    /// Folders (or files) linked to the main checkout's, relative to the repo root.
    #[serde(default)]
    pub link: Vec<String>,
    /// A Rust repo: the agent builds into the main checkout's `target`.
    #[serde(default)]
    pub shared_build_dir: bool,
    /// One command run in the new worktree before the agent starts, shown as a job.
    #[serde(default)]
    pub generator: String,
}

impl Recipe {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Drops bad and duplicate entries and chains a multi-line generator with `&&`.
    #[must_use]
    pub fn cleaned(&self) -> Self {
        let tidy = |entries: &[String]| {
            let mut kept: Vec<String> = Vec::new();
            for entry in entries.iter().filter_map(|raw| clean_entry(raw)) {
                if !kept.contains(&entry) {
                    kept.push(entry);
                }
            }
            kept
        };
        let generator: Vec<&str> =
            self.generator.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        Self {
            copy: tidy(&self.copy),
            link: tidy(&self.link),
            shared_build_dir: self.shared_build_dir,
            generator: generator.join(" && "),
        }
    }
}

/// A path inside the repo in its one spelling (`a/b`), or `None` for an absolute path, one that
/// climbs out with `..`, one inside `.git`, or nothing.
#[must_use]
pub fn clean_entry(raw: &str) -> Option<String> {
    let mut parts = Vec::new();
    for component in Path::new(raw.trim()).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!parts.is_empty() && parts[0] != ".git").then(|| parts.join("/"))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    repo: BTreeMap<String, Recipe>,
}

fn key_of(repo: &Path) -> String {
    repo.components().collect::<PathBuf>().to_string_lossy().into_owned()
}

impl Store {
    /// A missing or broken file is an empty store.
    #[must_use]
    pub fn load(paths: &TidewatchPaths) -> Self {
        let file = paths.worktree_setup_file();
        let Ok(text) = std::fs::read_to_string(&file) else { return Self::default() };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: {} is invalid ({e}); no worktree setup", file.display());
            Self::default()
        })
    }

    pub fn save(&self, paths: &TidewatchPaths) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(&paths.worktree_setup_file(), &text)
    }

    /// Reads the file afresh, applies `change` and writes it back.
    pub fn update(paths: &TidewatchPaths, change: impl FnOnce(&mut Self)) -> std::io::Result<()> {
        let mut store = Self::load(paths);
        change(&mut store);
        store.save(paths)
    }

    #[must_use]
    pub fn recipe(&self, repo: &Path) -> Recipe {
        self.repo.get(&key_of(repo)).map(Recipe::cleaned).unwrap_or_default()
    }

    /// An empty recipe removes the repo's entry.
    pub fn set(&mut self, repo: &Path, recipe: &Recipe) {
        let recipe = recipe.cleaned();
        if recipe.is_empty() {
            self.repo.remove(&key_of(repo));
        } else {
            self.repo.insert(key_of(repo), recipe);
        }
    }
}

/// The recipe saved for `repo`.
#[must_use]
pub fn recipe_for(paths: &TidewatchPaths, repo: &Path) -> Recipe {
    Store::load(paths).recipe(repo)
}

/// What [`apply`] did: the entries that landed and a note for each one that did not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub copied: Vec<String>,
    pub linked: Vec<String>,
    pub notes: Vec<String>,
}

/// Copies and links the recipe's entries from the `main` checkout into the new `worktree`. An
/// entry already present in the worktree is left alone, and nothing is written through a link.
#[must_use]
pub fn apply(recipe: &Recipe, main: &Path, worktree: &Path) -> Outcome {
    let recipe = recipe.cleaned();
    let main = main.canonicalize().unwrap_or_else(|_| main.to_path_buf());
    let mut outcome = Outcome::default();
    for rel in &recipe.link {
        match place(&main, worktree, rel, Kind::Link) {
            Ok(()) => outcome.linked.push(rel.clone()),
            Err(note) => outcome.notes.push(note),
        }
    }
    for rel in &recipe.copy {
        match place(&main, worktree, rel, Kind::Copy) {
            Ok(()) => outcome.copied.push(rel.clone()),
            Err(note) => outcome.notes.push(note),
        }
    }
    if let Some(note) = hide_links(worktree, &outcome.linked) {
        outcome.notes.push(note);
    }
    outcome
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Copy,
    Link,
}

fn place(main: &Path, worktree: &Path, rel: &str, kind: Kind) -> Result<(), String> {
    let source = main.join(rel);
    let target = worktree.join(rel);
    let Ok(meta) = std::fs::metadata(&source) else {
        return Err(format!("{rel} is not in the main checkout, skipped"));
    };
    if kind == Kind::Copy && !meta.is_file() {
        return Err(format!("{rel} is a folder, so it needs a link, skipped"));
    }
    if goes_through_a_link(worktree, rel) {
        return Err(format!("{rel} sits inside a linked folder, skipped"));
    }
    if target.symlink_metadata().is_ok() {
        return Err(format!("{rel} is already in the worktree, left as it is"));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{rel}: {e}"))?;
    }
    match kind {
        Kind::Copy => std::fs::copy(&source, &target).map(|_| ()),
        Kind::Link => std::os::unix::fs::symlink(&source, &target),
    }
    .map_err(|e| format!("{rel}: {e}"))
}

fn goes_through_a_link(worktree: &Path, rel: &str) -> bool {
    let mut here = worktree.to_path_buf();
    let mut parts: Vec<&str> = rel.split('/').collect();
    parts.pop();
    parts.into_iter().any(|part| {
        here.push(part);
        here.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink())
    })
}

/// `node_modules/` in a `.gitignore` does not match a link called `node_modules`, so the link
/// would show as an untracked file. Links git does not ignore are added to `info/exclude`.
fn hide_links(worktree: &Path, linked: &[String]) -> Option<String> {
    let visible: Vec<String> =
        linked.iter().filter(|rel| !is_ignored(worktree, rel)).cloned().collect();
    if visible.is_empty() {
        return None;
    }
    let lines: Vec<String> = visible.iter().map(|rel| format!("/{rel}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    exclude_from_git(worktree, &lines).err().map(|e| format!("git exclude for the links: {e}"))
}

fn is_ignored(dir: &Path, rel: &str) -> bool {
    limited::output(
        Command::new("git").arg("-C").arg(dir).args(["check-ignore", "-q", "--", rel]),
        limited::DEFAULT,
    )
    .is_ok_and(|out| out.status.success())
}

/// The environment the recipe adds to an agent's launch in a worktree of `repo`.
#[must_use]
pub fn env_for(recipe: &Recipe, repo: &Path) -> Vec<(String, String)> {
    if recipe.shared_build_dir && repo.join("Cargo.toml").is_file() {
        vec![(BUILD_DIR_VAR.to_owned(), repo.join("target").display().to_string())]
    } else {
        Vec::new()
    }
}

/// Gitignored `.env*` files in the main checkout that a recipe could copy, templates left out.
#[must_use]
pub fn env_file_suggestions(repo: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(repo)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_env_name(name) && is_ignored(repo, name))
        .collect();
    found.sort();
    found
}

fn is_env_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".env") else { return false };
    rest.is_empty()
        || rest.strip_prefix('.').is_some_and(|suffix| {
            !suffix.is_empty() && !TEMPLATE_SUFFIXES.iter().any(|t| suffix.ends_with(t))
        })
}

/// Why `command` may not run as a generator while folders are linked: an install run in the
/// worktree would write through the link into the main checkout.
#[must_use]
pub fn install_through_links(recipe: &Recipe) -> Option<&'static str> {
    if recipe.link.is_empty() || !recipe.generator.split(['&', ';', '|']).any(is_install) {
        return None;
    }
    Some("the generator looks like an install, which would write through the linked folders")
}

fn is_install(segment: &str) -> bool {
    let words: Vec<&str> = segment.split_whitespace().collect();
    let installs = ["install", "i", "ci", "add", "update", "up", "upgrade", "remove", "uninstall"];
    match words.as_slice() {
        ["yarn"] => true,
        [tool, verb, ..] => {
            ["npm", "pnpm", "yarn", "bun", "pip", "pip3", "bundle", "composer", "deno"]
                .contains(tool)
                && installs.contains(verb)
        }
        _ => false,
    }
}

/// The setup line that runs the generator and reports it as a job on the tab's progress feed. It
/// always ends well, so a failing generator never keeps the agent from starting.
#[must_use]
pub fn generator_line(recipe: &Recipe) -> Option<String> {
    let command = recipe.generator.trim();
    if command.is_empty() || install_through_links(recipe).is_some() {
        return None;
    }
    let report = |state: &str| {
        let line = serde_json::json!({ "name": JOB_NAME, "phase": command, "state": state });
        format!("printf '%s\\n' {} >> \"$f\"", shell_quote(&line.to_string()))
    };
    let failed_note =
        shell_quote(&format!("Tidewatch: {command} failed, starting the agent anyway."));
    Some(format!(
        "{{ f=\"${{TIDEWATCH_PROGRESS:-/dev/null}}\"; {}; if sh -c {}; then {}; else {}; echo {failed_note}; fi; true; }}",
        report("running"),
        shell_quote(command),
        report("done"),
        report("failed"),
    ))
}

/// `setup` with the generator line in front, when the recipe has a usable generator.
#[must_use]
pub fn with_generator(recipe: &Recipe, setup: &str) -> String {
    match generator_line(recipe) {
        Some(line) if setup.trim().is_empty() => line,
        Some(line) => format!("{line}\n{setup}"),
        None => setup.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(copy: &[&str], link: &[&str], generator: &str) -> Recipe {
        Recipe {
            copy: copy.iter().map(ToString::to_string).collect(),
            link: link.iter().map(ToString::to_string).collect(),
            shared_build_dir: false,
            generator: generator.to_owned(),
        }
    }

    #[test]
    fn entries_are_plain_paths_inside_the_repo() {
        let cases = [
            (".env", Some(".env")),
            ("  ./apps/web/.env.local ", Some("apps/web/.env.local")),
            ("node_modules/", Some("node_modules")),
            ("", None),
            ("/etc/passwd", None),
            ("../outside", None),
            ("a/../../b", None),
            (".git/config", None),
            (".gitignore", Some(".gitignore")),
        ];
        for (raw, want) in cases {
            assert_eq!(clean_entry(raw).as_deref(), want, "input {raw:?}");
        }
    }

    #[test]
    fn cleaning_a_recipe_drops_bad_entries_and_duplicates_and_chains_generator_lines() {
        let messy = recipe(&[".env", "./.env", "../x", ""], &["node_modules", "/abs"], "a\n\n  b ");
        let clean = messy.cleaned();
        assert_eq!(clean.copy, [".env"]);
        assert_eq!(clean.link, ["node_modules"]);
        assert_eq!(clean.generator, "a && b");
    }

    #[test]
    fn env_names_skip_templates() {
        let cases = [
            (".env", true),
            (".env.local", true),
            (".env.production", true),
            (".env.example", false),
            (".env.sample", false),
            (".env.local.template", false),
            (".envrc", false),
            (".env.", false),
            ("env", false),
        ];
        for (name, want) in cases {
            assert_eq!(is_env_name(name), want, "name {name:?}");
        }
    }

    #[test]
    fn the_shared_build_dir_is_for_rust_repos_only() {
        let repo = tempfile::tempdir().unwrap();
        let shared = Recipe { shared_build_dir: true, ..Recipe::default() };
        assert_eq!(env_for(&shared, repo.path()), []);
        std::fs::write(repo.path().join("Cargo.toml"), "[package]\n").unwrap();
        assert_eq!(
            env_for(&shared, repo.path()),
            [(BUILD_DIR_VAR.to_owned(), repo.path().join("target").display().to_string())]
        );
        assert_eq!(env_for(&Recipe::default(), repo.path()), []);
    }

    #[test]
    fn an_install_is_refused_only_while_something_is_linked() {
        let cases = [
            ("npm install", true),
            ("npm ci && npm run prepare", true),
            ("pnpm i", true),
            ("yarn", true),
            ("bun add left-pad", true),
            ("npm run prepare", false),
            ("npm run install-hooks", false),
            ("pnpm codegen", false),
            ("make generate", false),
        ];
        for (command, refused) in cases {
            let linked = recipe(&[], &["node_modules"], command);
            assert_eq!(install_through_links(&linked).is_some(), refused, "{command}");
            let unlinked = recipe(&[], &[], command);
            assert_eq!(install_through_links(&unlinked), None, "{command} with no link");
        }
    }

    #[test]
    fn the_store_round_trips_per_repo_and_an_empty_recipe_removes_the_entry() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let web = Path::new("/work/acme-web");
        let api = Path::new("/work/acme-api");
        Store::update(&paths, |store| {
            store.set(web, &recipe(&[".env"], &["node_modules"], "npm run prepare"));
        })
        .unwrap();
        Store::update(&paths, |store| store.set(api, &recipe(&[".env.local"], &[], ""))).unwrap();
        let store = Store::load(&paths);
        assert_eq!(store.recipe(web), recipe(&[".env"], &["node_modules"], "npm run prepare"));
        assert_eq!(store.recipe(Path::new("/work/acme-web/")).copy, [".env"], "trailing slash");
        assert_eq!(store.recipe(api).copy, [".env.local"]);
        assert_eq!(store.recipe(Path::new("/work/other")), Recipe::default());
        Store::update(&paths, |store| store.set(web, &Recipe::default())).unwrap();
        assert_eq!(Store::load(&paths).recipe(web), Recipe::default());
        assert_eq!(Store::load(&paths).recipe(api).copy, [".env.local"]);
    }

    #[test]
    fn a_broken_store_file_is_an_empty_store() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        write_atomic(&paths.worktree_setup_file(), "repo = [[[").unwrap();
        assert_eq!(Store::load(&paths), Store::default());
    }

    #[test]
    fn the_generator_line_reports_a_job_and_never_stops_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        let feed = dir.path().join("feed.jsonl");
        let run = |generator: &str| {
            let line = generator_line(&recipe(&[], &[], generator)).unwrap();
            let out = Command::new("sh")
                .arg("-c")
                .arg(format!("{line} && echo after"))
                .env("TIDEWATCH_PROGRESS", &feed)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(text.ends_with("after\n"), "{text}");
            std::fs::read_to_string(&feed).unwrap()
        };
        let good = run("echo generated > made.txt");
        assert_eq!(good.lines().count(), 2, "{good}");
        assert!(good.contains(r#""state":"running""#) && good.contains(r#""state":"done""#));
        assert_eq!(std::fs::read_to_string(dir.path().join("made.txt")).unwrap(), "generated\n");
        let bad = run("exit 3");
        assert!(bad.lines().last().unwrap().contains(r#""state":"failed""#), "{bad}");
        let reports: Vec<_> = bad.lines().filter_map(crate::job_feed::parse_line).collect();
        assert_eq!(reports.len(), 4, "every line is a job report");
        assert!(reports.iter().all(|report| report.name == JOB_NAME));
    }

    #[test]
    fn the_generator_line_works_without_a_feed_and_quotes_odd_commands() {
        let dir = tempfile::tempdir().unwrap();
        let line =
            generator_line(&recipe(&[], &[], "printf '%s' \"it's $((1+1))\" > out.txt")).unwrap();
        let status = Command::new("sh")
            .arg("-c")
            .arg(&line)
            .env_remove("TIDEWATCH_PROGRESS")
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(dir.path().join("out.txt")).unwrap(), "it's 2");
    }

    #[test]
    fn no_generator_line_for_nothing_or_an_install_through_a_link() {
        assert_eq!(generator_line(&Recipe::default()), None);
        assert_eq!(generator_line(&recipe(&[], &["node_modules"], "npm ci")), None);
        assert!(generator_line(&recipe(&[], &[], "npm ci")).is_some());
    }

    #[test]
    fn the_generator_goes_in_front_of_the_profile_setup() {
        let with = recipe(&[], &[], "make gen");
        assert_eq!(with_generator(&Recipe::default(), "npm ci"), "npm ci");
        let both = with_generator(&with, "npm ci");
        assert!(both.ends_with("\nnpm ci") && both.starts_with("{ f="), "{both}");
        assert_eq!(with_generator(&with, "  ").lines().count(), 1);
    }
}
