//! The pieces of a Settings page that edits a list: list rows, editor card, option cards, save bar.

use egui::RectAlign;
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Layout, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use stdusk_egui::widgets::style_menu;

use super::{
    BAR, CARD, CARD_LINE, HEADER_H, ITEM, ITEM_LINE, MUTED, ORANGE, RED, SOFT, TEAL, TEXT, Tone,
    badge, button, button_width, font_semibold, header_at, heading,
};
use crate::screen_kit::TEXT_BODY;
use crate::screen_kit::{Glyph, Size, glyph_button, selection_row};

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
    let corner = CornerRadius { nw: 0, ne: 0, sw: CARD_RADIUS - 1, se: CARD_RADIUS - 1 };
    ui.painter().rect_filled(rect, corner, BAR);
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, CARD_LINE));
    let mut action = None;
    let inner_rect = rect.shrink2(vec2(20.0, 0.0));
    let buttons_w = button_width(ui, "Save", Size::Regular)
        + if bar.can_revert { 10.0 + button_width(ui, "Revert", Size::Regular) } else { 0.0 };
    let mut text_room = inner_rect;
    text_room.max.x -= buttons_w + 10.0;
    let mut message = ui.new_child(
        UiBuilder::new().max_rect(text_room).layout(Layout::left_to_right(Align::Center)),
    );
    message.spacing_mut().item_spacing.x = 10.0;
    let (dot_rect, _) = message.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
    let (dot, text, color) = match (bar.error, bar.saving) {
        (Some(error), _) => (RED, error, RED),
        (None, true) => (TEAL, "Saving...", SOFT),
        (None, false) => (ORANGE, "Unsaved changes", SOFT),
    };
    message.painter().circle_filled(dot_rect.center(), 4.0, dot);
    message
        .add(egui::Label::new(RichText::new(text).size(13.0).color(color)).truncate())
        .on_hover_text(text);
    let mut inner = ui.new_child(
        UiBuilder::new().max_rect(inner_rect).layout(Layout::left_to_right(Align::Center)),
    );
    inner.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let can_save = bar.dirty && !bar.saving;
        let save = ui
            .add_enabled_ui(can_save, |ui| button(ui, "Save", Tone::Primary, Size::Regular))
            .inner;
        let save = save.on_hover_text(format!(
            "Writes {}. Comments in the file are not kept.",
            if bar.file.is_empty() { "the file" } else { bar.file }
        ));
        if save.clicked() {
            action = Some(BarAction::Save);
        }
        if bar.can_revert
            && ui
                .add_enabled_ui(!bar.saving, |ui| button(ui, "Revert", Tone::Quiet, Size::Regular))
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
        if !item.selected {
            painter.rect(
                rect,
                CornerRadius::same(8),
                ITEM,
                Stroke::new(1.0, ITEM_LINE),
                StrokeKind::Inside,
            );
        }
        selection_row(painter, rect, item.selected, response.hovered());
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
            FontId::proportional(TEXT_BODY),
            TEXT,
        );
        painter.text(
            pos2(rect.right() - 12.0, rect.center().y),
            Align2::RIGHT_CENTER,
            item.tag,
            FontId::proportional(12.0),
            item.tag_color,
        );
        kelp_ui::widgets::focus_ring(ui, &response, 8.0);
        response.on_hover_cursor(CursorIcon::PointingHand)
    })
    .inner
}

const CARD_HEADER: f32 = 76.0;
const MENU_W: f32 = 180.0;
const CARD_RADIUS: u8 = 12;

pub(crate) struct CardRects {
    pub(crate) header: Rect,
    pub(crate) body: Rect,
    pub(crate) bar: Rect,
}

pub(crate) fn split_card(ui: &Ui, rect: Rect, bar: bool) -> CardRects {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(CARD_RADIUS), CARD);
    painter.rect_stroke(
        rect,
        CornerRadius::same(CARD_RADIUS),
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

/// The card's header: the face, the name and a line under it on the left, and a `...` button at
/// the right edge whose menu holds Delete. Returns whether Delete was picked.
pub(crate) fn card_header(
    ui: &mut Ui,
    header: Rect,
    badge_look: Option<(&str, (Color32, Color32))>,
    name: &str,
    line: &str,
) -> bool {
    let more_w = Size::Compact.height();
    let mut room = header.shrink2(vec2(20.0, 18.0));
    room.max.x -= more_w + 14.0;
    let mut left =
        ui.new_child(UiBuilder::new().max_rect(room).layout(Layout::left_to_right(Align::Center)));
    left.spacing_mut().item_spacing.x = 14.0;
    if let Some((letter, tint)) = badge_look {
        badge(&mut left, letter, tint, 40.0);
    }
    left.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(egui::Label::new(heading(name, 20.0)).truncate());
        ui.add(egui::Label::new(RichText::new(line).size(13.0).color(MUTED)).truncate())
            .on_hover_text(line);
    });
    let at = Rect::from_center_size(
        pos2(header.right() - 20.0 - more_w / 2.0, header.center().y),
        vec2(more_w, more_w),
    );
    let mut right = ui.new_child(UiBuilder::new().max_rect(at));
    let more = glyph_button(&mut right, Glyph::More, "More for this profile", Size::Compact);
    let mut delete = false;
    let asked = ui.id().with("dev_profile_menu");
    if ui.data(|data| data.get_temp::<bool>(asked)).is_none() {
        ui.data_mut(|data| data.insert_temp(asked, true));
        if std::env::var("TIDEWATCH_DEV_PROFILE_MENU").is_ok_and(|v| v == "1") {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&more));
        }
    }
    egui::Popup::menu(&more).align(RectAlign::BOTTOM_END).gap(4.0).width(MENU_W).show(|ui| {
        style_menu(ui);
        let row = egui::Button::new(RichText::new("Delete").color(RED))
            .min_size(vec2(ui.available_width(), 0.0));
        if ui.add(row).clicked() {
            delete = true;
            ui.close();
        }
    });
    delete
}

const LIST_WIDTH: f32 = 300.0;
const PAGE_BOTTOM: f32 = 24.0;
const COLUMN_GAP: f32 = 20.0;

/// The page's column: the same centered column every Settings page keeps, from the top of `area`.
pub(crate) fn page_area(area: Rect) -> Rect {
    let column = crate::settings_view::content_column(area);
    Rect::from_min_max(column.min, column.max - vec2(0.0, PAGE_BOTTOM))
}

/// The page title with its button at the right, in a 48 px header over a divider across `area`.
/// Returns where the header ends and whether the button was clicked. The header carries the file
/// the page edits as a tooltip.
pub(crate) fn page_top(
    ui: &mut Ui,
    area: Rect,
    inner: Rect,
    (title, subtitle, file): (&str, &str, &str),
    action: &str,
) -> (f32, bool) {
    let bar = Rect::from_min_size(inner.min, vec2(inner.width(), HEADER_H));
    let width = button_width(ui, action, Size::Compact);
    let title_rect =
        header_at(ui, bar, bar.right() - width - 14.0, area.x_range(), title, subtitle);
    ui.interact(title_rect, ui.id().with("page_top_title"), Sense::hover()).on_hover_text(file);
    let at = Rect::from_center_size(
        pos2(bar.right() - width / 2.0, bar.center().y),
        vec2(width, Size::Compact.height()),
    );
    let mut right = ui.new_child(UiBuilder::new().max_rect(at));
    (bar.bottom(), button(&mut right, action, Tone::Quiet, Size::Compact).clicked())
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
