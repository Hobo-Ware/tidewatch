//! Draws the welcome screen of a window with no repo.

use std::path::{Path, PathBuf};

use egui::{Align2, Pos2, Rect, Ui, Vec2, pos2, vec2};
use stdusk_egui::colors;

use crate::brand;
use crate::mascot::{self, Pose};
use crate::screen_kit::{body, clickable, heading, mono};
use crate::welcome::{Recent, shown_path};

const COLUMN_W: f32 = 560.0;
const MASCOT_H: f32 = 132.0;
const ROW_H: f32 = 46.0;

pub(crate) enum Action {
    Open,
    Recent(PathBuf),
}

pub(crate) fn show(
    ui: &mut Ui,
    area: Rect,
    recent: &[Recent],
    home: Option<&Path>,
    shortcut: &str,
) -> Option<Action> {
    ui.painter().rect_filled(area, 0.0, colors::bg());
    let width = COLUMN_W.min(area.width() - 40.0);
    let rows = recent.len() as f32;
    let list_h = if recent.is_empty() { 0.0 } else { 26.0 + 22.0 + rows * (ROW_H + 4.0) };
    let total = MASCOT_H + 26.0 + 90.0 + 26.0 + 48.0 + list_h + 26.0 + 20.0;
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
        colors::fg(),
    );
    let text = painter.layout(
        "Every tab is a worktree of the repo you open, with an agent or a workflow in it."
            .to_owned(),
        body(15.0),
        colors::dim(),
        width,
    );
    let text_pos = pos2(cx - text.size().x / 2.0, title.bottom() + 10.0);
    let text_bottom = text_pos.y + text.size().y;
    painter.galley(text_pos, text, colors::dim());
    y = text_bottom + 26.0;
    let button = open_button(ui, pos2(cx, y), shortcut);
    if button.clicked() {
        action = Some(Action::Open);
    }
    y += 48.0 + 26.0;
    if !recent.is_empty() {
        painter.text(
            pos2(cx - width / 2.0 + 12.0, y),
            Align2::LEFT_TOP,
            "RECENT",
            body(11.5),
            colors::dim(),
        );
        y += 22.0;
        for (index, repo) in recent.iter().enumerate() {
            let row = Rect::from_min_size(pos2(cx - width / 2.0, y), vec2(width, ROW_H));
            let response = clickable(ui, row, ("recent", index));
            let lit = index == 0 || response.hovered();
            if lit {
                let fill = if response.hovered() { colors::hover() } else { colors::elevated() };
                painter.rect_filled(row, 10.0, fill);
            }
            let name = painter.text(
                row.left_center() + vec2(12.0, 0.0),
                Align2::LEFT_CENTER,
                &repo.name,
                heading(15.0),
                colors::fg(),
            );
            painter.text(
                pos2(name.right() + 12.0, row.center().y),
                Align2::LEFT_CENTER,
                shown_path(&repo.dir, home),
                mono(12.5),
                colors::dim(),
            );
            painter.text(
                row.right_center() - vec2(12.0, 0.0),
                Align2::RIGHT_CENTER,
                repo.tabs_text(),
                body(12.0),
                colors::dim(),
            );
            if response.clicked() {
                action = Some(Action::Recent(repo.dir.clone()));
            }
            y += ROW_H + 4.0;
        }
        y += 26.0 - 4.0;
    }
    hint(ui, pos2(cx, y));
    action
}

fn open_button(ui: &mut Ui, top_center: Pos2, shortcut: &str) -> egui::Response {
    let label = ui.painter().layout_no_wrap("Open repo...".to_owned(), heading(15.0), colors::bg());
    let keys = ui.painter().layout_no_wrap(
        shortcut.to_owned(),
        mono(12.0),
        colors::bg().gamma_multiply(0.7),
    );
    let size = vec2(22.0 + label.size().x + 10.0 + keys.size().x + 22.0, 46.0);
    let rect = Rect::from_min_size(top_center - vec2(size.x / 2.0, 0.0), size);
    let response = clickable(ui, rect, "open-repo");
    let fill = if response.hovered() { brand::TEAL.gamma_multiply(0.9) } else { brand::TEAL };
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, fill);
    painter.galley(
        pos2(rect.left() + 22.0, rect.center().y - label.size().y / 2.0),
        label.clone(),
        colors::bg(),
    );
    painter.galley(
        pos2(rect.left() + 22.0 + label.size().x + 10.0, rect.center().y - keys.size().y / 2.0),
        keys,
        colors::bg(),
    );
    response
}

fn hint(ui: &Ui, top_center: Pos2) {
    let painter = ui.painter();
    let lead = painter.layout_no_wrap("or from a terminal: ".to_owned(), body(13.0), colors::dim());
    let command = painter.layout_no_wrap(
        "tidewatch ~/code/my-repo".to_owned(),
        mono(12.5),
        colors::fg().gamma_multiply(0.75),
    );
    let x = top_center.x - f32::midpoint(lead.size().x, command.size().x);
    let command_x = x + lead.size().x;
    painter.galley(pos2(x, top_center.y), lead, colors::dim());
    painter.galley(pos2(command_x, top_center.y), command, colors::dim());
}

#[cfg(test)]
mod tests;
