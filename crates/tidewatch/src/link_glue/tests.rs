use std::time::{Duration, Instant};

use tidewatch_core::link_board::TitleFrom;
use tidewatch_core::tools::Tools;

use super::*;

const NOW: i64 = 1_000_000_000;
const MIN: i64 = 60_000;

fn no_gh() -> Tools {
    Tools::with_path("/nonexistent")
}

fn own<'a>(number: u64, url: &'a str, title: Option<&'a str>) -> OwnPr<'a> {
    OwnPr { number, url, title, state: LinkState::Unknown }
}

struct Fx {
    dir: tempfile::TempDir,
    ctx: egui::Context,
}

impl Fx {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap(), ctx: egui::Context::default() }
    }

    fn notes(&self) -> PathBuf {
        self.dir.path().join("notes.json")
    }

    fn file(&self) -> PathBuf {
        links_file_beside(&self.notes())
    }

    fn tick(&self, host: &mut Host, ids: &[u64]) {
        self.tick_at(host, ids, NOW);
    }

    fn tick_at(&self, host: &mut Host, ids: &[u64], now_ms: i64) {
        let notes = self.notes();
        let facts: Vec<Facts<'_>> =
            ids.iter().map(|id| Facts { id: *id, notes_file: &notes }).collect();
        host.tick(&facts, &self.ctx, now_ms);
    }

    #[cfg(unix)]
    fn fake_gh(&self, body: &str) -> Tools {
        use crate::test_exec::write_executable_from_a_child;
        write_executable_from_a_child(&self.dir.path().join("gh"), &format!("#!/bin/sh\n{body}\n"));
        let path = format!("{}:/usr/bin:/bin", self.dir.path().display());
        Tools::with_path(path)
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("gh.log"))
            .map(|log| log.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn until(&self, host: &mut Host, ids: &[u64], done: impl Fn(&Host) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(host) && Instant::now() < deadline {
            self.tick(host, ids);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn link(url: &str, title: &str, from: TitleFrom) -> Link {
    Link {
        url: url.to_owned(),
        title: title.to_owned(),
        note: None,
        first_seen_ms: NOW,
        title_from: from,
        state: LinkState::Unknown,
    }
}

fn gh_link(url: &str, state: LinkState) -> Link {
    Link { state, ..link(url, "t", TitleFrom::Agent) }
}

#[test]
fn a_saved_board_is_read_when_the_tab_appears() {
    let fx = Fx::new();
    let mut saved = LinkBoard::default();
    saved.absorb(&link("https://a.dev/1", "One", TitleFrom::Agent));
    link_store::save(&fx.file(), &saved).unwrap();
    let mut host = Host::new(true, false, no_gh());
    assert!(host.board(1).is_none());
    fx.until(&mut host, &[1], |host| host.board(1).is_some());
    assert_eq!(host.board(1).unwrap().links[0].title, "One");
}

#[test]
fn links_found_before_the_read_ends_are_kept_with_what_the_file_holds() {
    let fx = Fx::new();
    let mut saved = LinkBoard::default();
    saved.absorb(&link("https://a.dev/1", "One", TitleFrom::Agent));
    link_store::save(&fx.file(), &saved).unwrap();
    let mut host = Host::new(true, false, no_gh());
    fx.tick(&mut host, &[1]);
    host.found(1, vec![link("https://a.dev/2", "Two", TitleFrom::Seen)], &fx.ctx);
    fx.until(&mut host, &[1], |host| host.board(1).is_some_and(|board| board.len() == 2));
    let board = host.board(1).unwrap();
    assert_eq!(board.len(), 2);
    let deadline = Instant::now() + Duration::from_secs(5);
    while link_store::load(&fx.file()).is_none_or(|disk| disk.len() < 2)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(link_store::load(&fx.file()).unwrap().len(), 2, "what was found is saved too");
}

#[test]
fn what_the_agent_reported_wins_over_what_was_found_in_the_same_board() {
    let fx = Fx::new();
    let mut host = Host::new(false, false, no_gh());
    fx.tick(&mut host, &[1]);
    host.arrived(1, &[link("https://a.dev/1", "Reported", TitleFrom::Agent)]);
    host.found(1, vec![link("https://a.dev/1", "Artifact", TitleFrom::Guess)], &fx.ctx);
    assert_eq!(host.board(1).unwrap().links[0].title, "Reported");
}

#[test]
fn nothing_is_written_when_the_host_may_not_write() {
    let fx = Fx::new();
    let mut host = Host::new(false, false, no_gh());
    fx.tick(&mut host, &[1]);
    host.found(1, vec![link("https://a.dev/1", "One", TitleFrom::Seen)], &fx.ctx);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(host.board(1).unwrap().len(), 1);
    assert!(!fx.file().exists());
}

#[test]
fn the_fake_switch_gives_every_tab_the_mockup_links_and_writes_nothing() {
    let fx = Fx::new();
    let mut host = Host::new(true, true, no_gh());
    fx.tick(&mut host, &[1, 2]);
    for id in [1, 2] {
        assert_eq!(host.board(id).unwrap().len(), 8);
    }
    host.found(1, vec![link("https://a.dev/9", "More", TitleFrom::Seen)], &fx.ctx);
    host.arrived(1, &[link("https://a.dev/8", "More", TitleFrom::Agent)]);
    host.own_pr(1, &own(3, "https://github.com/o/r/pull/3", Some("t")), NOW, &fx.ctx);
    assert_eq!(host.board(1).unwrap().len(), 8, "a capture is not changed by what arrives");
    std::thread::sleep(Duration::from_millis(100));
    assert!(!fx.file().exists());
}

#[test]
fn a_closed_tab_is_forgotten_and_a_link_for_a_tab_that_is_not_open_is_dropped() {
    let fx = Fx::new();
    let mut host = Host::new(false, false, no_gh());
    host.found(7, vec![link("https://a.dev/1", "One", TitleFrom::Seen)], &fx.ctx);
    host.arrived(7, &[link("https://a.dev/1", "One", TitleFrom::Agent)]);
    assert!(host.board(7).is_none());
    fx.tick(&mut host, &[1]);
    host.found(1, vec![link("https://a.dev/1", "One", TitleFrom::Seen)], &fx.ctx);
    fx.tick(&mut host, &[]);
    assert!(host.board(1).is_none());
}

#[test]
fn the_tabs_own_pull_request_joins_the_board_and_its_title_is_taken_when_it_arrives() {
    let fx = Fx::new();
    let mut host = Host::new(false, false, no_gh());
    fx.tick(&mut host, &[1]);
    let url = "https://github.com/o/r/pull/42";
    host.own_pr(1, &own(42, url, None), NOW, &fx.ctx);
    assert_eq!(host.board(1).unwrap().links[0].title, "PR #42");
    host.own_pr(1, &own(42, url, None), NOW + 5, &fx.ctx);
    host.own_pr(1, &own(42, url, Some("feat: dashboard")), NOW + 9, &fx.ctx);
    let board = host.board(1).unwrap();
    assert_eq!(board.len(), 1);
    assert_eq!(board.links[0].title, "#42 feat: dashboard");
    assert_eq!(board.links[0].first_seen_ms, NOW, "the first-seen time stays");
}

#[test]
fn what_arrived_while_the_file_was_read_beats_what_the_file_held_for_the_same_link() {
    let fx = Fx::new();
    let mut saved = LinkBoard::default();
    saved.absorb(&link("https://a.dev/1", "Old title", TitleFrom::Seen));
    link_store::save(&fx.file(), &saved).unwrap();
    let mut host = Host::new(false, false, no_gh());
    fx.tick(&mut host, &[1]);
    host.found(1, vec![link("https://a.dev/1", "New title", TitleFrom::Seen)], &fx.ctx);
    fx.until(&mut host, &[1], |host| {
        host.tabs.get(&1).is_some_and(|tab| tab.board.len() == 1 && tab.loaded)
    });
    assert_eq!(host.board(1).unwrap().links[0].title, "New title");
}

const PR_MERGED: &str = "https://github.com/o/r/pull/1";
const ISSUE_CLOSED: &str = "https://github.com/o/r/issues/2";
const PR_OPEN: &str = "https://github.com/o/r/pull/3";
const PR_SETTLED: &str = "https://github.com/o/r/pull/4";
const WEB: &str = "https://a.dev/5";

#[cfg(unix)]
const ANSWERS: &str = r#"echo "$@" >> "$(dirname "$0")/gh.log"
case "$3" in
  */pull/1) echo '{"state":"MERGED"}' ;;
  */issues/2) echo '{"state":"CLOSED"}' ;;
  *) echo '{"state":"OPEN"}' ;;
esac"#;

fn state(host: &Host, url: &str) -> LinkState {
    host.board(1).unwrap().links.iter().find(|link| link.url == url).unwrap().state
}

#[cfg(unix)]
fn loaded_host(fx: &Fx, tools: Tools, writable: bool) -> Host {
    let mut host = Host::new(writable, false, tools);
    fx.until(&mut host, &[1], |host| host.tabs.get(&1).is_some_and(|tab| tab.loaded));
    let links = [PR_MERGED, ISSUE_CLOSED, PR_OPEN, WEB].map(|url| gh_link(url, LinkState::Unknown));
    host.arrived(1, &links);
    host.arrived(1, &[gh_link(PR_SETTLED, LinkState::Merged)]);
    host
}

#[cfg(unix)]
fn until_looked_up(fx: &Fx, host: &mut Host) {
    fx.until(host, &[1], |host| {
        [PR_MERGED, ISSUE_CLOSED, PR_OPEN].iter().all(|url| !state(host, url).is_unknown())
    });
}

#[cfg(unix)]
#[test]
fn a_pull_request_or_issue_link_is_looked_up_once_and_a_merged_or_web_link_never() {
    let fx = Fx::new();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    until_looked_up(&fx, &mut host);
    assert_eq!(state(&host, PR_MERGED), LinkState::Merged);
    assert_eq!(state(&host, ISSUE_CLOSED), LinkState::Closed);
    assert_eq!(state(&host, PR_OPEN), LinkState::Open);
    assert_eq!(state(&host, PR_SETTLED), LinkState::Merged);
    assert_eq!(state(&host, WEB), LinkState::Unknown);
    fx.tick_at(&mut host, &[1], NOW + 5 * MIN);
    std::thread::sleep(Duration::from_millis(100));
    let mut calls = fx.calls();
    calls.sort();
    assert_eq!(
        calls,
        [
            format!("issue view {ISSUE_CLOSED} --json state"),
            format!("pr view {PR_MERGED} --json state"),
            format!("pr view {PR_OPEN} --json state"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn only_a_link_still_open_is_looked_up_again_after_30_minutes() {
    let fx = Fx::new();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    until_looked_up(&fx, &mut host);
    fx.tick_at(&mut host, &[1], NOW + 29 * MIN);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fx.calls().len(), 3);
    fx.tick_at(&mut host, &[1], NOW + 30 * MIN);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fx.calls().len() < 4 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fx.calls().last().unwrap(), &format!("pr view {PR_OPEN} --json state"));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fx.calls().len(), 4, "the merged and the closed one stay settled");
}

#[cfg(unix)]
#[test]
fn a_lookup_runs_off_the_calling_thread_and_the_answer_is_saved_in_links_json() {
    let fx = Fx::new();
    let release = fx.dir.path().join("release");
    let gate = format!("until [ -f '{}' ]; do sleep 0.02; done\n{ANSWERS}", release.display());
    let mut host = loaded_host(&fx, fx.fake_gh(&gate), true);
    let started = Instant::now();
    fx.tick(&mut host, &[1]);
    assert!(started.elapsed() < Duration::from_secs(1), "the tick did not wait for gh");
    assert_eq!(state(&host, PR_OPEN), LinkState::Unknown);
    std::fs::write(&release, "").unwrap();
    until_looked_up(&fx, &mut host);
    let deadline = Instant::now() + Duration::from_secs(5);
    while link_store::load(&fx.file()).is_none_or(|disk| {
        disk.links.iter().filter(|link| link.state == LinkState::Merged).count() < 2
    }) && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    let disk = link_store::load(&fx.file()).unwrap();
    let saved = |url: &str| disk.links.iter().find(|link| link.url == url).unwrap().state;
    assert_eq!((saved(PR_MERGED), saved(ISSUE_CLOSED)), (LinkState::Merged, LinkState::Closed));
}

#[cfg(unix)]
#[test]
fn a_failed_lookup_leaves_the_state_unknown_and_waits_30_minutes_to_try_again() {
    let fx = Fx::new();
    let mut host =
        loaded_host(&fx, fx.fake_gh("echo x >> \"$(dirname \"$0\")/gh.log\"; exit 1"), true);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fx.calls().len() < 3 && Instant::now() < deadline {
        fx.tick(&mut host, &[1]);
        std::thread::sleep(Duration::from_millis(5));
    }
    fx.tick_at(&mut host, &[1], NOW + 10 * MIN);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fx.calls().len(), 3);
    assert_eq!(state(&host, PR_OPEN), LinkState::Unknown);
}

#[cfg(unix)]
#[test]
fn nothing_is_looked_up_when_the_host_may_not_write() {
    let fx = Fx::new();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), false);
    fx.tick(&mut host, &[1]);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fx.calls(), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn the_tabs_own_pull_request_takes_its_state_from_the_watch_and_is_not_looked_up() {
    let fx = Fx::new();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    let watched = OwnPr { state: LinkState::Merged, ..own(3, PR_OPEN, Some("feat: x")) };
    host.own_pr(1, &watched, NOW, &fx.ctx);
    assert_eq!(state(&host, PR_OPEN), LinkState::Merged);
    fx.until(&mut host, &[1], |host| {
        !state(host, PR_MERGED).is_unknown() && !state(host, ISSUE_CLOSED).is_unknown()
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(fx.calls().iter().all(|call| !call.contains(PR_OPEN)), "{:?}", fx.calls());
    let open_again = OwnPr { state: LinkState::Open, ..watched };
    host.own_pr(1, &open_again, NOW + 1, &fx.ctx);
    assert_eq!(state(&host, PR_OPEN), LinkState::Open, "the watch's newer answer wins");
}
