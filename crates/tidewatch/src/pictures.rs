//! Pictures and files in a sent bubble that the sentence holds no token for: the chip, its thumbnail
//! decoded once on a `Jobs` thread and kept in egui's memory by path or by the picture's hash. A
//! picture a transcript carries inline is `Source::Pasted`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use egui::epaint::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, ColorImage, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Ui, vec2,
};
use image::imageops::FilterType;
use tidewatch_core::image_paste::is_image_file;
use tidewatch_core::transcript_image::Picture;

use crate::attachments;
use crate::brand;
use crate::jobs::Jobs;
use crate::screen_kit::Size;

pub(crate) const CHIP_H: f32 = 48.0;
pub(crate) const CHIP_GAP: f32 = 8.0;
pub(crate) const REMOVABLE_W: f32 = 240.0;
pub(crate) const PLAIN_W: f32 = 206.0;
const THUMB_W: f32 = 52.0;
const THUMB_H: f32 = 36.0;
const EDGE: f32 = 6.0;
const TEXT_GAP: f32 = 10.0;
const TEXT_W: f32 = 128.0;
const BUTTON: f32 = Size::Compact.height();
const BUTTON_EDGE: f32 = 8.0;
const SMALL_PX: u32 = 104;
const SMALL_PY: u32 = 72;
const LARGE_PX: u32 = 360;
const MAX_DECODE_BYTES: usize = 48 << 20;
pub(crate) const PASTES_DIR: &str = "pastes";
pub(crate) const OVER_TEXT_GAP: f32 = 8.0;

/// Where a chip's picture comes from.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Source<'a> {
    File(&'a Path),
    /// A picture a transcript carries inline.
    Pasted(&'a Picture),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    File(PathBuf),
    Pasted(u64),
}

impl Source<'_> {
    fn key(self) -> Key {
        match self {
            Self::File(path) => Key::File(path.to_owned()),
            Self::Pasted(picture) => Key::Pasted(picture.hash),
        }
    }

    fn label(self, n: usize) -> String {
        match self {
            Self::File(path) => label(path, n),
            Self::Pasted(picture) => format!("Image {}", picture.number.map_or(n, |n| n as usize)),
        }
    }

    fn gone(self) -> &'static str {
        match self {
            Self::File(_) => "File not found",
            Self::Pasted(_) => "Cannot read",
        }
    }
}

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

/// The width of one chip in a block `width` wide: its own width, or the block's when that is narrower.
pub(crate) fn chip_width(width: f32, removable: bool) -> f32 {
    (if removable { REMOVABLE_W } else { PLAIN_W }).min(width)
}

/// The width of the block for `count` sent chips in `room`: as many chips as fit in a row, side by
/// side, and never wider than `room`.
pub(crate) fn block_width(count: usize, room: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let fit = (((room + CHIP_GAP) / (PLAIN_W + CHIP_GAP)).floor() as usize).max(1);
    let per_row = count.min(fit) as f32;
    (per_row * PLAIN_W + (per_row - 1.0) * CHIP_GAP).min(room)
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

pub(crate) struct Loaded {
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) bytes: u64,
    pub(crate) dir: bool,
    small: Option<TextureHandle>,
    large: Option<(TextureHandle, [f32; 2])>,
}

#[derive(Clone)]
pub(crate) enum Look {
    Loading,
    Ready(Arc<Loaded>),
    Missing,
}

struct Slots {
    looks: Mutex<HashMap<Key, Look>>,
    jobs: Mutex<Jobs<(Key, Look)>>,
    decodes: AtomicUsize,
}

#[derive(Clone)]
struct Cache(Arc<Slots>);

impl Default for Cache {
    fn default() -> Self {
        Self(Arc::new(Slots {
            looks: Mutex::default(),
            jobs: Mutex::new(Jobs::new()),
            decodes: AtomicUsize::new(0),
        }))
    }
}

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

fn thumbnails(
    ctx: &egui::Context,
    name: &str,
    bytes: u64,
    decoded: Option<&image::DynamicImage>,
) -> Look {
    let size = decoded.map(|image| (image.width(), image.height()));
    let small = decoded.map(|image| {
        texture(
            ctx,
            format!("{name}#small"),
            &image.resize_to_fill(SMALL_PX, SMALL_PY, FilterType::Lanczos3),
        )
    });
    let large = decoded.map(|image| {
        let fitted = image.resize(LARGE_PX, LARGE_PX, FilterType::Triangle);
        let shown = [fitted.width() as f32, fitted.height() as f32];
        (texture(ctx, format!("{name}#large"), &fitted), shown)
    });
    Look::Ready(Arc::new(Loaded { size, bytes, dir: false, small, large }))
}

fn load_file(ctx: &egui::Context, path: &Path) -> Look {
    let Ok(meta) = std::fs::metadata(path) else { return Look::Missing };
    if meta.is_dir() || !is_image_file(path) {
        return Look::Ready(Arc::new(Loaded {
            size: None,
            bytes: meta.len(),
            dir: meta.is_dir(),
            small: None,
            large: None,
        }));
    }
    thumbnails(ctx, &path.display().to_string(), meta.len(), image::open(path).ok().as_ref())
}

fn load_pasted(ctx: &egui::Context, picture: &Picture) -> Look {
    let Some(bytes) = picture.bytes() else { return Look::Missing };
    let decoded =
        (bytes.len() <= MAX_DECODE_BYTES).then(|| image::load_from_memory(&bytes).ok()).flatten();
    thumbnails(ctx, &format!("pasted-{:016x}", picture.hash), bytes.len() as u64, decoded.as_ref())
}

/// The thumbnails of `source`: `Loading` the first time, while a `Jobs` thread decodes it and wakes
/// the window. A result is taken into the cache here, on the UI thread, by whoever looks next.
pub(crate) fn look(ctx: &egui::Context, source: Source<'_>) -> Look {
    let cache = cache(ctx);
    let key = source.key();
    {
        let mut looks = cache.0.looks.lock().unwrap_or_else(PoisonError::into_inner);
        let jobs = cache.0.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        looks.extend(jobs.drain());
        if let Some(look) = looks.get(&key) {
            return look.clone();
        }
        looks.insert(key.clone(), Look::Loading);
    }
    let owned = match source {
        Source::File(path) => Owned::File(path.to_owned()),
        Source::Pasted(picture) => Owned::Pasted(picture.clone()),
    };
    let work_ctx = ctx.clone();
    let slots = Arc::clone(&cache.0);
    cache.0.jobs.lock().unwrap_or_else(PoisonError::into_inner).spawn(ctx, move || {
        slots.decodes.fetch_add(1, Ordering::SeqCst);
        let look = match &owned {
            Owned::File(path) => load_file(&work_ctx, path),
            Owned::Pasted(picture) => load_pasted(&work_ctx, picture),
        };
        (key, look)
    });
    Look::Loading
}

enum Owned {
    File(PathBuf),
    Pasted(Picture),
}

#[cfg(test)]
pub(crate) fn decodes(ctx: &egui::Context) -> usize {
    cache(ctx).0.decodes.load(Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn wait_ready(ctx: &egui::Context, source: Source<'_>) -> Look {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let seen = look(ctx, source);
        if !matches!(seen, Look::Loading) || std::time::Instant::now() > deadline {
            return seen;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) opened: bool,
    pub(crate) removed: bool,
}

fn truncated(
    ui: &Ui,
    text: &str,
    font: FontId,
    ink: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, ink);
    job.wrap = TextWrapping::truncate_at_width(width);
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

fn paint_file_glyph(ui: &Ui, rect: Rect, path: &Path, dir: bool) {
    let painter = ui.painter();
    let radius = CornerRadius::same(6);
    painter.rect_filled(rect, radius, brand::SUNKEN);
    painter.rect_stroke(rect, radius, Stroke::new(1.0, brand::FIELD_LINE), StrokeKind::Inside);
    let ink = Stroke::new(1.5, brand::SOFT);
    if dir {
        let body = Rect::from_center_size(rect.center() + vec2(0.0, 2.0), vec2(26.0, 18.0));
        let tab = Rect::from_min_size(body.left_top() - vec2(0.0, 4.0), vec2(10.0, 4.0));
        painter.rect_stroke(body, CornerRadius::same(3), ink, StrokeKind::Inside);
        painter.rect_stroke(tab, CornerRadius::same(2), ink, StrokeKind::Inside);
        return;
    }
    let fold = 11.0;
    let (right, top) = (rect.right(), rect.top());
    painter.add(egui::Shape::convex_polygon(
        vec![Pos2::new(right - fold, top), Pos2::new(right, top), Pos2::new(right, top + fold)],
        brand::ITEM,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(right - fold, top),
            Pos2::new(right - fold, top + fold),
            Pos2::new(right, top + fold),
        ],
        brand::FIELD_LINE,
        Stroke::NONE,
    ));
    painter.text(
        rect.center() + vec2(0.0, 2.0),
        egui::Align2::CENTER_CENTER,
        attachments::ext_label(path),
        FontId::proportional(12.0),
        brand::SOFT,
    );
}

fn paint_remove(ui: &Ui, rect: Rect, hovered: bool) {
    let painter = ui.painter();
    let line = if hovered { brand::FAINT } else { brand::FIELD_LINE };
    if hovered {
        painter.rect_filled(rect, CornerRadius::same(Size::Compact.radius()), brand::BUTTON);
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(Size::Compact.radius()),
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let c = rect.center();
    let ink = Stroke::new(1.5, brand::SOFT);
    painter.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], ink);
    painter.line_segment([c + vec2(3.5, -3.5), c + vec2(-3.5, 3.5)], ink);
}

/// Draws one chip in `rect` (`CHIP_H` tall, `REMOVABLE_W` wide with a remove button, `PLAIN_W`
/// without). A click anywhere but the button is `opened`; the hover shows a bigger preview.
pub(crate) fn paint(
    ui: &mut Ui,
    rect: Rect,
    picture: (Source<'_>, usize),
    removable: bool,
) -> Outcome {
    let (source, n) = picture;
    let id = ui.id().with(("picture-chip", source.key(), n));
    let body = ui.interact(rect, id, Sense::click());
    let look = look(ui.ctx(), source);
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
    let file = match source {
        Source::File(path) if !is_image_file(path) => Some(path),
        _ => None,
    };
    match file {
        Some(path) => paint_file_glyph(ui, thumb, path, loaded.as_ref().is_some_and(|l| l.dir)),
        None => paint_thumb(ui, thumb, loaded.as_ref().and_then(|loaded| loaded.small.as_ref())),
    }
    let nominal = if removable { REMOVABLE_W } else { PLAIN_W };
    let text_w = (TEXT_W - (nominal - rect.width()).max(0.0)).max(0.0);
    let name_font = FontId::proportional(14.0);
    let label = source.label(n);
    let shown = match file {
        Some(_) => attachments::fit_middle(&label, |text| {
            ui.painter().layout_no_wrap(text.to_owned(), name_font.clone(), brand::TEXT).size().x
                <= text_w
        }),
        None => label,
    };
    let name = truncated(ui, &shown, name_font, brand::TEXT, text_w);
    let detail_text = match &look {
        Look::Ready(loaded) if loaded.dir => "Folder".to_owned(),
        Look::Ready(loaded) => meta_text(loaded.size, loaded.bytes),
        Look::Loading => String::new(),
        Look::Missing => source.gone().to_owned(),
    };
    let detail = truncated(ui, &detail_text, FontId::proportional(12.0), brand::MUTED, text_w);
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
    let mut body = body.on_hover_cursor(egui::CursorIcon::PointingHand);
    if let Some(path) = file {
        body = body.on_hover_text(path.display().to_string());
    }
    if let Some(loaded) = loaded {
        loaded.preview_on(body);
    }
    outcome
}

impl Loaded {
    pub(crate) fn thumbnail(&self) -> Option<&TextureHandle> {
        self.small.as_ref()
    }

    /// The big picture as the hover of `response`, once it is decoded.
    pub(crate) fn preview_on(&self, response: egui::Response) {
        if let Some((texture, size)) = &self.large {
            response.on_hover_ui(|ui| {
                ui.add(egui::Image::new(egui::load::SizedTexture::new(
                    texture.id(),
                    vec2(size[0], size[1]),
                )));
            });
        }
    }
}

/// Opens the picture in the system's viewer.
pub(crate) fn open_picture(path: &Path) {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    if let Err(error) = std::process::Command::new(program).arg(path).spawn() {
        eprintln!("tidewatch: cannot open {}: {error}", path.display());
    }
}

/// Shows a file in Finder (the file manager's folder elsewhere), so that clicking a chip never runs
/// a program.
pub(crate) fn reveal(path: &Path) {
    let mut command =
        std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
    if cfg!(target_os = "macos") {
        command.arg("-R").arg(path);
    } else {
        command.arg(path.parent().unwrap_or(path));
    }
    if let Err(error) = command.spawn() {
        eprintln!("tidewatch: cannot show {}: {error}", path.display());
    }
}

pub(crate) fn open_chip(path: &Path) {
    if is_image_file(path) || path.is_dir() {
        open_picture(path);
    } else {
        reveal(path);
    }
}

/// Writes a transcript's picture into `dir` under the name its bytes give it, unless that file is
/// already there, and returns the path.
pub(crate) fn write_once(dir: &Path, picture: &Picture) -> std::io::Result<PathBuf> {
    let path = dir.join(picture.file_name());
    if path.exists() {
        return Ok(path);
    }
    let bytes = picture
        .bytes()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not base64"))?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Opens a transcript's picture in the system's viewer: written once to `dir` first, on its own
/// thread so the frame never waits on the disk.
pub(crate) fn open_pasted(picture: &Picture, dir: Option<&Path>) {
    let Some(dir) = dir.map(Path::to_owned) else { return };
    let picture = picture.clone();
    std::thread::spawn(move || match write_once(&dir, &picture) {
        Ok(path) => open_picture(&path),
        Err(error) => eprintln!("tidewatch: cannot save the picture to open it: {error}"),
    });
}

/// Draws the chips of `pictures` in the block at `origin`, `width` wide, and opens a clicked one
/// (a transcript's picture through the folder `pastes`). Returns the chip whose button was clicked.
pub(crate) fn paint_block<'a>(
    ui: &mut Ui,
    origin: Pos2,
    width: f32,
    pictures: impl IntoIterator<Item = Source<'a>>,
    (removable, pastes): (bool, Option<&Path>),
) -> Option<usize> {
    let chip_w = chip_width(width, removable);
    let mut removed = None;
    for (index, source) in pictures.into_iter().enumerate() {
        let rect = chip_rect(origin, index, width, chip_w);
        let outcome = paint(ui, rect, (source, index + 1), removable);
        if outcome.opened {
            match source {
                Source::File(path) => open_chip(path),
                Source::Pasted(picture) => open_pasted(picture, pastes),
            }
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
    fn a_chip_is_never_wider_than_the_block_it_sits_in() {
        let cases = [
            (600.0, false, PLAIN_W),
            (206.0, false, 206.0),
            (150.0, false, 150.0),
            (600.0, true, REMOVABLE_W),
            (180.0, true, 180.0),
        ];
        for (width, removable, want) in cases {
            assert_eq!(chip_width(width, removable), want, "{width} {removable}");
        }
    }

    #[test]
    fn a_block_of_sent_chips_is_as_many_side_by_side_as_the_room_holds() {
        let cases = [
            (0, 500.0, 0.0),
            (1, 500.0, PLAIN_W),
            (2, 500.0, 2.0 * PLAIN_W + CHIP_GAP),
            (3, 500.0, 2.0 * PLAIN_W + CHIP_GAP),
            (2, 300.0, PLAIN_W),
            (1, 150.0, 150.0),
        ];
        for (count, room, want) in cases {
            assert_eq!(block_width(count, room), want, "{count} in {room}");
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

    fn ready(ctx: &egui::Context, path: &Path) -> Look {
        wait_ready(ctx, Source::File(path))
    }

    fn png(dir: &Path, name: &str, size: (u32, u32)) -> PathBuf {
        let path = dir.join(name);
        let image = image::RgbaImage::from_fn(size.0, size.1, |x, y| {
            image::Rgba([(x % 255) as u8, (y % 255) as u8, 90, 255])
        });
        image.save(&path).unwrap();
        path
    }

    #[test]
    fn a_picture_is_decoded_once_and_its_size_and_thumbnails_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = png(dir.path(), "shot-1.png", (800, 513));
        let ctx = egui::Context::default();
        assert!(matches!(look(&ctx, Source::File(&path)), Look::Loading));
        let Look::Ready(loaded) = ready(&ctx, &path) else { panic!("the picture loads") };
        assert_eq!(loaded.size, Some((800, 513)));
        assert_eq!(loaded.bytes, std::fs::metadata(&path).unwrap().len());
        assert!(loaded.small.is_some());
        let (_, shown) = loaded.large.as_ref().unwrap();
        assert_eq!(shown[0], LARGE_PX as f32);
        assert!(shown[1] < shown[0]);
        let Look::Ready(again) = look(&ctx, Source::File(&path)) else { panic!("kept") };
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
                got = (paint(ui, rect, (Source::File(path), 1), self.removable), rect);
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

    fn pasted(number: Option<u32>, rgb: [u8; 3]) -> Picture {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(640, 400, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        let text = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        Picture::new(number, "image/png", &text)
    }

    #[test]
    fn each_transcript_picture_is_decoded_once_however_often_it_is_drawn_or_repeated() {
        let ctx = egui::Context::default();
        let (red, blue) = (pasted(Some(1), [200, 40, 40]), pasted(Some(2), [40, 40, 200]));
        let again = Picture::new(Some(9), "image/png", &red.base64);
        for _ in 0..5 {
            for picture in [&red, &blue, &again] {
                wait_ready(&ctx, Source::Pasted(picture));
            }
        }
        assert_eq!(decodes(&ctx), 2);
        let Look::Ready(first) = look(&ctx, Source::Pasted(&red)) else { panic!("ready") };
        let Look::Ready(second) = look(&ctx, Source::Pasted(&again)) else { panic!("ready") };
        assert!(Arc::ptr_eq(&first, &second), "the same bytes share one texture set");
        assert_eq!(first.size, Some((640, 400)));
        assert!(first.small.is_some() && first.large.is_some());
    }

    #[test]
    fn a_picture_that_is_not_base64_is_missing_and_one_that_is_not_an_image_has_no_thumbnail() {
        let ctx = egui::Context::default();
        let broken = Picture::new(None, "image/png", "not base64!");
        assert!(matches!(wait_ready(&ctx, Source::Pasted(&broken)), Look::Missing));
        let text = Picture::new(None, "image/png", "aGVsbG8=");
        let Look::Ready(loaded) = wait_ready(&ctx, Source::Pasted(&text)) else { panic!("ready") };
        assert_eq!((loaded.size, loaded.bytes, loaded.small.is_none()), (None, 5, true));
    }

    #[test]
    fn a_transcript_picture_is_written_to_the_folder_once_and_opened_from_that_file() {
        let dir = tempfile::tempdir().unwrap();
        let pastes = dir.path().join("pastes");
        let picture = pasted(Some(1), [10, 20, 30]);
        let path = write_once(&pastes, &picture).unwrap();
        assert_eq!(path, pastes.join(picture.file_name()));
        assert_eq!(std::fs::read(&path).unwrap(), picture.bytes().unwrap());
        std::fs::write(&path, b"kept").unwrap();
        assert_eq!(write_once(&pastes, &picture).unwrap(), path);
        assert_eq!(std::fs::read(&path).unwrap(), b"kept", "the second open writes nothing");
        assert!(write_once(&pastes, &Picture::new(None, "image/png", "!!")).is_err());
    }

    #[test]
    fn a_pasted_chip_reads_image_and_its_number_with_its_size() {
        let picture = pasted(Some(4), [10, 20, 30]);
        let frame = Frame { ctx: egui::Context::default(), removable: false };
        crate::fonts::install(&frame.ctx);
        wait_ready(&frame.ctx, Source::Pasted(&picture));
        let mut texts = Vec::new();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 200.0))),
            ..egui::RawInput::default()
        };
        for _ in 0..2 {
            let out = frame.ctx.run_ui(raw.clone(), |ui| {
                let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), vec2(PLAIN_W, CHIP_H));
                paint(ui, rect, (Source::Pasted(&picture), 1), false);
            });
            texts = out
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        assert!(texts.iter().any(|text| text == "Image 4"), "{texts:?}");
        assert!(texts.iter().any(|text| text.starts_with("640 \u{d7} 400")), "{texts:?}");
    }

    fn chip_texts(path: &Path) -> Vec<String> {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        wait_ready(&ctx, Source::File(path));
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 200.0))),
            ..egui::RawInput::default()
        };
        let mut texts = Vec::new();
        for _ in 0..2 {
            let out = ctx.run_ui(raw.clone(), |ui| {
                let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), vec2(REMOVABLE_W, CHIP_H));
                paint(ui, rect, (Source::File(path), 1), true);
            });
            texts = out
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        texts
    }

    #[test]
    fn a_file_chip_reads_its_extension_a_middle_cut_name_and_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("quarterly-revenue-by-region-2026-final.csv");
        std::fs::write(&long, vec![b'x'; 12_345]).unwrap();
        let texts = chip_texts(&long);
        assert!(texts.iter().any(|text| text == "CSV"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "12 KB"), "{texts:?}");
        let name = texts.iter().find(|text| text.contains('\u{2026}')).expect("a cut name");
        assert!(name.starts_with("quar") && name.contains(".csv"), "{name}");
        let short = dir.path().join("a.pdf");
        std::fs::write(&short, b"%PDF").unwrap();
        let texts = chip_texts(&short);
        assert!(
            ["PDF", "a.pdf", "4 B"].iter().all(|want| texts.iter().any(|t| t == want)),
            "{texts:?}"
        );
    }

    #[test]
    fn a_folder_chip_says_folder_and_a_missing_file_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("exports");
        std::fs::create_dir(&folder).unwrap();
        let texts = chip_texts(&folder);
        assert!(texts.iter().any(|text| text == "exports") && texts.iter().any(|t| t == "Folder"));
        assert!(!texts.iter().any(|text| text == "FILE"), "{texts:?}");
        let texts = chip_texts(&dir.path().join("gone.zip"));
        assert!(texts.iter().any(|text| text == "File not found"), "{texts:?}");
    }
}
