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
    Poofed::bare(name, &format!("did {name}"), None, 60, date)
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
        (Some("readonly"), Fake::ReadOnly),
        (Some("promoted"), Fake::Promoted),
        (Some("named"), Fake::Named),
        (Some("onpr"), Fake::OnPr),
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
    assert_eq!(on.kind, Kind::Edit);
    assert_eq!(Fake::ReadOnly.state(now).unwrap().kind, Kind::ReadOnly);
    assert_eq!(Fake::Promoted.state(now).unwrap().kind, Kind::Edit);
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
        work: Work::default(),
    };
    assert_eq!(run_check(Step::Change, &spec, true), Err("no commit ahead of main".to_owned()));
    let said =
        |step, pr| run_check(step, &spec, pr).map(|checked| (checked.detail, checked.pr_url));
    assert_eq!(said(Step::Proof, true), Ok(("Worktree clean".to_owned(), None)));
    assert_eq!(
        said(Step::Tests, true),
        Ok(("No test command for this repo".to_owned(), None)),
        "no test command is a pass"
    );
    assert_eq!(said(Step::HandOff, false), Ok((String::new(), None)));
}

#[test]
fn a_second_note_for_an_agent_waits_behind_the_first_and_a_closed_tab_forgets_them() {
    let mut host = Host::new(None, false, Fake::Off);
    host.queue_paste(3, "fix it".into());
    host.queue_paste(4, "and this".into());
    host.queue_paste(3, "fix it better".into());
    let mut tabs = host.pending_tabs();
    tabs.sort_unstable();
    assert_eq!(tabs, [3, 4]);
    assert_eq!(host.take_paste(3).as_deref(), Some("fix it"));
    host.put_back_paste(3, "fix it".into());
    assert_eq!(host.take_paste(3).as_deref(), Some("fix it"));
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

fn left_behind(repo: &Repo) -> Poofed {
    let mut poofed = Poofed::bare("rename-helper", "Renamed it.", None, 60, day(20));
    poofed.repo = repo.worktree.repo.clone();
    poofed.branch = repo.worktree.branch.clone();
    poofed
}

#[test]
fn the_branch_of_a_poofed_tab_is_read_from_the_repo_after_its_folder_is_gone() {
    let repo = repo_with_worktree();
    git(&repo.worktree.path, &["commit", "-q", "--allow-empty", "-m", "feat: the work"]);
    let done =
        run_poof(&repo.paths, 7, Shell(Arc::new(AtomicBool::new(false))), &repo.worktree, None);
    assert!(matches!(done, Done::Poofed { .. }) && !repo.worktree.path.exists());
    let ctx = egui::Context::default();
    let mut host = Host::new(None, false, Fake::Off);
    assert_eq!(host.branch(7), None, "nothing asked yet");
    host.read_branch(&ctx, 7, Tools::system(), &left_behind(&repo));
    assert_eq!(host.branch(7), None, "git is being asked");
    wait(&mut host, &ctx, |host| host.branch(7).is_some());
    let Some(Branch::Found(found)) = host.branch(7) else { panic!("{:?}", host.branch(7)) };
    assert_eq!((found.base.as_str(), found.ahead), ("main", 1));
}

#[test]
fn a_branch_deleted_since_the_poof_says_so_and_is_asked_once() {
    let repo = repo_with_worktree();
    let gone = Poofed { branch: "agent/never-was".into(), ..left_behind(&repo) };
    let ctx = egui::Context::default();
    let mut host = Host::new(None, false, Fake::Off);
    host.read_branch(&ctx, 3, Tools::system(), &gone);
    wait(&mut host, &ctx, |host| host.branch(3).is_some());
    assert_eq!(host.branch(3), Some(&Branch::Missing));
    host.forget(3);
    assert_eq!(host.branch(3), None);
}

#[test]
fn the_poofed_switch_gives_a_summary_its_branch_without_asking_git() {
    let ctx = egui::Context::default();
    let mut host = Host::new(None, false, Fake::Poofed);
    let record = fake_poofed(day(20), 0).remove(0);
    host.read_branch(&ctx, 1, Tools::system(), &record);
    assert_eq!(host.branch(1), Some(&fake_branch()));
}

fn spec(repo: &Repo, started_at: Option<String>) -> LookSpec {
    LookSpec { tools: Tools::system(), dir: repo.worktree.path.clone(), started_at }
}

#[test]
fn a_worktree_counts_as_edited_once_a_file_changes_or_a_commit_lands_after_the_start() {
    let repo = repo_with_worktree();
    let dir = &repo.worktree.path;
    let start = check::head(&Tools::system(), dir);
    assert!(start.is_some());
    assert_eq!(run_look(&spec(&repo, start.clone())), Ok(false));
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    assert_eq!(run_look(&spec(&repo, start.clone())), Ok(true));
    git(dir, &["add", "notes.txt"]);
    git(dir, &["commit", "-q", "-m", "notes"]);
    assert_eq!(run_look(&spec(&repo, start)), Ok(true), "a clean tree with a new commit");
    assert_eq!(run_look(&spec(&repo, None)), Ok(false), "no start to compare with");
}

fn hanging(program: &str, dir: &std::path::Path) -> Tools {
    crate::test_exec::write_executable_from_a_child(
        &dir.join(program),
        "#!/bin/sh\nexec sleep 30\n",
    );
    let path = std::env::join_paths(
        std::iter::once(dir.to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap();
    Tools::with_path(path).with_limit(Duration::from_millis(300))
}

#[test]
fn a_check_whose_git_or_gh_never_answers_fails_its_step_with_the_limit_named() {
    let repo = repo_with_worktree();
    let tools = tempfile::tempdir().unwrap();
    let stuck_git = CheckSpec {
        tools: hanging("git", tools.path()),
        worktree: repo.worktree.clone(),
        work: Work::default(),
    };
    assert_eq!(
        run_check(Step::Proof, &stuck_git, true).err(),
        Some("git took longer than 300 ms and was stopped".to_owned())
    );
    let tools = tempfile::tempdir().unwrap();
    let stuck_gh = CheckSpec {
        tools: hanging("gh", tools.path()),
        worktree: repo.worktree.clone(),
        work: Work::default(),
    };
    assert_eq!(
        run_check(Step::HandOff, &stuck_gh, true).err(),
        Some("gh took longer than 300 ms and was stopped".to_owned())
    );
}

#[test]
fn a_look_releases_the_held_answer_only_for_the_latest_ticket() {
    let repo = repo_with_worktree();
    let ctx = egui::Context::default();
    let mut host = Host::new(None, false, Fake::Off);
    assert!(host.look_due(1, 0.0));
    host.look(&ctx, 1, spec(&repo, None), 0.0);
    assert!(!host.look_due(1, 10.0), "one look at a time");
    host.say_after_look(1, "the answer".into());
    host.look(&ctx, 1, spec(&repo, None), 1.0);
    assert_eq!(host.looked(1, 1), None, "the older look does not release it");
    assert_eq!(host.looked(1, 2).as_deref(), Some("the answer"));
    assert!(!host.look_due(1, 2.0) && host.look_due(1, 1.0 + LOOK_EVERY_SECS));
    assert_eq!(host.looked(1, 2), None);
}
