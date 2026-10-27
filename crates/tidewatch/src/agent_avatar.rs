//! Draws an agent's face: always a circle. A dark plate with a 1 px ring, and on it the cleaned SVG
//! rasterized once per picture and pixel size (resvg, fitted to what it paints and cut to the circle, the texture kept
//! in egui's memory). A letter face is a circle too. The reason shows on hover, Redraw or Use
//! letter on right-click; a click on the menu is left in egui's memory for `take_choice`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use egui::{
    Align2, Color32, ColorImage, Context, Id, Rect, Response, Sense, Stroke, TextureHandle,
    TextureOptions, Ui,
};
use resvg::tiny_skia::{FillRule, Mask, PathBuilder, Transform};
use stdusk_egui::widgets::{menu_item, style_menu};
use tidewatch_core::avatar::Avatar;
use tidewatch_core::avatar_check::{FACE_FILL, rasterize, rasterize_fitted};

use crate::brand;
use crate::screen_kit::{badge_colors, badge_letter, heading};

const CHOICE: &str = "agent-avatar-choice";
const LOCKED: &str = "agent-avatar-locked";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Draw,
    Redraw,
    UseLetter,
}

fn key_of(avatar: &Avatar) -> u64 {
    let mut hasher = DefaultHasher::new();
    avatar.svg.hash(&mut hasher);
    hasher.finish()
}

fn render(svg: &str, px: u32) -> Option<ColorImage> {
    let mut pixmap = if svg == crate::avatar_glue::MEESEEKS_SVG {
        rasterize(svg, px, 1.0)?
    } else {
        rasterize_fitted(svg, px, FACE_FILL)?
    };
    let half = px as f32 / 2.0;
    let circle = PathBuilder::from_circle(half, half, (half - 1.0).max(0.5))?;
    let mut mask = Mask::new(px, px)?;
    mask.fill_path(&circle, FillRule::Winding, true, Transform::identity());
    pixmap.apply_mask(&mask);
    Some(ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data()))
}

/// The picture at `px` pixels square, made once and kept. `None` when it cannot be drawn.
pub(crate) fn texture(ctx: &Context, avatar: &Avatar, px: u32) -> Option<TextureHandle> {
    let px = px.max(1);
    let id = Id::new(("agent-avatar", key_of(avatar), px));
    if let Some(cached) = ctx.data(|d| d.get_temp::<Option<TextureHandle>>(id)) {
        return cached;
    }
    let made = render(&avatar.svg, px)
        .map(|image| ctx.load_texture(format!("avatar-{px}"), image, TextureOptions::LINEAR));
    ctx.data_mut(|d| d.insert_temp(id, made.clone()));
    made
}

fn plate(painter: &egui::Painter, rect: Rect, fill: Color32) {
    painter.circle_filled(rect.center(), rect.width().min(rect.height()) / 2.0, fill);
}

fn ring(painter: &egui::Painter, rect: Rect) {
    let radius = rect.width().min(rect.height()) / 2.0 - 0.5;
    painter.circle_stroke(rect.center(), radius, Stroke::new(1.0, brand::FACE_RING));
}

/// Paints the avatar as a round face in `rect`, rasterized for the screen's pixel density. Returns
/// whether it drew; a picture that cannot be drawn leaves the empty plate.
pub(crate) fn paint(ctx: &Context, painter: &egui::Painter, rect: Rect, avatar: &Avatar) -> bool {
    let px = (rect.width() * ctx.pixels_per_point()).round() as u32;
    plate(painter, rect, brand::FACE_PLATE);
    let art = texture(ctx, avatar, px);
    if let Some(art) = &art {
        painter.image(
            art.id(),
            rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    ring(painter, rect);
    art.is_some()
}

/// The avatar's reason as a tip over `rect`.
pub(crate) fn hover(ui: &Ui, rect: Rect, tab: u64, avatar: &Avatar) {
    ui.interact(rect, ui.id().with(("avatar-tip", tab)), Sense::hover())
        .on_hover_text(&avatar.reason);
}

pub(crate) fn lock(ctx: &Context, tabs: Vec<u64>) {
    ctx.data_mut(|d| d.insert_temp(Id::new(LOCKED), Arc::new(tabs)));
}

pub(crate) fn is_locked(ctx: &Context, tab: u64) -> bool {
    ctx.data(|d| d.get_temp::<Arc<Vec<u64>>>(Id::new(LOCKED)))
        .is_some_and(|tabs| tabs.contains(&tab))
}

/// The right-click menu on whatever `response` is the avatar's place.
pub(crate) fn menu(response: &Response, tab: u64) {
    if is_locked(&response.ctx, tab) {
        return;
    }
    response.context_menu(|ui| {
        style_menu(ui);
        menu_items(ui, tab, true);
    });
}

pub(crate) fn menu_items(ui: &mut Ui, tab: u64, picture: bool) {
    if is_locked(ui.ctx(), tab) {
        return;
    }
    for (label, choice) in choices(picture) {
        if menu_item(ui, label, "").clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(Id::new(CHOICE), Some((tab, choice))));
            ui.ctx().request_repaint();
            ui.close();
        }
    }
}

fn choices(picture: bool) -> &'static [(&'static str, Choice)] {
    if picture {
        &[("Redraw", Choice::Redraw), ("Use letter", Choice::UseLetter)]
    } else {
        &[("Draw avatar", Choice::Draw)]
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Face {
    Picture(Arc<Avatar>),
    Letter { letter: String, tint: (Color32, Color32) },
}

impl Face {
    pub(crate) fn of(profile: &str, agent_id: &str, avatar: Option<Arc<Avatar>>) -> Self {
        avatar.map_or_else(
            || Self::Letter {
                letter: profile
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map_or_else(|| badge_letter(agent_id), |c| c.to_uppercase().to_string()),
                tint: badge_colors(agent_id),
            },
            Self::Picture,
        )
    }

    pub(crate) fn avatar(&self) -> Option<&Avatar> {
        match self {
            Self::Picture(avatar) => Some(avatar),
            Self::Letter { .. } => None,
        }
    }
}

pub(crate) fn paint_face(ctx: &Context, painter: &egui::Painter, rect: Rect, face: &Face) -> bool {
    match face {
        Face::Picture(avatar) => paint(ctx, painter, rect, avatar),
        Face::Letter { letter, tint } => {
            plate(painter, rect, tint.0);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                letter,
                heading(rect.width() * 0.55),
                tint.1,
            );
            ring(painter, rect);
            false
        }
    }
}

/// What the menu chose since the last call.
pub(crate) fn take_choice(ctx: &Context) -> Option<(u64, Choice)> {
    ctx.data_mut(|d| d.remove_temp::<Option<(u64, Choice)>>(Id::new(CHOICE))).flatten()
}

#[cfg(test)]
mod tests;
