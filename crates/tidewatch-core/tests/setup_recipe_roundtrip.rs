//! Real throwaway repos: a recipe applied to a worktree made the way a tab makes it.

mod common;

use std::path::{Path, PathBuf};

use common::{commit_file, git_available, ok, repos};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::setup_recipe::{Recipe, apply};
use tidewatch_core::worktree::{self, BranchChoice, RemoveMode, Worktree};

struct Fixture {
    repos: common::Repos,
    paths: TidewatchPaths,
    _home: tempfile::TempDir,
}

fn fixture() -> Option<Fixture> {
    if !git_available() {
        return None;
    }
    let repos = repos();
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    Some(Fixture { repos, paths, _home: home })
}

impl Fixture {
    fn main(&self) -> &Path {
        &self.repos.work
    }

    fn worktree(&self) -> Worktree {
        worktree::add_named(&self.paths, self.main(), "brave-otter", None).unwrap()
    }

    fn write(&self, rel: &str, text: &str) {
        let file = self.main().join(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
}

fn recipe(copy: &[&str], link: &[&str]) -> Recipe {
    Recipe {
        copy: copy.iter().map(ToString::to_string).collect(),
        link: link.iter().map(ToString::to_string).collect(),
        ..Recipe::default()
    }
}

fn status(dir: &Path) -> String {
    ok(dir, &["status", "--porcelain", "--ignored"])
}

#[test]
fn listed_files_are_copied_with_their_content_and_folders() {
    let Some(fx) = fixture() else { return };
    commit_file(fx.main(), ".gitignore", ".env*\n", "chore: ignore env");
    fx.write(".env", "TOKEN=abc\n");
    fx.write("apps/web/.env.local", "PORT=1\n");
    let wt = fx.worktree();
    let outcome = apply(&recipe(&[".env", "apps/web/.env.local"], &[]), fx.main(), &wt.path);
    assert_eq!(outcome.copied, [".env", "apps/web/.env.local"]);
    assert_eq!(outcome.notes.len(), 0);
    assert_eq!(std::fs::read_to_string(wt.path.join(".env")).unwrap(), "TOKEN=abc\n");
    assert_eq!(std::fs::read_to_string(wt.path.join("apps/web/.env.local")).unwrap(), "PORT=1\n");
    assert!(
        !std::fs::symlink_metadata(wt.path.join(".env")).unwrap().file_type().is_symlink(),
        "a copy is a real file"
    );
    assert_eq!(ok(&wt.path, &["status", "--porcelain"]), "", "ignored copies leave the tree clean");
}

#[test]
fn a_missing_source_is_a_note_and_the_rest_still_lands() {
    let Some(fx) = fixture() else { return };
    fx.write(".env", "A=1\n");
    let wt = fx.worktree();
    let outcome = apply(&recipe(&[".env.local", ".env"], &["node_modules"]), fx.main(), &wt.path);
    assert_eq!(outcome.copied, [".env"]);
    assert_eq!(outcome.linked.len(), 0);
    assert_eq!(
        outcome.notes,
        [
            "node_modules is not in the main checkout, skipped",
            ".env.local is not in the main checkout, skipped"
        ]
    );
    assert!(wt.path.join(".env").is_file());
}

#[test]
fn a_linked_folder_points_at_the_main_checkouts_and_stays_out_of_status() {
    let Some(fx) = fixture() else { return };
    fx.write("node_modules/pkg/index.js", "module.exports = 1\n");
    commit_file(fx.main(), ".gitignore", "node_modules/\n", "chore: ignore deps");
    let wt = fx.worktree();
    let outcome = apply(&recipe(&[], &["node_modules"]), fx.main(), &wt.path);
    assert_eq!(outcome.linked, ["node_modules"]);
    let link = wt.path.join("node_modules");
    let target = std::fs::read_link(&link).unwrap();
    assert!(target.is_absolute(), "{target:?}");
    assert_eq!(target, fx.main().canonicalize().unwrap().join("node_modules"));
    assert_eq!(
        std::fs::read_to_string(link.join("pkg/index.js")).unwrap(),
        "module.exports = 1\n",
        "readable through the link"
    );
    assert_eq!(
        ok(&wt.path, &["status", "--porcelain"]),
        "",
        "a trailing-slash ignore misses a link; the exclude covers it"
    );
}

#[test]
fn removing_the_worktree_leaves_the_linked_folder_in_the_main_checkout() {
    let Some(fx) = fixture() else { return };
    fx.write("node_modules/pkg/index.js", "x\n");
    let wt = fx.worktree();
    let outcome = apply(&recipe(&[], &["node_modules"]), fx.main(), &wt.path);
    assert_eq!(outcome.linked, ["node_modules"]);
    worktree::remove(&fx.paths, &wt, BranchChoice::Delete, RemoveMode::Safe).unwrap();
    assert!(!wt.path.exists());
    assert!(fx.main().join("node_modules/pkg/index.js").is_file());
}

#[test]
fn nothing_already_in_the_worktree_is_overwritten() {
    let Some(fx) = fixture() else { return };
    commit_file(fx.main(), "config.json", "{\"tracked\":true}\n", "chore: config");
    fx.write("config.json", "{\"changed\":true}\n");
    let wt = fx.worktree();
    let outcome = apply(&recipe(&["config.json"], &[]), fx.main(), &wt.path);
    assert_eq!(outcome.copied.len(), 0);
    assert_eq!(outcome.notes, ["config.json is already in the worktree, left as it is"]);
    assert_eq!(
        std::fs::read_to_string(wt.path.join("config.json")).unwrap(),
        "{\"tracked\":true}\n"
    );
}

#[test]
fn a_copy_never_writes_through_a_link_into_the_main_checkout() {
    let Some(fx) = fixture() else { return };
    fx.write("deps/keep.txt", "keep\n");
    fx.write("deps/.env", "SECRET=1\n");
    let wt = fx.worktree();
    let outcome = apply(&recipe(&["deps/.env"], &["deps"]), fx.main(), &wt.path);
    assert_eq!(outcome.linked, ["deps"]);
    assert_eq!(outcome.copied.len(), 0);
    assert_eq!(outcome.notes, ["deps/.env sits inside a linked folder, skipped"]);
    assert_eq!(std::fs::read_to_string(fx.main().join("deps/.env")).unwrap(), "SECRET=1\n");
}

#[test]
fn entries_that_climb_out_or_are_absolute_are_dropped() {
    let Some(fx) = fixture() else { return };
    let outside: PathBuf = fx.repos.root.path().join("outside.txt");
    std::fs::write(&outside, "x").unwrap();
    let wt = fx.worktree();
    let outcome =
        apply(&recipe(&["../outside.txt", outside.to_str().unwrap()], &[]), fx.main(), &wt.path);
    assert_eq!(outcome.copied.len(), 0);
    assert!(!wt.path.join("outside.txt").exists());
    assert_eq!(status(&wt.path), "");
}
