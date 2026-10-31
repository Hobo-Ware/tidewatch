//! The pieces every Settings page is built from: palette, cards and rows, switch, buttons, inputs.

use egui::scroll_area::ScrollAreaOutput;
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Layout, Margin, Mesh, Painter,
    Rangef, Rect, Response, RichText, Sense, Stroke, Ui, UiBuilder, Vec2, epaint::Vertex, pos2,
    vec2,
};
use kelp_ui::theme::semibold;

use crate::screen_kit::TEXT_BODY;
use crate::screen_kit::TEXT_SECONDARY;
use crate::screen_kit::{self, Size, danger_button, primary_button, quiet_button};
use crate::screen_kit::{TagFill, paint_tag, tag_width};
use crate::segmented;

pub(crate) mod editor;

pub(crate) use crate::brand::*;

const SECTION_GAP: f32 = 22.0;
const ROW_GAP: f32 = 16.0;
const SWITCH_SIZE: Vec2 = vec2(40.0, 22.0);
const INLINE_MAX: usize = 5;
pub(crate) const HEADER_H: f32 = 48.0;

pub(crate) fn heading(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(semibold()).size(size).color(TEXT)
}

pub(crate) fn font_semibold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub(crate) fn caption(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(13.0).color(MUTED));
}

/// A caption that closes a card: set off by a divider and inset like the rows above it.
pub(crate) fn card_caption(ui: &mut Ui, text: &str) {
    divider(ui);
    Frame::new().inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| caption(ui, text));
}

pub(crate) fn note(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(RichText::new(text).size(13.0).color(color));
}

pub(crate) fn page_header(ui: &mut Ui, title: &str, subtitle: &str) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let divider = ui.clip_rect().x_range();
    header_at(ui, rect, rect.right(), divider, title, subtitle);
    response
}

/// The page header drawn in `bar`: the title and its subtitle clipped at `text_right`, and the
/// divider under the bar across `divider`. Returns the title's rect.
pub(crate) fn header_at(
    ui: &Ui,
    bar: Rect,
    text_right: f32,
    divider: Rangef,
    title: &str,
    subtitle: &str,
) -> Rect {
    let painter = ui.painter_at(Rect::from_min_max(bar.min, pos2(text_right, bar.bottom())));
    let title =
        painter.text(bar.left_center(), Align2::LEFT_CENTER, title, font_semibold(16.0), TEXT);
    painter.text(
        pos2(title.right() + 14.0, bar.center().y + 1.0),
        Align2::LEFT_CENTER,
        subtitle,
        FontId::proportional(13.0),
        MUTED,
    );
    ui.painter().hline(divider, bar.bottom(), Stroke::new(1.0, CARD_LINE));
    title
}

pub(crate) fn footer_note(ui: &mut Ui, text: &str) {
    ui.add_space(SECTION_GAP);
    ui.label(RichText::new(text).monospace().size(13.0).color(FAINT));
}

pub(crate) fn section<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(SECTION_GAP);
    ui.label(heading(title, 14.0).color(SOFT));
    ui.add_space(8.0);
    card(ui, add)
}

pub(crate) fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_LINE))
        .corner_radius(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            add(ui)
        })
        .inner
}

pub(crate) fn divider(ui: &Ui) {
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

    pub(crate) fn button(ui: &Ui, label: &str, size: Size) -> Self {
        Self { width: button_width(ui, label, size), height: size.height() }
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
            ui.label(RichText::new(title).size(TEXT_BODY).color(TEXT));
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
    let track = lerp_color(FIELD_LINE, TEAL, t);
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
    Danger,
}

pub(crate) fn button_width(ui: &Ui, label: &str, size: Size) -> f32 {
    screen_kit::button_width(ui, label, size)
}

pub(crate) fn button(ui: &mut Ui, label: &str, tone: Tone, size: Size) -> Response {
    let enabled = ui.is_enabled();
    match tone {
        Tone::Primary => primary_button(ui, label, enabled, size),
        Tone::Quiet => quiet_button(ui, label, enabled, size),
        Tone::Danger => danger_button(ui, label, size),
    }
}

pub(crate) fn choice_width(ui: &Ui, label: &str) -> f32 {
    let font = font_semibold(13.0);
    ui.painter().layout_no_wrap(label.to_owned(), font, SOFT).size().x + 28.0
}

pub(crate) fn inline_width(ui: &Ui, labels: &[&str]) -> f32 {
    labels.iter().map(|label| choice_width(ui, label)).sum::<f32>()
}

/// Whether `labels` fit one segmented control in the row the next widget lands in.
pub(crate) fn fits_inline(ui: &Ui, labels: &[&str]) -> bool {
    labels.len() <= INLINE_MAX && inline_width(ui, labels) <= ui.available_width()
}

/// The either/or control: a segmented control, or wrapping chips when there are too many options.
/// `active` is `None` when the current value is none of `labels`. Returns the option clicked.
pub(crate) fn pick(ui: &mut Ui, id: &str, labels: &[&str], active: Option<usize>) -> Option<usize> {
    let id = ui.id().with(("pick", id));
    let active = active.unwrap_or(usize::MAX);
    if fits_inline(ui, labels) {
        segmented::inline(ui, id, labels, active)
    } else {
        segmented::wrap_chips(ui, id, labels, active)
    }
}

/// A small rounded label in `ink`, for a state such as clean or dirty.
pub(crate) fn pill(ui: &mut Ui, text: &str, ink: Color32) {
    let width = tag_width(ui.painter(), text);
    let (rect, _) = ui.allocate_exact_size(vec2(width, Size::Tag.height()), Sense::hover());
    paint_tag(
        ui.painter(),
        (rect.left(), rect.center().y),
        text,
        ink,
        TagFill::Tint(ink.gamma_multiply(0.14)),
    );
}

const FADE_H: f32 = 28.0;

/// Fades the edges of a scroll area that has more content beyond them into `ground`.
pub(crate) fn fade_edges<R>(ui: &Ui, list: &ScrollAreaOutput<R>, ground: Color32) {
    let view = list.inner_rect;
    let below = list.content_size.y - list.state.offset.y - view.height() > 1.0;
    if below {
        fade(
            ui.painter(),
            Rect::from_min_max(pos2(view.left(), view.bottom() - FADE_H), view.max),
            ground,
            true,
        );
    }
    if list.state.offset.y > 1.0 {
        fade(
            ui.painter(),
            Rect::from_min_max(view.min, pos2(view.right(), view.top() + FADE_H)),
            ground,
            false,
        );
    }
}

fn fade(painter: &Painter, rect: Rect, ground: Color32, to_bottom: bool) {
    let clear = Color32::TRANSPARENT;
    let (top, bottom) = if to_bottom { (clear, ground) } else { (ground, clear) };
    let vertex = |pos, color| Vertex { pos, uv: egui::epaint::WHITE_UV, color };
    let mesh = Mesh {
        vertices: vec![
            vertex(rect.left_top(), top),
            vertex(rect.right_top(), top),
            vertex(rect.right_bottom(), bottom),
            vertex(rect.left_bottom(), bottom),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        ..Mesh::default()
    };
    painter.add(mesh);
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
        ui.style_mut().override_font_id = Some(FontId::monospace(TEXT_SECONDARY));
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
