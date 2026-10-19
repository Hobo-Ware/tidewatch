//! Draws an agent's avatar: the cleaned SVG rasterized once per picture and pixel size (resvg, the
//! texture kept in egui's memory), with the reason on hover and Redraw or Use letter on right-click.
//! A click on the menu is left in egui's memory for `take_choice`.

use std::hash::{DefaultHasher, Hash, Hasher};

use egui::{ColorImage, Context, Id, Rect, Response, Sense, TextureHandle, TextureOptions, Ui};
use stdusk_egui::widgets::{menu_item, style_menu};
use tidewatch_core::avatar::Avatar;

const CHOICE: &str = "agent-avatar-choice";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Redraw,
    UseLetter,
}

fn key_of(avatar: &Avatar) -> u64 {
    let mut hasher = DefaultHasher::new();
    avatar.svg.hash(&mut hasher);
    hasher.finish()
}

fn render(svg: &str, px: u32) -> Option<ColorImage> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let size = tree.size();
    let scale = (px as f32 / size.width()).min(px as f32 / size.height());
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
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

/// Paints the avatar in `rect`, rasterized for the screen's pixel density. Returns whether it drew.
pub(crate) fn paint(ctx: &Context, painter: &egui::Painter, rect: Rect, avatar: &Avatar) -> bool {
    let px = (rect.width() * ctx.pixels_per_point()).round() as u32;
    let Some(texture) = texture(ctx, avatar, px) else { return false };
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    true
}

/// The avatar's reason as a tip over `rect`.
pub(crate) fn hover(ui: &Ui, rect: Rect, tab: u64, avatar: &Avatar) {
    ui.interact(rect, ui.id().with(("avatar-tip", tab)), Sense::hover())
        .on_hover_text(&avatar.reason);
}

/// The right-click menu on whatever `response` is the avatar's place.
pub(crate) fn menu(response: &Response, tab: u64) {
    response.context_menu(|ui| {
        style_menu(ui);
        for (label, choice) in [("Redraw", Choice::Redraw), ("Use letter", Choice::UseLetter)] {
            if menu_item(ui, label, "").clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(Id::new(CHOICE), Some((tab, choice))));
                ui.ctx().request_repaint();
                ui.close();
            }
        }
    });
}

/// What the menu chose since the last call.
pub(crate) fn take_choice(ctx: &Context) -> Option<(u64, Choice)> {
    ctx.data_mut(|d| d.remove_temp::<Option<(u64, Choice)>>(Id::new(CHOICE))).flatten()
}

#[cfg(test)]
mod tests;
