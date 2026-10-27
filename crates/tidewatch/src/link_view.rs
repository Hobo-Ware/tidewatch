//! The Links card of the Progress panel, and the Read first lines of a question: what the agent
//! shared with the user, newest first, each row a kind tag, a title that opens the link and a copy
//! button. The rows are pure (`link_model`); this only draws them.

use egui::{Align2, FontId, OpenUrl, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use stdusk_egui::ui::icons;
use tidewatch_core::link_board::{Link, LinkBoard};

use crate::brand;
use crate::link_model::{self, ink, listed};
use crate::reader_view::{ellipsized, paint_item};
use crate::screen_kit::{
    Size, TEXT_BODY, TEXT_SECONDARY, TEXT_TAG, TagFill, body, heading, icon_button, paint_tag,
    tag_width,
};
use crate::selectable;

const LINK_ICON: &str = "\u{E2E2}";
const HEAD_H: f32 = 36.0;
const ROW_H: f32 = 44.0;
const FOLD_H: f32 = 36.0;
const PAD_X: f32 = 12.0;
const GAP: f32 = 10.0;
const ICON_PX: f32 = 14.0;
const MIN_TEXT_W: f32 = 24.0;
pub(crate) const READ_FIRST_H: f32 = 22.0;
pub(crate) const READ_FIRST_MAX: usize = 2;

/// The card: a header, the listed rows and the fold under them. Returns the URL whose copy button
/// was pressed. `all` is whether the fold is open; a press on it flips it.
pub(crate) fn show(
    ui: &mut Ui,
    width: f32,
    board: &LinkBoard,
    now_ms: i64,
    all: &mut bool,
) -> Option<String> {
    let rows = listed(board, *all);
    let count = board.len();
    let fold = link_model::fold_label(count, *all);
    let height = HEAD_H + ROW_H * rows.len() as f32 + fold.as_ref().map_or(0.0, |_| FOLD_H);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter().clone();
    paint_item(&painter, rect, false);
    head(ui, rect, count);
    let tag_col = tag_width(&painter, "Artifact");
    let mut copied = None;
    for (index, link) in rows.iter().enumerate() {
        let top = rect.top() + HEAD_H + ROW_H * index as f32;
        let row = Rect::from_min_size(pos2(rect.left(), top), vec2(width, ROW_H));
        painter.hline(row.x_range(), row.top(), Stroke::new(1.0, brand::ITEM_LINE));
        if row_of(ui, row, link, (index, tag_col), now_ms) {
            copied = Some(link.url.clone());
        }
    }
    if let Some(label) = fold {
        let top = rect.top() + HEAD_H + ROW_H * rows.len() as f32;
        let area = Rect::from_min_size(pos2(rect.left(), top), vec2(width, FOLD_H));
        painter.hline(area.x_range(), area.top(), Stroke::new(1.0, brand::ITEM_LINE));
        let galley = painter.layout_no_wrap(label, body(TEXT_SECONDARY), brand::TEAL_TEXT);
        let hit = Rect::from_min_size(
            pos2(area.left() + PAD_X, area.center().y - FOLD_H / 2.0),
            vec2(galley.size().x + 8.0, FOLD_H),
        );
        let response = ui.interact(hit, ui.id().with("links-fold"), Sense::click());
        let ink = if response.hovered() { brand::TEXT } else { brand::TEAL_TEXT };
        painter.galley(pos2(hit.left(), area.center().y - galley.size().y / 2.0), galley, ink);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            *all = !*all;
        }
    }
    copied
}

fn head(ui: &Ui, rect: Rect, count: usize) {
    let painter = ui.painter();
    let mid = rect.top() + HEAD_H / 2.0;
    painter.text(
        pos2(rect.left() + PAD_X, mid),
        Align2::LEFT_CENTER,
        LINK_ICON,
        FontId::proportional(ICON_PX),
        brand::MUTED,
    );
    let label = painter.layout_no_wrap("LINKS".to_owned(), heading(TEXT_SECONDARY), brand::SOFT);
    let left = rect.left() + PAD_X + ICON_PX + 8.0;
    painter.galley(pos2(left, mid - label.size().y / 2.0), label.clone(), brand::SOFT);
    paint_tag(
        painter,
        (left + label.size().x + 8.0, mid),
        &count.to_string(),
        brand::SOFT,
        TagFill::Tint(brand::ITEM_LINE),
    );
}

fn row_of(
    ui: &mut Ui,
    row: Rect,
    link: &Link,
    (index, tag_col): (usize, f32),
    now_ms: i64,
) -> bool {
    let painter = ui.painter().clone();
    let mid = row.center().y;
    let kind = link.kind();
    paint_tag(
        &painter,
        (row.left() + PAD_X, mid),
        kind.word(),
        ink(kind),
        TagFill::Line(ink(kind)),
    );
    let button = Size::Compact.height();
    let text_left = row.left() + PAD_X + tag_col + GAP;
    let button_left = row.right() - PAD_X - button;
    let text_w = (button_left - GAP - text_left).max(MIN_TEXT_W);
    let meta =
        ellipsized(&painter, &link_model::meta(link, now_ms), body(TEXT_TAG), brand::MUTED, text_w);
    let probe = ellipsized(&painter, &link.title, body(TEXT_BODY), brand::TEXT, text_w);
    let block = probe.size().y + meta.size().y + 1.0;
    let top = mid - block / 2.0;
    let hit = Rect::from_min_size(pos2(text_left, top), vec2(probe.size().x, probe.size().y));
    let response = ui.interact(hit, ui.id().with(("link-open", index, &link.url)), Sense::click());
    let ink_of_title = if response.hovered() { brand::TEAL_TEXT } else { brand::TEXT };
    let title = if response.hovered() {
        ellipsized(&painter, &link.title, body(TEXT_BODY), ink_of_title, text_w)
    } else {
        probe
    };
    painter.galley(pos2(text_left, top), title, ink_of_title);
    if response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&link.url).clicked() {
        ui.ctx().open_url(OpenUrl::new_tab(&link.url));
    }
    selectable::galley(ui, pos2(text_left, top + hit.height() + 1.0), meta, brand::MUTED);
    let at = Rect::from_min_size(pos2(button_left, mid - button / 2.0), vec2(button, button));
    let mut child = ui.new_child(UiBuilder::new().id_salt(("link-copy", index)).max_rect(at));
    icon_button(&mut child, icons::COPY, "Copy link", Size::Compact).clicked()
}

pub(crate) fn read_first_height(count: usize) -> f32 {
    READ_FIRST_H * count.min(READ_FIRST_MAX) as f32
}

/// The `Read first:` lines of a question, from `top_left` across `width`: the links sent for it,
/// at most two, each title opening its link.
pub(crate) fn paint_read_first(ui: &mut Ui, top_left: Pos2, width: f32, links: &[&Link]) {
    let painter = ui.painter().clone();
    for (index, link) in links.iter().take(READ_FIRST_MAX).enumerate() {
        let mid = top_left.y + READ_FIRST_H * index as f32 + READ_FIRST_H / 2.0;
        painter.text(
            pos2(top_left.x, mid),
            Align2::LEFT_CENTER,
            LINK_ICON,
            FontId::proportional(TEXT_SECONDARY),
            brand::MUTED,
        );
        let label =
            painter.layout_no_wrap("Read first:".to_owned(), body(TEXT_SECONDARY), brand::MUTED);
        let label_left = top_left.x + TEXT_SECONDARY + 6.0;
        painter.galley(pos2(label_left, mid - label.size().y / 2.0), label.clone(), brand::MUTED);
        let title_left = label_left + label.size().x + 6.0;
        let room = (top_left.x + width - title_left).max(MIN_TEXT_W);
        let probe = ellipsized(&painter, &link.title, body(TEXT_SECONDARY), brand::TEAL_TEXT, room);
        let hit = Rect::from_min_size(pos2(title_left, mid - probe.size().y / 2.0), probe.size());
        let response =
            ui.interact(hit, ui.id().with(("read-first", index, &link.url)), Sense::click());
        let underline =
            if response.hovered() { Stroke::new(1.0, brand::TEAL_TEXT) } else { Stroke::NONE };
        painter.galley_with_override_text_color(hit.min, probe, brand::TEAL_TEXT);
        painter.hline(hit.x_range(), hit.bottom(), underline);
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&link.url)
            .clicked()
        {
            ui.ctx().open_url(OpenUrl::new_tab(&link.url));
        }
    }
}
