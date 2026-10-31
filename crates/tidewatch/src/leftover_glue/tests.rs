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
    let row = f.host.dialog.as_ref().unwrap().rows.iter().find(|r| r.leftover.path == broken.path);
    let row = row.expect("its row");
    assert_eq!(row.state, crate::leftovers::RowState::Unreadable);
    assert_eq!(
        f.host.dialog.as_ref().unwrap().trash_target(&broken.path),
        Some((broken.path.clone(), Some(broken.clone())))
    );
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

fn by_hand(f: &Fixture, name: &str, branch: &str) -> Worktree {
    let path = f.paths.worktree_dir(&f.repo, name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    git(&f.repo, &["worktree", "add", "-q", "-b", branch, path.to_str().unwrap()]);
    Worktree { repo: f.repo.clone(), path, branch: branch.to_owned() }
}

fn dialog_of(f: &Fixture) -> &Cleanup {
    f.host.dialog.as_ref().unwrap()
}

#[test]
fn an_unrecorded_worktree_proven_clean_joins_clean_up_and_loses_folder_and_branch() {
    let mut f = Fixture::new();
    let hand = by_hand(&f, "by-hand", "hand");
    f.scan();
    assert!(!f.host.leftovers[0].recorded);
    f.host.open(&f.ctx);
    f.settle();
    assert_eq!(dialog_of(&f).clean_count(), 1);
    f.answer(&Answer::CleanUp);
    f.settle();
    assert!(!hand.path.exists());
    assert!(!branch_exists(&f, &hand));
    assert_eq!(f.host.leftovers.len(), 0);
    assert_eq!(tidewatch_core::removal_log::load(&f.paths).len(), 1);
}

#[test]
fn an_unrecorded_worktree_with_unpushed_commits_is_discarded_and_its_branch_kept() {
    let mut f = Fixture::new();
    let hand = by_hand(&f, "by-hand", "hand");
    commit(&hand, "one.txt");
    commit(&hand, "two.txt");
    let tip = git(&f.repo, &["rev-parse", "hand"]);
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    assert!(Cleanup::is_discardable(&dialog_of(&f).rows[0]));
    f.answer(&Answer::CleanUp);
    f.settle();
    assert!(hand.path.exists(), "Clean up never discards");

    f.answer(&Answer::Discard(hand.path.clone(), BranchChoice::Delete));
    f.settle();
    assert!(!hand.path.exists());
    assert_eq!(git(&f.repo, &["rev-parse", "hand"]), tip);
    assert_eq!(f.host.leftovers.len(), 0);
    let logged = tidewatch_core::removal_log::load(&f.paths);
    assert_eq!(logged.len(), 1);
    assert_eq!((logged[0].branch.as_str(), logged[0].mode.as_str()), ("hand", "discard"));
    assert_eq!(logged[0].sha.as_deref(), Some(tip.trim()));
    assert!(logged[0].branch_kept);
    assert_eq!(f.toasts.len(), 1);
    assert!(f.toasts[0].contains("branch kept: 2 commits only here"), "{:?}", f.toasts);
}

#[test]
fn an_unrecorded_discard_of_unsaved_work_deletes_a_branch_that_holds_nothing_unique() {
    let mut f = Fixture::new();
    let hand = by_hand(&f, "by-hand", "hand");
    std::fs::write(hand.path.join("wip.txt"), "wip").unwrap();
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    f.answer(&Answer::Discard(hand.path.clone(), BranchChoice::Delete));
    f.settle();
    assert!(!hand.path.exists());
    assert!(!branch_exists(&f, &hand));
    assert!(f.toasts[0].contains("git branch hand "), "{:?}", f.toasts);
    assert!(!f.toasts[0].contains("branch kept"), "{:?}", f.toasts);
}

#[test]
fn an_unrecorded_worktree_on_a_detached_head_is_discarded_with_its_commits_listed() {
    let mut f = Fixture::new();
    let hand = by_hand(&f, "by-hand", "hand");
    git(&hand.path, &["checkout", "-q", "--detach"]);
    commit(&hand, "loose.txt");
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    let row = &dialog_of(&f).rows[0];
    assert!(
        crate::leftovers::right_text(row, 0).contains("1 commit found nowhere else"),
        "{:?}",
        row.state
    );
    f.answer(&Answer::Discard(hand.path.clone(), BranchChoice::Delete));
    f.settle();
    assert!(!hand.path.exists());
    assert!(branch_exists(&f, &hand), "a detached worktree deletes no branch");
    assert!(tidewatch_core::removal_log::load(&f.paths)[0].head.is_some());
}

fn forget_every_record(f: &Fixture) {
    let _ = std::fs::remove_file(f.paths.state_dir().join("worktrees.toml"));
}

#[test]
fn a_worktree_of_ours_the_registry_lost_is_cleaned_up_when_proven_safe() {
    let mut f = Fixture::new();
    let lost = f.worktree();
    forget_every_record(&f);
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    assert_eq!(f.host.dialog.as_ref().unwrap().clean_up_blocker(), None);
    f.answer(&Answer::CleanUp);
    f.settle();
    assert!(!lost.path.exists());
    assert!(!branch_exists(&f, &lost));
    assert_eq!(f.host.leftovers.len(), 0);
}

#[test]
fn a_worktree_of_ours_the_registry_lost_keeps_its_work_until_its_own_discard() {
    let mut f = Fixture::new();
    let lost = f.worktree();
    commit(&lost, "work.txt");
    forget_every_record(&f);
    f.scan();
    f.host.open(&f.ctx);
    f.settle();
    let dialog = f.host.dialog.as_ref().unwrap();
    assert!(Cleanup::is_discardable(&dialog.rows[0]));
    assert!(dialog.clean_up_blocker().is_some());
    f.answer(&Answer::CleanUp);
    f.settle();
    assert!(lost.path.exists(), "Clean up never discards");

    f.answer(&Answer::Discard(lost.path.clone(), BranchChoice::Delete));
    f.settle();
    assert!(!lost.path.exists());
    assert_eq!(tidewatch_core::removal_log::load(&f.paths).len(), 1);
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
