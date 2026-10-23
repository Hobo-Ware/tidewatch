//! The state of the Worktree setup page: the repos of the window, the recipe of the one picked and
//! the drafts being typed. Every edit hands back the recipe to save; nothing here draws.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::setup_recipe::{self, Recipe, Store};

pub(crate) struct RepoEntry {
    pub(crate) name: String,
    pub(crate) dir: PathBuf,
}

#[derive(Default)]
enum Scan {
    #[default]
    Idle,
    Wanted,
    Running(Receiver<Vec<String>>),
}

#[derive(Default)]
pub(crate) struct Editor {
    repos: Vec<RepoEntry>,
    selected: usize,
    paths: Option<TidewatchPaths>,
    recipe: Recipe,
    has_cargo: bool,
    suggestions: Vec<String>,
    scan: Scan,
    pub(crate) copy_draft: String,
    pub(crate) link_draft: String,
    pub(crate) generator: String,
    pub(crate) problem: Option<&'static str>,
}

const BAD_ENTRY: &str = "Use a path inside the repo, such as .env or apps/web/.env.local";

impl Editor {
    pub(crate) fn new(repos: Vec<RepoEntry>, paths: Option<TidewatchPaths>, front: &Path) -> Self {
        let selected = repos.iter().position(|repo| repo.dir == front).unwrap_or(0);
        let mut editor = Self { repos, paths, ..Self::default() };
        editor.select(selected);
        editor
    }

    pub(crate) fn repos(&self) -> &[RepoEntry] {
        &self.repos
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn repo(&self) -> Option<&RepoEntry> {
        self.repos.get(self.selected)
    }

    pub(crate) fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    pub(crate) fn has_cargo(&self) -> bool {
        self.has_cargo
    }

    pub(crate) fn select(&mut self, index: usize) {
        self.selected = index;
        self.suggestions.clear();
        self.scan = Scan::Idle;
        self.copy_draft.clear();
        self.link_draft.clear();
        self.problem = None;
        let Some(repo) = self.repos.get(index) else { return };
        self.recipe = self
            .paths
            .as_ref()
            .map(|paths| Store::load(paths).recipe(&repo.dir))
            .unwrap_or_default();
        self.generator.clone_from(&self.recipe.generator);
        self.has_cargo = repo.dir.join("Cargo.toml").is_file();
        self.scan = Scan::Wanted;
    }

    /// Starts the look at the main checkout's `.env*` files when one is wanted, and takes its
    /// answer once it is in. True when the suggestions just arrived.
    pub(crate) fn poll(&mut self, ctx: &egui::Context) -> bool {
        if let Scan::Wanted = self.scan
            && let Some(repo) = self.repos.get(self.selected)
        {
            let (sender, receiver) = channel();
            let (dir, ctx) = (repo.dir.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = sender.send(setup_recipe::env_file_suggestions(&dir));
                ctx.request_repaint();
            });
            self.scan = Scan::Running(receiver);
        }
        let Scan::Running(receiver) = &self.scan else { return false };
        let Ok(found) = receiver.try_recv() else { return false };
        self.suggestions = found;
        self.scan = Scan::Idle;
        true
    }

    /// Gitignored `.env*` files of the main checkout the recipe does not copy yet.
    pub(crate) fn offered(&self) -> Vec<&str> {
        self.suggestions
            .iter()
            .map(String::as_str)
            .filter(|name| !self.recipe.copy.iter().any(|copied| copied == name))
            .collect()
    }

    fn changed(&mut self, recipe: &Recipe) -> Option<Recipe> {
        let cleaned = recipe.cleaned();
        if cleaned == self.recipe {
            return None;
        }
        self.recipe = cleaned.clone();
        self.problem = setup_recipe::install_through_links(&self.recipe);
        Some(cleaned)
    }

    pub(crate) fn add_copy(&mut self, raw: &str) -> Option<Recipe> {
        let Some(entry) = setup_recipe::clean_entry(raw) else {
            self.problem = Some(BAD_ENTRY);
            return None;
        };
        if self.copy_draft.trim() == raw.trim() {
            self.copy_draft.clear();
        }
        let mut recipe = self.recipe.clone();
        recipe.copy.push(entry);
        self.changed(&recipe)
    }

    pub(crate) fn add_link(&mut self, raw: &str) -> Option<Recipe> {
        let Some(entry) = setup_recipe::clean_entry(raw) else {
            self.problem = Some(BAD_ENTRY);
            return None;
        };
        if self.link_draft.trim() == raw.trim() {
            self.link_draft.clear();
        }
        let mut recipe = self.recipe.clone();
        recipe.link.push(entry);
        self.changed(&recipe)
    }

    pub(crate) fn remove_copy(&mut self, index: usize) -> Option<Recipe> {
        let mut recipe = self.recipe.clone();
        (index < recipe.copy.len()).then(|| recipe.copy.remove(index))?;
        self.changed(&recipe)
    }

    pub(crate) fn remove_link(&mut self, index: usize) -> Option<Recipe> {
        let mut recipe = self.recipe.clone();
        (index < recipe.link.len()).then(|| recipe.link.remove(index))?;
        self.changed(&recipe)
    }

    pub(crate) fn share_build_dir(&mut self, on: bool) -> Option<Recipe> {
        self.changed(&Recipe { shared_build_dir: on, ..self.recipe.clone() })
    }

    pub(crate) fn commit_generator(&mut self) -> Option<Recipe> {
        let recipe = Recipe { generator: self.generator.clone(), ..self.recipe.clone() };
        let changed = self.changed(&recipe);
        self.generator.clone_from(&self.recipe.generator);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    struct Fixture {
        _home: tempfile::TempDir,
        paths: TidewatchPaths,
        web: PathBuf,
        api: PathBuf,
    }

    fn fixture() -> Fixture {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().canonicalize().unwrap();
        let repo = |name: &str| {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            git(&dir, &["init", "-q", "-b", "main"]);
            dir
        };
        let (web, api) = (repo("acme-web"), repo("acme-api"));
        std::fs::write(web.join(".gitignore"), ".env*\n").unwrap();
        for name in [".env", ".env.local", ".env.example"] {
            std::fs::write(web.join(name), "A=1\n").unwrap();
        }
        std::fs::write(api.join("Cargo.toml"), "[package]\n").unwrap();
        Fixture { paths: TidewatchPaths::new(&root), _home: home, web, api }
    }

    fn editor(fx: &Fixture, front: &Path) -> Editor {
        let repos = [("acme-web", &fx.web), ("acme-api", &fx.api)]
            .map(|(name, dir)| RepoEntry { name: name.into(), dir: dir.clone() });
        Editor::new(repos.into(), Some(fx.paths.clone()), front)
    }

    fn arrive(editor: &mut Editor) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let ctx = egui::Context::default();
        while !editor.poll(&ctx) {
            assert!(Instant::now() < deadline, "the suggestions never arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_repo_in_front_is_picked_and_the_ignored_env_files_are_offered() {
        let fx = fixture();
        let mut editor = editor(&fx, &fx.web);
        assert_eq!(editor.selected(), 0);
        arrive(&mut editor);
        assert_eq!(
            editor.offered(),
            [".env", ".env.local"],
            "the tracked-looking template is left out"
        );
        assert!(!editor.has_cargo());
        let mut other = self::editor(&fx, &fx.api);
        assert_eq!(other.selected(), 1);
        arrive(&mut other);
        assert!(other.has_cargo());
        assert_eq!(other.offered().len(), 0);
    }

    #[test]
    fn adding_a_suggestion_returns_the_recipe_to_save_and_stops_offering_it() {
        let fx = fixture();
        let mut editor = editor(&fx, &fx.web);
        arrive(&mut editor);
        let saved = editor.add_copy(".env.local").expect("a change");
        assert_eq!(saved.copy, [".env.local"]);
        assert_eq!(editor.offered(), [".env"]);
        assert_eq!(editor.add_copy("./.env.local"), None, "a duplicate changes nothing");
    }

    #[test]
    fn a_bad_entry_is_refused_with_a_reason_and_keeps_the_draft() {
        let fx = fixture();
        let mut editor = editor(&fx, &fx.web);
        editor.copy_draft = "../secrets".into();
        assert_eq!(editor.add_copy("../secrets"), None);
        assert_eq!(editor.problem, Some(BAD_ENTRY));
        assert_eq!(editor.copy_draft, "../secrets");
        assert_eq!(editor.add_link("/etc"), None);
        assert_eq!(editor.recipe(), &Recipe::default());
    }

    #[test]
    fn every_edit_comes_back_as_a_whole_recipe_and_removal_goes_by_position() {
        let fx = fixture();
        let mut editor = editor(&fx, &fx.web);
        editor.add_copy(".env");
        editor.add_link("node_modules");
        editor.share_build_dir(true);
        let saved = editor.commit_generator_with("npm run prepare").unwrap();
        assert_eq!(saved.generator, "npm run prepare");
        assert!(saved.shared_build_dir);
        assert_eq!(editor.remove_copy(0).unwrap().copy.len(), 0);
        assert_eq!(editor.remove_copy(0), None, "nothing left at that place");
        assert_eq!(editor.remove_link(0).unwrap().link.len(), 0);
    }

    #[test]
    fn a_generator_that_installs_is_flagged_while_a_folder_is_linked() {
        let fx = fixture();
        let mut editor = editor(&fx, &fx.web);
        editor.add_link("node_modules");
        editor.commit_generator_with("npm ci").unwrap();
        assert!(editor.problem.is_some());
        editor.commit_generator_with("npm run prepare").unwrap();
        assert_eq!(editor.problem, None);
    }

    #[test]
    fn switching_repo_loads_that_repos_recipe_from_the_store() {
        let fx = fixture();
        let mut editor = editor(&fx, &fx.web);
        let saved = editor.add_copy(".env").unwrap();
        Store::update(&fx.paths, |store| store.set(&fx.web, &saved)).unwrap();
        editor.select(1);
        assert_eq!(editor.recipe(), &Recipe::default());
        editor.select(0);
        assert_eq!(editor.recipe().copy, [".env"]);
        assert_eq!(editor.generator, "");
    }

    impl Editor {
        fn commit_generator_with(&mut self, text: &str) -> Option<Recipe> {
            self.generator = text.to_owned();
            self.commit_generator()
        }
    }
}
