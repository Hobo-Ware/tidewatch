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
const ROW_H_STACKED: f32 = 68.0;
const TAG_LINE_H: f32 = Size::Compact.height();
const MIN_STACK_TEXT_W: f32 = 90.0;
const FOLD_H: f32 = 36.0;
const PAD_X: f32 = 12.0;
const GAP: f32 = 10.0;
const ICON_PX: f32 = 14.0;
const CHEVRON: f32 = 14.0;
const MIN_TEXT_W: f32 = 24.0;
const MIN_TITLE_BESIDE_TAG: f32 = 80.0;
pub(crate) const READ_FIRST_H: f32 = 22.0;
pub(crate) const READ_FIRST_MAX: usize = 2;

/// Whether the rows put the tag and copy button on a line above the title: in a narrow panel, or
/// when the title would get under 90 px beside them.
fn stacks(painter: &egui::Painter, width: f32, stack: bool) -> bool {
    let beside =
        width - 2.0 * PAD_X - tag_width(painter, "Artifact") - Size::Compact.height() - 2.0 * GAP;
    stack || beside < MIN_STACK_TEXT_W
}

pub(crate) fn show(
    ui: &mut Ui,
    width: f32,
    stack: bool,
    board: &LinkBoard,
    now_ms: i64,
    (all, archive): (&mut bool, &mut bool),
    age: Option<&str>,
) -> Option<String> {
    let (main, archived) = link_model::split(board);
    if link_model::fold_label(main.len(), true).is_none() {
        *all = false;
    }
    if archived.is_empty() {
        *archive = false;
    }
    let rows = listed(&main, *all);
    let stacked = stacks(ui.painter(), width, stack);
    let row_h = if stacked { ROW_H_STACKED } else { ROW_H };
    let (head_h, age_below) = head_height(ui.painter(), width, main.len(), age);
    let fold = link_model::fold_label(main.len(), *all);
    let group = link_model::archive_label(archived.len());
    let archived_rows = if *archive { archived.as_slice() } else { &[] };
    let height = head_h
        + row_h * (rows.len() + archived_rows.len()) as f32
        + FOLD_H * (usize::from(fold.is_some()) + usize::from(group.is_some())) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter().clone();
    paint_item(&painter, rect, false);
    head(ui, rect, main.len(), age, age_below);
    let tag_col = tag_width(&painter, "Artifact");
    let mut copied = None;
    let mut top = rect.top() + head_h;
    let mut place = |ui: &mut Ui, links: &[&Link], first: usize, top: &mut f32| {
        for (at, link) in links.iter().enumerate() {
            let row = Rect::from_min_size(pos2(rect.left(), *top), vec2(width, row_h));
            painter.hline(row.x_range(), row.top(), Stroke::new(1.0, brand::ITEM_LINE));
            if row_of(ui, row, link, (first + at, tag_col), (now_ms, stacked)) {
                copied = Some(link.url.clone());
            }
            *top += row_h;
        }
    };
    place(ui, rows, 0, &mut top);
    if let Some(label) = fold {
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
        top += FOLD_H;
    }
    if let Some(label) = group {
        let area = Rect::from_min_size(pos2(rect.left(), top), vec2(width, FOLD_H));
        painter.hline(area.x_range(), area.top(), Stroke::new(1.0, brand::ITEM_LINE));
        if archive_header(ui, area, &label, *archive) {
            *archive = !*archive;
        }
        top += FOLD_H;
    }
    place(ui, archived_rows, rows.len(), &mut top);
    copied
}

fn archive_header(ui: &Ui, area: Rect, label: &str, open: bool) -> bool {
    let painter = ui.painter();
    let response = ui.interact(area, ui.id().with("links-archive"), Sense::click());
    let ink = if response.hovered() { brand::TEXT } else { brand::SOFT };
    let mid = area.center().y;
    let caret = pos2(area.left() + PAD_X + CHEVRON / 2.0, mid);
    let stroke = Stroke::new(1.5, ink);
    let half = CHEVRON / 3.0;
    let points = if open {
        [
            caret + vec2(-half, -half / 2.0),
            caret + vec2(0.0, half / 2.0),
            caret + vec2(half, -half / 2.0),
        ]
    } else {
        [
            caret + vec2(-half / 2.0, -half),
            caret + vec2(half / 2.0, 0.0),
            caret + vec2(-half / 2.0, half),
        ]
    };
    painter.line_segment([points[0], points[1]], stroke);
    painter.line_segment([points[1], points[2]], stroke);
    let galley = painter.layout_no_wrap(label.to_owned(), body(TEXT_SECONDARY), ink);
    let left = area.left() + PAD_X + CHEVRON + 6.0;
    painter.galley(pos2(left, mid - galley.size().y / 2.0), galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The header's height, and whether the age goes on a line of its own because the label, count
/// and age do not fit side by side.
fn head_height(
    painter: &egui::Painter,
    width: f32,
    count: usize,
    age: Option<&str>,
) -> (f32, bool) {
    let Some(age) = age else { return (HEAD_H, false) };
    let label = painter.layout_no_wrap("LINKS".to_owned(), heading(TEXT_SECONDARY), brand::SOFT);
    let used =
        PAD_X + ICON_PX + 8.0 + label.size().x + 8.0 + tag_width(painter, &count.to_string());
    let age_w = crate::panel_blocks::age_width(painter, age);
    if used + 12.0 + age_w + PAD_X <= width { (HEAD_H, false) } else { (HEAD_H + 22.0, true) }
}

fn head(ui: &Ui, rect: Rect, count: usize, age: Option<&str>, age_below: bool) {
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
    if let Some(age) = age {
        if age_below {
            let galley = painter.layout_no_wrap(age.to_owned(), body(TEXT_TAG), brand::MUTED);
            painter.galley(
                pos2(rect.left() + PAD_X, mid + HEAD_H / 2.0 - 4.0),
                galley,
                brand::MUTED,
            );
        } else {
            crate::panel_blocks::paint_age(painter, rect.right() - PAD_X, mid, age);
        }
    }
}

fn row_of(
    ui: &mut Ui,
    row: Rect,
    link: &Link,
    (index, tag_col): (usize, f32),
    (now_ms, stacked): (i64, bool),
) -> bool {
    let painter = ui.painter().clone();
    let kind = link.kind();
    let button = Size::Compact.height();
    let button_left = row.right() - PAD_X - button;
    let (mid, text_left, mut text_right, tag_mid) = if stacked {
        let line = row.top() + 6.0 + TAG_LINE_H / 2.0;
        (
            row.top() + 6.0 + TAG_LINE_H + 4.0 + (row.height() - TAG_LINE_H - 14.0) / 2.0,
            row.left() + PAD_X,
            row.right() - PAD_X,
            line,
        )
    } else {
        let mid = row.center().y;
        (mid, row.left() + PAD_X + tag_col + GAP, button_left - GAP, mid)
    };
    paint_tag(
        &painter,
        (row.left() + PAD_X, tag_mid),
        kind.word(),
        ink(kind),
        TagFill::Line(ink(kind)),
    );
    let mut meta = link_model::meta(link, now_ms);
    if let Some(word) = link_model::state_tag(link) {
        let left = text_right - tag_width(&painter, word);
        if stacked {
            let tag_end = row.left() + PAD_X + tag_width(&painter, kind.word()) + GAP;
            let at = button_left - GAP - tag_width(&painter, word);
            if at >= tag_end {
                paint_tag(
                    &painter,
                    (at, tag_mid),
                    word,
                    brand::MUTED,
                    TagFill::Line(brand::ITEM_LINE),
                );
            } else {
                meta = format!("{word} \u{b7} {meta}");
            }
        } else if left - GAP - text_left >= MIN_TITLE_BESIDE_TAG {
            paint_tag(&painter, (left, mid), word, brand::MUTED, TagFill::Line(brand::ITEM_LINE));
            text_right = left - GAP;
        } else {
            meta = format!("{word} \u{b7} {meta}");
        }
    }
    let text_w = (text_right - text_left).max(MIN_TEXT_W);
    let meta = ellipsized(&painter, &meta, body(TEXT_TAG), brand::MUTED, text_w);
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
    let button_mid = if stacked { tag_mid } else { mid };
    let at =
        Rect::from_min_size(pos2(button_left, button_mid - button / 2.0), vec2(button, button));
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
