//! Repo services in `~/.config/tidewatch/services.toml`, never in the repo, keyed by the repo's
//! path: long-running processes (a database proxy, a dev server, a tunnel) that run next to the
//! agents. This module is the saved list and its rules; `service_supervisor.rs` decides when one
//! runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_COMMAND_CHARS: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub command: String,
    /// Read from a services.toml an older version wrote, never written back.
    #[serde(default, skip_serializing)]
    pub port: Option<u16>,
    #[serde(default = "yes")]
    pub autostart: bool,
}

fn yes() -> bool {
    true
}

impl Service {
    #[must_use]
    pub fn new(name: &str, command: &str, autostart: bool) -> Self {
        Self {
            name: name.trim().to_owned(),
            command: command.trim().to_owned(),
            port: None,
            autostart,
        }
    }
}

/// What is wrong with `service` as an addition to `others`, each problem as a sentence that says
/// how to fix it. A rename passes the old name in `renaming`, which may keep its own spelling.
#[must_use]
pub fn problems(service: &Service, others: &[Service], renaming: Option<&str>) -> Vec<String> {
    let mut found = Vec::new();
    let name = service.name.trim();
    let length = name.chars().count();
    if length == 0 {
        found.push("name must say what the service is, for example `db` or `api`".to_owned());
    } else if length > MAX_NAME_CHARS {
        found.push(format!("name must be at most {MAX_NAME_CHARS} characters, it has {length}"));
    } else if name.chars().any(char::is_control) {
        found.push("name must be one line of plain text".to_owned());
    }
    let taken = others.iter().any(|other| {
        other.name.trim().to_lowercase() == name.to_lowercase()
            && renaming.is_none_or(|old| old.trim().to_lowercase() != name.to_lowercase())
    });
    if taken && length > 0 {
        found.push(format!("name `{name}` is already a service of this repo, pick another"));
    }
    let command = service.command.trim();
    if command.is_empty() {
        found.push("command must say what to run, for example `npm run dev`".to_owned());
    } else if command.chars().count() > MAX_COMMAND_CHARS {
        found.push(format!("command must be at most {MAX_COMMAND_CHARS} characters"));
    }
    found
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct RepoServices {
    #[serde(default)]
    service: Vec<Service>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    repo: BTreeMap<String, RepoServices>,
}

fn key_of(repo: &Path) -> String {
    repo.components().collect::<PathBuf>().to_string_lossy().into_owned()
}

impl Store {
    /// A missing or broken file is an empty store; a broken one says so on stderr.
    #[must_use]
    pub fn load(paths: &TidewatchPaths) -> Self {
        let file = paths.services_file();
        let Ok(text) = std::fs::read_to_string(&file) else { return Self::default() };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: {} is invalid ({e}); no services", file.display());
            Self::default()
        })
    }

    pub fn save(&self, paths: &TidewatchPaths) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(&paths.services_file(), &text)
    }

    /// Reads the file afresh, applies `change` and writes it back, so a change made elsewhere
    /// since the last load is not lost. `change` returns the problems that stop it, and nothing is
    /// written then.
    pub fn update(
        paths: &TidewatchPaths,
        change: impl FnOnce(&mut Self) -> Result<(), Vec<String>>,
    ) -> Result<(), Vec<String>> {
        let mut store = Self::load(paths);
        change(&mut store)?;
        store.save(paths).map_err(|e| vec![format!("services.toml could not be written ({e})")])
    }

    #[must_use]
    pub fn services(&self, repo: &Path) -> Vec<Service> {
        self.repo.get(&key_of(repo)).map(|entry| entry.service.clone()).unwrap_or_default()
    }

    pub fn add(&mut self, repo: &Path, service: &Service) -> Result<(), Vec<String>> {
        let service = Service::new(&service.name, &service.command, service.autostart);
        let entry = self.repo.entry(key_of(repo)).or_default();
        let found = problems(&service, &entry.service, None);
        if !found.is_empty() {
            self.forget_if_empty(repo);
            return Err(found);
        }
        entry.service.push(service);
        Ok(())
    }

    /// Replaces the service called `old` (any spelling of its case) with `service`, in its place.
    pub fn replace(
        &mut self,
        repo: &Path,
        old: &str,
        service: &Service,
    ) -> Result<(), Vec<String>> {
        let service = Service::new(&service.name, &service.command, service.autostart);
        let entry = self.repo.entry(key_of(repo)).or_default();
        let Some(at) = entry.service.iter().position(|have| same_name(&have.name, old)) else {
            return Err(vec![format!("there is no service called `{old}` in this repo")]);
        };
        let found = problems(&service, &entry.service, Some(old));
        if !found.is_empty() {
            return Err(found);
        }
        entry.service[at] = service;
        Ok(())
    }

    /// Whether a service by that name was there.
    pub fn remove(&mut self, repo: &Path, name: &str) -> bool {
        let Some(entry) = self.repo.get_mut(&key_of(repo)) else { return false };
        let before = entry.service.len();
        entry.service.retain(|have| !same_name(&have.name, name));
        let removed = entry.service.len() != before;
        self.forget_if_empty(repo);
        removed
    }

    fn forget_if_empty(&mut self, repo: &Path) {
        if self.repo.get(&key_of(repo)).is_some_and(|entry| entry.service.is_empty()) {
            self.repo.remove(&key_of(repo));
        }
    }
}

#[must_use]
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// The services saved for `repo`.
#[must_use]
pub fn services_for(paths: &TidewatchPaths, repo: &Path) -> Vec<Service> {
    Store::load(paths).services(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        (home, paths)
    }

    type Case<'a> = (&'a str, &'a str, &'a [Service], usize);

    fn db() -> Service {
        Service::new("db", "pscale connect shop main --port 3309", true)
    }

    fn api() -> Service {
        Service::new("api", "npm run dev", false)
    }

    #[test]
    fn a_service_reads_back_as_it_was_saved_and_autostart_defaults_to_on() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &db()).unwrap();
        store.add(repo, &api()).unwrap();
        let text = toml::to_string_pretty(&store).unwrap();
        let read: Store = toml::from_str(&text).unwrap();
        assert_eq!(read.services(repo), [db(), api()]);
        let bare: Store = toml::from_str(
            "[[repo.\"/work/shop\".service]]\nname = \"db\"\ncommand = \"sleep 1\"\n",
        )
        .unwrap();
        assert_eq!(bare.services(repo), [Service::new("db", "sleep 1", true)]);
    }

    #[test]
    fn a_port_in_an_older_file_is_read_and_never_written_again() {
        let repo = Path::new("/work/shop");
        let mut store: Store = toml::from_str(
            "[[repo.\"/work/shop\".service]]\nname = \"db\"\ncommand = \"sleep 1\"\nport = 3309\n",
        )
        .unwrap();
        assert_eq!(store.services(repo)[0].port, Some(3309));
        store.add(repo, &api()).unwrap();
        let text = toml::to_string_pretty(&store).unwrap();
        assert!(!text.contains("port"), "{text}");
    }

    #[test]
    fn name_rules_table() {
        let long = "n".repeat(MAX_NAME_CHARS + 1);
        let exact = "n".repeat(MAX_NAME_CHARS);
        let cases: [Case; 8] = [
            ("fine", "sleep 1", &[], 0),
            ("", "sleep 1", &[], 1),
            ("   ", "sleep 1", &[], 1),
            (&long, "sleep 1", &[], 1),
            (&exact, "sleep 1", &[], 0),
            ("fine", "   ", &[], 1),
            ("DB", "sleep 1", &[db()], 1),
            ("two\nlines", "sleep 1", &[], 1),
        ];
        for (name, command, others, want) in cases {
            let service = Service::new(name, command, true);
            let found = problems(&service, others, None);
            assert_eq!(found.len(), want, "{name:?} {command:?}: {found:?}");
        }
    }

    #[test]
    fn every_problem_is_listed_so_one_call_fixes_them_all() {
        let service = Service::new("", "", true);
        assert_eq!(problems(&service, &[], None).len(), 2);
    }

    #[test]
    fn names_are_unique_ignoring_case_and_a_rename_may_change_its_own_case() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &db()).unwrap();
        store.add(repo, &api()).unwrap();
        let clash = Service::new("Db", "x", true);
        assert!(store.add(repo, &clash).is_err());
        assert!(store.add(Path::new("/work/other"), &clash).is_ok());
        let recased = Service::new("DB", "pscale connect shop main", true);
        store.replace(repo, "db", &recased).unwrap();
        assert_eq!(store.services(repo)[0], recased);
        let onto_other = Service::new("API", "x", true);
        assert!(store.replace(repo, "db", &onto_other).is_err());
        assert!(store.replace(repo, "nope", &onto_other).is_err());
        assert_eq!(store.services(repo).len(), 2);
    }

    #[test]
    fn removing_the_last_service_drops_the_repo_and_an_unknown_name_removes_nothing() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &db()).unwrap();
        assert!(!store.remove(repo, "api"));
        assert!(store.remove(repo, "DB"));
        assert_eq!(store, Store::default());
    }

    #[test]
    fn a_missing_or_broken_file_is_an_empty_store() {
        let (_home, paths) = paths();
        assert_eq!(Store::load(&paths), Store::default());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.services_file(), "this is [not toml").unwrap();
        assert_eq!(Store::load(&paths), Store::default());
    }

    #[test]
    fn an_update_reads_the_file_afresh_and_a_refused_change_writes_nothing() {
        let (_home, paths) = paths();
        let repo = Path::new("/work/shop");
        Store::update(&paths, |store| store.add(repo, &db())).unwrap();
        let stale = Store::load(&paths);
        Store::update(&paths, |store| store.add(repo, &api())).unwrap();
        assert_eq!(stale.services(repo).len(), 1);
        let before = std::fs::read_to_string(paths.services_file()).unwrap();
        let refused = Store::update(&paths, |store| store.add(repo, &db()));
        assert_eq!(refused.unwrap_err().len(), 1);
        assert_eq!(std::fs::read_to_string(paths.services_file()).unwrap(), before);
        assert_eq!(services_for(&paths, repo), [db(), api()]);
    }

    #[test]
    fn the_repo_key_ignores_a_trailing_slash() {
        let mut store = Store::default();
        store.add(Path::new("/work/shop/"), &db()).unwrap();
        assert_eq!(store.services(Path::new("/work/shop")), [db()]);
    }
}
