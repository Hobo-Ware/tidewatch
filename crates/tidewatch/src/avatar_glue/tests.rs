use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;

const SVG: &str = "<svg><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>";

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
        avatar_file_beside(&self.notes())
    }

    fn tick(&self, host: &mut Host, ids: &[u64]) {
        let notes = self.notes();
        let facts: Vec<Facts<'_>> =
            ids.iter().map(|id| Facts { id: *id, notes_file: &notes }).collect();
        host.tick(&facts, &self.ctx);
    }

    fn until(&self, host: &mut Host, id: u64, ready: impl Fn(&Host) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready(host) && Instant::now() < deadline {
            self.tick(host, &[id]);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(ready(host));
    }
}

#[test]
fn a_tab_shows_its_saved_avatar_once_the_file_is_read() {
    let fx = Fx::new();
    avatar_store::save(&fx.file(), &clean(SVG, "a dot").unwrap()).unwrap();
    let mut host = Host::new(true, false);
    fx.tick(&mut host, &[1]);
    assert!(host.get(1).is_none());
    fx.until(&mut host, 1, |host| host.get(1).is_some());
    assert_eq!(host.get(1).unwrap().reason, "a dot");
}

#[test]
fn a_tab_with_no_file_has_no_avatar_and_keeps_the_letter() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.tick(&mut host, &[1]);
    std::thread::sleep(Duration::from_millis(50));
    fx.tick(&mut host, &[1]);
    assert!(host.get(1).is_none());
}

#[test]
fn an_avatar_from_the_agent_replaces_what_was_there_and_a_late_read_does_not_win() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    avatar_store::save(&fx.file(), &clean(SVG, "old").unwrap()).unwrap();
    fx.tick(&mut host, &[1]);
    host.arrived(1, clean(SVG, "new").unwrap());
    std::thread::sleep(Duration::from_millis(50));
    fx.tick(&mut host, &[1]);
    assert_eq!(host.get(1).unwrap().reason, "new");
}

#[test]
fn an_avatar_for_a_tab_that_is_not_open_is_dropped() {
    let mut host = Host::new(true, false);
    host.arrived(7, clean(SVG, "x").unwrap());
    assert!(host.get(7).is_none());
}

#[test]
fn use_letter_clears_the_tab_and_the_file() {
    let fx = Fx::new();
    avatar_store::save(&fx.file(), &clean(SVG, "a dot").unwrap()).unwrap();
    let mut host = Host::new(true, false);
    fx.until(&mut host, 1, |host| host.get(1).is_some());
    host.clear(1, &fx.notes(), &fx.ctx);
    assert!(host.get(1).is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    while fx.file().exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!fx.file().exists());
}

#[test]
fn use_letter_leaves_the_file_when_the_host_may_not_write() {
    let fx = Fx::new();
    avatar_store::save(&fx.file(), &clean(SVG, "a dot").unwrap()).unwrap();
    let mut host = Host::new(false, false);
    fx.until(&mut host, 1, |host| host.get(1).is_some());
    host.clear(1, &fx.notes(), &fx.ctx);
    std::thread::sleep(Duration::from_millis(50));
    assert!(fx.file().exists());
}

#[test]
fn a_closed_tab_takes_its_avatar_with_it() {
    let fx = Fx::new();
    let mut host = Host::new(true, true);
    fx.tick(&mut host, &[1]);
    assert!(host.get(1).is_some());
    fx.tick(&mut host, &[]);
    assert!(host.get(1).is_none());
}

#[test]
fn the_fake_switch_hands_out_the_six_icons_in_turn_and_writes_nothing() {
    let fx = Fx::new();
    let mut host = Host::new(true, true);
    fx.tick(&mut host, &[1, 2, 3, 4, 5, 6, 7]);
    let reasons: Vec<_> = (1..=7).map(|id| host.get(id).unwrap().reason.clone()).collect();
    assert_eq!(reasons[0], reasons[6]);
    let mut unique = reasons[..6].to_vec();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 6);
    host.clear(2, &fx.notes(), &fx.ctx);
    std::thread::sleep(Duration::from_millis(30));
    assert!(!fx.file().exists());
}

#[test]
fn every_fake_icon_passes_the_sanitizer_untouched_in_its_shapes() {
    for (svg, reason) in FAKE {
        let avatar = clean(svg, reason).unwrap_or_else(|e| panic!("{reason}: {e}"));
        assert!(avatar.svg.contains("<rect width=\"64\" height=\"64\" rx=\"16\""), "{reason}");
        assert_eq!(avatar.svg.matches("script").count(), 0);
    }
}
