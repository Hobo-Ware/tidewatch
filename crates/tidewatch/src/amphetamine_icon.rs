//! Draws the Amphetamine tablet with egui's painter. Everything is a few circles and polylines on
//! the 24 unit glyph grid, so it stays sharp at any scale and costs no texture; the mascot's art is
//! rasterized instead because it is a finished picture, not geometry that moves.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, vec2};

use crate::amphetamine_motion::{
    self as motion, CENTER, FIZZ_COUNT, Look, PUFF_COUNT, RADIUS, Speck, TILT_DEG, Tablet,
};
use crate::brand;

pub(crate) const GLYPH_PX: f32 = 28.0;
pub(crate) const SEED: u32 = 21;
const GRID: f32 = 24.0;
const WOBBLE_AT: Vec2 = vec2(12.0, 12.0);
const FACE_DROP: f32 = 0.8;
const RIM_STROKE: f32 = 1.4;
const FACE_STROKE: f32 = 1.3;
const SHADE_STROKE: f32 = 1.5;
const SHADE_SCALE: f32 = 0.84;
const EYES: [(Vec2, f32); 2] = [(vec2(-3.4, -8.4), 2.5), (vec2(4.6, -7.6), 1.9)];
const SCLERA: Color32 = Color32::from_rgb(0xf6, 0xfa, 0xf9);
const PUPIL: Color32 = Color32::from_rgb(0x0a, 0x0c, 0x10);
const EYE_POINTS: usize = 16;
const GLOW_RINGS: usize = 6;

struct Frame<'a> {
    painter: &'a Painter,
    origin: Pos2,
    scale: f32,
    rock: f32,
}

fn rotate(point: Vec2, about: Vec2, degrees: f32) -> Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let d = point - about;
    about + vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
}

impl Frame<'_> {
    fn at(&self, point: Vec2) -> Pos2 {
        let tilted = rotate(rotate(point, WOBBLE_AT, self.rock), CENTER, TILT_DEG);
        self.origin + tilted * self.scale
    }

    fn outside(&self, point: Vec2) -> Pos2 {
        self.origin + point * self.scale
    }

    fn stroke(&self, units: f32, color: Color32) -> Stroke {
        Stroke::new(units * self.scale, color)
    }

    fn line(&self, points: impl IntoIterator<Item = Vec2>, units: f32, color: Color32) {
        let points: Vec<Pos2> = points.into_iter().map(|p| self.at(p)).collect();
        self.painter.add(Shape::line(points, self.stroke(units, color)));
    }

    fn blob(&self, center: Vec2, radius: Vec2, color: Color32) {
        if (radius.x - radius.y).abs() < 1e-3 {
            self.painter.circle_filled(self.at(center), radius.x * self.scale, color);
            return;
        }
        let points = (0..EYE_POINTS)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / EYE_POINTS as f32;
                self.at(center + vec2(radius.x * t.cos(), radius.y * t.sin()))
            })
            .collect();
        self.painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    }
}

fn quadratic(from: Vec2, control: Vec2, to: Vec2, steps: usize) -> Vec<Vec2> {
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            from * (1.0 - t).powi(2) + control * (2.0 * (1.0 - t) * t) + to * (t * t)
        })
        .collect()
}

fn with_alpha(color: Color32, alpha: f32) -> Color32 {
    color.gamma_multiply(alpha.clamp(0.0, 1.0))
}

/// Paints the tablet centered in `rect` for `look`, with the pointer `hover` (0 to 1) over it.
pub(crate) fn paint(painter: &Painter, rect: Rect, look: Look, hover: f64) {
    let tablet = motion::tablet(look, hover);
    let scale = GLYPH_PX / GRID;
    let frame = Frame {
        painter,
        origin: rect.center() - vec2(GRID, GRID) * scale / 2.0,
        scale,
        rock: tablet.rock_deg,
    };
    let age = match look {
        Look::On { age } | Look::Stopping { age } => age,
        Look::Off | Look::Pending => 0.0,
    };
    glow(&frame, &tablet);
    body(&frame, &tablet);
    face(&frame, &tablet);
    if tablet.eyes > 0.0 {
        let moving = matches!(look, Look::On { .. });
        for (slot, (offset, radius)) in EYES.into_iter().enumerate() {
            eye(&frame, slot, CENTER + offset, radius, tablet.eyes, if moving { age } else { 0.0 });
        }
    }
    if let Look::On { age } = look {
        specks(&frame, age, tablet.specks);
    }
}

fn glow(frame: &Frame, tablet: &Tablet) {
    if tablet.glow <= 0.0 {
        return;
    }
    let center = frame.at(CENTER);
    for ring in (1..=GLOW_RINGS).rev() {
        let falloff = 1.0 - ring as f32 / (GLOW_RINGS as f32 + 1.0);
        let radius = (RADIUS + 0.8 * ring as f32) * frame.scale;
        frame.painter.circle_filled(
            center,
            radius,
            with_alpha(brand::TEAL, tablet.glow * 0.16 * falloff * falloff),
        );
    }
}

fn body(frame: &Frame, tablet: &Tablet) {
    if tablet.smooth_rim > 0.0 {
        frame.painter.circle(
            frame.at(CENTER),
            RADIUS * frame.scale,
            with_alpha(tablet.fill, tablet.smooth_rim),
            frame.stroke(RIM_STROKE, with_alpha(tablet.rim, tablet.smooth_rim)),
        );
    }
    let rims = motion::rims();
    if tablet.rough_fill > 0.0 {
        let mut mesh = Mesh::default();
        let fill = with_alpha(tablet.fill, tablet.rough_fill);
        let hub = frame.at(CENTER);
        mesh.vertices.push(Vertex { pos: hub, uv: WHITE_UV, color: fill });
        for point in &rims.rough {
            mesh.vertices.push(Vertex { pos: frame.at(*point), uv: WHITE_UV, color: fill });
        }
        let ring = rims.rough.len() as u32;
        for i in 0..ring {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % ring);
        }
        frame.painter.add(Shape::mesh(mesh));
        let shade = rims.shade.iter().map(|p| CENTER + (*p - CENTER) * SHADE_SCALE);
        let mut shade: Vec<Pos2> = shade.map(|p| frame.at(p)).collect();
        shade.push(shade[0]);
        let ink = with_alpha(brand::GROUND, 0.3 * tablet.rough_fill);
        frame.painter.add(Shape::line(shade, frame.stroke(SHADE_STROKE, ink)));
        for (x, y, radius, light) in motion::CHALK {
            let color = if light {
                brand::TEXT.gamma_multiply(0.35)
            } else {
                brand::GROUND.gamma_multiply(0.3)
            };
            frame.painter.circle_filled(
                frame.at(vec2(x, y)),
                radius * frame.scale,
                with_alpha(color, tablet.rough_fill),
            );
        }
    }
    if tablet.rough_draw > 0.0 {
        let count = rims.rough.len();
        let drawn = tablet.rough_draw.clamp(0.0, 1.0) * count as f32;
        let whole = drawn.floor() as usize;
        let mut points: Vec<Vec2> = rims.rough[..whole.min(count)].to_vec();
        if whole < count {
            let next = rims.rough[whole];
            let before = rims.rough[whole.saturating_sub(1)];
            points.push(before + (next - before) * drawn.fract());
        } else {
            points.push(rims.rough[0]);
        }
        frame.line(points, RIM_STROKE, tablet.rim);
    }
}

fn face(frame: &Frame, tablet: &Tablet) {
    let drop = vec2(0.0, FACE_DROP);
    let ink = tablet.face;
    frame.painter.circle_stroke(
        frame.at(vec2(8.3, 10.3) + drop),
        1.5 * frame.scale,
        frame.stroke(FACE_STROKE, ink),
    );
    let open = 1.0 - tablet.wink;
    if open > 0.0 {
        let ry = 1.5 * (1.0 - 0.9 * tablet.wink);
        let center = vec2(15.7, 10.3) + drop;
        let ring = (0..=EYE_POINTS).map(|i| {
            let t = std::f32::consts::TAU * i as f32 / EYE_POINTS as f32;
            center + vec2(1.5 * t.cos(), ry * t.sin())
        });
        frame.line(ring, FACE_STROKE, with_alpha(ink, open));
    }
    if tablet.wink > 0.0 {
        let arc = quadratic(vec2(14.1, 10.7), vec2(15.7, 9.0), vec2(17.3, 10.7), 8);
        frame.line(arc.into_iter().map(|p| p + drop), FACE_STROKE, with_alpha(ink, tablet.wink));
    }
    let smile = quadratic(vec2(7.6, 14.2), vec2(12.0, 18.0), vec2(16.4, 14.2), 12);
    frame.line(smile.into_iter().map(|p| p + drop), FACE_STROKE, ink);
}

fn eye(frame: &Frame, slot: usize, center: Vec2, radius: f32, pop: f32, age: f64) {
    let alpha = pop.clamp(0.0, 1.0);
    let grow = 0.3 + 0.7 * pop;
    let squash = if age > 0.0 { motion::blink(slot, age) } else { 1.0 };
    let anchor = center + vec2(0.0, 0.8 * radius);
    let place = |at: Vec2| anchor + (at - anchor) * vec2(grow, grow * squash);
    let size = |r: f32| vec2(r * grow, r * grow * squash);
    frame.blob(place(center), size(radius), with_alpha(SCLERA, alpha));
    let dart = if age > 0.0 { motion::dart(slot, age) } else { Vec2::ZERO };
    let pupil = center + vec2(0.12, 0.12) * radius + dart;
    frame.blob(place(pupil), size(radius / 2.0), with_alpha(PUPIL, alpha));
    let spark = center + vec2(0.288, -0.1) * radius + dart;
    frame.blob(place(spark), size(radius * 0.15), with_alpha(Color32::WHITE, alpha));
}

fn specks(frame: &Frame, age: f64, steady: f32) {
    let paint = |speck: Speck, amount: f32| {
        let alpha = speck.alpha * amount;
        if alpha > 0.0 && speck.radius > 0.0 {
            frame.painter.circle_filled(
                frame.outside(speck.pos),
                speck.radius * frame.scale,
                with_alpha(speck.ink, alpha),
            );
        }
    };
    for index in 0..PUFF_COUNT {
        if let Some(speck) = motion::puff(SEED, index, age) {
            paint(speck, 1.0);
        }
    }
    if steady > 0.0 {
        for index in 0..FIZZ_COUNT {
            if let Some(speck) = motion::fizz(SEED, index, age) {
                paint(speck, steady);
            }
        }
    }
}

#[cfg(test)]
mod tests;
