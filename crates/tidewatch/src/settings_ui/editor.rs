//! The pieces of a Settings page that edits a list: list rows, editor card, option cards, save bar.

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Layout, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::{
    BAR, CARD, CARD_LINE, FIELD_LINE, GROUND, MUTED, ORANGE, RED, RED_FILL, RED_LINE, SOFT, TEAL,
    TEAL_FILL, TEAL_TEXT, TEXT, Tone, badge, button, button_width, font_semibold, heading,
    lerp_color,
};

const CARD_PAD: f32 = 10.0;

pub(crate) fn option_card_height(ui: &Ui, width: f32, detail: &str) -> f32 {
    let title = ui.painter().layout_no_wrap("Ag".to_owned(), font_semibold(13.0), TEXT);
    let detail = ui.painter().layout(
        detail.to_owned(),
        FontId::proportional(12.0),
        MUTED,
        width - 2.0 * CARD_PAD,
    );
    2.0 * CARD_PAD + title.size().y + 3.0 + detail.size().y
}

pub(crate) fn option_card(
    ui: &mut Ui,
    size: Vec2,
    title: &str,
    detail: &str,
    selected: bool,
    danger: bool,
) -> Response {
    let (title_color, line, fill) = match (selected, danger) {
        (true, false) => (TEAL_TEXT, TEAL, TEAL_FILL),
        (true, true) => (RED, RED, RED_FILL),
        (false, false) => (TEXT, FIELD_LINE, GROUND),
        (false, true) => (RED, RED_LINE, GROUND),
    };
    let title_galley = ui.painter().layout_no_wrap(
        title.to_owned(),
        if selected { font_semibold(13.0) } else { FontId::proportional(13.0) },
        title_color,
    );
    let detail_galley = ui.painter().layout(
        detail.to_owned(),
        FontId::proportional(12.0),
        MUTED,
        size.x - 2.0 * CARD_PAD,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill =
        if response.hovered() && !selected { lerp_color(fill, Color32::WHITE, 0.04) } else { fill };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), fill);
    painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, line), StrokeKind::Inside);
    let title_pos = rect.min + vec2(CARD_PAD, CARD_PAD);
    let detail_pos = title_pos + vec2(0.0, title_galley.size().y + 3.0);
    painter.galley(title_pos, title_galley, title_color);
    painter.galley(detail_pos, detail_galley, MUTED);
    kelp_ui::widgets::focus_ring(ui, &response, 10.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Bar<'a> {
    pub(crate) dirty: bool,
    pub(crate) saving: bool,
    pub(crate) error: Option<&'a str>,
    pub(crate) can_revert: bool,
    pub(crate) file: &'a str,
}

impl Bar<'_> {
    pub(crate) fn shows(&self) -> bool {
        self.dirty || self.saving || self.error.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarAction {
    Save,
    Revert,
}

pub(crate) const BAR_HEIGHT: f32 = 58.0;

pub(crate) fn save_bar(ui: &mut Ui, bar: &Bar) -> Option<BarAction> {
    let rect = ui.available_rect_before_wrap();
    let rect = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_HEIGHT));
    let corner = CornerRadius { nw: 0, ne: 0, sw: 13, se: 13 };
    ui.painter().rect_filled(rect, corner, BAR);
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, CARD_LINE));
    let mut action = None;
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(20.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 10.0;
    let (dot_rect, _) = inner.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
    let (dot, text, color) = match (bar.error, bar.saving) {
        (Some(error), _) => (RED, error, RED),
        (None, true) => (TEAL, "Saving...", SOFT),
        (None, false) => (ORANGE, "Unsaved changes", SOFT),
    };
    inner.painter().circle_filled(dot_rect.center(), 4.0, dot);
    inner.label(RichText::new(text).size(13.0).color(color));
    inner.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let can_save = bar.dirty && !bar.saving;
        let save = ui.add_enabled_ui(can_save, |ui| button(ui, "Save", Tone::Primary)).inner;
        let save = save.on_hover_text(format!(
            "Writes {}. Comments in the file are not kept.",
            if bar.file.is_empty() { "the file" } else { bar.file }
        ));
        if save.clicked() {
            action = Some(BarAction::Save);
        }
        if bar.can_revert
            && ui
                .add_enabled_ui(!bar.saving, |ui| button(ui, "Revert", Tone::Quiet))
                .inner
                .clicked()
        {
            action = Some(BarAction::Revert);
        }
    });
    ui.allocate_rect(rect, Sense::hover());
    action
}

pub(crate) struct Item<'a> {
    pub(crate) badge: Option<(&'a str, (Color32, Color32))>,
    pub(crate) name: &'a str,
    pub(crate) tag: &'a str,
    pub(crate) tag_color: Color32,
    pub(crate) selected: bool,
    pub(crate) dimmed: bool,
}

pub(crate) fn list_item(ui: &mut Ui, item: &Item) -> Response {
    ui.scope(|ui| {
        if item.dimmed {
            ui.set_opacity(0.55);
        }
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
        let painter = ui.painter();
        if item.selected {
            painter.rect_filled(rect, CornerRadius::same(10), TEAL_FILL);
            painter.rect_stroke(
                rect,
                CornerRadius::same(10),
                Stroke::new(1.0, TEAL),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(
                rect,
                CornerRadius::same(10),
                Color32::from_rgba_unmultiplied(255, 255, 255, 8),
            );
        }
        let mut name_x = rect.left() + 12.0;
        if let Some((letter, tint)) = item.badge {
            let badge =
                Rect::from_center_size(pos2(name_x + 13.0, rect.center().y), vec2(26.0, 26.0));
            painter.rect_filled(badge, CornerRadius::same(7), tint.0);
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                letter,
                font_semibold(13.0),
                tint.1,
            );
            name_x = badge.right() + 10.0;
        }
        painter.text(
            pos2(name_x, rect.center().y),
            Align2::LEFT_CENTER,
            item.name,
            FontId::proportional(15.0),
            TEXT,
        );
        painter.text(
            pos2(rect.right() - 12.0, rect.center().y),
            Align2::RIGHT_CENTER,
            item.tag,
            FontId::proportional(12.0),
            item.tag_color,
        );
        kelp_ui::widgets::focus_ring(ui, &response, 10.0);
        response.on_hover_cursor(CursorIcon::PointingHand)
    })
    .inner
}

const CARD_HEADER: f32 = 76.0;

pub(crate) struct CardRects {
    pub(crate) header: Rect,
    pub(crate) body: Rect,
    pub(crate) bar: Rect,
}

pub(crate) fn split_card(ui: &Ui, rect: Rect, bar: bool) -> CardRects {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(14), CARD);
    painter.rect_stroke(
        rect,
        CornerRadius::same(14),
        Stroke::new(1.0, CARD_LINE),
        StrokeKind::Inside,
    );
    let content = rect.shrink(1.0);
    let header = Rect::from_min_size(content.min, vec2(content.width(), CARD_HEADER));
    painter.hline(content.x_range(), header.bottom(), Stroke::new(1.0, CARD_LINE));
    let bar_height = if bar { BAR_HEIGHT } else { 0.0 };
    let body = Rect::from_min_max(
        pos2(content.left(), header.bottom() + 1.0),
        pos2(content.right(), (content.bottom() - bar_height).max(header.bottom() + 1.0)),
    );
    let bar = Rect::from_min_max(pos2(content.left(), body.bottom()), content.max);
    CardRects { header, body, bar }
}

pub(crate) fn card_header(
    ui: &mut Ui,
    header: Rect,
    badge_look: Option<(&str, (Color32, Color32))>,
    name: &str,
    line: &str,
) -> bool {
    let mut left = ui.new_child(
        UiBuilder::new()
            .max_rect(header.shrink2(vec2(20.0, 18.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    left.spacing_mut().item_spacing.x = 14.0;
    if let Some((letter, tint)) = badge_look {
        badge(&mut left, letter, tint, 40.0);
    }
    left.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(heading(name, 20.0));
        ui.label(RichText::new(line).size(13.0).color(MUTED));
    });
    let width = button_width(ui, "Delete");
    let at = Rect::from_center_size(
        pos2(header.right() - 20.0 - width / 2.0, header.center().y),
        vec2(width, 34.0),
    );
    let mut right = ui.new_child(UiBuilder::new().max_rect(at));
    button(&mut right, "Delete", Tone::Quiet).clicked()
}

const LIST_WIDTH: f32 = 300.0;
const COLUMN_GAP: f32 = 20.0;

pub(crate) fn page_area(area: Rect) -> Rect {
    Rect::from_min_max(area.min + vec2(32.0, 28.0), area.max - vec2(32.0, 24.0))
}

/// The page title with its button at the right. Returns where the header ends and whether the
/// button was clicked. The title carries the file the page edits as a tooltip.
pub(crate) fn page_top(
    ui: &mut Ui,
    inner: Rect,
    title: &str,
    subtitle: &str,
    file: &str,
    action: &str,
) -> (f32, bool) {
    let width = button_width(ui, action);
    let mut left =
        ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    super::page_header(&mut left, title, subtitle).on_hover_text(file);
    let bottom = left.min_rect().bottom();
    let at = Rect::from_min_size(pos2(inner.right() - width, bottom - 34.0), vec2(width, 34.0));
    let mut right = ui.new_child(UiBuilder::new().max_rect(at));
    (bottom, button(&mut right, action, Tone::Quiet).clicked())
}

pub(crate) fn columns(inner: Rect, header_bottom: f32) -> (Rect, Rect) {
    let top = header_bottom + 18.0;
    let list = Rect::from_min_size(
        pos2(inner.left(), top),
        vec2(LIST_WIDTH, (inner.bottom() - top).max(0.0)),
    );
    let card = Rect::from_min_max(pos2(list.right() + COLUMN_GAP, top), inner.max);
    (list, card)
}
