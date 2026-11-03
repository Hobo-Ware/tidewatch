//! Repo services in `~/.config/tidewatch/services.toml`, never in the repo, keyed by the repo's
//! path: long-running processes (a database proxy, a dev server, a tunnel) that run next to the
//! agents. A shared service runs in the main checkout for every tab; a tab service belongs to one
//! tab's folder, runs there, and is removed when that tab closes or its folder is gone. This module
//! is the saved list and its rules; `service_supervisor.rs` decides when one runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::{load_toml_or_default, save_toml_pretty};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub tab: Option<PathBuf>,
    pub name: String,
}

impl Key {
    #[must_use]
    pub fn shared(name: &str) -> Self {
        Self { tab: None, name: name.to_owned() }
    }

    #[must_use]
    pub fn matches(&self, service: &Service) -> bool {
        same_tab(self.tab.as_deref(), service.tab.as_deref())
            && same_name(&self.name, &service.name)
    }
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
            tab: None,
        }
    }

    #[must_use]
    pub fn in_tab(mut self, tab: Option<PathBuf>) -> Self {
        self.tab = tab.map(|tab| tab.components().collect());
        self
    }

    #[must_use]
    pub fn key(&self) -> Key {
        Key { tab: self.tab.clone(), name: self.name.clone() }
    }

    #[must_use]
    pub fn is_shared(&self) -> bool {
        self.tab.is_none()
    }

    fn cleaned(&self) -> Self {
        Self::new(&self.name, &self.command, self.autostart).in_tab(self.tab.clone())
    }
}

#[must_use]
pub fn same_tab(a: Option<&Path>, b: Option<&Path>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.components().eq(b.components()),
        _ => false,
    }
}

/// The services a name of `tab`'s must differ from: every one for a shared service, the shared
/// ones and the tab's own for a tab service.
#[must_use]
pub fn clashing(all: &[Service], tab: Option<&Path>) -> Vec<Service> {
    all.iter()
        .filter(|other| tab.is_none() || other.is_shared() || same_tab(other.tab.as_deref(), tab))
        .cloned()
        .collect()
}

/// The services a tab sees: the shared ones, then its own.
#[must_use]
pub fn visible_to(all: &[Service], tab: Option<&Path>) -> Vec<Service> {
    let shared = all.iter().filter(|service| service.is_shared());
    let own = all.iter().filter(|service| tab.is_some() && same_tab(service.tab.as_deref(), tab));
    shared.chain(own).cloned().collect()
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
        load_toml_or_default(&paths.services_file(), "no services")
    }

    pub fn save(&self, paths: &TidewatchPaths) -> std::io::Result<()> {
        save_toml_pretty(&paths.services_file(), self)
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
        let service = service.cleaned();
        let entry = self.repo.entry(key_of(repo)).or_default();
        let found = problems(&service, &clashing(&entry.service, service.tab.as_deref()), None);
        if !found.is_empty() {
            self.forget_if_empty(repo);
            return Err(found);
        }
        entry.service.push(service);
        Ok(())
    }

    /// Replaces the service `old` (any spelling of its name's case) with `service`, in its place
    /// and with its owner.
    pub fn replace(
        &mut self,
        repo: &Path,
        old: &Key,
        service: &Service,
    ) -> Result<(), Vec<String>> {
        let service = service.cleaned().in_tab(old.tab.clone());
        let entry = self.repo.entry(key_of(repo)).or_default();
        let Some(at) = entry.service.iter().position(|have| old.matches(have)) else {
            return Err(vec![format!("there is no service called `{}` in this repo", old.name)]);
        };
        let others = clashing(&entry.service, old.tab.as_deref());
        let found = problems(&service, &others, Some(&old.name));
        if !found.is_empty() {
            return Err(found);
        }
        entry.service[at] = service;
        Ok(())
    }

    /// Whether that service was there.
    pub fn remove(&mut self, repo: &Path, key: &Key) -> bool {
        !self.remove_where(repo, |have| key.matches(have)).is_empty()
    }

    /// Makes a tab service shared, so every tab sees it and it outlives its tab.
    pub fn share(&mut self, repo: &Path, key: &Key) -> Result<(), Vec<String>> {
        let entry = self.repo.entry(key_of(repo)).or_default();
        let Some(at) = entry.service.iter().position(|have| key.matches(have)) else {
            return Err(vec![format!("there is no service called `{}` in this repo", key.name)]);
        };
        let shared = entry.service[at].clone().in_tab(None);
        let others: Vec<Service> = entry
            .service
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != at)
            .map(|(_, other)| other.clone())
            .collect();
        let found = problems(&shared, &others, None);
        if !found.is_empty() {
            return Err(found);
        }
        entry.service[at] = shared;
        Ok(())
    }

    pub fn drop_tab(&mut self, repo: &Path, tab: &Path) -> Vec<Key> {
        self.remove_where(repo, |have| same_tab(have.tab.as_deref(), Some(tab)))
    }

    pub fn prune(&mut self, repo: &Path, gone: impl Fn(&Path) -> bool) -> Vec<Key> {
        self.remove_where(repo, |have| have.tab.as_deref().is_some_and(&gone))
    }

    fn remove_where(&mut self, repo: &Path, hit: impl Fn(&Service) -> bool) -> Vec<Key> {
        let Some(entry) = self.repo.get_mut(&key_of(repo)) else { return Vec::new() };
        let removed: Vec<Key> =
            entry.service.iter().filter(|have| hit(have)).map(Service::key).collect();
        entry.service.retain(|have| !hit(have));
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
        store.replace(repo, &Key::shared("db"), &recased).unwrap();
        assert_eq!(store.services(repo)[0], recased);
        let onto_other = Service::new("API", "x", true);
        assert!(store.replace(repo, &Key::shared("db"), &onto_other).is_err());
        assert!(store.replace(repo, &Key::shared("nope"), &onto_other).is_err());
        assert_eq!(store.services(repo).len(), 2);
    }

    #[test]
    fn removing_the_last_service_drops_the_repo_and_an_unknown_name_removes_nothing() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &db()).unwrap();
        assert!(!store.remove(repo, &Key::shared("api")));
        assert!(store.remove(repo, &Key::shared("DB")));
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

    fn tab_db(tab: &str) -> Service {
        Service::new("db", "mysqld --port 3320", true).in_tab(Some(PathBuf::from(tab)))
    }

    #[test]
    fn a_tab_service_keeps_its_folder_and_an_older_file_reads_as_shared() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &tab_db("/wt/a/")).unwrap();
        let text = toml::to_string_pretty(&store).unwrap();
        assert!(text.contains("tab = \"/wt/a\""), "{text}");
        let read: Store = toml::from_str(&text).unwrap();
        assert_eq!(read.services(repo), [tab_db("/wt/a")]);
        let mut shared = Store::default();
        shared.add(repo, &db()).unwrap();
        assert!(!toml::to_string_pretty(&shared).unwrap().contains("tab"));
        assert!(shared.services(repo)[0].is_shared());
    }

    #[test]
    fn two_tabs_may_each_have_a_db_but_not_next_to_a_shared_one() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &tab_db("/wt/a")).unwrap();
        store.add(repo, &tab_db("/wt/b")).unwrap();
        assert!(store.add(repo, &tab_db("/wt/a")).is_err());
        assert!(store.add(repo, &db()).is_err(), "a shared name must be free in every tab");
        store.add(repo, &api()).unwrap();
        let tab_api = api().in_tab(Some(PathBuf::from("/wt/c")));
        assert!(store.add(repo, &tab_api).is_err(), "a tab sees the shared api");
    }

    #[test]
    fn a_tab_sees_the_shared_services_then_its_own() {
        let all = [tab_db("/wt/a"), api(), tab_db("/wt/b")];
        let names = |tab: Option<&str>| -> Vec<Option<PathBuf>> {
            visible_to(&all, tab.map(Path::new)).into_iter().map(|s| s.tab).collect()
        };
        assert_eq!(names(Some("/wt/a")), [None, Some(PathBuf::from("/wt/a"))]);
        assert_eq!(names(None), [None]);
    }

    #[test]
    fn keys_tell_two_tabs_services_apart_and_an_edit_keeps_the_owner() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &tab_db("/wt/a")).unwrap();
        store.add(repo, &tab_db("/wt/b")).unwrap();
        let a = tab_db("/wt/a").key();
        store.replace(repo, &a, &Service::new("db", "mysqld --port 3321", true)).unwrap();
        assert_eq!(store.services(repo)[0].tab, Some(PathBuf::from("/wt/a")));
        assert_eq!(store.services(repo)[0].command, "mysqld --port 3321");
        assert_eq!(store.services(repo)[1], tab_db("/wt/b"));
        assert!(store.remove(repo, &a));
        assert_eq!(store.services(repo), [tab_db("/wt/b")]);
        assert!(!store.remove(repo, &Key::shared("db")));
    }

    #[test]
    fn sharing_a_tab_service_needs_a_name_no_other_tab_uses() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &tab_db("/wt/a")).unwrap();
        let proxy = Service::new("proxy", "pscale connect", true).in_tab(Some("/wt/a".into()));
        store.add(repo, &proxy).unwrap();
        store.add(repo, &tab_db("/wt/b")).unwrap();
        assert!(store.share(repo, &tab_db("/wt/a").key()).is_err());
        store.share(repo, &proxy.key()).unwrap();
        assert!(store.services(repo).iter().any(|s| s.name == "proxy" && s.is_shared()));
        assert!(store.share(repo, &Key::shared("nope")).is_err());
    }

    #[test]
    fn closing_a_tab_or_losing_its_folder_drops_only_its_services() {
        let repo = Path::new("/work/shop");
        let mut store = Store::default();
        store.add(repo, &api()).unwrap();
        store.add(repo, &tab_db("/wt/a")).unwrap();
        store.add(repo, &tab_db("/wt/b")).unwrap();
        assert_eq!(store.drop_tab(repo, Path::new("/wt/a/")), [tab_db("/wt/a").key()]);
        assert_eq!(store.prune(repo, |tab| tab == Path::new("/wt/b")), [tab_db("/wt/b").key()]);
        assert_eq!(store.services(repo), [api()]);
        assert!(store.prune(repo, |_| true).is_empty(), "a shared service is never pruned");
    }
}
