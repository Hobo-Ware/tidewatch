use std::collections::HashSet;
use std::time::{Duration, Instant};

use tidewatch_core::github_hook::Hit;
use tidewatch_core::link_board::{Access, Checks, LinkState, Status, TitleFrom};
use tidewatch_core::tools::Tools;

use super::*;

const NOW: i64 = 1_000_000_000;
const MIN: i64 = 60_000;

fn no_gh() -> Tools {
    Tools::with_path("/nonexistent")
}

fn own<'a>(number: u64, url: &'a str, title: Option<&'a str>) -> OwnPr<'a> {
    OwnPr { number, url, title, state: LinkState::Unknown, status: Status::default() }
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
        status: Status::default(),
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
        assert_eq!(host.board(id).unwrap().len(), 10);
    }
    host.found(1, vec![link("https://a.dev/9", "More", TitleFrom::Seen)], &fx.ctx);
    host.arrived(1, &[link("https://a.dev/8", "More", TitleFrom::Agent)]);
    host.own_pr(1, &own(3, "https://github.com/o/r/pull/3", Some("t")), NOW, &fx.ctx);
    assert_eq!(host.board(1).unwrap().len(), 10, "a capture is not changed by what arrives");
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
const ISSUE_OPEN: &str = "https://github.com/o/r/issues/3";
const WEB: &str = "https://a.dev/5";

#[cfg(unix)]
const ANSWERS: &str = r#"d="$(dirname "$0")"
echo "$@" >> "$d/gh.log"
if [ -f "$d/stderr.txt" ]; then cat "$d/stderr.txt" >&2; exit 1; fi
case "$3" in
  */pull/1) echo '{"number":1,"state":"MERGED"}' ;;
  */issues/2) echo '{"state":"CLOSED","stateReason":"NOT_PLANNED"}' ;;
  */issues/3) echo '{"state":"OPEN","comments":[{},{}]}' ;;
  *) if [ -f "$d/open.json" ]; then cat "$d/open.json"; else echo '{"number":3,"state":"OPEN"}'; fi ;;
esac"#;

fn link_at<'a>(host: &'a Host, tab: u64, url: &str) -> &'a Link {
    host.board(tab).unwrap().links.iter().find(|link| link.url == url).unwrap()
}

fn state(host: &Host, url: &str) -> LinkState {
    link_at(host, 1, url).state
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
fn calls_of(fx: &Fx) -> Vec<String> {
    let mut calls: Vec<String> = fx
        .calls()
        .iter()
        .filter(|call| !call.is_empty())
        .map(|call| call.split(" --json").next().unwrap_or_default().to_owned())
        .collect();
    calls.sort();
    calls
}

#[cfg(unix)]
fn wait_for_calls(fx: &Fx, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fx.calls().len() < count && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(100));
}

fn news(numbers: &[u64], issues: &[u64]) -> Hit {
    Hit {
        event: "check_run".into(),
        numbers: numbers.to_vec(),
        issues: issues.to_vec(),
        ..Hit::default()
    }
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
    assert_eq!(
        calls_of(&fx),
        [
            format!("issue view {ISSUE_CLOSED}"),
            format!("pr view {PR_MERGED}"),
            format!("pr view {PR_OPEN}"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn what_a_lookup_finds_besides_the_state_is_kept_on_the_link_and_saved() {
    let fx = Fx::new();
    let failing = r#"{"number":3,"state":"OPEN","isDraft":true,"reviewDecision":"CHANGES_REQUESTED","updatedAt":"2026-10-30T10:00:00Z","statusCheckRollup":[{"__typename":"CheckRun","name":"lint","workflowName":"ci","status":"COMPLETED","conclusion":"FAILURE"}]}"#;
    std::fs::write(fx.dir.path().join("open.json"), failing).unwrap();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    until_looked_up(&fx, &mut host);
    let found = link_at(&host, 1, PR_OPEN);
    assert_eq!(found.status.checks, Some(Checks::Fail));
    assert_eq!(found.status.review, Some(tidewatch_core::link_board::Review::ChangesRequested));
    assert_eq!((found.status.draft, found.status.checked_ms), (true, Some(NOW)));
    assert_eq!(
        link_at(&host, 1, ISSUE_CLOSED).status.reason,
        Some(tidewatch_core::link_board::Reason::NotPlanned)
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while link_store::load(&fx.file())
        .is_none_or(|disk| disk.links.iter().all(|link| link.status.checks.is_none()))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    let disk = link_store::load(&fx.file()).unwrap();
    let saved = disk.links.iter().find(|link| link.url == PR_OPEN).unwrap();
    assert_eq!(saved.status, found.status);
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
    wait_for_calls(&fx, 4);
    assert_eq!(
        fx.calls().last().unwrap().split(" --json").next().unwrap(),
        format!("pr view {PR_OPEN}")
    );
    assert_eq!(fx.calls().len(), 4, "the merged and the closed one stay settled");
}

#[cfg(unix)]
#[test]
fn a_link_to_a_repo_nobody_delivers_for_is_polled_every_5_minutes_while_its_tab_is_in_front() {
    let fx = Fx::new();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    until_looked_up(&fx, &mut host);
    host.set_world(Some(1), &HashSet::new());
    fx.tick_at(&mut host, &[1], NOW + 4 * MIN);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fx.calls().len(), 3);
    fx.tick_at(&mut host, &[1], NOW + 5 * MIN);
    wait_for_calls(&fx, 4);
    assert_eq!(fx.calls().len(), 4);
}

#[cfg(unix)]
#[test]
fn a_link_to_a_repo_a_forwarder_delivers_for_waits_30_minutes_even_in_front() {
    let fx = Fx::new();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    until_looked_up(&fx, &mut host);
    host.set_world(Some(1), &HashSet::from(["o/r".to_owned()]));
    fx.tick_at(&mut host, &[1], NOW + 29 * MIN);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fx.calls().len(), 3);
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
    assert_eq!(link_at(&host, 1, PR_OPEN).status, Status::default());
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

#[cfg(unix)]
#[test]
fn the_watchs_status_of_the_tabs_own_pull_request_shows_on_its_link() {
    let fx = Fx::new();
    let mut host = Host::new(false, false, no_gh());
    fx.tick(&mut host, &[1]);
    let status = Status { checks: Some(Checks::Fail), ..Status::default() };
    let watched = OwnPr { state: LinkState::Open, status: status.clone(), ..own(3, PR_OPEN, None) };
    host.own_pr(1, &watched, NOW, &fx.ctx);
    assert_eq!(link_at(&host, 1, PR_OPEN).status, Status { checked_ms: Some(NOW), ..status });
    host.own_pr(1, &watched, NOW + 9, &fx.ctx);
    assert_eq!(link_at(&host, 1, PR_OPEN).status.checked_ms, Some(NOW), "a repeat changes nothing");
}

#[cfg(unix)]
#[test]
fn a_link_held_by_two_tabs_is_looked_up_once_and_both_tabs_get_the_answer() {
    let fx = Fx::new();
    let mut host = Host::new(true, false, fx.fake_gh(ANSWERS));
    fx.until(&mut host, &[1, 2], |host| {
        host.tabs.len() == 2 && host.tabs.values().all(|tab| tab.loaded)
    });
    host.arrived(1, &[gh_link(PR_OPEN, LinkState::Unknown)]);
    host.arrived(2, &[gh_link(PR_OPEN, LinkState::Unknown)]);
    fx.until(&mut host, &[1, 2], |host| {
        [1, 2].iter().all(|tab| link_at(host, *tab, PR_OPEN).state == LinkState::Open)
    });
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(calls_of(&fx), [format!("pr view {PR_OPEN}")]);
}

#[cfg(unix)]
#[test]
fn news_names_a_pull_request_or_issue_and_every_link_to_it_in_every_tab_is_looked_up_again() {
    let fx = Fx::new();
    let mut host = Host::new(true, false, fx.fake_gh(ANSWERS));
    fx.until(&mut host, &[1, 2], |host| {
        host.tabs.len() == 2 && host.tabs.values().all(|tab| tab.loaded)
    });
    for tab in [1, 2] {
        host.arrived(
            tab,
            &[gh_link(PR_OPEN, LinkState::Unknown), gh_link(ISSUE_OPEN, LinkState::Unknown)],
        );
    }
    fx.until(&mut host, &[1, 2], |host| {
        [1, 2].iter().all(|tab| link_at(host, *tab, ISSUE_OPEN).state == LinkState::Open)
    });
    wait_for_calls(&fx, 2);
    assert_eq!(fx.calls().len(), 2);
    host.news("o/r", &news(&[3], &[]), &fx.ctx);
    fx.tick_at(&mut host, &[1, 2], NOW + 3_000);
    wait_for_calls(&fx, 3);
    assert_eq!(
        calls_of(&fx).last().unwrap(),
        &format!("pr view {PR_OPEN}"),
        "the pull request only"
    );
    assert_eq!(fx.calls().len(), 3, "one lookup, not one per tab");
    host.news("o/r", &news(&[], &[3]), &fx.ctx);
    fx.tick_at(&mut host, &[1, 2], NOW + 6_000);
    wait_for_calls(&fx, 4);
    assert_eq!(fx.calls().len(), 4);
    assert!(fx.calls().last().unwrap().starts_with("issue view"), "{:?}", fx.calls());
}

#[cfg(unix)]
#[test]
fn news_for_another_repo_or_another_number_looks_nothing_up() {
    let fx = Fx::new();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    until_looked_up(&fx, &mut host);
    wait_for_calls(&fx, 3);
    for (slug, hit) in
        [("o/other", news(&[3], &[])), ("o/r", news(&[9], &[9])), ("o/r", news(&[], &[1]))]
    {
        host.news(slug, &hit, &fx.ctx);
    }
    fx.tick_at(&mut host, &[1], NOW + 10_000);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(fx.calls().len(), 3);
    host.news("O/R", &news(&[3], &[]), &fx.ctx);
    fx.tick_at(&mut host, &[1], NOW + 10_000);
    wait_for_calls(&fx, 4);
    assert_eq!(fx.calls().len(), 4, "the repo is compared ignoring case");
}

#[cfg(unix)]
#[test]
fn a_burst_of_news_costs_one_lookup_every_two_seconds() {
    let fx = Fx::new();
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    until_looked_up(&fx, &mut host);
    wait_for_calls(&fx, 3);
    for _ in 0..20 {
        host.news("o/r", &news(&[3], &[]), &fx.ctx);
    }
    fx.tick_at(&mut host, &[1], NOW + 1_999);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fx.calls().len(), 3, "2 seconds have not passed since the last look");
    fx.tick_at(&mut host, &[1], NOW + 2_000);
    wait_for_calls(&fx, 4);
    assert_eq!(fx.calls().len(), 4);
    for _ in 0..20 {
        host.news("o/r", &news(&[3], &[]), &fx.ctx);
    }
    fx.tick_at(&mut host, &[1], NOW + 3_000);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fx.calls().len(), 4);
    fx.tick_at(&mut host, &[1], NOW + 4_000);
    wait_for_calls(&fx, 5);
    assert_eq!(fx.calls().len(), 5);
}

#[cfg(unix)]
#[test]
fn a_repo_gh_cannot_read_and_a_pull_request_that_is_gone_stop_the_looking() {
    let cases = [
        (
            "GraphQL: Could not resolve to a Repository with the name 'o/r'. (repository)",
            Access::NoAccess,
        ),
        (
            "GraphQL: Could not resolve to a PullRequest with the number of 3. (repository.pullRequest)",
            Access::Gone,
        ),
    ];
    for (message, want) in cases {
        let fx = Fx::new();
        std::fs::write(fx.dir.path().join("stderr.txt"), message).unwrap();
        let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
        fx.until(&mut host, &[1], |host| link_at(host, 1, PR_OPEN).status.access == want);
        wait_for_calls(&fx, 3);
        let calls = fx.calls().len();
        assert_eq!(link_at(&host, 1, PR_OPEN).status.checked_ms, Some(NOW));
        fx.tick_at(&mut host, &[1], NOW + 600 * MIN);
        host.news("o/r", &news(&[3], &[3]), &fx.ctx);
        fx.tick_at(&mut host, &[1], NOW + 601 * MIN);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(fx.calls().len(), calls, "{want:?}: nothing is looked up again");
    }
}

#[cfg(unix)]
#[test]
fn a_link_that_begins_to_need_action_counts_once_and_the_same_answer_again_counts_none() {
    let fx = Fx::new();
    let failing = r#"{"number":3,"state":"OPEN","statusCheckRollup":[{"__typename":"CheckRun","name":"lint","workflowName":"ci","status":"COMPLETED","conclusion":"FAILURE"}]}"#;
    let mut host = loaded_host(&fx, fx.fake_gh(ANSWERS), true);
    until_looked_up(&fx, &mut host);
    assert_eq!(host.alerts(1), None, "an open pull request with nothing wrong is no alert");
    std::fs::write(fx.dir.path().join("open.json"), failing).unwrap();
    host.news("o/r", &news(&[3], &[]), &fx.ctx);
    fx.tick_at(&mut host, &[1], NOW + 3_000);
    fx.until(&mut host, &[1], |host| host.alerts(1) == Some(1));
    host.news("o/r", &news(&[3], &[]), &fx.ctx);
    fx.tick_at(&mut host, &[1], NOW + 6_000);
    wait_for_calls(&fx, 5);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(host.alerts(1), Some(1), "the same failure again is not new");
    let changes = r#"{"number":3,"state":"OPEN","reviewDecision":"CHANGES_REQUESTED"}"#;
    std::fs::write(fx.dir.path().join("open.json"), changes).unwrap();
    host.news("o/r", &news(&[3], &[]), &fx.ctx);
    fx.tick_at(&mut host, &[1], NOW + 9_000);
    fx.until(&mut host, &[1], |host| host.alerts(1) == Some(2));
}

#[test]
fn the_fake_switch_marks_the_mockups_repo_live_and_a_real_host_only_what_it_is_told() {
    let fake = Host::new(false, true, no_gh());
    assert!(fake.live().contains(crate::link_model::FAKE_LIVE_REPO));
    let mut real = Host::new(false, false, no_gh());
    assert_eq!(real.live().len(), 0);
    real.set_world(Some(1), &HashSet::from(["o/r".to_owned()]));
    assert!(real.live().contains("o/r"));
}

#[test]
fn only_the_links_that_were_new_to_a_board_are_told() {
    let fx = Fx::new();
    let mut host = Host::new(false, false, no_gh());
    fx.tick(&mut host, &[1]);
    let one = link("https://a.dev/1", "One", TitleFrom::Agent);
    host.arrived(1, std::slice::from_ref(&one));
    host.found(1, vec![one.clone()], &fx.ctx);
    host.found(1, vec![link("https://a.dev/2", "Two", TitleFrom::Seen)], &fx.ctx);
    host.arrived(7, &[link("https://a.dev/3", "Closed tab", TitleFrom::Agent)]);
    let told: Vec<(u64, String)> =
        host.take_told().into_iter().map(|(tab, link)| (tab, link.url)).collect();
    assert_eq!(told, [(1, "https://a.dev/1".to_owned()), (1, "https://a.dev/2".to_owned())]);
    assert_eq!(host.take_told().len(), 0);
}
