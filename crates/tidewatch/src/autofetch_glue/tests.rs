use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::repo::Repo;

const MINUTE: Duration = Duration::from_mins(1);

fn no_paths() -> Vec<PathBuf> {
    Vec::new()
}

fn path(name: &str) -> PathBuf {
    PathBuf::from("/repos").join(name)
}

fn prefs(minutes: u32) -> GitPrefs {
    GitPrefs { background_fetch: true, fetch_minutes: minutes }
}

#[test]
fn a_repo_is_fetched_once_per_interval_not_when_it_is_first_seen() {
    let start = Instant::now();
    let mut schedule = Schedule::new(5 * MINUTE);
    let web = [path("web")];
    assert_eq!(schedule.plan(&web, start).start, no_paths());
    assert_eq!(schedule.plan(&web, start + 4 * MINUTE).start, no_paths());
    assert_eq!(schedule.plan(&web, start + 5 * MINUTE).start, web);
    schedule.finish(&web[0], &Ok(()));
    assert_eq!(schedule.plan(&web, start + 9 * MINUTE).start, no_paths());
    assert_eq!(schedule.plan(&web, start + 10 * MINUTE).start, web);
}

#[test]
fn repos_with_several_worktrees_are_one_fetch_each() {
    let start = Instant::now();
    let mut schedule = Schedule::new(MINUTE);
    let roots = [path("web"), path("api"), path("web")];
    schedule.plan(&roots, start);
    let plan = schedule.plan(&roots, start + MINUTE);
    assert_eq!(plan.start, [path("web"), path("api")]);
}

#[test]
fn a_repo_is_never_fetched_while_its_last_fetch_is_running() {
    let start = Instant::now();
    let mut schedule = Schedule::new(MINUTE);
    let roots = [path("web"), path("api")];
    schedule.plan(&roots, start);
    assert_eq!(schedule.plan(&roots, start + MINUTE).start.len(), 2);
    schedule.finish(&path("api"), &Ok(()));
    let plan = schedule.plan(&roots, start + 2 * MINUTE);
    assert_eq!(plan.start, [path("api")]);
    schedule.finish(&path("web"), &Ok(()));
    schedule.finish(&path("api"), &Ok(()));
    assert_eq!(schedule.plan(&roots, start + 3 * MINUTE).start.len(), 2);
}

#[test]
fn the_next_wake_is_the_soonest_repo() {
    let start = Instant::now();
    let mut schedule = Schedule::new(5 * MINUTE);
    schedule.plan(&[path("web")], start);
    let plan = schedule.plan(&[path("web"), path("api")], start + 2 * MINUTE);
    assert_eq!(plan.wake, Some(3 * MINUTE));
    assert_eq!(Schedule::new(MINUTE).plan(&[], start).wake, None);
}

#[test]
fn a_failed_fetch_leaves_a_note_and_the_next_interval_tries_again() {
    let start = Instant::now();
    let mut schedule = Schedule::new(MINUTE);
    let web = [path("web")];
    schedule.plan(&web, start);
    schedule.plan(&web, start + MINUTE);
    assert!(!schedule.finish(&web[0], &Err("no route to host".to_owned())));
    assert_eq!(schedule.failure(&web[0]), Some("no route to host"));
    assert_eq!(schedule.plan(&web, start + 2 * MINUTE).start, web);
    assert!(schedule.finish(&web[0], &Ok(())));
    assert_eq!(schedule.failure(&web[0]), None);
}

#[test]
fn a_closed_repo_is_forgotten_unless_its_fetch_is_still_running() {
    let start = Instant::now();
    let mut schedule = Schedule::new(MINUTE);
    let web = [path("web")];
    schedule.plan(&web, start);
    schedule.plan(&web, start + MINUTE);
    schedule.plan(&[], start + 2 * MINUTE);
    assert_eq!(schedule.plan(&web, start + 2 * MINUTE).start, no_paths());
    schedule.finish(&web[0], &Ok(()));
    schedule.plan(&[], start + 3 * MINUTE);
    assert_eq!(schedule.failure(&web[0]), None);
    let reopened = schedule.plan(&web, start + 4 * MINUTE);
    assert_eq!(reopened.start, no_paths());
    assert_eq!(reopened.wake, Some(MINUTE));
}

#[test]
fn a_new_interval_moves_the_next_fetch_and_restart_counts_from_now() {
    let start = Instant::now();
    let mut schedule = Schedule::new(5 * MINUTE);
    let web = [path("web")];
    schedule.plan(&web, start);
    schedule.set_every(MINUTE);
    assert_eq!(schedule.plan(&web, start + 2 * MINUTE).start, web);
    schedule.finish(&web[0], &Ok(()));
    schedule.restart(10 * MINUTE, start + 3 * MINUTE);
    assert_eq!(schedule.plan(&web, start + 12 * MINUTE).start, no_paths());
    assert_eq!(schedule.plan(&web, start + 13 * MINUTE).start, web);
}

#[test]
fn only_repos_are_fetched_and_a_plain_folder_is_not() {
    let workspaces = [
        Workspace::Repo(Repo { root: path("web"), label: "web".to_owned() }),
        Workspace::Other { dir: path("notes"), label: "notes".to_owned() },
        Workspace::Repo(Repo { root: path("api"), label: "api".to_owned() }),
    ];
    assert_eq!(roots(&workspaces), [path("web"), path("api")]);
}

#[test]
fn a_reason_is_the_line_git_blames_without_its_prefix_and_is_kept_short() {
    let cases = [
        (
            "git fetch --all --prune failed: fatal: unable to access 'https://x.test/a.git/': Could not resolve host: x.test",
            "unable to access 'https://x.test/a.git/': Could not resolve host: x.test",
        ),
        (
            "git fetch --all --prune failed: Fetching origin\nerror: cannot lock ref 'refs/remotes/origin/a'\n",
            "cannot lock ref 'refs/remotes/origin/a'",
        ),
        ("Fetching origin\nSomething odd.", "Something odd"),
        ("", "git fetch failed"),
    ];
    for (message, want) in cases {
        assert_eq!(reason_of(message), want, "{message:?}");
    }
    let long = reason_of(&format!("fatal: {}", "x".repeat(400)));
    assert_eq!(long.chars().count(), REASON_MAX + 3);
    assert!(long.ends_with("..."));
}

static CALLS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

#[allow(clippy::unnecessary_wraps)]
fn count_and_succeed(root: &Path) -> Result<(), String> {
    CALLS.lock().unwrap().push(root.to_owned());
    Ok(())
}

fn settle(host: &mut Host, ctx: &egui::Context, roots: &[PathBuf], now: Instant) -> Vec<PathBuf> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut fetched = Vec::new();
    while fetched.is_empty() && Instant::now() < deadline {
        fetched = host.drive(ctx, roots, now);
        std::thread::sleep(Duration::from_millis(5));
    }
    fetched
}

#[test]
fn the_host_fetches_each_repo_group_once_per_interval_and_reports_what_it_fetched() {
    let ctx = egui::Context::default();
    let start = Instant::now();
    let mut host = Host::with_fetch(prefs(2), true, count_and_succeed);
    let roots = roots(&[
        Workspace::Repo(Repo { root: path("web"), label: "web".to_owned() }),
        Workspace::Other { dir: path("notes"), label: "notes".to_owned() },
    ]);
    assert_eq!(host.drive(&ctx, &roots, start), no_paths());
    assert_eq!(host.drive(&ctx, &roots, start + MINUTE), no_paths());
    let fetched = settle(&mut host, &ctx, &roots, start + 2 * MINUTE);
    assert_eq!(fetched, [path("web")]);
    assert_eq!(host.drive(&ctx, &roots, start + 3 * MINUTE), no_paths());
    assert_eq!(*CALLS.lock().unwrap(), [path("web")]);
}

static STRAYS: AtomicUsize = AtomicUsize::new(0);

#[allow(clippy::unnecessary_wraps)]
fn count_a_stray(_: &Path) -> Result<(), String> {
    STRAYS.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

#[test]
fn nothing_is_fetched_when_the_switch_is_off_or_the_run_is_not_live_or_it_was_just_turned_on() {
    let ctx = egui::Context::default();
    let start = Instant::now();
    let roots = [path("web")];
    let off = GitPrefs { background_fetch: false, fetch_minutes: 2 };
    for (git, live) in [(off, true), (prefs(2), false)] {
        let mut host = Host::with_fetch(git, live, count_a_stray);
        host.drive(&ctx, &roots, start);
        host.drive(&ctx, &roots, start + 2 * MINUTE);
        host.drive(&ctx, &roots, start + 4 * MINUTE);
    }
    let mut host = Host::with_fetch(prefs(2), true, count_a_stray);
    host.drive(&ctx, &roots, start);
    host.set_prefs(off, start + MINUTE);
    host.drive(&ctx, &roots, start + 2 * MINUTE);
    host.set_prefs(prefs(2), start + 10 * MINUTE);
    host.drive(&ctx, &roots, start + 11 * MINUTE);
    std::thread::sleep(Duration::from_millis(100));
    host.drive(&ctx, &roots, start + 11 * MINUTE);
    assert_eq!(STRAYS.load(Ordering::SeqCst), 0);
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn commit(dir: &Path, file: &str) {
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", file]);
}

struct Throwaway {
    _dir: tempfile::TempDir,
    remote: PathBuf,
    seed: PathBuf,
    work: PathBuf,
}

fn throwaway() -> Throwaway {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let (remote, seed, work) = (base.join("remote.git"), base.join("seed"), base.join("work"));
    std::fs::create_dir_all(&seed).unwrap();
    git(&base, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    git(&seed, &["init", "-q", "-b", "main"]);
    commit(&seed, "a.txt");
    git(&seed, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&seed, &["push", "-q", "origin", "main"]);
    git(&base, &["clone", "-q", remote.to_str().unwrap(), "work"]);
    Throwaway { _dir: dir, remote, seed, work }
}

#[test]
fn a_real_fetch_moves_the_remote_branch_and_leaves_the_checkout_alone() {
    let repo = throwaway();
    commit(&repo.seed, "b.txt");
    git(&repo.seed, &["push", "-q", "origin", "main"]);
    let behind = |work: &Path| git(work, &["rev-list", "--count", "HEAD..origin/main"]);
    assert_eq!(behind(&repo.work), "0");
    let head = git(&repo.work, &["rev-parse", "HEAD"]);

    let ctx = egui::Context::default();
    let start = Instant::now();
    let mut host = Host::new(prefs(1), true);
    let roots = [repo.work.clone()];
    host.drive(&ctx, &roots, start);
    let fetched = settle(&mut host, &ctx, &roots, start + MINUTE);

    assert_eq!(fetched, roots);
    assert_eq!(behind(&repo.work), "1");
    assert_eq!(git(&repo.work, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&repo.work, &["status", "--porcelain"]), "");
    assert_eq!(host.failure_note(&repo.work), None);
}

#[test]
fn a_real_fetch_that_fails_leaves_a_note_and_succeeds_on_the_next_interval() {
    let repo = throwaway();
    let moved = repo.remote.with_extension("moved");
    std::fs::rename(&repo.remote, &moved).unwrap();

    let ctx = egui::Context::default();
    let start = Instant::now();
    let mut host = Host::new(prefs(1), true);
    let roots = [repo.work.clone()];
    host.drive(&ctx, &roots, start);
    let deadline = Instant::now() + Duration::from_secs(10);
    host.drive(&ctx, &roots, start + MINUTE);
    while host.failure_note(&repo.work).is_none() && Instant::now() < deadline {
        host.drive(&ctx, &roots, start + MINUTE);
        std::thread::sleep(Duration::from_millis(5));
    }
    let note = host.failure_note(&repo.work).expect("a note after the failed fetch");
    assert!(note.ends_with("Trying again in 1 min."), "{note}");

    std::fs::rename(&moved, &repo.remote).unwrap();
    let fetched = settle(&mut host, &ctx, &roots, start + 2 * MINUTE);
    assert_eq!(fetched, roots);
    assert_eq!(host.failure_note(&repo.work), None);
}

#[test]
fn a_repo_with_no_remote_fetches_quietly() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().canonicalize().unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    commit(&repo, "a.txt");
    assert_eq!(fetch_all(&repo), Ok(()));
}
