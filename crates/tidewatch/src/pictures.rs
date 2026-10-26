//! Pictures in a message: the chip a pasted screenshot becomes above the text of the message box and
//! inside a sent bubble, and the text rules behind it. The message the agent gets is the text with
//! the pictures' paths on lines of their own after it, so a bubble can tell them apart again.
//! The thumbnails are decoded once on a thread and kept in egui's memory by path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use egui::epaint::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, ColorImage, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Ui, vec2,
};
use image::imageops::FilterType;
use tidewatch_core::image_paste::{is_image_file, path_text};

use crate::brand;

pub(crate) const CHIP_H: f32 = 48.0;
pub(crate) const CHIP_GAP: f32 = 8.0;
pub(crate) const REMOVABLE_W: f32 = 240.0;
pub(crate) const PLAIN_W: f32 = 206.0;
const THUMB_W: f32 = 52.0;
const THUMB_H: f32 = 36.0;
const EDGE: f32 = 6.0;
const TEXT_GAP: f32 = 10.0;
const TEXT_W: f32 = 128.0;
const BUTTON: f32 = 26.0;
const BUTTON_EDGE: f32 = 8.0;
const SMALL_PX: u32 = 104;
const SMALL_PY: u32 = 72;
const LARGE_PX: u32 = 360;

/// What the agent is told a screenshot is called: `Screenshot 2` for a pasted one, else the file name.
pub(crate) fn label(path: &Path, n: usize) -> String {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    let png = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
    if name.starts_with("shot-") && png { format!("Screenshot {n}") } else { name.to_owned() }
}

pub(crate) fn size_text(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{} KB", (bytes + 500) / 1_000),
        _ => format!("{:.1} MB", bytes as f64 / 1_000_000.0),
    }
}

/// The small line under the name: `2000 x 513 · 214 KB`, only the size when the picture's own size
/// is not known.
pub(crate) fn meta_text(size: Option<(u32, u32)>, bytes: u64) -> String {
    match size {
        Some((w, h)) => format!("{w} \u{d7} {h} \u{b7} {}", size_text(bytes)),
        None => size_text(bytes),
    }
}

/// How many rows `count` chips take in a row `width` wide.
pub(crate) fn chip_rows(count: usize, width: f32, chip_w: f32) -> usize {
    if count == 0 {
        return 0;
    }
    let per_row = (((width + CHIP_GAP) / (chip_w + CHIP_GAP)).floor() as usize).max(1);
    count.div_ceil(per_row)
}

/// The height of `rows` rows of chips.
pub(crate) fn chips_height(rows: usize) -> f32 {
    if rows == 0 { 0.0 } else { rows as f32 * CHIP_H + (rows - 1) as f32 * CHIP_GAP }
}

/// Where chip `index` sits in a block whose top left is `origin` and whose rows are `width` wide.
pub(crate) fn chip_rect(origin: Pos2, index: usize, width: f32, chip_w: f32) -> Rect {
    let per_row = (((width + CHIP_GAP) / (chip_w + CHIP_GAP)).floor() as usize).max(1);
    let (row, column) = (index / per_row, index % per_row);
    let at = origin + vec2(column as f32 * (chip_w + CHIP_GAP), row as f32 * (CHIP_H + CHIP_GAP));
    Rect::from_min_size(at, vec2(chip_w, CHIP_H))
}

fn is_picture_path(token: &str) -> bool {
    let path = Path::new(token);
    path.is_absolute() && is_image_file(path)
}

fn picture_line(line: &str) -> Option<Vec<PathBuf>> {
    let tokens = shell_words::split(line.trim()).ok()?;
    (!tokens.is_empty() && tokens.iter().all(|token| is_picture_path(token)))
        .then(|| tokens.into_iter().map(PathBuf::from).collect())
}

/// The message to send: `text`, then the paths of `pictures` on lines of their own.
pub(crate) fn message(text: &str, pictures: &[PathBuf]) -> String {
    if pictures.is_empty() {
        return text.to_owned();
    }
    let body = text.trim_end();
    let paths: Vec<String> =
        pictures.iter().map(|path| path_text(std::slice::from_ref(path))).collect();
    if body.is_empty() { paths.join("\n") } else { format!("{body}\n{}", paths.join("\n")) }
}

/// A sent message taken apart again: the text without the closing lines that are only picture
/// paths, and those pictures in order. A path in the middle of a sentence stays text.
pub(crate) fn split_trailing(text: &str) -> (String, Vec<PathBuf>) {
    let lines: Vec<&str> = text.trim_end().split('\n').collect();
    let mut keep = lines.len();
    let mut found: Vec<Vec<PathBuf>> = Vec::new();
    while keep > 0 {
        let Some(paths) = picture_line(lines[keep - 1]) else { break };
        found.push(paths);
        keep -= 1;
    }
    if found.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    found.reverse();
    (lines[..keep].join("\n").trim_end().to_owned(), found.into_iter().flatten().collect())
}

pub(crate) fn attach(pictures: &mut Vec<PathBuf>, paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        if !pictures.contains(&path) {
            pictures.push(path);
        }
    }
}

pub(crate) struct Loaded {
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) bytes: u64,
    small: Option<TextureHandle>,
    large: Option<(TextureHandle, [f32; 2])>,
}

#[derive(Clone)]
pub(crate) enum Look {
    Loading,
    Ready(Arc<Loaded>),
    Missing,
}

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<HashMap<PathBuf, Look>>>);

fn cache(ctx: &egui::Context) -> Cache {
    ctx.data_mut(|data| data.get_temp_mut_or_default::<Cache>(Id::new("picture-cache")).clone())
}

fn texture(ctx: &egui::Context, name: String, image: &image::DynamicImage) -> TextureHandle {
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    ctx.load_texture(
        name,
        ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        TextureOptions::LINEAR,
    )
}

fn load(ctx: &egui::Context, path: &Path) -> Look {
    let Ok(meta) = std::fs::metadata(path) else { return Look::Missing };
    let decoded = image::open(path).ok();
    let name = path.display().to_string();
    let size = decoded.as_ref().map(|image| (image.width(), image.height()));
    let small = decoded.as_ref().map(|image| {
        texture(
            ctx,
            format!("{name}#small"),
            &image.resize_to_fill(SMALL_PX, SMALL_PY, FilterType::Lanczos3),
        )
    });
    let large = decoded.as_ref().map(|image| {
        let fitted = image.resize(LARGE_PX, LARGE_PX, FilterType::Triangle);
        let shown = [fitted.width() as f32, fitted.height() as f32];
        (texture(ctx, format!("{name}#large"), &fitted), shown)
    });
    Look::Ready(Arc::new(Loaded { size, bytes: meta.len(), small, large }))
}

/// The thumbnails of `path`: `Loading` the first time, while a thread decodes it and wakes the window.
pub(crate) fn look(ctx: &egui::Context, path: &Path) -> Look {
    let cache = cache(ctx);
    {
        let mut slots = cache.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(slot) = slots.get(path) {
            return slot.clone();
        }
        slots.insert(path.to_owned(), Look::Loading);
    }
    let (ctx, path) = (ctx.clone(), path.to_owned());
    std::thread::spawn(move || {
        let loaded = load(&ctx, &path);
        cache.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(path, loaded);
        ctx.request_repaint();
    });
    Look::Loading
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) opened: bool,
    pub(crate) removed: bool,
}

fn truncated(ui: &Ui, text: &str, font: FontId, ink: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, ink);
    job.wrap = TextWrapping::truncate_at_width(TEXT_W);
    ui.painter().layout_job(job)
}

fn paint_thumb(ui: &Ui, rect: Rect, small: Option<&TextureHandle>) {
    let painter = ui.painter();
    let radius = CornerRadius::same(6);
    painter.rect_filled(rect, radius, brand::SUNKEN);
    match small {
        Some(texture) => {
            painter.image(
                texture.id(),
                rect.shrink(1.0),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            for (at, width) in [(0.28, 0.6), (0.5, 0.78), (0.72, 0.4)] {
                let bar = Rect::from_center_size(
                    rect.center() + vec2(0.0, (at - 0.5) * rect.height()),
                    vec2(rect.width() * width, 3.0),
                );
                painter.rect_filled(bar, CornerRadius::same(2), brand::FAINT);
            }
        }
    }
    painter.rect_stroke(rect, radius, Stroke::new(1.0, brand::FIELD_LINE), StrokeKind::Inside);
}

fn paint_remove(ui: &Ui, rect: Rect, hovered: bool) {
    let painter = ui.painter();
    let line = if hovered { brand::FAINT } else { brand::FIELD_LINE };
    if hovered {
        painter.rect_filled(rect, CornerRadius::same(7), brand::BUTTON);
    }
    painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, line), StrokeKind::Inside);
    let c = rect.center();
    let ink = Stroke::new(1.5, brand::SOFT);
    painter.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], ink);
    painter.line_segment([c + vec2(3.5, -3.5), c + vec2(-3.5, 3.5)], ink);
}

/// Draws one chip in `rect` (`CHIP_H` tall, `REMOVABLE_W` wide with a remove button, `PLAIN_W`
/// without). A click anywhere but the button is `opened`; the hover shows a bigger preview.
pub(crate) fn paint(ui: &mut Ui, rect: Rect, picture: (&Path, usize), removable: bool) -> Outcome {
    let (path, n) = picture;
    let id = ui.id().with(("picture-chip", path, n));
    let body = ui.interact(rect, id, Sense::click());
    let look = look(ui.ctx(), path);
    let loaded = match &look {
        Look::Ready(loaded) => Some(loaded.clone()),
        Look::Loading | Look::Missing => None,
    };
    let line = if body.hovered() { brand::FIELD_LINE } else { brand::ITEM_LINE };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(10),
        brand::ITEM,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let thumb = Rect::from_min_size(
        rect.left_top() + vec2(EDGE, (CHIP_H - THUMB_H) / 2.0),
        vec2(THUMB_W, THUMB_H),
    );
    paint_thumb(ui, thumb, loaded.as_ref().and_then(|loaded| loaded.small.as_ref()));
    let name = truncated(ui, &label(path, n), FontId::proportional(14.0), brand::TEXT);
    let detail_text = match &look {
        Look::Ready(loaded) => meta_text(loaded.size, loaded.bytes),
        Look::Loading => String::new(),
        Look::Missing => "File not found".to_owned(),
    };
    let detail = truncated(ui, &detail_text, FontId::proportional(12.0), brand::MUTED);
    let block = name.size().y + detail.size().y;
    let x = thumb.right() + TEXT_GAP;
    let top = rect.center().y - block / 2.0;
    let painter = ui.painter();
    painter.galley(Pos2::new(x, top), name.clone(), brand::TEXT);
    painter.galley(Pos2::new(x, top + name.size().y), detail, brand::MUTED);
    let mut outcome = Outcome::default();
    if removable {
        let button = Rect::from_min_size(
            Pos2::new(rect.right() - BUTTON_EDGE - BUTTON, rect.center().y - BUTTON / 2.0),
            vec2(BUTTON, BUTTON),
        );
        let remove = ui
            .interact(button, id.with("remove"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        paint_remove(ui, button, remove.hovered());
        outcome.removed = remove.clicked();
        outcome.opened = body.clicked() && !outcome.removed && !remove.hovered();
    } else {
        outcome.opened = body.clicked();
    }
    let body = body.on_hover_cursor(egui::CursorIcon::PointingHand);
    if let Some(loaded) = loaded
        && let Some((texture, size)) = &loaded.large
    {
        body.on_hover_ui(|ui| {
            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                texture.id(),
                vec2(size[0], size[1]),
            )));
        });
    }
    outcome
}

/// Opens the picture in the system's viewer.
pub(crate) fn open_picture(path: &Path) {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    if let Err(error) = std::process::Command::new(program).arg(path).spawn() {
        eprintln!("tidewatch: cannot open {}: {error}", path.display());
    }
}

/// Draws the chips of `pictures` in the block at `origin`, `width` wide, and opens a clicked one.
pub(crate) fn paint_block(
    ui: &mut Ui,
    origin: Pos2,
    width: f32,
    pictures: &[PathBuf],
    removable: bool,
) -> Option<usize> {
    let chip_w = if removable { REMOVABLE_W } else { PLAIN_W };
    let mut removed = None;
    for (index, path) in pictures.iter().enumerate() {
        let rect = chip_rect(origin, index, width, chip_w);
        let outcome = paint(ui, rect, (path, index + 1), removable);
        if outcome.opened {
            open_picture(path);
        }
        if outcome.removed {
            removed = Some(index);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    #[test]
    fn a_pasted_shot_is_a_numbered_screenshot_and_any_other_file_keeps_its_name() {
        let cases = [
            ("/s/pastes/shot-1793011211356.png", 1, "Screenshot 1"),
            ("/s/pastes/shot-1793011211356-2.png", 12, "Screenshot 12"),
            ("/Users/me/Desktop/IMG_0042.png", 3, "IMG_0042.png"),
            ("/Users/me/Desktop/shot-notes.jpg", 1, "shot-notes.jpg"),
        ];
        for (path, n, want) in cases {
            assert_eq!(label(Path::new(path), n), want, "{path}");
        }
    }

    #[test]
    fn sizes_read_in_bytes_kilobytes_and_megabytes() {
        let cases = [
            (0, "0 B"),
            (999, "999 B"),
            (1_000, "1 KB"),
            (214_400, "214 KB"),
            (999_499, "999 KB"),
            (1_000_000, "1.0 MB"),
            (2_345_678, "2.3 MB"),
        ];
        for (bytes, want) in cases {
            assert_eq!(size_text(bytes), want, "{bytes}");
        }
    }

    #[test]
    fn the_detail_line_is_dimensions_then_size() {
        assert_eq!(meta_text(Some((2000, 513)), 214_000), "2000 \u{d7} 513 \u{b7} 214 KB");
        assert_eq!(meta_text(None, 5_000), "5 KB");
    }

    #[test]
    fn chips_wrap_into_rows_of_the_room_they_have() {
        let cases = [
            (0, 600.0, 0),
            (1, 100.0, 1),
            (2, 500.0, 1),
            (2, 480.0, 2),
            (3, 600.0, 2),
            (5, 1000.0, 2),
        ];
        for (count, width, want) in cases {
            assert_eq!(chip_rows(count, width, REMOVABLE_W), want, "{count} in {width}");
        }
        assert_eq!(chips_height(0), 0.0);
        assert_eq!(chips_height(1), CHIP_H);
        assert_eq!(chips_height(2), 2.0 * CHIP_H + CHIP_GAP);
    }

    #[test]
    fn a_chip_sits_after_the_one_before_it_or_on_the_next_row() {
        let origin = Pos2::new(10.0, 20.0);
        let first = chip_rect(origin, 0, 600.0, REMOVABLE_W);
        let second = chip_rect(origin, 1, 600.0, REMOVABLE_W);
        let third = chip_rect(origin, 2, 600.0, REMOVABLE_W);
        assert_eq!(first.min, origin);
        assert_eq!(second.min, Pos2::new(10.0 + REMOVABLE_W + CHIP_GAP, 20.0));
        assert_eq!(third.min, Pos2::new(10.0, 20.0 + CHIP_H + CHIP_GAP));
    }

    #[test]
    fn the_message_puts_each_path_on_its_own_line_after_the_text() {
        let two = [p("/s/shot-1.png"), p("/s/my shots/shot-2.png")];
        let cases = [
            ("look at these", &two[..], "look at these\n/s/shot-1.png\n'/s/my shots/shot-2.png'"),
            ("look  \n", &two[..1], "look\n/s/shot-1.png"),
            ("", &two[..1], "/s/shot-1.png"),
            ("plain\n", &[][..], "plain\n"),
        ];
        for (text, pictures, want) in cases {
            assert_eq!(message(text, pictures), want, "{text:?}");
        }
    }

    #[test]
    fn the_closing_picture_lines_come_back_out_of_a_message() {
        let cases = [
            (
                "look at these\n/s/shot-1.png\n'/s/my shots/shot-2.png'",
                "look at these",
                vec!["/s/shot-1.png", "/s/my shots/shot-2.png"],
            ),
            ("/s/shot-1.png", "", vec!["/s/shot-1.png"]),
            ("a\n\nb\n/s/shot-1.png\n\n", "a\n\nb", vec!["/s/shot-1.png"]),
            ("two on a line\n/a/1.png /a/2.jpg", "two on a line", vec!["/a/1.png", "/a/2.jpg"]),
            ("see /s/shot-1.png in the middle", "see /s/shot-1.png in the middle", vec![]),
            ("a relative one\nshots/1.png", "a relative one\nshots/1.png", vec![]),
            ("not a picture\n/s/notes.txt", "not a picture\n/s/notes.txt", vec![]),
            (
                "path then words\n/s/shot-1.png\nthanks",
                "path then words\n/s/shot-1.png\nthanks",
                vec![],
            ),
            ("unbalanced\n'/s/shot-1.png", "unbalanced\n'/s/shot-1.png", vec![]),
            ("", "", vec![]),
        ];
        for (text, want_text, want_paths) in cases {
            let (got_text, got_paths) = split_trailing(text);
            let want: Vec<PathBuf> = want_paths.into_iter().map(p).collect();
            assert_eq!((got_text.as_str(), got_paths), (want_text, want), "{text:?}");
        }
    }

    #[test]
    fn what_the_composer_sends_is_what_a_bubble_splits_back() {
        let pictures = [p("/s/shot-1.png"), p("/s/my shots/shot-2.png")];
        let sent = message("fix the header\n\n- one\n- two", &pictures);
        let (text, back) = split_trailing(&sent);
        assert_eq!(text, "fix the header\n\n- one\n- two");
        assert_eq!(back, pictures);
    }

    #[test]
    fn a_picture_is_attached_once() {
        let mut pictures = vec![p("/s/shot-1.png")];
        attach(&mut pictures, [p("/s/shot-1.png"), p("/s/shot-2.png")]);
        assert_eq!(pictures, [p("/s/shot-1.png"), p("/s/shot-2.png")]);
    }

    fn png(dir: &Path, name: &str, size: (u32, u32)) -> PathBuf {
        let path = dir.join(name);
        let image = image::RgbaImage::from_fn(size.0, size.1, |x, y| {
            image::Rgba([(x % 255) as u8, (y % 255) as u8, 90, 255])
        });
        image.save(&path).unwrap();
        path
    }

    fn ready(ctx: &egui::Context, path: &Path) -> Look {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let seen = look(ctx, path);
            if !matches!(seen, Look::Loading) || std::time::Instant::now() > deadline {
                return seen;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_picture_is_decoded_once_and_its_size_and_thumbnails_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = png(dir.path(), "shot-1.png", (800, 513));
        let ctx = egui::Context::default();
        assert!(matches!(look(&ctx, &path), Look::Loading));
        let Look::Ready(loaded) = ready(&ctx, &path) else { panic!("the picture loads") };
        assert_eq!(loaded.size, Some((800, 513)));
        assert_eq!(loaded.bytes, std::fs::metadata(&path).unwrap().len());
        assert!(loaded.small.is_some());
        let (_, shown) = loaded.large.as_ref().unwrap();
        assert_eq!(shown[0], LARGE_PX as f32);
        assert!(shown[1] < shown[0]);
        let Look::Ready(again) = look(&ctx, &path) else { panic!("kept") };
        assert!(Arc::ptr_eq(&loaded, &again), "the second look is the first decode");
    }

    #[test]
    fn a_file_that_is_gone_is_missing_and_one_that_is_not_a_png_has_a_size_and_no_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        assert!(matches!(ready(&ctx, &dir.path().join("shot-9.png")), Look::Missing));
        let jpeg = dir.path().join("photo.jpg");
        std::fs::write(&jpeg, b"not really a jpeg").unwrap();
        let Look::Ready(loaded) = ready(&ctx, &jpeg) else { panic!("an existing file is ready") };
        assert_eq!((loaded.size, loaded.bytes, loaded.small.is_none()), (None, 17, true));
    }

    struct Frame {
        ctx: egui::Context,
        removable: bool,
    }

    impl Frame {
        fn run(&self, path: &Path, events: Vec<egui::Event>) -> (Outcome, Rect) {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 200.0))),
                events,
                focused: true,
                ..egui::RawInput::default()
            };
            let mut got = (Outcome::default(), Rect::NOTHING);
            let _ = self.ctx.run_ui(raw, |ui| {
                let width = if self.removable { REMOVABLE_W } else { PLAIN_W };
                let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), vec2(width, CHIP_H));
                got = (paint(ui, rect, (path, 1), self.removable), rect);
            });
            got
        }

        fn click(&self, path: &Path, at: impl Fn(Rect) -> Pos2) -> Outcome {
            let (_, rect) = self.run(path, vec![]);
            let pos = at(rect);
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.run(path, vec![egui::Event::PointerMoved(pos)]);
            self.run(path, vec![button(true)]);
            self.run(path, vec![button(false)]).0
        }
    }

    #[test]
    fn a_click_on_the_chip_opens_it_and_a_click_on_its_button_removes_it_and_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = png(dir.path(), "shot-1.png", (80, 50));
        let frame = Frame { ctx: egui::Context::default(), removable: true };
        crate::fonts::install(&frame.ctx);
        ready(&frame.ctx, &path);
        let body = frame.click(&path, |rect| rect.left_center() + vec2(80.0, 0.0));
        assert_eq!(body, Outcome { opened: true, removed: false });
        let button =
            frame.click(&path, |rect| rect.right_center() - vec2(BUTTON_EDGE + BUTTON / 2.0, 0.0));
        assert_eq!(button, Outcome { opened: false, removed: true });
    }

    #[test]
    fn a_sent_chip_has_no_button_and_opens_on_any_click() {
        let dir = tempfile::tempdir().unwrap();
        let path = png(dir.path(), "shot-1.png", (80, 50));
        let frame = Frame { ctx: egui::Context::default(), removable: false };
        crate::fonts::install(&frame.ctx);
        ready(&frame.ctx, &path);
        let end = frame.click(&path, |rect| rect.right_center() - vec2(12.0, 0.0));
        assert_eq!(end, Outcome { opened: true, removed: false });
    }
}
