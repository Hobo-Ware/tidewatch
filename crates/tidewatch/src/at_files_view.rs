//! The `@` file list above a message box: the keys that work it and the card that shows it, in the
//! look of the emoji list.

use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Area, CornerRadius, FontId, Frame, Id, Key, Margin, Modifiers, Order, Pos2, Rect,
    Sense, Stroke, Ui, pos2, vec2,
};

use crate::at_files::{self, Hit, MAX_ROWS, Source};
use crate::brand;
use crate::emoji_pick::{self, Menu, Pressed, Query};
use crate::screen_kit::{TEXT_BODY, heading, selected_ink, selection_row};

pub(crate) const ROW_H: f32 = 34.0;
pub(crate) const WIDTH: f32 = 420.0;
const GAP: f32 = 8.0;
const PAD: i8 = 6;
const ROW_PAD: f32 = 12.0;

fn menu_id(id: Id) -> Id {
    id.with("at-files-menu")
}

fn cache_id(id: Id) -> Id {
    id.with("at-files-hits")
}

pub(crate) fn load(ui: &Ui, id: Id) -> Menu {
    ui.data(|data| data.get_temp(menu_id(id))).unwrap_or_default()
}

pub(crate) fn store(ui: &Ui, id: Id, menu: Menu) {
    ui.data_mut(|data| data.insert_temp(menu_id(id), menu));
}

type Cached = (String, usize, usize, Arc<Vec<Hit>>);

/// The rows for `query` while the list is open, kept until the word or the file list changes.
pub(crate) fn hits(
    ui: &Ui,
    id: Id,
    menu: &Menu,
    (query, files): (Option<&Query>, Option<Source<'_>>),
) -> Arc<Vec<Hit>> {
    let (Some(query), Some(files)) = (query, files) else { return Arc::default() };
    if !menu.is_open(Some(query)) {
        return Arc::default();
    }
    let key = (query.term.clone(), files.paths.len(), files.paths.as_ptr() as usize);
    let cached: Option<Cached> = ui.data(|data| data.get_temp(cache_id(id)));
    if let Some((term, len, ptr, rows)) = cached
        && (term.as_str(), len, ptr) == (key.0.as_str(), key.1, key.2)
    {
        return rows;
    }
    let rows = Arc::new(at_files::find(&query.term, files.paths, MAX_ROWS));
    ui.data_mut(|data| {
        data.insert_temp::<Cached>(cache_id(id), (key.0, key.1, key.2, rows.clone()))
    });
    rows
}

fn consume(ui: &Ui, key: Key) -> bool {
    ui.input_mut(|input| input.consume_key(Modifiers::NONE, key))
}

/// Up, Down, Enter, Tab and Esc of an open list. Returns the cursor, in chars, when a pick put a
/// token into `text`.
pub(crate) fn keys(
    ui: &Ui,
    text: &mut String,
    menu: &mut Menu,
    (rows, query, root): (&[Hit], &Query, &std::path::Path),
    cursor: usize,
) -> Option<usize> {
    let pressed = if consume(ui, Key::Escape) {
        emoji_pick::Key::Close
    } else if consume(ui, Key::ArrowDown) {
        emoji_pick::Key::Down
    } else if consume(ui, Key::ArrowUp) {
        emoji_pick::Key::Up
    } else if consume(ui, Key::Enter) || consume(ui, Key::Tab) {
        emoji_pick::Key::Accept
    } else {
        return None;
    };
    match menu.press(pressed, rows.len()) {
        Pressed::Accepted(at) => rows.get(at).map(|hit| pick(text, query, cursor, root, hit)),
        Pressed::Moved | Pressed::Closed => None,
    }
}

pub(crate) fn pick(
    text: &mut String,
    query: &Query,
    cursor: usize,
    root: &std::path::Path,
    hit: &Hit,
) -> usize {
    let byte = text.char_indices().nth(cursor).map_or(text.len(), |(byte, _)| byte);
    at_files::pick(text, query, byte, &at_files::target(root, hit))
}

fn row_job(hit: &Hit, selected: bool) -> LayoutJob {
    let name_at = hit.name_start();
    let ink = selected_ink(selected, brand::TEXT);
    let mut job = LayoutJob::default();
    for (index, c) in hit.path.chars().enumerate() {
        let lit = hit.lit.contains(&index);
        let in_name = index >= hit.path[..name_at].chars().count();
        let format = match (lit, in_name) {
            (true, _) => TextFormat::simple(heading(TEXT_BODY), brand::TEAL_TEXT),
            (false, true) => TextFormat::simple(FontId::proportional(TEXT_BODY), ink),
            (false, false) => TextFormat::simple(FontId::proportional(TEXT_BODY), brand::MUTED),
        };
        job.append(&c.to_string(), 0.0, format);
    }
    job.wrap.max_width = WIDTH - 2.0 * f32::from(PAD) - 2.0 * ROW_PAD - 24.0;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

fn paint_row(ui: &Ui, rect: Rect, hit: &Hit, selected: bool, hovered: bool) {
    let painter = ui.painter_at(rect);
    selection_row(&painter, rect, selected, hovered);
    let galley = painter.layout_job(row_job(hit, selected));
    painter.galley(
        pos2(rect.left() + ROW_PAD, rect.center().y - galley.size().y / 2.0),
        galley,
        brand::TEXT,
    );
    if selected {
        painter.text(
            pos2(rect.right() - ROW_PAD, rect.center().y),
            Align2::RIGHT_CENTER,
            "Tab",
            FontId::proportional(12.0),
            brand::MUTED,
        );
    }
}

/// Draws the list with its left edge at `at.x` and its bottom `at.y` above the box. Returns the row
/// that was clicked.
pub(crate) fn show(ui: &Ui, id: Id, at: Pos2, rows: &[Hit], picked: usize) -> Option<usize> {
    let mut clicked = None;
    Area::new(menu_id(id))
        .order(Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(pos2(at.x, at.y - GAP))
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(brand::ITEM)
                .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(PAD))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (index, hit) in rows.iter().enumerate() {
                        let size = vec2(WIDTH - 2.0 * f32::from(PAD), ROW_H);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        paint_row(ui, rect, hit, index == picked, response.hovered());
                        if response.clicked() {
                            clicked = Some(index);
                        }
                    }
                });
        });
    clicked
}
