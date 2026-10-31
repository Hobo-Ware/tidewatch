use std::time::{Duration, Instant};

use super::*;

fn world(text: &str) -> (tempfile::TempDir, TidewatchPaths) {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(paths.house_style_file(), text).unwrap();
    (home, paths)
}

fn until<T>(mut step: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(found) = step() {
            return found;
        }
        assert!(Instant::now() < deadline, "never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_page_reads_the_file_on_a_thread_and_lists_the_dead_lines() {
    let (_home, paths) = world("Be brief.\nCall `stage_done` at the end.\n");
    let ctx = egui::Context::default();
    let mut host = Host::new();
    assert_eq!(host.tick(Some(&paths), true, 0.0, &ctx), None);
    until(|| {
        host.tick(Some(&paths), true, 0.1, &ctx);
        (!host.lines().is_empty()).then_some(())
    });
    assert_eq!(host.lines()[0].number, 2);
}

#[test]
fn nothing_is_read_while_the_page_is_not_showing() {
    let (_home, paths) = world("Call stage_done.\n");
    let ctx = egui::Context::default();
    let mut host = Host::new();
    host.tick(Some(&paths), false, 0.0, &ctx);
    std::thread::sleep(Duration::from_millis(100));
    host.tick(Some(&paths), false, 1.0, &ctx);
    assert_eq!(host.lines(), []);
}

#[test]
fn a_removal_edits_only_that_line_and_the_list_catches_up() {
    let (_home, paths) = world("Be brief.\nCall `stage_done` at the end.\nKeep it plain.\n");
    let ctx = egui::Context::default();
    let mut host = Host::new();
    let mut now = 0.0;
    until(|| {
        host.tick(Some(&paths), true, now, &ctx);
        now += 0.01;
        (!host.lines().is_empty()).then_some(())
    });
    let line = host.lines()[0].clone();
    host.remove(&paths, line.number, line.text, &ctx);
    let outcome = until(|| {
        now += 0.01;
        host.tick(Some(&paths), true, now, &ctx)
    });
    assert_eq!(outcome, Outcome::Removed);
    assert_eq!(
        std::fs::read_to_string(paths.house_style_file()).unwrap(),
        "Be brief.\nKeep it plain.\n"
    );
    until(|| {
        now += 0.01;
        host.tick(Some(&paths), true, now, &ctx);
        host.lines().is_empty().then_some(())
    });
}

#[test]
fn a_file_edited_since_the_read_is_left_alone() {
    let (_home, paths) = world("Call stage_done.\n");
    let ctx = egui::Context::default();
    let mut host = Host::new();
    host.remove(&paths, 1, "Call something else".into(), &ctx);
    let outcome = until(|| host.tick(Some(&paths), false, 0.0, &ctx));
    assert_eq!(outcome, Outcome::FileChanged);
    assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), "Call stage_done.\n");
}

#[test]
fn a_missing_file_has_nothing_to_warn_about() {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let ctx = egui::Context::default();
    let mut host = Host::new();
    host.tick(Some(&paths), true, 0.0, &ctx);
    std::thread::sleep(Duration::from_millis(100));
    host.tick(Some(&paths), true, 0.1, &ctx);
    assert_eq!(host.lines(), []);
}
