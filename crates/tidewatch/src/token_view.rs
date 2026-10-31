//! Tokens on screen: `slot_job` leaves a chip-wide gap in the laid-out text, `paint` draws the chip in it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::text::{CCursor, Galley};
use egui::{
    Color32, Context, CornerRadius, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};

use crate::attachments::fit_middle;
use crate::brand;
use crate::pictures;
use crate::screen_kit::{Size, body};
use crate::tokens::{self, Target, Token};
use tidewatch_core::image_paste::is_image_file;

const MARGIN: f32 = 3.0;
const GLYPH: f32 = 12.0;
const GLYPH_GAP: f32 = 5.0;
const MAX_LABEL_CHARS: usize = 28;
const HIDDEN_PX: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Picture,
    File,
    Folder,
}

#[derive(Clone, Default)]
struct Dirs(Arc<Mutex<HashMap<PathBuf, bool>>>);

fn is_dir(ctx: &Context, path: &Path) -> bool {
    let dirs =
        ctx.data_mut(|data| data.get_temp_mut_or_default::<Dirs>(Id::new("token-dirs")).clone());
    let mut known = dirs.0.lock().unwrap_or_else(PoisonError::into_inner);
    *known.entry(path.to_path_buf()).or_insert_with(|| path.is_dir())
}

fn kind_of(ctx: &Context, target: &Target) -> Kind {
    match target {
        Target::Pasted(_) => Kind::Picture,
        Target::File(path) if is_image_file(path) => Kind::Picture,
        Target::File(path) if is_dir(ctx, path) => Kind::Folder,
        Target::File(_) => Kind::File,
    }
}

/// The words each token of `text` shows, in order: `Screenshot 2` for a pasted screenshot, `Image 1`
/// for a transcript's picture, else the file name cut in the middle when it is long.
pub(crate) fn labels(text: &str) -> Vec<String> {
    let mut shots = 0;
    tokens::find(text)
        .iter()
        .map(|token| match &token.target {
            Target::Pasted(number) => format!("Image {number}"),
            Target::File(path) => {
                let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
                if name.starts_with("shot-") && is_image_file(path) {
                    shots += 1;
                }
                let label = pictures::label(path, shots);
                fit_middle(&label, |text| text.chars().count() <= MAX_LABEL_CHARS)
            }
        })
        .collect()
}

fn label_font() -> FontId {
    body(Size::Tag.font())
}

fn label_width(ctx: &Context, label: &str) -> f32 {
    let job = LayoutJob::simple_singleline(label.to_owned(), label_font(), Color32::WHITE);
    ctx.fonts_mut(|fonts| fonts.layout_job(job)).size().x
}

fn chip_width(ctx: &Context, label: &str) -> f32 {
    2.0 * Size::Tag.pad() + GLYPH + GLYPH_GAP + label_width(ctx, label)
}

fn hidden(plain: &TextFormat, line: f32) -> TextFormat {
    TextFormat {
        font_id: FontId::new(HIDDEN_PX, plain.font_id.family.clone()),
        color: Color32::TRANSPARENT,
        line_height: Some(line),
        underline: Stroke::NONE,
        strikethrough: Stroke::NONE,
        ..plain.clone()
    }
}

/// `job` with each token's characters shrunk to a hair and the chip's width put before the closing
/// one, so the cursor stands before the chip at the token's start and after it at its end.
pub(crate) fn slot_job(ctx: &Context, job: LayoutJob) -> LayoutJob {
    let found = tokens::find(&job.text);
    if found.is_empty() {
        return job;
    }
    let names = labels(&job.text);
    let mut out = LayoutJob { text: String::new(), sections: Vec::new(), ..job.clone() };
    for section in &job.sections {
        let range = usize::from(section.byte_range.start)..usize::from(section.byte_range.end);
        let line = section
            .format
            .line_height
            .unwrap_or_else(|| ctx.fonts_mut(|fonts| fonts.row_height(&section.format.font_id)));
        let plain = TextFormat { line_height: Some(line), ..section.format.clone() };
        let faint = hidden(&plain, line);
        let mut lead = section.leading_space;
        let mut put = |out: &mut LayoutJob, text: &str, format: &TextFormat| {
            if !text.is_empty() {
                out.append(text, lead, format.clone());
                lead = 0.0;
            }
        };
        let mut from = range.start;
        for (token, name) in found.iter().zip(&names) {
            if token.bytes.start < range.start || token.bytes.end > range.end {
                continue;
            }
            let close = token.bytes.end - tokens::CLOSE.len_utf8();
            put(&mut out, &job.text[from..token.bytes.start], &plain);
            put(&mut out, &job.text[token.bytes.start..close], &faint);
            let hair = LayoutJob::simple_singleline(
                job.text[token.bytes.start..close].to_owned(),
                faint.font_id.clone(),
                Color32::TRANSPARENT,
            );
            let hairs = ctx.fonts_mut(|fonts| fonts.layout_job(hair)).size().x;
            let slot = (chip_width(ctx, name) + 2.0 * MARGIN - hairs).max(0.0);
            out.append(&job.text[close..token.bytes.end], slot, faint.clone());
            from = token.bytes.end;
        }
        put(&mut out, &job.text[from..range.end], &plain);
    }
    out
}

/// Where each token of `galley` (painted with its top left at `pos`) has its chip.
pub(crate) fn chips(pos: Pos2, galley: &Galley) -> Vec<(Token, Rect)> {
    tokens::find(&galley.job.text)
        .into_iter()
        .filter_map(|token| {
            let glyph = |index: usize| CCursor { prefer_next_row: true, ..CCursor::new(index) };
            let start = galley.pos_from_cursor(glyph(token.chars.start));
            let end = galley.pos_from_cursor(glyph(token.chars.end - 1));
            if (start.min.y - end.min.y).abs() > 1.0 || end.min.x <= start.min.x {
                return None;
            }
            let rect = Rect::from_min_max(
                pos2(
                    pos.x + start.min.x + MARGIN,
                    pos.y + start.center().y - Size::Tag.height() / 2.0,
                ),
                pos2(
                    pos.x + end.min.x - MARGIN,
                    pos.y + start.center().y + Size::Tag.height() / 2.0,
                ),
            );
            Some((token, rect))
        })
        .collect()
}

fn paint_glyph(painter: &Painter, rect: Rect, kind: Kind) {
    let ink = Stroke::new(1.2, brand::SOFT);
    let c = rect.center();
    match kind {
        Kind::Picture => {
            let frame = Rect::from_center_size(c, vec2(GLYPH, GLYPH - 2.0));
            painter.rect_stroke(frame, CornerRadius::same(2), ink, StrokeKind::Inside);
            painter.circle_filled(frame.left_top() + vec2(3.5, 3.5), 1.2, brand::SOFT);
            painter.line_segment(
                [frame.left_bottom() + vec2(1.5, -1.5), frame.center() + vec2(0.0, 0.5)],
                ink,
            );
            painter.line_segment(
                [frame.center() + vec2(0.0, 0.5), frame.right_bottom() + vec2(-1.5, -1.5)],
                ink,
            );
        }
        Kind::File => {
            let page = Rect::from_center_size(c, vec2(GLYPH - 3.0, GLYPH));
            painter.rect_stroke(page, CornerRadius::same(2), ink, StrokeKind::Inside);
            for dy in [-1.5, 1.5] {
                painter.line_segment(
                    [c + vec2(-1.8, dy), c + vec2(1.8, dy)],
                    Stroke::new(1.0, brand::SOFT),
                );
            }
        }
        Kind::Folder => {
            let body = Rect::from_center_size(c + vec2(0.0, 1.0), vec2(GLYPH, GLYPH - 4.0));
            let tab = Rect::from_min_size(body.left_top() - vec2(0.0, 2.0), vec2(5.0, 2.0));
            painter.rect_stroke(body, CornerRadius::same(2), ink, StrokeKind::Inside);
            painter.rect_stroke(tab, CornerRadius::same(1), ink, StrokeKind::Inside);
        }
    }
}

fn clicked_id() -> Id {
    Id::new("token-clicked")
}

/// The number of the transcript picture whose chip was clicked since the last call.
pub(crate) fn take_clicked(ctx: &Context) -> Option<u32> {
    ctx.data_mut(|data| data.remove_temp(clicked_id()))
}

fn paint_thumbnail(painter: &Painter, rect: Rect, texture: &egui::TextureHandle) {
    let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.image(texture.id(), rect, whole, Color32::WHITE);
    painter.rect_stroke(
        rect,
        CornerRadius::same(2),
        Stroke::new(1.0, brand::FIELD_LINE),
        StrokeKind::Inside,
    );
}

fn open(ctx: &Context, target: &Target) {
    match target {
        Target::File(path) => pictures::open_chip(path),
        Target::Pasted(number) => {
            ctx.data_mut(|data| data.insert_temp(clicked_id(), *number));
        }
    }
}

/// Draws the chip of every token of `galley`, painted with its top left at `pos`. With `clickable`
/// a chip opens its picture or shows its file, and its path is the tooltip.
pub(crate) fn paint(ui: &Ui, id: Id, (pos, galley): (Pos2, &Galley), clickable: bool) {
    let names = labels(&galley.job.text);
    for ((token, rect), name) in chips(pos, galley).into_iter().zip(names) {
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let kind = kind_of(ui.ctx(), &token.target);
        let loaded = match (&token.target, kind) {
            (Target::File(path), Kind::Picture) => {
                match pictures::look(ui.ctx(), pictures::Source::File(path)) {
                    pictures::Look::Ready(loaded) => Some(loaded),
                    pictures::Look::Loading | pictures::Look::Missing => None,
                }
            }
            _ => None,
        };
        let hovered = clickable && ui.rect_contains_pointer(rect);
        if clickable {
            let slot = id.with(("token", token.bytes.start));
            let response = ui.interact(rect, slot, Sense::click());
            if response.clicked() {
                open(ui.ctx(), &token.target);
            }
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            match (&token.target, &loaded) {
                (_, Some(loaded)) => loaded.preview_on(response),
                (Target::File(path), None) => {
                    response.on_hover_text(path.display().to_string());
                }
                (Target::Pasted(_), None) => {}
            }
        }
        let painter = ui.painter();
        let fill = if hovered { brand::ITEM } else { brand::SUNKEN };
        let radius = CornerRadius::same(Size::Tag.radius());
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(rect, radius, Stroke::new(1.0, brand::FIELD_LINE), StrokeKind::Inside);
        let glyph = Rect::from_center_size(
            pos2(rect.left() + Size::Tag.pad() + GLYPH / 2.0, rect.center().y),
            vec2(GLYPH, GLYPH),
        );
        match loaded.as_deref().and_then(pictures::Loaded::thumbnail) {
            Some(texture) => paint_thumbnail(painter, glyph, texture),
            None => paint_glyph(painter, glyph, kind),
        }
        painter.text(
            pos2(glyph.right() + GLYPH_GAP, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            label_font(),
            brand::SOFT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn galley(ctx: &Context, text: &str) -> std::sync::Arc<Galley> {
        let font = FontId::proportional(16.0);
        let job = LayoutJob::simple(text.to_owned(), font, Color32::WHITE, 600.0);
        let slotted = slot_job(ctx, job);
        ctx.fonts_mut(|fonts| fonts.layout_job(slotted))
    }

    #[test]
    fn a_token_takes_exactly_its_chip_and_the_gap_around_it_in_the_row() {
        let ctx = crate::fonts::test_context();
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let text = format!("ab {} cd", tokens::encode(&Target::File("/s/q3-report.pdf".into())));
        let laid = galley(&ctx, &text);
        let found = chips(Pos2::ZERO, &laid);
        assert_eq!(found.len(), 1);
        let want = chip_width(&ctx, "q3-report.pdf");
        assert!((found[0].1.width() - want).abs() < 1.0, "{} vs {want}", found[0].1.width());
        let long = format!(
            "ab {} cd",
            tokens::encode(&Target::File("/var/folders/x/T/a/b/c/d/e/pastes/q3-report.pdf".into()))
        );
        let wide = chips(Pos2::ZERO, &galley(&ctx, &long));
        assert!(
            (wide[0].1.width() - want).abs() < 1.0,
            "a long path is no wider: {}",
            wide[0].1.width()
        );
    }

    #[test]
    fn labels_number_pasted_screenshots_and_cut_long_names_in_the_middle() {
        let long = "/s/a-really-long-file-name-that-keeps-going-and-going.pdf";
        let text = [
            tokens::encode(&Target::File("/s/shot-1.png".into())),
            tokens::encode(&Target::File("/s/doc.pdf".into())),
            tokens::encode(&Target::File("/s/shot-2.png".into())),
            tokens::encode(&Target::Pasted(4)),
            tokens::encode(&Target::File(long.into())),
        ]
        .join(" ");
        let names = labels(&text);
        assert_eq!(&names[..4], ["Screenshot 1", "doc.pdf", "Screenshot 2", "Image 4"]);
        assert_eq!(names[4].chars().count(), MAX_LABEL_CHARS);
        assert!(names[4].contains('\u{2026}'));
    }
}
