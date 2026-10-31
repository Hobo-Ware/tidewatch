//! Draws the slash completion list above the reply box and reads its keys. The rows look like the
//! command palette's: a teal-tinted selected row with a bar, the name in text ink, the description
//! muted after it.

use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, Area, CornerRadius, EventFilter, FontId, Frame, Id, Key, Margin, Modifiers, Order,
    Rect, Sense, Stroke, Ui, pos2, vec2,
};

use crate::brand;
use crate::bullets;
use crate::screen_kit::{selected_ink, selection_row};
use crate::slash::{self, Entry, Key as MenuKey, Menu, Pressed};

const ROW_H: f32 = 30.0;
const WIDTH: f32 = 560.0;
const GAP: f32 = 6.0;
const PAD: i8 = 6;

fn menu_id(id: Id) -> Id {
    id.with("slash-menu")
}

pub(crate) fn load(ui: &Ui, id: Id) -> Menu {
    ui.data(|data| data.get_temp(menu_id(id))).unwrap_or_default()
}

pub(crate) fn store(ui: &Ui, id: Id, menu: Menu) {
    ui.data_mut(|data| data.insert_temp(menu_id(id), menu));
}

fn consume(ui: &Ui, key: Key) -> bool {
    ui.input_mut(|input| input.consume_key(Modifiers::NONE, key))
}

fn accept(ui: &Ui, id: Id, text: &mut String, completed: String) {
    *text = completed;
    bullets::place_cursor(ui, id, text.chars().count());
}

/// The list's keys, taken before the text box sees them. True when one was used.
pub(crate) fn keys(ui: &Ui, id: Id, text: &mut String, menu: &mut Menu, rows: &[&Entry]) -> bool {
    if rows.is_empty() {
        return false;
    }
    if menu.open && consume(ui, Key::Escape) {
        menu.press(MenuKey::Close, text, rows);
        ui.memory_mut(|memory| memory.request_focus(id));
        return true;
    }
    if !ui.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    let enter = !menu.enter_sends(text, rows) && consume(ui, Key::Enter);
    let pressed = if consume(ui, Key::ArrowDown) {
        Some(MenuKey::Down)
    } else if consume(ui, Key::ArrowUp) {
        Some(MenuKey::Up)
    } else if enter || consume(ui, Key::Tab) {
        Some(MenuKey::Accept)
    } else {
        None
    };
    let Some(pressed) = pressed else { return false };
    if let Pressed::Accepted(completed) = menu.press(pressed, text, rows) {
        accept(ui, id, text, completed);
    }
    true
}

/// While the list is open Tab, the arrows and Esc stay with the text box instead of moving focus.
pub(crate) fn hold_keys(ui: &Ui, id: Id) {
    let filter =
        EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };
    ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
}

fn paint_row(ui: &Ui, rect: Rect, entry: &Entry, selected: bool, hovered: bool) {
    let painter = ui.painter_at(rect);
    selection_row(&painter, rect, selected, hovered);
    let mut job = LayoutJob::default();
    let name = TextFormat::simple(FontId::proportional(14.0), selected_ink(selected, brand::TEXT));
    job.append(&format!("/{}", entry.name), 0.0, name);
    let muted = TextFormat::simple(FontId::proportional(12.0), brand::MUTED);
    job.append(&entry.description, 12.0, muted);
    job.wrap = TextWrapping::truncate_at_width((rect.width() - 20.0).max(40.0));
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        brand::TEXT,
    );
}

/// Draws the list just above `above`. Returns the row that was clicked.
pub(crate) fn show(ui: &Ui, id: Id, above: Rect, rows: &[&Entry], picked: usize) -> Option<usize> {
    let width = above.width().min(WIDTH);
    let mut clicked = None;
    Area::new(menu_id(id))
        .order(Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(pos2(above.left(), above.top() - GAP))
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(brand::CARD)
                .stroke(Stroke::new(1.0, brand::FIELD_LINE))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(PAD))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (at, entry) in rows.iter().enumerate() {
                        let (rect, response) = ui.allocate_exact_size(
                            vec2(width - 2.0 * f32::from(PAD), ROW_H),
                            Sense::click(),
                        );
                        paint_row(ui, rect, entry, at == picked, response.hovered());
                        if response.clicked() {
                            clicked = Some(at);
                        }
                    }
                });
        });
    clicked
}

pub(crate) fn complete_with_click(ui: &Ui, id: Id, text: &mut String, row: &Entry) {
    accept(ui, id, text, slash::completion(row));
    ui.memory_mut(|memory| memory.request_focus(id));
}
