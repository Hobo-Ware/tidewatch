//! Draws the colour picture of each emoji over the transparent slot `emoji_text::slot_job` left in
//! a laid out text. The picture is as wide as its slot and sits `LIFT_EM` of the text size above
//! the baseline of the text beside it, level with the middle of the lowercase letters.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use egui::epaint::text::LayoutJob;
use egui::epaint::text::TextFormat;
use egui::text::Galley;
use egui::{
    Color32, ColorImage, Context, Id, Pos2, Rect, TextureFilter, TextureHandle, TextureOptions, Ui,
    pos2, vec2,
};

use crate::emoji_motion;
use crate::emoji_pack;
use crate::emoji_text::{self, SLOT_EM};

pub(crate) const LIFT_EM: f32 = 0.375;
const KEPT_PICTURES: usize = 512;
const OPTIONS: TextureOptions = TextureOptions {
    magnification: TextureFilter::Linear,
    minification: TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(TextureFilter::Linear),
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Slot {
    pub(crate) rect: Rect,
    pub(crate) key: String,
    pub(crate) at: usize,
}

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<HashMap<String, Option<TextureHandle>>>>);

fn cache(ctx: &Context) -> Cache {
    ctx.data_mut(|data| data.get_temp_mut_or_default::<Cache>(Id::new("emoji-pictures")).clone())
}

fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// The still picture of the emoji with `key`, uploaded the first time something draws it.
pub(crate) fn still(ctx: &Context, key: &str) -> Option<TextureHandle> {
    let cache = cache(ctx);
    let mut loaded = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
    if loaded.len() >= KEPT_PICTURES && !loaded.contains_key(key) {
        loaded.clear();
    }
    loaded
        .entry(key.to_owned())
        .or_insert_with(|| {
            let image = decode(emoji_pack::still().get(key)?)?;
            Some(ctx.load_texture(format!("emoji-{key}"), image, OPTIONS))
        })
        .clone()
}

fn baseline_in_row(ctx: &Context, plain: &TextFormat) -> f32 {
    let mut job = LayoutJob::default();
    job.append("x", 0.0, TextFormat { color: Color32::WHITE, ..plain.clone() });
    ctx.fonts_mut(|fonts| fonts.layout_job(job))
        .rows
        .first()
        .and_then(|row| row.glyphs.first())
        .map_or(0.0, |glyph| glyph.pos.y)
}

fn baseline_of(ctx: &Context, known: &mut Vec<(TextFormat, f32)>, plain: TextFormat) -> f32 {
    if let Some((_, baseline)) = known.iter().find(|(format, _)| *format == plain) {
        return *baseline;
    }
    let baseline = baseline_in_row(ctx, &plain);
    known.push((plain, baseline));
    baseline
}

/// Where each emoji of `galley` (painted with its top left at `pos`) has its picture.
pub(crate) fn slots(ctx: &Context, pos: Pos2, galley: &Galley) -> Vec<Slot> {
    let text = &galley.job.text;
    let found = emoji_text::clusters(text);
    if found.is_empty() {
        return Vec::new();
    }
    let mut chars_at = Vec::with_capacity(found.len());
    let (mut byte, mut chars) = (0, 0);
    for cluster in &found {
        chars += text[byte..cluster.bytes.start].chars().count();
        byte = cluster.bytes.start;
        chars_at.push(chars);
    }
    let mut out = Vec::with_capacity(found.len());
    let mut baselines: Vec<(TextFormat, f32)> = Vec::new();
    let (mut next, mut row_start) = (0, 0);
    for placed in &galley.rows {
        let glyphs = &placed.row.glyphs;
        let row_end = row_start + glyphs.len();
        while next < found.len() && chars_at[next] < row_end {
            let cluster = &found[next];
            let glyph = chars_at[next].checked_sub(row_start).and_then(|i| glyphs.get(i));
            next += 1;
            let Some(glyph) = glyph.filter(|g| text[cluster.bytes.clone()].starts_with(g.chr))
            else {
                continue;
            };
            let format =
                galley.job.format_at_byte(egui::text::ByteIndex(cluster.bytes.start)).clone();
            let size = format.font_id.size;
            let plain = TextFormat {
                font_id: egui::FontId::new(size / SLOT_EM, format.font_id.family.clone()),
                ..format
            };
            let baseline = baseline_of(ctx, &mut baselines, plain.clone());
            let centre = pos2(
                pos.x + placed.pos.x + glyph.pos.x + glyph.advance_width / 2.0,
                pos.y + placed.pos.y + baseline - LIFT_EM * plain.font_id.size,
            );
            out.push(Slot {
                rect: Rect::from_center_size(centre, vec2(size, size)),
                key: cluster.key.clone(),
                at: cluster.bytes.start,
            });
        }
        row_start = row_end + usize::from(placed.ends_with_newline);
    }
    out
}

const WHOLE: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

/// Draws the emoji with `key` in `rect`: its animation while one plays (see `emoji_motion`), else
/// its still picture. `id` names the slot, so what it remembers is its own.
fn draw(ui: &Ui, id: Id, rect: Rect, key: &str) {
    let ctx = ui.ctx();
    let mut playing = None;
    if emoji_motion::has_clip(key) {
        let allowed = emoji_motion::allowed(ctx);
        let hovered = allowed && ui.rect_contains_pointer(rect);
        let state = emoji_motion::state_of(ctx, id);
        let fresh = emoji_motion::is_fresh(ctx);
        let clip = (allowed && state.wants_clip(hovered, fresh))
            .then(|| emoji_motion::clip(ctx, key))
            .flatten();
        let input = emoji_motion::Input {
            now: ctx.input(|input| input.time),
            hovered,
            fresh,
            allowed,
            total: clip.as_ref().map_or(0.0, |clip| clip.total()),
        };
        let (next, elapsed) = emoji_motion::advance(state, input);
        if next != state {
            emoji_motion::store(ctx, id, next);
        }
        playing = clip.zip(elapsed);
    }
    if let Some((clip, elapsed)) = playing
        && let Some(frame) = clip.frame_at(elapsed)
    {
        ui.painter().image(clip.texture().id(), rect, clip.uv(frame), Color32::WHITE);
        ctx.request_repaint_after(clip.until_next_frame(elapsed));
        return;
    }
    if let Some(texture) = still(ctx, key) {
        ui.painter().image(texture.id(), rect, WHOLE, Color32::WHITE);
    }
}

/// Paints the pictures of the emoji in `galley`, which is painted with its top left at `pos`.
/// `id` must differ per text on the screen.
pub(crate) fn paint(ui: &Ui, id: Id, pos: Pos2, galley: &Galley) {
    if galley.job.text.is_ascii() {
        return;
    }
    for slot in slots(ui.ctx(), pos, galley) {
        if ui.is_rect_visible(slot.rect) {
            draw(ui, id.with(("emoji", slot.at)), slot.rect, &slot.key);
        }
    }
}

/// One emoji as a picture in `rect`, for a list that draws no text around it. Returns whether the
/// emoji has a picture.
pub(crate) fn paint_in(ui: &Ui, id: Id, rect: Rect, emoji: &str) -> bool {
    let key = emoji_text::key_of(emoji);
    if !emoji_pack::still().contains(&key) {
        return false;
    }
    draw(ui, id, rect, &key);
    true
}

#[cfg(test)]
mod tests;
