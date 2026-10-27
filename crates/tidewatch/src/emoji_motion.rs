//! When an animated emoji plays. A new message plays its emoji once, then they rest on the still
//! picture; pointing at one plays it again; a message loaded from history stays still; the Chat
//! switch in Settings and macOS Reduce motion turn all of it off. `advance` is the whole rule, a
//! pure function of the slot's state, the clock and what the screen says.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use egui::{ColorImage, Context, Id, Rect, TextureHandle, TextureOptions, pos2};
use image::AnimationDecoder;
use image::codecs::webp::WebPDecoder;

use crate::emoji_pack;

const KEPT_CLIPS: usize = 64;
const SHORTEST_FRAME_SECS: f64 = 0.016;
const REDUCE_MOTION_EVERY_SECS: f64 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct State {
    started: Option<f64>,
    seen: bool,
    hovering: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Input {
    pub(crate) now: f64,
    pub(crate) hovered: bool,
    pub(crate) fresh: bool,
    pub(crate) allowed: bool,
    pub(crate) total: f64,
}

impl State {
    pub(crate) fn wants_clip(self, hovered: bool, fresh: bool) -> bool {
        self.started.is_some() || (!self.seen && fresh) || (hovered && !self.hovering)
    }
}

/// The slot's next state and, while its animation plays, how many seconds in it is.
pub(crate) fn advance(state: State, input: Input) -> (State, Option<f64>) {
    let Input { now, hovered, fresh, allowed, total } = input;
    if !allowed {
        return (State { started: None, seen: true, hovering: hovered }, None);
    }
    let mut next = state;
    if !next.seen {
        next.seen = true;
        if fresh {
            next.started = Some(now);
        }
    }
    if hovered && !next.hovering {
        next.started = Some(now);
    }
    next.hovering = hovered;
    match next.started {
        Some(at) if now - at < total => (next, Some(now - at)),
        _ => {
            next.started = None;
            (next, None)
        }
    }
}

pub(crate) struct Clip {
    texture: TextureHandle,
    columns: usize,
    cell: [usize; 2],
    ends: Vec<f64>,
}

impl Clip {
    pub(crate) fn total(&self) -> f64 {
        self.ends.last().copied().unwrap_or(0.0)
    }

    pub(crate) fn frame_at(&self, elapsed: f64) -> Option<usize> {
        self.ends.iter().position(|end| elapsed < *end)
    }

    pub(crate) fn until_next_frame(&self, elapsed: f64) -> Duration {
        let end = self.frame_at(elapsed).map_or(elapsed, |frame| self.ends[frame]);
        Duration::from_secs_f64((end - elapsed).max(SHORTEST_FRAME_SECS))
    }

    pub(crate) fn texture(&self) -> &TextureHandle {
        &self.texture
    }

    /// The part of the atlas that holds `frame`, pulled in by half a texel so a neighbour never
    /// bleeds into it.
    pub(crate) fn uv(&self, frame: usize) -> Rect {
        let [width, height] = self.texture.size().map(|side| side as f32);
        let (column, row) = (frame % self.columns, frame / self.columns);
        let [cell_w, cell_h] = self.cell.map(|side| side as f32);
        let min = pos2(column as f32 * cell_w + 0.5, row as f32 * cell_h + 0.5);
        Rect::from_min_max(
            pos2(min.x / width, min.y / height),
            pos2((min.x + cell_w - 1.0) / width, (min.y + cell_h - 1.0) / height),
        )
    }
}

struct Decoded {
    atlas: ColorImage,
    columns: usize,
    cell: [usize; 2],
    ends: Vec<f64>,
}

fn decode(bytes: &[u8]) -> Option<Decoded> {
    let frames = WebPDecoder::new(Cursor::new(bytes)).ok()?.into_frames().collect_frames().ok()?;
    let first = frames.first()?.buffer();
    let cell = [first.width() as usize, first.height() as usize];
    let columns = (frames.len() as f64).sqrt().ceil() as usize;
    let rows = frames.len().div_ceil(columns);
    let mut atlas = image::RgbaImage::new((cell[0] * columns) as u32, (cell[1] * rows) as u32);
    let mut ends = Vec::with_capacity(frames.len());
    let mut elapsed = 0.0;
    for (index, frame) in frames.iter().enumerate() {
        let (column, row) = (index % columns, index / columns);
        image::imageops::replace(
            &mut atlas,
            frame.buffer(),
            (column * cell[0]) as i64,
            (row * cell[1]) as i64,
        );
        let (numer, denom) = frame.delay().numer_denom_ms();
        elapsed += (f64::from(numer) / f64::from(denom.max(1)) / 1000.0).max(SHORTEST_FRAME_SECS);
        ends.push(elapsed);
    }
    let size = [atlas.width() as usize, atlas.height() as usize];
    Some(Decoded {
        atlas: ColorImage::from_rgba_unmultiplied(size, atlas.as_raw()),
        columns,
        cell,
        ends,
    })
}

#[derive(Clone, Default)]
struct Clips(Arc<Mutex<HashMap<String, Option<Arc<Clip>>>>>);

/// The animation of the emoji with `key`, decoded and uploaded the first time it is asked for.
/// `None` when it has none, or its file does not decode.
pub(crate) fn clip(ctx: &Context, key: &str) -> Option<Arc<Clip>> {
    let clips =
        ctx.data_mut(|data| data.get_temp_mut_or_default::<Clips>(Id::new("emoji-clips")).clone());
    let mut loaded = clips.0.lock().unwrap_or_else(PoisonError::into_inner);
    if loaded.len() >= KEPT_CLIPS && !loaded.contains_key(key) {
        loaded.clear();
    }
    loaded
        .entry(key.to_owned())
        .or_insert_with(|| {
            let Decoded { atlas, columns, cell, ends } = decode(emoji_pack::moving().get(key)?)?;
            let texture =
                ctx.load_texture(format!("emoji-moving-{key}"), atlas, TextureOptions::LINEAR);
            Some(Arc::new(Clip { texture, columns, cell, ends }))
        })
        .clone()
}

#[cfg(test)]
pub(crate) fn clip_cached(ctx: &Context, key: &str) -> Option<Arc<Clip>> {
    let clips =
        ctx.data_mut(|data| data.get_temp_mut_or_default::<Clips>(Id::new("emoji-clips")).clone());
    let loaded = clips.0.lock().unwrap_or_else(PoisonError::into_inner);
    loaded.get(key).cloned().flatten()
}

pub(crate) fn has_clip(key: &str) -> bool {
    emoji_pack::moving().contains(key)
}

fn allowed_id() -> Id {
    Id::new("emoji-motion-allowed")
}

fn fresh_id() -> Id {
    Id::new("emoji-motion-fresh")
}

/// Turns every animation on or off; the window sets it each frame from Settings and the system.
pub(crate) fn set_allowed(ctx: &Context, allowed: bool) {
    ctx.data_mut(|data| data.insert_temp(allowed_id(), allowed));
}

pub(crate) fn allowed(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(allowed_id())).unwrap_or(true)
}

/// Runs `add` with the emoji drawn in it counted as those of a message that has just arrived.
pub(crate) fn fresh_scope<R>(ctx: &Context, fresh: bool, add: impl FnOnce() -> R) -> R {
    let before = is_fresh(ctx);
    ctx.data_mut(|data| data.insert_temp(fresh_id(), fresh));
    let out = add();
    ctx.data_mut(|data| data.insert_temp(fresh_id(), before));
    out
}

pub(crate) fn is_fresh(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(fresh_id())).unwrap_or(false)
}

pub(crate) fn state_of(ctx: &Context, slot: Id) -> State {
    ctx.data(|data| data.get_temp::<State>(slot)).unwrap_or_default()
}

pub(crate) fn store(ctx: &Context, slot: Id, state: State) {
    ctx.data_mut(|data| data.insert_temp(slot, state));
}

/// Whether the system asks for less motion, looked at again every couple of seconds.
#[derive(Default)]
pub(crate) struct ReduceMotion {
    read_at: Option<f64>,
    value: bool,
}

impl ReduceMotion {
    pub(crate) fn asked(&mut self, now: f64, read: impl FnOnce() -> bool) -> bool {
        if self.read_at.is_none_or(|at| now - at >= REDUCE_MOTION_EVERY_SECS) {
            self.read_at = Some(now);
            self.value = read();
        }
        self.value
    }
}

#[cfg(test)]
mod tests;
