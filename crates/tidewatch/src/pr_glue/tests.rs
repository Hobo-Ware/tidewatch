use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::pr_cards::Cards;
use tidewatch_core::pr_status::{Body, Seen};
use tidewatch_core::prefs::GithubPrefs;
use tidewatch_core::status::AgentStatus;
use tidewatch_core::tools::Tools;
use tidewatch_core::worktree::{self, Worktree};

use super::*;
use crate::pr_watch::Facts;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Merged {
    _home: tempfile::TempDir,
    paths: TidewatchPaths,
    worktree: Worktree,
    host: Host,
    ctx: egui::Context,
    busy: Busy,
}

fn merged_tab() -> Merged {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().canonicalize().unwrap();
    let repo = root.join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let paths = TidewatchPaths::new(&root);
    let worktree = worktree::add_named(&paths, &repo, "brave-otter", None).unwrap();
    git(&worktree.path, &["commit", "-q", "--allow-empty", "-m", "feat: work"]);
    let head = git(&worktree.path, &["rev-parse", "HEAD"]).trim().to_owned();
    let mut cards = Cards::default();
    cards.follow(42);
    cards.seen = Some(Seen { ended: true, head, ..Seen::default() });
    cards.add(10, Body::Merged { number: 42, base: "main".into() });
    cards.save(&paths.pr_cards_file(&repo, "brave-otter")).unwrap();
    let host = Host::new(Tools::with_path("/nonexistent"), GithubPrefs::default(), Dev::default());
    Merged {
        _home: home,
        paths,
        worktree,
        host,
        ctx: egui::Context::default(),
        busy: Busy::default(),
    }
}

impl Merged {
    fn tick(&mut self) {
        let status = AgentStatus::Done;
        let facts = [Facts {
            id: 1,
            status: &status,
            title: "brave-otter",
            dir: &self.worktree.path,
            repo: &self.worktree.repo,
            name: "brave-otter",
            branch: &self.worktree.branch,
            live: false,
            pr: None,
            autonomous: false,
            busy: self.busy,
        }];
        let _ = self.host.tick(&facts, &self.paths, &self.ctx, 0.0, 0);
    }

    fn until(&mut self, done: impl Fn(&Cleanup) -> bool) -> Vec<Applied> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut applied = Vec::new();
        loop {
            self.tick();
            applied.extend(self.host.drain(0.0, 0));
            if self.host.watch(1).is_some_and(|watch| done(&watch.cleanup)) {
                return applied;
            }
            assert!(Instant::now() < deadline, "the check never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn settled(cleanup: &Cleanup) -> bool {
    matches!(cleanup, Cleanup::Checked(_) | Cleanup::Failed(_))
}

#[test]
fn a_merged_tab_with_nothing_unsaved_is_checked_on_its_own_and_found_safe() {
    let mut tab = merged_tab();
    tab.until(settled);
    let Cleanup::Checked(report) = &tab.host.watch(1).unwrap().cleanup else { panic!("checked") };
    assert!(report.is_safe(), "{report:?}");
    assert_eq!(report.local_commits, Some(0));
}

#[test]
fn removal_is_checked_again_and_refused_when_something_appeared_since() {
    let mut tab = merged_tab();
    tab.until(settled);
    std::fs::write(tab.worktree.path.join("idea.md"), "unsaved\n").unwrap();
    let ctx = tab.ctx.clone();
    tab.host.check_before_removal(1, tab.worktree.clone(), Busy::default(), &tab.paths, &ctx);
    let applied = tab.until(settled);
    assert!(applied.iter().all(|a| a.out != Out::Remove), "{applied:?}");
    assert!(applied.iter().any(|a| matches!(a.out, Out::Toast(_))), "{applied:?}");
    let Cleanup::Checked(report) = &tab.host.watch(1).unwrap().cleanup else { panic!("checked") };
    assert_eq!(report.untracked, ["idea.md"]);
}

#[test]
fn removal_goes_ahead_once_the_second_check_is_clean() {
    let mut tab = merged_tab();
    tab.until(settled);
    let ctx = tab.ctx.clone();
    tab.host.check_before_removal(1, tab.worktree.clone(), Busy::default(), &tab.paths, &ctx);
    let applied = tab.until(settled);
    assert_eq!(applied.iter().filter(|a| a.out == Out::Remove).count(), 1, "{applied:?}");
}

#[test]
fn a_kept_card_is_not_checked_again() {
    let mut tab = merged_tab();
    tab.until(settled);
    tab.host.dismiss(1, 1, false);
    tab.host.tabs.get_mut(&1).unwrap().cleanup = Cleanup::Unchecked;
    for _ in 0..3 {
        tab.tick();
    }
    assert_eq!(tab.host.watch(1).unwrap().cleanup, Cleanup::Unchecked);
}

fn click_remove(tab: &mut Merged, busy: Busy) -> Vec<Applied> {
    let ctx = tab.ctx.clone();
    let mut applied =
        tab.host.check_before_removal(1, tab.worktree.clone(), busy, &tab.paths, &ctx);
    if applied.is_empty() {
        applied = tab.until(settled);
    }
    applied
}

fn refused(applied: &[Applied]) -> bool {
    applied.iter().all(|a| a.out != Out::Remove)
        && applied.iter().any(|a| matches!(a.out, Out::Toast(_)))
}

#[test]
fn a_click_on_a_stale_offer_is_refused_for_every_reason_the_agent_still_needs_the_worktree() {
    let reasons = [
        Busy { working: true, ..Busy::default() },
        Busy { open_ledger: true, ..Busy::default() },
        Busy { job: true, ..Busy::default() },
    ];
    for busy in reasons {
        let mut tab = merged_tab();
        tab.until(settled);
        let applied = click_remove(&mut tab, busy);
        assert!(refused(&applied), "{busy:?}: {applied:?}");
        assert_eq!(applied.iter().filter(|a| a.out == Out::Toast(STAYS.into())).count(), 1);
    }
}

#[test]
fn a_commit_made_after_the_merge_refuses_a_click_on_the_offer_that_was_safe() {
    let mut tab = merged_tab();
    tab.until(settled);
    git(&tab.worktree.path, &["commit", "-q", "--allow-empty", "-m", "feat: after the merge"]);
    let applied = click_remove(&mut tab, Busy::default());
    assert!(refused(&applied), "{applied:?}");
    let Cleanup::Checked(report) = &tab.host.watch(1).unwrap().cleanup else { panic!("checked") };
    assert_eq!(report.local_commits, Some(1));
}

#[test]
fn the_offer_returns_after_the_agent_lets_go_and_the_removal_then_goes_ahead() {
    let mut tab = merged_tab();
    tab.until(settled);
    tab.busy = Busy { working: true, open_ledger: true, job: true };
    tab.tick();
    assert_eq!(tab.host.watch(1).unwrap().busy, tab.busy);
    tab.busy = Busy::default();
    tab.tick();
    assert_eq!(tab.host.watch(1).unwrap().cleanup, Cleanup::Checking);
    tab.until(settled);
    let applied = click_remove(&mut tab, Busy::default());
    assert_eq!(applied.iter().filter(|a| a.out == Out::Remove).count(), 1, "{applied:?}");
}
