use std::time::{Duration, Instant};

use tidewatch_core::workflow_profile::PullRequests;

use super::*;

struct World {
    _home: tempfile::TempDir,
    paths: TidewatchPaths,
    repo: PathBuf,
}

fn world(rules: &str) -> World {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let repo = home.path().join("web");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("AGENTS.md"), rules).unwrap();
    World { _home: home, paths, repo }
}

fn settled(host: &mut Host, ctx: &egui::Context, repo: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        host.sync(&[repo], ctx);
        host.drive();
        if host.detected(repo).is_some() || host.confirmed(repo) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("detection did not finish");
}

#[test]
fn a_repo_with_no_profile_gets_a_card_with_what_was_found() {
    let w = world("Catch up with `git merge origin/main`.");
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(w.paths.clone()), Mode::Live);
    assert_eq!(host.proposal(&w.repo).map(Detected::workflow), None, "nothing before the look");
    settled(&mut host, &ctx, &w.repo);
    let proposal = host.proposal(&w.repo).unwrap();
    assert_eq!(proposal.catch_up.value, CatchUp::Merge);
    assert_eq!(host.profile(&w.repo).catch_up, CatchUp::Merge, "used before it is confirmed");
    assert!(!host.confirmed(&w.repo));
}

#[test]
fn accepting_saves_the_values_and_a_new_window_shows_no_card() {
    let w = world("Catch up with `git merge origin/main`.");
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(w.paths.clone()), Mode::Live);
    settled(&mut host, &ctx, &w.repo);
    host.accept(&w.repo);
    assert_eq!(host.proposal(&w.repo).map(Detected::workflow), None);
    assert!(host.confirmed(&w.repo));
    let deadline = Instant::now() + Duration::from_secs(5);
    while Store::load(&w.paths).get(&w.repo).is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(Store::load(&w.paths).get(&w.repo).map(|w| w.catch_up), Some(CatchUp::Merge));

    let mut later = Host::new(Some(w.paths.clone()), Mode::Live);
    settled(&mut later, &ctx, &w.repo);
    assert_eq!(later.proposal(&w.repo).map(Detected::workflow), None);
    assert_eq!(later.profile(&w.repo).catch_up, CatchUp::Merge);
}

#[test]
fn dismissing_hides_the_card_but_keeps_using_the_found_values() {
    let w = world("Open pull requests with `gh pr create --draft`. Never use a fixup.");
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(w.paths.clone()), Mode::Live);
    settled(&mut host, &ctx, &w.repo);
    host.dismiss(&w.repo);
    assert_eq!(host.proposal(&w.repo).map(Detected::workflow), None);
    assert_eq!(host.profile(&w.repo).fixes, Fixes::NewCommits);
    assert!(!host.confirmed(&w.repo));
    assert_eq!(Store::load(&w.paths), Store::default(), "nothing was written");
}

#[test]
fn a_choice_made_in_settings_counts_as_confirmed() {
    let w = world("");
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(w.paths.clone()), Mode::Live);
    settled(&mut host, &ctx, &w.repo);
    let chosen = Workflow { pull_requests: PullRequests::Ready, ..Workflow::default() };
    host.set(&w.repo, chosen);
    assert_eq!(host.profile(&w.repo), chosen);
    assert_eq!(host.proposal(&w.repo).map(Detected::workflow), None);
}

#[test]
fn a_quiet_run_detects_but_never_shows_the_card_or_writes() {
    let w = world("Always rebase.");
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(w.paths.clone()), Mode::Quiet);
    settled(&mut host, &ctx, &w.repo);
    assert_eq!(host.proposal(&w.repo).map(Detected::workflow), None);
    assert_eq!(host.profile(&w.repo).catch_up, CatchUp::Rebase);
    host.set(&w.repo, Workflow { catch_up: CatchUp::Merge, ..Workflow::default() });
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(Store::load(&w.paths), Store::default());
}

#[test]
fn the_dev_switch_shows_stand_in_values_and_writes_nothing() {
    let w = world("Always merge origin/main.");
    let ctx = egui::Context::default();
    let mut host = Host::new(Some(w.paths.clone()), Mode::Fake);
    host.sync(&[&w.repo], &ctx);
    assert_eq!(host.proposal(&w.repo).cloned(), Some(stand_in()));
    host.accept(&w.repo);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!w.paths.workflow_file().exists());
}

#[test]
fn without_a_home_the_defaults_are_used() {
    let ctx = egui::Context::default();
    let mut host = Host::new(None, Mode::Live);
    host.sync(&[Path::new("/work/web")], &ctx);
    host.drive();
    assert_eq!(host.profile(Path::new("/work/web")), Workflow::default());
    assert_eq!(host.proposal(Path::new("/work/web")).map(Detected::workflow), None);
}
