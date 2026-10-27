//! Whether an avatar can be read on its plate: the SVG is rasterized small and judged by the
//! pixels. A picture is refused when most of it is too dark against the plate, when it runs past
//! the circle the face is cut to, or when it shows almost nothing. `rasterize` is also the one
//! place the app draws the picture, so the check and the screen see the same pixels.

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

/// The plate every face sits on, as sRGB. The app's `brand::FACE_PLATE` is this color.
pub const PLATE: [u8; 3] = [0x22, 0x29, 0x37];
/// How much of the plate's radius the drawn part of a picture reaches, wherever it sits in its
/// viewBox.
pub const FACE_FILL: f32 = 0.8;
pub const MIN_CONTRAST: f64 = 3.0;
const CHECK_PX: u32 = 48;
const PAINTED_ALPHA: u8 = 128;
const MAX_DARK_SHARE: f64 = 0.5;
const MAX_SPILL_SHARE: f64 = 0.02;
const MIN_PAINTED_SHARE: f64 = 0.01;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreadable {
    Undrawable,
    NearlyEmpty,
    TooDark { percent: u32 },
    Spills { percent: u32 },
}

impl std::fmt::Display for Unreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Undrawable => write!(f, "the svg could not be drawn"),
            Self::NearlyEmpty => write!(
                f,
                "the icon shows almost nothing: draw filled shapes or thick strokes in visible colors"
            ),
            Self::TooDark { percent } => write!(
                f,
                "{percent}% of the icon is too dark to read on its plate ({}): use colors at least {MIN_CONTRAST}:1 against it, bright fills and strokes with no dark tile behind them",
                hex(PLATE),
            ),
            Self::Spills { percent } => write!(
                f,
                "{percent}% of the icon is outside the circle it is shown in, so its corners are cropped: keep the drawing inside the middle circle of the viewBox (the circle that touches the middle of each edge) and draw no background tile"
            ),
        }
    }
}

#[must_use]
pub fn hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

/// The picture fitted and centered in a `px` square, at `inset` of its width (1.0 fills it).
/// Premultiplied RGBA; `None` when the SVG cannot be drawn.
#[must_use]
pub fn rasterize(svg: &str, px: u32, inset: f32) -> Option<Pixmap> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = Pixmap::new(px, px)?;
    let size = tree.size();
    let room = px as f32 * inset;
    let scale = (room / size.width()).min(room / size.height());
    let left = (px as f32 - size.width() * scale) / 2.0;
    let top = (px as f32 - size.height() * scale) / 2.0;
    let transform = Transform::from_scale(scale, scale).post_translate(left, top);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(pixmap)
}

const PROBE_PX: u32 = 96;

/// The picture scaled and moved so what it paints, not its viewBox, is centered and reaches
/// `fill` of the square's half width at its farthest painted pixel, so nothing is cut by the
/// circle and small art is not left tiny. `None` when it cannot be drawn or paints nothing.
#[must_use]
pub fn rasterize_fitted(svg: &str, px: u32, fill: f32) -> Option<Pixmap> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let probe = rasterize(svg, PROBE_PX, 1.0)?;
    let painted: Vec<(f32, f32)> = probe
        .pixels()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.alpha() > 0)
        .map(|(i, _)| ((i as u32 % PROBE_PX) as f32, (i as u32 / PROBE_PX) as f32))
        .collect();
    if painted.is_empty() {
        return None;
    }
    let (min_x, max_x, min_y, max_y) =
        painted.iter().fold((f32::MAX, f32::MIN, f32::MAX, f32::MIN), |(a, b, c, d), &(x, y)| {
            (a.min(x), b.max(x + 1.0), c.min(y), d.max(y + 1.0))
        });
    let (cx, cy) = (f32::midpoint(min_x, max_x), f32::midpoint(min_y, max_y));
    let reach = painted
        .iter()
        .map(|&(x, y)| ((x + 0.5 - cx).abs() + 0.5).hypot((y + 0.5 - cy).abs() + 0.5))
        .fold(0.0_f32, f32::max);
    let size = tree.size();
    let probe_scale = (PROBE_PX as f32 / size.width()).min(PROBE_PX as f32 / size.height());
    let left = (PROBE_PX as f32 - size.width() * probe_scale) / 2.0;
    let top = (PROBE_PX as f32 - size.height() * probe_scale) / 2.0;
    let half = px as f32 / 2.0;
    let zoom = half * fill / reach;
    let transform = Transform::from_scale(probe_scale, probe_scale)
        .post_translate(left - cx, top - cy)
        .post_scale(zoom, zoom)
        .post_translate(half, half);
    let mut pixmap = Pixmap::new(px, px)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(pixmap)
}

fn linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn luminance(rgb: [u8; 3]) -> f64 {
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

/// The WCAG contrast ratio of two sRGB colors, from 1 to 21.
#[must_use]
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn percent(part: u32, whole: u32) -> u32 {
    (f64::from(part) * 100.0 / f64::from(whole.max(1))).round() as u32
}

/// Every reason the picture cannot be read, empty-handed when it can. A pixel counts as painted
/// from half opacity up and is judged by its own color, so a thin stroke's soft edge does not
/// count against it.
///
/// # Errors
/// The list of what to fix, never empty.
pub fn check(svg: &str) -> Result<(), Vec<Unreadable>> {
    let Some(pixmap) = rasterize(svg, CHECK_PX, 1.0) else {
        return Err(vec![Unreadable::Undrawable]);
    };
    let half = CHECK_PX as f32 / 2.0;
    let (mut painted, mut dark, mut outside) = (0_u32, 0_u32, 0_u32);
    for (index, pixel) in pixmap.pixels().iter().enumerate() {
        if pixel.alpha() < PAINTED_ALPHA {
            continue;
        }
        painted += 1;
        let x = (index as u32 % CHECK_PX) as f32 + 0.5 - half;
        let y = (index as u32 / CHECK_PX) as f32 + 0.5 - half;
        if x.hypot(y) > half + 0.5 {
            outside += 1;
        }
        let ink = pixel.demultiply();
        if contrast([ink.red(), ink.green(), ink.blue()], PLATE) < MIN_CONTRAST {
            dark += 1;
        }
    }
    let area = CHECK_PX * CHECK_PX;
    if f64::from(painted) < f64::from(area) * MIN_PAINTED_SHARE {
        return Err(vec![Unreadable::NearlyEmpty]);
    }
    let mut problems = Vec::new();
    if f64::from(dark) > f64::from(painted) * MAX_DARK_SHARE {
        problems.push(Unreadable::TooDark { percent: percent(dark, painted) });
    }
    if f64::from(outside) > f64::from(painted) * MAX_SPILL_SHARE {
        problems.push(Unreadable::Spills { percent: percent(outside, painted) });
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

#[cfg(test)]
mod tests;
