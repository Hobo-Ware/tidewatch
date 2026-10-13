use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::worktree::{self, Worktree};

use super::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Fixture {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    paths: TidewatchPaths,
    host: Host,
    ctx: egui::Context,
    toasts: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = home.join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        Self {
            paths: TidewatchPaths::new(&home),
            host: Host::new(),
            ctx: egui::Context::default(),
            toasts: Vec::new(),
            repo,
            _dir: dir,
        }
    }

    fn worktree(&self) -> Worktree {
        worktree::add(&self.paths, &self.repo).unwrap()
    }

    fn scan(&mut self) {
        self.host.add(worktree::leftovers(&self.paths, &[]));
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.toasts.extend(self.host.tick());
            if self.host.dialog.as_ref().is_some_and(|d| !d.busy()) {
                return;
            }
            assert!(Instant::now() < deadline, "the dialog never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn answer(&mut self, answer: &Answer) {
        self.host.answer(&self.ctx, Some(&self.paths), None, answer);
    }
}

fn commit(wt: &Worktree, file: &str) {
    std::fs::write(wt.path.join(file), "x").unwrap();
    git(&wt.path, &["add", file]);
    git(&wt.path, &["commit", "-q", "-m", file]);
}

#[test]
fn clean_up_removes_the_clean_ones_keeps_a_branch_with_commits_and_leaves_the_dirty_one() {
    let mut f = Fixture::new();
    let clean = f.worktree();
    let committed = f.worktree();
    commit(&committed, "work.txt");
    let dirty = f.worktree();
    std::fs::write(dirty.path.join("precious.txt"), "keep").unwrap();
    f.scan();
    assert_eq!(f.host.leftovers.len(), 3);

    f.host.open(&f.ctx);
    assert!(f.host.is_open());
    f.settle();
    f.answer(&Answer::CleanUp);
    f.settle();

    assert!(!clean.path.exists() && !committed.path.exists());
    assert_eq!(git(&f.repo, &["branch", "--list", &clean.branch]).trim(), "", "empty branch went");
    assert!(
        git(&f.repo, &["branch", "--list", &committed.branch]).contains(&committed.branch),
        "the branch with a commit stayed"
    );
    assert_eq!(std::fs::read_to_string(dirty.path.join("precious.txt")).unwrap(), "keep");
    assert_eq!(f.host.leftovers.len(), 1);
    assert_eq!(f.host.leftovers[0].path, dirty.path);
    assert_eq!(f.toasts, ["Removed 2 leftover worktrees"]);
}

#[test]
fn a_dirty_worktree_goes_only_through_its_own_discard() {
    let mut f = Fixture::new();
    let dirty = f.worktree();
    std::fs::write(dirty.path.join("scratch.txt"), "x").unwrap();
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    f.answer(&Answer::CleanUp);
    f.settle();
    assert!(dirty.path.exists(), "Clean up never discards");
    assert_eq!(f.toasts, Vec::<String>::new());

    f.answer(&Answer::Discard(dirty.path.clone()));
    f.settle();
    assert!(!dirty.path.exists());
    assert_eq!(f.host.leftovers.len(), 0);
    assert_eq!(f.toasts, ["Removed 1 leftover worktree"]);
}

#[test]
fn discard_does_nothing_for_a_clean_worktree_and_close_ends_the_dialog() {
    let mut f = Fixture::new();
    let clean = f.worktree();
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    f.answer(&Answer::Discard(clean.path.clone()));
    f.settle();
    assert!(clean.path.exists());
    f.answer(&Answer::Close);
    assert!(!f.host.is_open());
    assert_eq!(f.host.leftovers.len(), 1);
}

#[test]
fn a_worktree_with_no_record_is_listed_but_never_removed() {
    let mut f = Fixture::new();
    let by_hand = f.paths.worktree_dir(&f.repo, "by-hand");
    std::fs::create_dir_all(by_hand.parent().unwrap()).unwrap();
    git(&f.repo, &["worktree", "add", "-q", "-b", "hand", by_hand.to_str().unwrap()]);
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    f.answer(&Answer::CleanUp);
    f.settle();
    assert!(by_hand.exists());
    assert_eq!(f.host.leftovers.len(), 1);
}

#[test]
fn the_chip_shows_only_when_something_is_left_over_and_a_click_opens_the_dialog() {
    use egui::{Event, PointerButton, Rect, pos2, vec2};
    let mut f = Fixture::new();
    let ctx = egui::Context::default();
    let run = |host: &mut Host, events: Vec<Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 100.0))),
            focused: true,
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| ui.horizontal(|ui| host.chip(ui)).inner);
    };
    run(&mut f.host, vec![]);
    assert!(!f.host.is_open());
    let _ = f.worktree();
    f.scan();
    run(&mut f.host, vec![]);
    let at = pos2(24.0, 12.0);
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    run(&mut f.host, vec![Event::PointerMoved(at)]);
    run(&mut f.host, vec![button(true)]);
    run(&mut f.host, vec![button(false)]);
    assert!(f.host.is_open(), "a click on the chip opens the dialog");
}
