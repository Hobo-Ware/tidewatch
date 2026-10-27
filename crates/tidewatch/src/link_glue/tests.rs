use std::time::{Duration, Instant};

use tidewatch_core::link_board::TitleFrom;

use super::*;

const NOW: i64 = 1_000_000_000;

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
        let notes = self.notes();
        let facts: Vec<Facts<'_>> =
            ids.iter().map(|id| Facts { id: *id, notes_file: &notes }).collect();
        host.tick(&facts, &self.ctx, NOW);
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
    }
}

#[test]
fn a_saved_board_is_read_when_the_tab_appears() {
    let fx = Fx::new();
    let mut saved = LinkBoard::default();
    saved.absorb(&link("https://a.dev/1", "One", TitleFrom::Agent));
    link_store::save(&fx.file(), &saved).unwrap();
    let mut host = Host::new(true, false);
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
    let mut host = Host::new(true, false);
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
    let mut host = Host::new(false, false);
    fx.tick(&mut host, &[1]);
    host.arrived(1, &[link("https://a.dev/1", "Reported", TitleFrom::Agent)]);
    host.found(1, vec![link("https://a.dev/1", "Artifact", TitleFrom::Guess)], &fx.ctx);
    assert_eq!(host.board(1).unwrap().links[0].title, "Reported");
}

#[test]
fn nothing_is_written_when_the_host_may_not_write() {
    let fx = Fx::new();
    let mut host = Host::new(false, false);
    fx.tick(&mut host, &[1]);
    host.found(1, vec![link("https://a.dev/1", "One", TitleFrom::Seen)], &fx.ctx);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(host.board(1).unwrap().len(), 1);
    assert!(!fx.file().exists());
}

#[test]
fn the_fake_switch_gives_every_tab_the_mockup_links_and_writes_nothing() {
    let fx = Fx::new();
    let mut host = Host::new(true, true);
    fx.tick(&mut host, &[1, 2]);
    for id in [1, 2] {
        assert_eq!(host.board(id).unwrap().len(), 5);
    }
    host.found(1, vec![link("https://a.dev/9", "More", TitleFrom::Seen)], &fx.ctx);
    host.arrived(1, &[link("https://a.dev/8", "More", TitleFrom::Agent)]);
    host.own_pr(1, (3, "https://github.com/o/r/pull/3", Some("t")), NOW, &fx.ctx);
    assert_eq!(host.board(1).unwrap().len(), 5, "a capture is not changed by what arrives");
    std::thread::sleep(Duration::from_millis(100));
    assert!(!fx.file().exists());
}

#[test]
fn a_closed_tab_is_forgotten_and_a_link_for_a_tab_that_is_not_open_is_dropped() {
    let fx = Fx::new();
    let mut host = Host::new(false, false);
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
    let mut host = Host::new(false, false);
    fx.tick(&mut host, &[1]);
    let url = "https://github.com/o/r/pull/42";
    host.own_pr(1, (42, url, None), NOW, &fx.ctx);
    assert_eq!(host.board(1).unwrap().links[0].title, "PR #42");
    host.own_pr(1, (42, url, None), NOW + 5, &fx.ctx);
    host.own_pr(1, (42, url, Some("feat: dashboard")), NOW + 9, &fx.ctx);
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
    let mut host = Host::new(false, false);
    fx.tick(&mut host, &[1]);
    host.found(1, vec![link("https://a.dev/1", "New title", TitleFrom::Seen)], &fx.ctx);
    fx.until(&mut host, &[1], |host| {
        host.tabs.get(&1).is_some_and(|tab| tab.board.len() == 1 && tab.loaded)
    });
    assert_eq!(host.board(1).unwrap().links[0].title, "New title");
}
