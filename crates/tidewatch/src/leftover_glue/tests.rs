use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::worktree::{self, BranchChoice, Worktree};

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
            host: Host::new(Some(TidewatchPaths::new(&home))),
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

fn branch_exists(f: &Fixture, wt: &Worktree) -> bool {
    git(&f.repo, &["branch", "--list", &wt.branch]).contains(&wt.branch)
}

#[test]
fn clean_up_removes_only_the_worktrees_proven_safe_and_leaves_the_rest_whole() {
    let mut f = Fixture::new();
    let clean = f.worktree();
    let committed = f.worktree();
    commit(&committed, "work.txt");
    let dirty = f.worktree();
    std::fs::write(dirty.path.join("precious.txt"), "keep").unwrap();
    let broken = f.worktree();
    std::fs::write(broken.path.join(".git"), "gitdir: /nowhere/at/all\n").unwrap();
    f.scan();
    assert_eq!(f.host.leftovers.len(), 4);

    f.host.open(&f.ctx);
    assert!(f.host.is_open());
    f.settle();
    f.answer(&Answer::CleanUp);
    f.settle();

    assert!(!clean.path.exists());
    assert!(!branch_exists(&f, &clean), "the proven-safe branch went");
    assert!(committed.path.exists(), "unpushed commits keep the whole worktree");
    assert!(branch_exists(&f, &committed));
    assert_eq!(std::fs::read_to_string(dirty.path.join("precious.txt")).unwrap(), "keep");
    assert!(broken.path.exists(), "a worktree git cannot read is never removed");
    assert!(branch_exists(&f, &broken));
    assert_eq!(f.host.leftovers.len(), 3);
    assert_eq!(f.toasts.len(), 1);
    assert!(
        f.toasts[0]
            .starts_with("Removed 1 leftover worktree. To bring the branch back: git branch ")
    );
    assert!(f.toasts[0].contains(&clean.branch), "{:?}", f.toasts);
}

#[cfg(unix)]
#[test]
fn a_process_running_in_a_leftover_keeps_it_out_of_clean_up() {
    let mut f = Fixture::new();
    let live = f.worktree();
    let mut child = Command::new("sleep").arg("30").current_dir(&live.path).spawn().unwrap();
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    f.answer(&Answer::CleanUp);
    f.settle();
    let removed = !live.path.exists();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!removed, "a running agent's folder is never cleaned up from under it");
    assert_eq!(f.host.leftovers.len(), 1);
}

#[test]
fn a_worktree_that_is_not_safe_goes_only_through_its_own_discard_and_the_removal_is_logged() {
    let mut f = Fixture::new();
    let dirty = f.worktree();
    std::fs::write(dirty.path.join("scratch.txt"), "x").unwrap();
    commit(&dirty, "work.txt");
    std::fs::write(dirty.path.join("scratch.txt"), "y").unwrap();
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    f.answer(&Answer::CleanUp);
    f.settle();
    assert!(dirty.path.exists(), "Clean up never discards");
    assert_eq!(f.toasts, Vec::<String>::new());

    f.answer(&Answer::Discard(dirty.path.clone(), BranchChoice::Delete));
    f.settle();
    assert!(!dirty.path.exists());
    assert!(!branch_exists(&f, &dirty));
    assert_eq!(f.host.leftovers.len(), 0);
    let logged = tidewatch_core::removal_log::load(&f.paths);
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].branch, dirty.branch);
    assert!(logged[0].sha.is_some());
    assert_eq!(f.toasts.len(), 1);
    assert!(f.toasts[0].contains("git branch"), "{:?}", f.toasts);
}

#[test]
fn a_discard_that_keeps_the_branch_leaves_it_in_the_repo() {
    let mut f = Fixture::new();
    let wt = f.worktree();
    commit(&wt, "work.txt");
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    f.answer(&Answer::Discard(wt.path.clone(), BranchChoice::Keep));
    f.settle();
    assert!(!wt.path.exists());
    assert!(branch_exists(&f, &wt));
}

#[test]
fn discard_does_nothing_for_a_worktree_proven_safe_and_close_ends_the_dialog() {
    let mut f = Fixture::new();
    let clean = f.worktree();
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    f.answer(&Answer::Discard(clean.path.clone(), BranchChoice::Delete));
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
