//! Draws the welcome screen of a window with no repo.

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use stdusk_egui::colors;

use crate::screen_kit::{ORANGE, body, clickable, heading, mono};
use crate::welcome::{Recent, shown_path};

const COLUMN_W: f32 = 560.0;
const MARK_BOX: f32 = 88.0;
const ROW_H: f32 = 46.0;
const ARTWORK_ZOOM: f32 = 1.45;

pub(crate) enum Action {
    Open,
    Recent(PathBuf),
}

pub(crate) fn paint_mark(ui: &Ui, rect: Rect) {
    let scale = rect.width() / 824.0;
    let zoom = scale * ARTWORK_ZOOM;
    let at = |x: f32, y: f32| rect.center() + vec2((x - 512.0) * zoom, (y - 530.0) * zoom);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same((185.0 * scale) as u8),
        colors::bg(),
        Stroke::new(1.0, colors::border()),
        StrokeKind::Inside,
    );
    let width = 44.7 * zoom;
    let teal = colors::accent();
    let chevron = |dx: f32, color: Color32| {
        let points = vec![at(282.1 + dx, 409.8), at(384.3 + dx, 486.5), at(282.1 + dx, 563.1)];
        painter.add(Shape::line(points, Stroke::new(width, color)));
    };
    chevron(0.0, colors::fg());
    chevron(102.2, teal.gamma_multiply(0.6));
    chevron(204.4, teal.gamma_multiply(0.28));
    let curve = |from: (f32, f32), c1: (f32, f32), c2: (f32, f32), to: (f32, f32)| -> Vec<Pos2> {
        (0..=14)
            .map(|i| {
                let t = i as f32 / 14.0;
                let u = 1.0 - t;
                let mix = |a: f32, b: f32, c: f32, d: f32| {
                    u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
                };
                at(mix(from.0, c1.0, c2.0, to.0), mix(from.1, c1.1, c2.1, to.1))
            })
            .collect()
    };
    let mut wave = curve((448.1, 639.7), (499.2, 595.0), (550.3, 595.0), (601.4, 639.7));
    wave.extend(
        curve((601.4, 639.7), (652.5, 684.4), (690.8, 684.4), (741.9, 639.7)).into_iter().skip(1),
    );
    painter.add(Shape::line(wave, Stroke::new(width, teal)));
    painter.circle_filled(at(690.8, 384.3), 44.7 * zoom, ORANGE);
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
    let total = MARK_BOX + 26.0 + 90.0 + 26.0 + 48.0 + list_h + 26.0 + 20.0;
    let cx = area.center().x;
    let mut y = (area.center().y - total / 2.0).max(area.top() + 16.0);
    let mut action = None;
    paint_mark(ui, Rect::from_center_size(pos2(cx, y + MARK_BOX / 2.0), vec2(MARK_BOX, MARK_BOX)));
    y += MARK_BOX + 26.0;
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
    let fill =
        if response.hovered() { colors::accent().gamma_multiply(0.9) } else { colors::accent() };
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
