//! The Remote pill on an agent tab's status line and the popover it opens: the session's name,
//! Copy link (or Copy name until a link was seen) and Turn off.

use egui::{
    Color32, CornerRadius, Id, Painter, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    Ui, pos2, vec2,
};

use crate::brand;
use crate::screen_kit::{Size, body, primary_button, quiet_button};

const PILL_H: f32 = 22.0;
const PILL_PAD: f32 = 10.0;
const GLYPH: f32 = 13.0;
const GLYPH_GAP: f32 = 6.0;
const POPOVER_W: f32 = 290.0;
const PLACED: &str = "remote-pill-placed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pill {
    pub(crate) name: String,
    pub(crate) app: &'static str,
    pub(crate) link: Option<String>,
    /// The setting a restart reads. Off after Turn off, while the running agent still has it.
    pub(crate) saved_on: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    CopyLink(String),
    CopyName(String),
    TurnOff,
}

pub(crate) fn subline(pill: &Pill) -> String {
    match &pill.link {
        Some(_) => format!("Open it in {}", pill.app),
        None => format!("No link seen in the terminal yet. Look for this name in {}.", pill.app),
    }
}

pub(crate) fn note(pill: &Pill) -> &'static str {
    if pill.saved_on {
        "Turn off applies next time the agent starts."
    } else {
        "Off. Applies next time the agent starts."
    }
}

pub(crate) fn copy_label(pill: &Pill) -> &'static str {
    if pill.link.is_some() { "Copy link" } else { "Copy name" }
}

fn copy_action(pill: &Pill) -> Action {
    match &pill.link {
        Some(link) => Action::CopyLink(link.clone()),
        None => Action::CopyName(pill.name.clone()),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Placed {
    pub(crate) pill: Option<Rect>,
    pub(crate) copy: Option<Rect>,
    pub(crate) turn_off: Option<Rect>,
}

pub(crate) fn placed_id() -> Id {
    Id::new(PLACED)
}

/// Whether the popover opens by itself (the dev switch) or only on a click.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Popover {
    #[default]
    OnClick,
    Open,
}

pub(crate) struct Output {
    pub(crate) left: f32,
    pub(crate) action: Option<Action>,
}

pub(crate) fn paint_phone(painter: &Painter, center: Pos2, size: f32, ink: Color32) {
    let unit = size / 24.0;
    let stroke = Stroke::new((1.6 * size / 18.0).max(1.2), ink);
    let body = Rect::from_center_size(center, vec2(10.0 * unit, 19.0 * unit));
    painter.rect_stroke(
        body,
        CornerRadius::same((2.5 * unit).round() as u8),
        stroke,
        StrokeKind::Middle,
    );
    let bar = center + vec2(0.0, 7.5 * unit);
    painter.line_segment([bar - vec2(unit, 0.0), bar + vec2(unit, 0.0)], stroke);
}

fn pill_ink(saved_on: bool) -> Color32 {
    if saved_on { brand::TEAL_TEXT } else { brand::SOFT }
}

pub(crate) fn show(
    ui: &mut Ui,
    right: f32,
    center_y: f32,
    pill: &Pill,
    popover: Popover,
) -> Output {
    let ink = pill_ink(pill.saved_on);
    let painter = ui.painter().clone();
    let label = painter.layout_no_wrap("Remote".to_owned(), body(12.5), ink);
    let width = PILL_PAD + GLYPH + GLYPH_GAP + label.size().x + PILL_PAD;
    let rect =
        Rect::from_min_size(pos2(right - width, center_y - PILL_H / 2.0), vec2(width, PILL_H));
    let response = ui.interact(rect, ui.id().with("remote-pill"), Sense::click());
    let fill = if pill.saved_on { brand::TEAL.gamma_multiply(0.10) } else { brand::ITEM };
    let fill = if response.hovered() { fill.gamma_multiply(1.6) } else { fill };
    painter.rect_filled(rect, CornerRadius::same(255), fill);
    paint_phone(&painter, pos2(rect.left() + PILL_PAD + GLYPH / 2.0, center_y), GLYPH, ink);
    painter.galley(
        pos2(rect.left() + PILL_PAD + GLYPH + GLYPH_GAP, center_y - label.size().y / 2.0),
        label,
        ink,
    );
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if popover == Popover::Open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    let mut placed = Placed { pill: Some(rect), ..Placed::default() };
    let action = popover_menu(&response, pill, &mut placed);
    ui.ctx().data_mut(|data| data.insert_temp(placed_id(), placed));
    Output { left: rect.left(), action }
}

fn popover_menu(response: &Response, pill: &Pill, placed: &mut Placed) -> Option<Action> {
    let mut action = None;
    egui::Popup::menu(response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(
        |ui| {
            ui.set_width(POPOVER_W);
            ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
            ui.label(RichText::new(&pill.name).size(14.0).color(brand::TEXT));
            ui.label(RichText::new(subline(pill)).size(12.5).color(brand::MUTED));
            ui.horizontal(|ui| {
                let copy = primary_button(ui, copy_label(pill), true, Size::Compact);
                placed.copy = Some(copy.rect);
                if copy.clicked() {
                    action = Some(copy_action(pill));
                    ui.close();
                }
                if pill.saved_on {
                    let off = quiet_button(ui, "Turn off", true, Size::Compact);
                    placed.turn_off = Some(off.rect);
                    if off.clicked() {
                        action = Some(Action::TurnOff);
                    }
                }
            });
            ui.label(RichText::new(note(pill)).size(12.0).color(brand::FAINT));
        },
    );
    action
}

#[cfg(test)]
mod tests;
