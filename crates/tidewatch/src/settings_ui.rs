//! The pieces every Settings page is built from: palette, cards and rows, switch, buttons, inputs.

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Layout, Margin, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, vec2,
};
use kelp_ui::theme::semibold;

pub(crate) mod editor;

pub(crate) use crate::brand::*;

const SECTION_GAP: f32 = 22.0;
const ROW_GAP: f32 = 16.0;
const SWITCH_SIZE: Vec2 = vec2(40.0, 22.0);
const BUTTON_HEIGHT: f32 = 34.0;

pub(crate) fn heading(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(semibold()).size(size).color(TEXT)
}

pub(crate) fn font_semibold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub(crate) fn caption(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(13.0).color(MUTED));
}

pub(crate) fn note(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(RichText::new(text).size(12.0).color(color));
}

pub(crate) fn page_header(ui: &mut Ui, title: &str, subtitle: &str) -> Response {
    ui.spacing_mut().item_spacing.y = 4.0;
    let response = ui.label(heading(title, 26.0));
    ui.label(RichText::new(subtitle).size(13.0).color(FAINT));
    response
}

pub(crate) fn footer_note(ui: &mut Ui, text: &str) {
    ui.add_space(SECTION_GAP);
    ui.label(RichText::new(text).monospace().size(11.5).color(FAINT));
}

pub(crate) fn section<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(SECTION_GAP);
    ui.label(RichText::new(title.to_uppercase()).size(12.0).color(FAINT));
    ui.add_space(8.0);
    card(ui, add)
}

pub(crate) fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_LINE))
        .corner_radius(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            add(ui)
        })
        .inner
}

fn divider(ui: &Ui) {
    if ui.min_rect().height() > 0.5 {
        let rect = ui.available_rect_before_wrap();
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, CARD_LINE));
    }
}

/// What the control of a row needs, so the text beside it wraps in time and stays centered.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Control {
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl Control {
    pub(crate) const NONE: Self = Self { width: 0.0, height: 0.0 };
    pub(crate) const SWITCH: Self = Self { width: SWITCH_SIZE.x, height: SWITCH_SIZE.y };

    pub(crate) const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub(crate) fn button(ui: &Ui, label: &str) -> Self {
        Self { width: button_width(ui, label), height: BUTTON_HEIGHT }
    }
}

pub(crate) fn row<R>(
    ui: &mut Ui,
    title: &str,
    desc: &str,
    control: Control,
    add_control: impl FnOnce(&mut Ui) -> R,
) -> R {
    row_with(
        ui,
        control,
        |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(title).size(15.0).color(TEXT));
            if !desc.is_empty() {
                ui.label(RichText::new(desc).size(13.0).color(MUTED));
            }
        },
        add_control,
    )
}

pub(crate) fn row_with<R>(
    ui: &mut Ui,
    control: Control,
    left: impl Fn(&mut Ui),
    add_control: impl FnOnce(&mut Ui) -> R,
) -> R {
    padded_row(ui, 16, control, left, add_control)
}

/// `row_with` with its own vertical padding, for dense lists such as the shortcuts. The text is
/// laid out once unseen to learn its height, then drawn centered against the control.
pub(crate) fn padded_row<R>(
    ui: &mut Ui,
    padding: i8,
    control: Control,
    left: impl Fn(&mut Ui),
    add_control: impl FnOnce(&mut Ui) -> R,
) -> R {
    divider(ui);
    Frame::new()
        .inner_margin(Margin::symmetric(18, padding))
        .show(ui, |ui| {
            let full = ui.available_rect_before_wrap();
            let text_width = (full.width() - control.width - ROW_GAP).max(120.0);
            let text = Rect::from_min_size(full.min, vec2(text_width, full.height()));
            let column = |builder: UiBuilder| builder.layout(Layout::top_down(Align::Min));
            let mut probe = ui.new_child(column(UiBuilder::new().max_rect(text).invisible()));
            left(&mut probe);
            let left_height = probe.min_rect().height();
            let height = left_height.max(control.height);
            let shifted = text.translate(vec2(0.0, (height - left_height) / 2.0));
            let mut left_ui = ui.new_child(column(UiBuilder::new().max_rect(shifted)));
            left(&mut left_ui);
            let right = Rect::from_min_size(full.min, vec2(full.width(), height));
            let mut right_ui = ui.new_child(
                UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)),
            );
            let result = add_control(&mut right_ui);
            ui.allocate_rect(right, Sense::hover());
            result
        })
        .inner
}

pub(crate) fn switch_row(ui: &mut Ui, title: &str, desc: &str, on: bool) -> Option<bool> {
    let mut value = on;
    let changed = row(ui, title, desc, Control::SWITCH, |ui| switch(ui, &mut value).changed());
    changed.then_some(value)
}

pub(crate) fn switch_line(ui: &mut Ui, label: &str, on: &mut bool) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let changed = switch(ui, on).changed();
        ui.label(RichText::new(label).size(14.0).color(TEXT));
        changed
    })
    .inner
}

pub(crate) fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(SWITCH_SIZE, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });
    let t = ui.ctx().animate_bool(response.id, *on);
    let off = Color32::from_rgb(0x2a, 0x31, 0x40);
    let track = lerp_color(off, TEAL, t);
    let knob = lerp_color(MUTED, KEY_EDGE, t);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), track);
    let travel = rect.width() - 6.0 - 16.0;
    let center = egui::pos2(rect.left() + 3.0 + 8.0 + travel * t, rect.center().y);
    painter.circle_filled(center, 8.0, knob);
    kelp_ui::widgets::focus_ring(ui, &response, 11.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

pub(crate) fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
    Color32::from_rgb(mix(from.r(), to.r()), mix(from.g(), to.g()), mix(from.b(), to.b()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Quiet,
    Primary,
}

fn button_text_width(ui: &Ui, label: &str) -> f32 {
    ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(14.0), TEXT).size().x
}

pub(crate) fn button_width(ui: &Ui, label: &str) -> f32 {
    button_text_width(ui, label) + 28.0
}

pub(crate) fn button(ui: &mut Ui, label: &str, tone: Tone) -> Response {
    let enabled = ui.is_enabled();
    let (text_color, font) = match tone {
        Tone::Primary => (KEY_EDGE, font_semibold(14.0)),
        Tone::Quiet => (TEXT, FontId::proportional(14.0)),
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, text_color);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(28.0, 16.0), Sense::click());
    let (fill, text_color) = match (tone, enabled, response.hovered()) {
        (Tone::Primary, true, true) => (lerp_color(TEAL, Color32::WHITE, 0.15), text_color),
        (Tone::Primary, true, false) => (TEAL, text_color),
        (Tone::Primary, false, _) => {
            (lerp_color(CARD, TEAL, 0.35), lerp_color(CARD, KEY_EDGE, 0.6))
        }
        (Tone::Quiet, true, true) => (lerp_color(BUTTON, Color32::WHITE, 0.08), text_color),
        (Tone::Quiet, true, false) => (BUTTON, text_color),
        (Tone::Quiet, false, _) => (BUTTON, FAINT),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(9), fill);
    painter.galley(rect.center() - galley.size() / 2.0, galley, text_color);
    kelp_ui::widgets::focus_ring(ui, &response, 9.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

pub(crate) fn choice_width(ui: &Ui, label: &str) -> f32 {
    let font = font_semibold(13.0);
    ui.painter().layout_no_wrap(label.to_owned(), font, SOFT).size().x + 28.0
}

pub(crate) fn choice(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let (text_color, font) =
        if selected { (KEY_EDGE, font_semibold(13.0)) } else { (SOFT, FontId::proportional(13.0)) };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, text_color);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(28.0, 14.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(8), TEAL);
    } else {
        let fill =
            if response.hovered() { lerp_color(GROUND, Color32::WHITE, 0.04) } else { GROUND };
        painter.rect_filled(rect, CornerRadius::same(8), fill);
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, FIELD_LINE),
            StrokeKind::Inside,
        );
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, text_color);
    kelp_ui::widgets::focus_ring(ui, &response, 8.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

pub(crate) fn text_input(ui: &mut Ui, value: &mut String, hint: &str) -> Response {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.extreme_bg_color = GROUND;
        visuals.selection.stroke = Stroke::new(1.0, TEAL);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, FIELD_LINE);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, FAINT);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.corner_radius = CornerRadius::same(9);
        }
        ui.style_mut().override_font_id = Some(FontId::proportional(14.0));
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(ui.available_width())
                .margin(Margin::symmetric(12, 10))
                .text_color(TEXT)
                .hint_text(RichText::new(hint).color(FAINT)),
        )
    })
    .inner
}

pub(crate) fn text_area(ui: &mut Ui, value: &mut String, hint: &str, rows: usize) -> bool {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.extreme_bg_color = GROUND;
        visuals.selection.stroke = Stroke::new(1.0, TEAL);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, FIELD_LINE);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, FAINT);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.corner_radius = CornerRadius::same(9);
        }
        ui.style_mut().override_font_id = Some(FontId::monospace(12.5));
        ui.add(
            egui::TextEdit::multiline(value)
                .desired_width(ui.available_width())
                .desired_rows(rows)
                .margin(Margin::symmetric(12, 10))
                .text_color(TEXT)
                .hint_text(RichText::new(hint).color(FAINT)),
        )
        .changed()
    })
    .inner
}

pub(crate) fn field(ui: &mut Ui, label: &str, value: &mut String, hint: &str) -> bool {
    ui.spacing_mut().item_spacing.y = 6.0;
    caption(ui, label);
    text_input(ui, value, hint).changed()
}

pub(crate) fn key_cap_width(ui: &Ui, text: &str, size: f32) -> f32 {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::monospace(size), TEXT);
    (galley.size().x + 16.0).max(26.0)
}

pub(crate) fn key_cap(ui: &mut Ui, text: &str, size: f32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::monospace(size), TEXT);
    let width = (galley.size().x + 16.0).max(26.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, galley.size().y + 8.0 + 2.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), KEY_EDGE);
    let face = Rect::from_min_max(rect.min, rect.max - vec2(0.0, 2.0));
    painter.rect_filled(face, CornerRadius::same(6), BUTTON);
    painter.galley(face.center() - galley.size() / 2.0, galley, TEXT);
    response
}

pub(crate) fn disclosure<R>(
    ui: &mut Ui,
    id: &str,
    label: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let id = ui.make_persistent_id(("disclosure", id));
    let mut open = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(14.0), SOFT);
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 22.0, galley.size().y + 8.0), Sense::click());
    if response.clicked() {
        open = !open;
        ui.data_mut(|data| data.insert_temp(id, open));
    }
    let color = if response.hovered() { TEXT } else { SOFT };
    let c = egui::pos2(rect.left() + 6.0, rect.center().y);
    let stroke = Stroke::new(1.6, color);
    let points = if open {
        [c + vec2(-3.5, -2.0), c + vec2(0.0, 2.0), c + vec2(3.5, -2.0)]
    } else {
        [c + vec2(-2.0, -3.5), c + vec2(2.0, 0.0), c + vec2(-2.0, 3.5)]
    };
    ui.painter().line_segment([points[0], points[1]], stroke);
    ui.painter().line_segment([points[1], points[2]], stroke);
    ui.painter().galley(
        egui::pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    kelp_ui::widgets::focus_ring(ui, &response, 6.0);
    response.on_hover_cursor(CursorIcon::PointingHand);
    open.then(|| add(ui))
}

pub(crate) fn badge(ui: &mut Ui, letter: &str, tint: (Color32, Color32), size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let radius = if size > 30.0 { 10 } else { 7 };
    let font = if size > 30.0 { font_semibold(18.0) } else { font_semibold(13.0) };
    ui.painter().rect_filled(rect, CornerRadius::same(radius), tint.0);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, letter, font, tint.1);
}

#[cfg(test)]
mod tests;
