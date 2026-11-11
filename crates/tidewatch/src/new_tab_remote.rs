//! The Remote control row of the new-tab form: a phone glyph, the title, one muted line and a switch.

use egui::{CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use crate::brand;
use crate::new_tab::RemoteOffer;
use crate::remote_pill::paint_phone;
use crate::screen_kit::TEXT_SECONDARY;
use crate::screen_kit::{body, heading};
use crate::settings_ui::switch;

pub(crate) const ROW_H: f32 = 62.0;
const PAD: f32 = 16.0;
const GLYPH_BOX: f32 = 34.0;
const GLYPH: f32 = 18.0;
const TEXT_GAP: f32 = 14.0;
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 22.0;

/// Where the row and its switch were drawn, and the new value when the switch was clicked.
pub(crate) struct Shown {
    pub(crate) rect: Rect,
    pub(crate) switch: Rect,
    pub(crate) toggled: Option<bool>,
}

pub(crate) fn show(ui: &mut Ui, offer: &RemoteOffer, on: bool) -> Shown {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let enabled = offer.is_available();
    let lit = enabled && on;
    let painter = ui.painter().clone();
    let radius = CornerRadius::same(12);
    painter.rect_filled(rect, radius, brand::card());
    if lit {
        let glow = Stroke::new(4.0, brand::teal().gamma_multiply(0.08));
        painter.rect_stroke(rect, radius, glow, StrokeKind::Outside);
    }
    let line = if lit { brand::teal() } else { brand::card_line() };
    painter.rect_stroke(rect, radius, Stroke::new(1.0, line), StrokeKind::Inside);

    let glyph_box = Rect::from_center_size(
        pos2(rect.left() + PAD + GLYPH_BOX / 2.0, rect.center().y),
        vec2(GLYPH_BOX, GLYPH_BOX),
    );
    let (tile, ink) = match (enabled, lit) {
        (true, true) => (brand::teal().gamma_multiply(0.12), brand::teal()),
        (true, false) => (brand::item(), brand::soft()),
        (false, _) => (brand::item(), brand::faint()),
    };
    painter.rect_filled(glyph_box, CornerRadius::same(9), tile);
    paint_phone(&painter, glyph_box.center(), GLYPH, ink);

    let switch_rect = Rect::from_min_size(
        pos2(rect.right() - PAD - SWITCH_W, rect.center().y - SWITCH_H / 2.0),
        vec2(SWITCH_W, SWITCH_H),
    );
    let text_left = glyph_box.right() + TEXT_GAP;
    let room = (switch_rect.left() - TEXT_GAP - text_left).max(1.0);
    let title_ink = if enabled { brand::text() } else { brand::muted() };
    let title = painter.layout_no_wrap("Remote control".to_owned(), heading(14.0), title_ink);
    let sub = crate::reader_view::ellipsized(
        &painter,
        offer.line(),
        body(TEXT_SECONDARY),
        brand::muted(),
        room,
    );
    let stack = title.size().y + 2.0 + sub.size().y;
    let top = rect.center().y - stack / 2.0;
    let sub_at = pos2(text_left, top + title.size().y + 2.0);
    painter.galley(pos2(text_left, top), title, title_ink);
    painter.galley(sub_at, sub, brand::muted());

    let mut shown = enabled && on;
    let mut child = ui.new_child(UiBuilder::new().max_rect(switch_rect));
    let changed = child.add_enabled_ui(enabled, |ui| switch(ui, &mut shown).changed()).inner;
    Shown { rect, switch: switch_rect, toggled: changed.then_some(shown) }
}
