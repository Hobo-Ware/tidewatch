use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;

const SVG: &str =
    "<svg viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"20\" fill=\"#2dd4bf\"/></svg>";
const DARK: &str =
    "<svg viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"20\" fill=\"#2a2f3a\"/></svg>";
const SPILLING: &str =
    "<svg viewBox=\"0 0 64 64\"><rect width=\"64\" height=\"64\" fill=\"#2dd4bf\"/></svg>";

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
fn the_fake_switch_hands_out_the_icons_in_turn_and_writes_nothing() {
    let fx = Fx::new();
    let mut host = Host::new(true, true);
    fx.tick(&mut host, &[1, 2, 3, 4, 5, 6, 7, 8]);
    let reasons: Vec<_> = FAKE.iter().map(|(_, reason)| (*reason).to_owned()).collect();
    for id in [1, 2, 4, 5, 6, 7] {
        assert_eq!(host.get(id).unwrap().reason, reasons[(id - 1) as usize], "tab {id}");
    }
    assert!(host.get(3).is_none(), "the dark sample shows the letter");
    assert_eq!(host.get(8).unwrap().reason, reasons[0]);
    let mut unique = reasons.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), FAKE.len());
    assert_eq!(host.take_unreadable(), Vec::<u64>::new());
    host.clear(2, &fx.notes(), &fx.ctx);
    std::thread::sleep(Duration::from_millis(30));
    assert!(!fx.file().exists());
}

#[test]
fn every_fake_icon_but_the_dark_sample_passes_the_readability_check() {
    for (index, (svg, reason)) in FAKE.iter().enumerate() {
        let avatar = clean(svg, reason).unwrap_or_else(|e| panic!("{reason}: {e}"));
        assert_eq!(avatar.svg.matches("script").count(), 0);
        let readable = avatar_check::check(&avatar.svg);
        assert_eq!(readable.is_ok(), index != 2, "{reason}: {readable:?}");
    }
}

#[test]
fn a_saved_avatar_that_cannot_be_read_shows_the_letter_is_removed_and_is_queued_once() {
    for (what, svg) in [("dark on the plate", DARK), ("past the circle", SPILLING)] {
        let fx = Fx::new();
        avatar_store::save(&fx.file(), &clean(svg, "old").unwrap()).unwrap();
        let mut host = Host::new(true, false);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut queued = Vec::new();
        while queued.is_empty() && Instant::now() < deadline {
            fx.tick(&mut host, &[1]);
            queued = host.take_unreadable();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(queued, [1], "{what}");
        assert!(host.get(1).is_none(), "{what}: the letter shows");
        assert!(!fx.file().exists(), "{what}: the old file is gone");
        std::thread::sleep(Duration::from_millis(50));
        fx.tick(&mut host, &[1]);
        assert_eq!(host.take_unreadable(), Vec::<u64>::new(), "{what}: queued once");
    }
}

#[test]
fn a_readable_saved_avatar_queues_nothing_and_a_host_that_writes_nothing_keeps_the_file() {
    let fx = Fx::new();
    avatar_store::save(&fx.file(), &clean(SVG, "good").unwrap()).unwrap();
    let mut host = Host::new(true, false);
    fx.until(&mut host, 1, |host| host.get(1).is_some());
    assert_eq!(host.take_unreadable(), Vec::<u64>::new());

    let fx = Fx::new();
    avatar_store::save(&fx.file(), &clean(DARK, "old").unwrap()).unwrap();
    let mut host = Host::new(false, false);
    for _ in 0..40 {
        fx.tick(&mut host, &[1]);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(host.get(1).is_none());
    assert_eq!(host.take_unreadable(), Vec::<u64>::new());
    assert!(fx.file().exists());
}

#[test]
fn the_meeseeks_face_is_the_art_file_as_it_is_and_rasterizes_at_every_tab_size() {
    let file =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/meeseeks.svg"))
            .unwrap();
    let face = meeseeks_avatar();
    assert_eq!(face.svg, file);
    assert!(face.svg.contains("viewBox=\"0 0 64 64\""));
    let ctx = egui::Context::default();
    for px in [20, 24, 40, 52, 72] {
        assert!(crate::agent_avatar::texture(&ctx, &face, px).is_some(), "{px} px");
    }
}

#[test]
fn a_meeseeks_tab_wears_the_fixed_face_whatever_the_agent_draws_or_the_file_holds() {
    let fx = Fx::new();
    avatar_store::save(&fx.file(), &clean(SVG, "its own").unwrap()).unwrap();
    let mut host = Host::new(true, false);
    host.fixed_faces(&[1], &fx.ctx);
    fx.tick(&mut host, &[1]);
    assert_eq!(host.get(1).unwrap().reason, "Caaan do! Every Meeseeks wears this face.");
    host.arrived(1, clean(SVG, "set_avatar").unwrap());
    std::thread::sleep(Duration::from_millis(50));
    fx.tick(&mut host, &[1]);
    host.fixed_faces(&[1], &fx.ctx);
    assert_eq!(host.get(1).unwrap().reason, "Caaan do! Every Meeseeks wears this face.");
    host.clear(1, &fx.notes(), &fx.ctx);
    assert!(host.get(1).is_some(), "Use letter does nothing to a fixed face");
    assert!(fx.file().exists());
    assert!(crate::agent_avatar::is_locked(&fx.ctx, 1));
}

#[test]
fn a_tab_that_takes_over_gets_its_own_avatar_back_and_the_menu_with_it() {
    let fx = Fx::new();
    avatar_store::save(&fx.file(), &clean(SVG, "its own").unwrap()).unwrap();
    let mut host = Host::new(true, false);
    host.fixed_faces(&[1], &fx.ctx);
    fx.tick(&mut host, &[1]);
    host.fixed_faces(&[], &fx.ctx);
    assert!(host.get(1).is_none());
    assert!(!crate::agent_avatar::is_locked(&fx.ctx, 1));
    fx.until(&mut host, 1, |host| host.get(1).is_some());
    assert_eq!(host.get(1).unwrap().reason, "its own");
}

#[test]
fn the_fake_switch_value_picks_the_mockups_or_the_bundled_icons() {
    let cases = [
        (None, Fake::Off),
        (Some(""), Fake::Off),
        (Some("0"), Fake::Off),
        (Some("1"), Fake::Mockups),
        (Some("icons"), Fake::Icons),
    ];
    for (value, want) in cases {
        assert_eq!(fake_mode_of(value), want, "{value:?}");
    }
}

#[test]
fn every_fake_bundled_icon_is_drawn_in_its_own_color_and_reads_on_the_plate() {
    let reasons: Vec<_> = FAKE_ICONS.iter().map(|(_, _, reason)| *reason).collect();
    let mut unique = reasons.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), FAKE_ICONS.len());
    for (index, (icon, color, reason)) in FAKE_ICONS.iter().enumerate() {
        let face = fake_icon_face(index).unwrap_or_else(|| panic!("{icon} in {color}"));
        assert_eq!(face.reason, *reason);
        assert!(face.svg.contains(&avatar_icons::color_hex(color).unwrap()), "{icon}");
    }
    assert_eq!(
        fake_icon_face(FAKE_ICONS.len()).map(|f| f.reason.clone()),
        Some(FAKE_ICONS[0].2.to_owned())
    );
}
