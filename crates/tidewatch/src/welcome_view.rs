//! Draws the welcome screen of a window with no repo.

use std::path::{Path, PathBuf};

use egui::{
    Align, Align2, CornerRadius, Layout, Pos2, Rect, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2,
    vec2,
};

use crate::brand;
use crate::mascot::{self, Pose};
use crate::screen_kit::{
    body, clickable, heading, mono, paint_pill_card, quiet_button, selection_row,
};
use crate::welcome::{Recent, shown_path};

const COLUMN_W: f32 = 560.0;
const MASCOT_H: f32 = 132.0;
const ROW_H: f32 = 46.0;
const SESSIONS_H: f32 = 44.0;
const SESSIONS_GAP: f32 = 18.0;

pub(crate) enum Action {
    Open,
    Recent(PathBuf),
    /// Review the agent sessions found in other terminals.
    MoveSessions,
}

pub(crate) fn show(
    ui: &mut Ui,
    area: Rect,
    recent: &[Recent],
    home: Option<&Path>,
    shortcut: &str,
    elsewhere: Option<(usize, usize)>,
) -> Option<Action> {
    ui.painter().rect_filled(area, 0.0, brand::GROUND);
    let width = COLUMN_W.min(area.width() - 40.0);
    let rows = recent.len() as f32;
    let list_h = if recent.is_empty() { 0.0 } else { 26.0 + 22.0 + rows * (ROW_H + 4.0) };
    let sessions_h = elsewhere.map_or(0.0, |_| SESSIONS_H + SESSIONS_GAP);
    let total = MASCOT_H + 26.0 + 90.0 + 26.0 + 48.0 + sessions_h + list_h + 26.0 + 20.0;
    let cx = area.center().x;
    let mut y = (area.center().y - total / 2.0).max(area.top() + 16.0);
    let mut action = None;
    let mascot = Vec2::new(mascot::width_at(Pose::Peek, MASCOT_H), MASCOT_H);
    mascot::show(ui, Rect::from_center_size(pos2(cx, y + MASCOT_H / 2.0), mascot), Pose::Peek);
    y += MASCOT_H + 26.0;
    let painter = ui.painter().clone();
    let title = painter.text(
        pos2(cx, y),
        Align2::CENTER_TOP,
        "Open a repo to start",
        heading(30.0),
        brand::TEXT,
    );
    let text = painter.layout(
        "Every tab is a worktree of the repo you open, with an agent or a workflow in it."
            .to_owned(),
        body(15.0),
        brand::MUTED,
        width,
    );
    let text_pos = pos2(cx - text.size().x / 2.0, title.bottom() + 10.0);
    let text_bottom = text_pos.y + text.size().y;
    painter.galley(text_pos, text, brand::MUTED);
    y = text_bottom + 26.0;
    let button = open_button(ui, pos2(cx, y));
    if button.clicked() {
        action = Some(Action::Open);
    }
    y += 48.0 + 26.0;
    if let Some((running, idle)) = elsewhere {
        let line = Rect::from_min_size(pos2(cx - width / 2.0, y), vec2(width, SESSIONS_H));
        if sessions_line(ui, line, running, idle) {
            action = Some(Action::MoveSessions);
        }
        y += SESSIONS_H + SESSIONS_GAP;
    }
    if !recent.is_empty() {
        painter.text(
            pos2(cx - width / 2.0 + 12.0, y),
            Align2::LEFT_TOP,
            "RECENT",
            body(11.5),
            brand::MUTED,
        );
        y += 22.0;
        for (index, repo) in recent.iter().enumerate() {
            let row = Rect::from_min_size(pos2(cx - width / 2.0, y), vec2(width, ROW_H));
            let response = clickable(ui, row, ("recent", index));
            painter.rect(
                row,
                CornerRadius::same(8),
                brand::ITEM,
                Stroke::new(1.0, brand::ITEM_LINE),
                StrokeKind::Inside,
            );
            selection_row(&painter, row, false, response.hovered());
            let name = painter.text(
                row.left_center() + vec2(12.0, 0.0),
                Align2::LEFT_CENTER,
                &repo.name,
                heading(15.0),
                brand::TEXT,
            );
            painter.text(
                pos2(name.right() + 12.0, row.center().y),
                Align2::LEFT_CENTER,
                shown_path(&repo.dir, home),
                mono(12.5),
                brand::MUTED,
            );
            painter.text(
                row.right_center() - vec2(12.0, 0.0),
                Align2::RIGHT_CENTER,
                repo.tabs_text(),
                body(12.0),
                brand::MUTED,
            );
            if response.clicked() {
                action = Some(Action::Recent(repo.dir.clone()));
            }
            y += ROW_H + 4.0;
        }
        y += 26.0 - 4.0;
    }
    hint(ui, pos2(cx, y));
    ui.painter().text(
        pos2(cx, area.bottom() - 24.0),
        Align2::CENTER_BOTTOM,
        format!("{shortcut} opens a repo"),
        body(12.0),
        brand::MUTED,
    );
    action
}

/// The line that says agent sessions run in other terminals, with the button that reviews them.
fn sessions_line(ui: &mut Ui, rect: Rect, running: usize, idle: usize) -> bool {
    paint_pill_card(ui.painter(), rect);
    let noun = if running == 1 { "agent session runs" } else { "agent sessions run" };
    ui.painter().text(
        rect.left_center() + vec2(14.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{running} {noun} in other terminals"),
        body(13.0),
        brand::SOFT,
    );
    let mut inner = ui.new_child(
        UiBuilder::new()
            .id_salt("welcome_sessions")
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    let label = if idle > 0 { "Move idle ones here" } else { "None are idle" };
    quiet_button(&mut inner, label, idle > 0).clicked()
}

fn open_button(ui: &mut Ui, top_center: Pos2) -> egui::Response {
    let label =
        ui.painter().layout_no_wrap("Open repo...".to_owned(), heading(15.0), brand::KEY_EDGE);
    let size = vec2(label.size().x + 44.0, 46.0);
    let rect = Rect::from_min_size(top_center - vec2(size.x / 2.0, 0.0), size);
    let response = clickable(ui, rect, "open-repo");
    let fill = if response.hovered() { brand::TEAL_TEXT } else { brand::TEAL };
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, fill);
    painter.galley(rect.center() - label.size() / 2.0, label, brand::KEY_EDGE);
    response
}

fn hint(ui: &Ui, top_center: Pos2) {
    let painter = ui.painter();
    let lead = painter.layout_no_wrap("or from a terminal: ".to_owned(), body(13.0), brand::MUTED);
    let command =
        painter.layout_no_wrap("tidewatch ~/code/my-repo".to_owned(), mono(12.5), brand::SOFT);
    let x = top_center.x - f32::midpoint(lead.size().x, command.size().x);
    let command_x = x + lead.size().x;
    painter.galley(pos2(x, top_center.y), lead, brand::MUTED);
    painter.galley(pos2(command_x, top_center.y), command, brand::MUTED);
}

#[cfg(test)]
mod tests;
