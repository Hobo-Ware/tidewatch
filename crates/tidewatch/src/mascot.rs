//! The Tidewatch mascot: a shoggoth in a porcelain smiley mask. Each pose is a base picture plus
//! the real eyes, rasterized once per pixel size and drawn as textures, so the eyes can blink and
//! dart on their own while the mask stays put on the body.

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{
    ColorImage, Context, Id, Pos2, Rect, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};
use serde::Deserialize;

use crate::mascot_motion::{self, EyeFrame};

const SPEC: &[u8] = include_bytes!("../assets/mascot/mascot.json");
const BREATHE_ANCHOR: Vec2 = vec2(0.5, 0.9);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Pose {
    Peek,
    Asleep,
    Caught,
}

impl Pose {
    fn key(self) -> &'static str {
        match self {
            Self::Peek => "peek",
            Self::Asleep => "asleep",
            Self::Caught => "caught",
        }
    }

    fn base(self) -> &'static [u8] {
        match self {
            Self::Peek => include_bytes!("../assets/mascot/mascot-peek-base.svg"),
            Self::Asleep => include_bytes!("../assets/mascot/mascot-asleep-base.svg"),
            Self::Caught => include_bytes!("../assets/mascot/mascot-caught-base.svg"),
        }
    }

    fn breathes(self) -> bool {
        self == Self::Asleep
    }

    fn seed(self) -> u32 {
        (self as u32 + 1) * 16
    }

    fn spec(self) -> &'static PoseSpec {
        static SPECS: OnceLock<HashMap<String, PoseSpec>> = OnceLock::new();
        let all = SPECS.get_or_init(|| {
            serde_json::from_slice(SPEC).expect("assets/mascot/mascot.json is valid")
        });
        &all[self.key()]
    }
}

#[derive(Deserialize)]
struct PoseSpec {
    #[serde(rename = "viewBox")]
    view_box: String,
    eyes: Vec<EyeSpec>,
}

#[derive(Deserialize)]
struct EyeSpec {
    kind: String,
    x: f32,
    y: f32,
    size: f32,
}

impl PoseSpec {
    fn extent(&self) -> Vec2 {
        let parts: Vec<f32> =
            self.view_box.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        vec2(parts[2], parts[3])
    }
}

fn eye_svg(kind: &str) -> Option<&'static [u8]> {
    Some(match kind {
        "d" => include_bytes!("../assets/mascot/eye-d.svg"),
        "slit" => include_bytes!("../assets/mascot/eye-slit.svg"),
        "round" => include_bytes!("../assets/mascot/eye-round.svg"),
        "tiny" => include_bytes!("../assets/mascot/eye-tiny.svg"),
        _ => return None,
    })
}

/// The width a pose takes at `height`.
pub(crate) fn width_at(pose: Pose, height: f32) -> f32 {
    let extent = pose.spec().extent();
    height * extent.x / extent.y
}

/// Draws `pose` as large as fits in `area`, centered, and returns the rectangle it took. It moves
/// only while it is on screen in a focused, visible window, and asks for repaints only then.
pub(crate) fn show(ui: &Ui, area: Rect, pose: Pose) -> Rect {
    let spec = pose.spec();
    let extent = spec.extent();
    let unit = (area.width() / extent.x).min(area.height() / extent.y);
    let rect = Rect::from_center_size(area.center(), extent * unit);
    let ctx = ui.ctx();
    let (focused, minimized) = ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
    let animating = mascot_motion::should_animate(
        ui.is_rect_visible(rect),
        focused.unwrap_or(false),
        minimized != Some(true),
    );
    if let Some(after) = mascot_motion::next_repaint(animating) {
        ctx.request_repaint_after(after);
    }
    let time = if animating { ctx.input(|i| i.time) } else { 0.0 };
    let Some(raster) = raster(ctx, pose, rect.height() * ctx.pixels_per_point()) else {
        return rect;
    };
    let lift = vec2(0.0, mascot_motion::bob(time) * unit);
    let (lift, swell) = if pose.breathes() {
        (Vec2::ZERO, mascot_motion::breathe(time))
    } else {
        (lift, Vec2::splat(1.0))
    };
    let anchor = rect.min + rect.size() * BREATHE_ANCHOR;
    let body = Rect::from_min_max(
        anchor + (rect.min - anchor) * swell,
        anchor + (rect.max - anchor) * swell,
    )
    .translate(lift);
    paint(ui, &raster.base, body);
    for (index, (eye, texture)) in spec.eyes.iter().zip(&raster.eyes).enumerate() {
        let Some(texture) = texture else { continue };
        let frame = if animating {
            mascot_motion::eye(pose.seed() + index as u32, time)
        } else {
            EyeFrame::STILL
        };
        let top_left: Pos2 = rect.min + vec2(eye.x, eye.y) * unit;
        let size = Vec2::splat(eye.size * unit);
        let center = top_left + size / 2.0 + frame.dart * unit + lift;
        paint(ui, texture, Rect::from_center_size(center, vec2(size.x, size.y * frame.open)));
    }
    rect
}

fn paint(ui: &Ui, texture: &TextureHandle, rect: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture.id(), rect, uv, egui::Color32::WHITE);
}

#[derive(Clone)]
struct Raster {
    base: TextureHandle,
    eyes: Vec<Option<TextureHandle>>,
}

#[derive(Clone)]
struct Cached {
    pixels_high: u32,
    raster: Option<Raster>,
}

fn raster(ctx: &Context, pose: Pose, pixels_high: f32) -> Option<Raster> {
    let pixels_high = pixels_high.round().max(1.0) as u32;
    let id = Id::new(("mascot", pose));
    if let Some(cached) = ctx.data(|d| d.get_temp::<Cached>(id))
        && cached.pixels_high == pixels_high
    {
        return cached.raster;
    }
    let made = rasterize(ctx, pose, pixels_high);
    ctx.data_mut(|d| d.insert_temp(id, Cached { pixels_high, raster: made.clone() }));
    made
}

fn rasterize(ctx: &Context, pose: Pose, pixels_high: u32) -> Option<Raster> {
    let spec = pose.spec();
    let extent = spec.extent();
    let per_unit = pixels_high as f32 / extent.y;
    let load = |name: String, svg: &[u8], size: Vec2| {
        let image = render(svg, size)?;
        Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
    };
    let base = load(format!("mascot-{}", pose.key()), pose.base(), extent * per_unit)?;
    let eyes = spec
        .eyes
        .iter()
        .map(|eye| {
            let svg = eye_svg(&eye.kind)?;
            load(
                format!("mascot-{}-eye-{}", pose.key(), eye.kind),
                svg,
                Vec2::splat(eye.size * per_unit),
            )
        })
        .collect();
    Some(Raster { base, eyes })
}

fn render(svg: &[u8], size: Vec2) -> Option<ColorImage> {
    let (width, height) = (size.x.round().max(1.0) as u32, size.y.round().max(1.0) as u32);
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / tree.size().width(),
        height as f32 / tree.size().height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(ColorImage::from_rgba_premultiplied([width as usize, height as usize], pixmap.data()))
}

#[cfg(test)]
mod tests;
