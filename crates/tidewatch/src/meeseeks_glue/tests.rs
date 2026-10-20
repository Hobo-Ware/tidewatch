use std::process::Command as Process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tidewatch_core::civil_date::Date;
use tidewatch_core::meeseeks::Struggle;

use super::*;

fn day(d: u8) -> Date {
    Date::new(2026, 10, d).unwrap()
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

struct Repo {
    _dir: tempfile::TempDir,
    paths: TidewatchPaths,
    worktree: Worktree,
}

fn repo_with_worktree() -> Repo {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let repo = home.join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let paths = TidewatchPaths::new(&home);
    let worktree = worktree::add_named(&paths, &repo, "rename-helper", None).unwrap();
    Repo { _dir: dir, paths, worktree }
}

struct Shell(Arc<AtomicBool>);

impl Drop for Shell {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn entry(name: &str, date: Date) -> Poofed {
    Poofed {
        name: name.into(),
        summary: format!("did {name}"),
        pr_url: None,
        duration_secs: 60,
        date,
    }
}

fn wait(host: &mut Host, ctx: &egui::Context, ready: impl Fn(&Host) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready(host) && Instant::now() < deadline {
        let _ = host.drain(ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ready(host));
}

#[test]
fn the_switch_reads_its_three_values_and_anything_else_is_off() {
    let cases = [
        (Some("on"), Fake::On),
        (Some("struggling"), Fake::Struggling),
        (Some("poofed"), Fake::Poofed),
        (Some("yes"), Fake::Off),
        (Some(""), Fake::Off),
        (None, Fake::Off),
    ];
    for (value, want) in cases {
        assert_eq!(Fake::parse(value), want, "{value:?}");
    }
}

#[test]
fn each_fake_state_shows_what_the_mockup_does() {
    let now = 100 * 60_000;
    let on = Fake::On.state(now).unwrap();
    assert_eq!((on.turns, on.failures, on.phase), (8, 0, Phase::Checking(Step::Tests)));
    assert_eq!(on.elapsed_ms(now), 12 * 60_000);
    let stuck = Fake::Struggling.state(now).unwrap();
    assert_eq!(stuck.struggle(now), Some(Struggle::FailedChecks));
    assert!(Fake::Poofed.state(now).is_none() && Fake::Off.state(now).is_none());
}

#[test]
fn a_clean_tree_is_poofed_the_agent_ends_the_folder_goes_and_the_branch_stays() {
    let repo = repo_with_worktree();
    git(&repo.worktree.path, &["commit", "-q", "--allow-empty", "-m", "feat: the work"]);
    let ended = Arc::new(AtomicBool::new(false));
    let done = run_poof(&repo.paths, 7, Shell(ended.clone()), &repo.worktree, None);
    assert!(matches!(done, Done::Poofed { tab: 7 }));
    assert!(ended.load(Ordering::SeqCst), "the agent's shell was dropped");
    assert!(!repo.worktree.path.exists());
    let branches = Process::new("git")
        .arg("-C")
        .arg(&repo.worktree.repo)
        .args(["branch", "--format=%(refname:short)"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&branches.stdout).contains("agent/rename-helper"));
}

#[test]
fn a_dirty_tree_refuses_the_poof_before_the_agent_is_touched_and_hands_the_shell_back() {
    let repo = repo_with_worktree();
    std::fs::write(repo.worktree.path.join("loose.txt"), "x").unwrap();
    let ended = Arc::new(AtomicBool::new(false));
    let done = run_poof(&repo.paths, 7, Shell(ended.clone()), &repo.worktree, None);
    let Done::Refused { tab, why, shell } = done else { panic!("expected a refusal") };
    assert_eq!((tab, why.as_str()), (7, "1 file with uncommitted changes"));
    assert!(shell.is_some());
    assert!(!ended.load(Ordering::SeqCst), "the agent is still running");
    assert!(repo.worktree.path.join("loose.txt").exists());
}

#[test]
fn the_checks_dispatch_to_their_step_and_a_hand_off_without_a_pr_is_nothing_to_do() {
    let repo = repo_with_worktree();
    let spec = CheckSpec {
        tools: Tools::system(),
        worktree: repo.worktree.clone(),
        summary: "Did it".into(),
    };
    assert_eq!(run_check(Step::Change, &spec, true), Err("no commit ahead of main".to_owned()));
    assert_eq!(run_check(Step::Proof, &spec, true), Ok(None));
    assert_eq!(run_check(Step::Tests, &spec, true), Ok(None), "no test command is a pass");
    assert_eq!(run_check(Step::HandOff, &spec, false), Ok(None));
}

#[test]
fn text_for_the_agent_waits_in_the_host_once_and_a_closed_tab_forgets_it() {
    let mut host = Host::new(None, false, Fake::Off);
    host.queue_paste(3, "fix it".into());
    host.queue_paste(4, "and this".into());
    host.queue_paste(3, "fix it better".into());
    let mut tabs = host.pending_tabs();
    tabs.sort_unstable();
    assert_eq!(tabs, [3, 4]);
    assert_eq!(host.take_paste(3).as_deref(), Some("fix it better"));
    assert_eq!(host.take_paste(3), None);
    host.forget(4);
    assert_eq!(host.pending_tabs().len(), 0);
}

#[test]
fn a_struggle_is_told_once_per_tab() {
    let mut host = Host::new(None, false, Fake::Off);
    assert!(host.first_struggle(1));
    assert!(!host.first_struggle(1));
    assert!(host.first_struggle(2));
    host.forget(1);
    assert!(host.first_struggle(1));
}

#[test]
fn a_fade_takes_six_hundred_milliseconds_and_ends_exactly_once() {
    let mut host = Host::new(None, false, Fake::Off);
    assert_eq!(host.fade(1, 10.0), None);
    assert!(!host.fading());
    host.start_fade(1, 10.0);
    assert!(host.fading());
    assert_eq!(host.fade(1, 10.0), Some(0.0));
    let half = host.fade(1, 10.3).unwrap();
    assert!((half - 0.5).abs() < 1e-6, "{half}");
    assert_eq!(host.fade(1, 12.0), Some(1.0));
    assert_eq!(host.faded(10.5).len(), 0);
    assert_eq!(host.faded(10.61), [1]);
    assert_eq!(host.faded(11.0).len(), 0);
    assert!(!host.fading());
}

#[test]
fn the_list_of_the_day_is_the_loaded_file_then_what_poofed_since_latest_first_without_repeats() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("meeseeks.toml");
    meeseeks_log::append(&file, entry("old", day(19))).unwrap();
    meeseeks_log::append(&file, entry("first", day(20))).unwrap();
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(file.clone()), true, Fake::Off);
    host.record(&ctx, entry("second", day(20)));
    wait(&mut host, &ctx, |host| !host.loaded.is_empty());
    host.record(&ctx, entry("first", day(20)));
    let names = |host: &Host| -> Vec<String> {
        host.poofed_on(day(20)).into_iter().map(|e| e.name).collect()
    };
    assert_eq!(names(&host), ["second", "first"]);
    assert_eq!(host.poofed_on(day(21)).len(), 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Log::load(&file).entries.len() < 4 && Instant::now() < deadline {
        let _ = host.drain(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(Log::load(&file).entries.len(), 4, "every poof was written");
}

#[test]
fn a_host_that_may_not_write_remembers_a_poof_but_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("meeseeks.toml");
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(file.clone()), false, Fake::Off);
    host.record(&ctx, entry("a", day(20)));
    assert_eq!(host.poofed_on(day(20)).len(), 1);
    std::thread::sleep(Duration::from_millis(60));
    let _ = host.drain(&ctx);
    assert!(!file.exists());
}

#[test]
fn the_poofed_switch_lists_two_poofs_toasts_once_and_never_touches_the_disk() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("meeseeks.toml");
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(file.clone()), true, Fake::Poofed);
    host.record(&ctx, entry("real", day(20)));
    let shown = host.poofed_on(day(20));
    let facts: Vec<(&str, Option<String>, u64)> =
        shown.iter().map(|e| (e.name.as_str(), e.pr_label(), e.duration_secs)).collect();
    assert_eq!(
        facts,
        [
            ("promo-test", Some("PR #213".to_owned()), 360),
            ("lint-fix", Some("PR #211".to_owned()), 180)
        ]
    );
    assert_eq!(host.fake_toast(), Some("vite-6-3 poofed"));
    assert_eq!(host.fake_toast(), None);
    std::thread::sleep(Duration::from_millis(60));
    let _ = host.drain(&ctx);
    assert!(!file.exists());
}
