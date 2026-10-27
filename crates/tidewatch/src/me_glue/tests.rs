use std::path::PathBuf;
use std::time::{Duration, Instant};

use image::{ImageFormat, Rgba, RgbaImage};

use super::*;

fn png() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([9, 99, 199, 255])))
        .to_rgb8()
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn no_tools() -> (tempfile::TempDir, Tools) {
    let dir = tempfile::tempdir().unwrap();
    let tools = Tools::with_path(dir.path());
    (dir, tools)
}

#[test]
fn the_source_follows_the_switch_and_the_run_kind() {
    let cases = [
        (None, false, Source::Live),
        (None, true, Source::Local),
        (Some(""), true, Source::Local),
        (Some("/tmp/me.jpg"), false, Source::Fake(Fake::Image(PathBuf::from("/tmp/me.jpg")))),
        (Some("/tmp/me.jpg"), true, Source::Fake(Fake::Image(PathBuf::from("/tmp/me.jpg")))),
        (Some("letter:kai"), true, Source::Fake(Fake::Letter("K".to_owned()))),
        (Some("letter:"), false, Source::Fake(Fake::Letter("Y".to_owned()))),
    ];
    for (fake, background, want) in cases {
        assert_eq!(source(fake, background), want, "fake {fake:?} background {background}");
    }
}

#[test]
fn with_nothing_saved_the_face_is_a_letter_and_a_refresh_is_due() {
    let home = tempfile::tempdir().unwrap();
    let (_bare, tools) = no_tools();
    let paths = TidewatchPaths::new(home.path());
    let (me, refresh) = load(Some(&paths), &tools);
    assert!(matches!(me, Me::Letter(letter) if letter == "Y"));
    assert!(refresh);
}

#[test]
fn a_saved_picture_is_read_and_is_due_again_only_after_a_day() {
    let home = tempfile::tempdir().unwrap();
    let (_bare, tools) = no_tools();
    let paths = TidewatchPaths::new(home.path());
    std::fs::create_dir_all(paths.me_dir()).unwrap();
    std::fs::write(paths.me_avatar_file(), png()).unwrap();
    let now = now_secs();
    let cases = [(now, false), (now - 3600, false), (now - 2 * 86_400, true)];
    for (stamp, want) in cases {
        std::fs::write(paths.me_avatar_stamp_file(), stamp.to_string()).unwrap();
        let (me, refresh) = load(Some(&paths), &tools);
        assert!(matches!(me, Me::Picture(_)), "stamp {stamp}");
        assert_eq!(refresh, want, "stamp {stamp}");
    }
}

#[test]
fn a_saved_file_that_is_not_a_picture_falls_back_to_the_letter_and_asks_again() {
    let home = tempfile::tempdir().unwrap();
    let (_bare, tools) = no_tools();
    let paths = TidewatchPaths::new(home.path());
    std::fs::create_dir_all(paths.me_dir()).unwrap();
    std::fs::write(paths.me_avatar_file(), "<html>").unwrap();
    std::fs::write(paths.me_avatar_stamp_file(), now_secs().to_string()).unwrap();
    let (me, refresh) = load(Some(&paths), &tools);
    assert!(matches!(me, Me::Letter(_)));
    assert!(refresh);
}

#[test]
fn a_fake_image_that_cannot_be_read_shows_the_letter() {
    let me = load_fake(&Fake::Image(PathBuf::from("/nowhere/me.jpg")));
    assert!(matches!(me, Me::Letter(letter) if letter == "Y"));
}

#[test]
fn a_background_run_never_asks_gh_and_a_fake_one_never_refreshes() {
    let home = tempfile::tempdir().unwrap();
    let (_bare, tools) = no_tools();
    let paths = TidewatchPaths::new(home.path());
    for source in [Source::Local, Source::Fake(Fake::Letter("K".to_owned()))] {
        let ctx = egui::Context::default();
        let host = Host::begin(&ctx, Some(&paths), tools.clone(), source);
        let deadline = Instant::now() + Duration::from_secs(5);
        while me_face::current(&ctx).is_none() && Instant::now() < deadline {
            host.tick(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(me_face::current(&ctx).is_some());
        std::thread::sleep(Duration::from_millis(50));
        assert!(host.jobs.drain().is_empty(), "no second job was started");
        assert!(!paths.me_dir().exists(), "nothing was fetched or written");
    }
}
