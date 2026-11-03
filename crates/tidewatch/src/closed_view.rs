//! Draws the Resume side of the new-tab form: the repo's closed sessions, one two-line row each
//! (CLI badge, the label the tab had, when it closed, Resume; the goal it worked on, else its
//! branch, under it), the picked one highlighted. Drawn from plain data (`closed_state.rs`);
//! reports which row was picked or resumed.

use egui::{Align2, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};

use crate::brand;
use crate::closed_state::Row;
use crate::reader_view::ellipsized;
use crate::screen_kit::{
    FACE, Size, TEXT_BODY, TEXT_SECONDARY, TEXT_TAG, body, button_width, clickable,
    paint_cli_badge, quiet_button, selected_ink, selection_row,
};

pub(crate) const ROW_H: f32 = 58.0;
const INTRO_H: f32 = 34.0;
const PAD: f32 = 10.0;
const FACE_GAP: f32 = 10.0;
const LINE_SHIFT: f32 = 10.0;
const WHEN_GAP: f32 = 12.0;

pub(crate) const EMPTY: &str = "No session of this repo closed in the last 14 days.";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Picked {
    Select(usize),
    Resume(String),
}

/// The second line of a row: what the session was for, else the branch it worked on.
pub(crate) fn detail(row: &Row) -> &str {
    row.summary.as_deref().unwrap_or(&row.branch)
}

pub(crate) fn show(ui: &mut Ui, area: Rect, rows: &[Row], picked: usize) -> Option<Picked> {
    let intro = Rect::from_min_size(area.min, vec2(area.width(), INTRO_H));
    let text = if rows.is_empty() {
        EMPTY
    } else {
        "Each one comes back in its worktree, with its conversation."
    };
    ui.painter().text(
        intro.left_center(),
        Align2::LEFT_CENTER,
        text,
        body(TEXT_SECONDARY),
        brand::MUTED,
    );
    let list = Rect::from_min_max(pos2(area.left(), intro.bottom()), area.max);
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    ScrollArea::vertical()
        .id_salt("resume-list")
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let width = ui.available_width() - brand::SCROLL_ROOM;
            let mut out = None;
            for (index, row) in rows.iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
                out = out.or(draw_row(ui, rect, row, index, index == picked));
            }
            out
        })
        .inner
}

fn draw_row(ui: &mut Ui, rect: Rect, row: &Row, index: usize, picked: bool) -> Option<Picked> {
    let response = clickable(ui, rect, ("resume-row", &row.session_id));
    selection_row(ui.painter(), rect, picked, response.hovered());
    let label = if row.busy { "Opening..." } else { "Resume" };
    let button_w = button_width(ui, label, Size::Compact);
    let button = Rect::from_center_size(
        pos2(rect.right() - PAD - button_w / 2.0, rect.center().y),
        vec2(button_w, Size::Compact.height()),
    );
    let mut button_ui = ui.new_child(UiBuilder::new().max_rect(button));
    let resumed = quiet_button(&mut button_ui, label, !row.busy, Size::Compact)
        .on_hover_text("Bring the worktree back and continue the conversation in a new tab")
        .clicked();
    let painter = ui.painter_at(rect);
    let top = rect.center().y - LINE_SHIFT;
    let bottom = rect.center().y + LINE_SHIFT;
    let face = Rect::from_center_size(pos2(rect.left() + PAD + FACE / 2.0, top), vec2(FACE, FACE));
    paint_cli_badge(&painter, face, &row.cli_id);
    let left = face.right() + FACE_GAP;
    let right = button.left() - WHEN_GAP;
    let when = painter.layout_no_wrap(row.closed.clone(), body(TEXT_TAG), brand::MUTED);
    let when_w = when.size().x;
    painter.galley(pos2(right - when_w, top - when.size().y / 2.0), when, brand::MUTED);
    let ink = selected_ink(picked, brand::TEXT);
    let room = (right - when_w - WHEN_GAP - left).max(0.0);
    let title = ellipsized(&painter, &row.title, body(TEXT_BODY), ink, room);
    painter.galley(pos2(left, top - title.size().y / 2.0), title, ink);
    let detail_room = (right - left).max(0.0);
    let line = ellipsized(&painter, detail(row), body(TEXT_SECONDARY), brand::SOFT, detail_room);
    painter.galley(pos2(left, bottom - line.size().y / 2.0), line, brand::SOFT);
    if resumed {
        return Some(Picked::Resume(row.session_id.clone()));
    }
    let response = response.on_hover_text(detail(row));
    if response.double_clicked() {
        return Some(Picked::Resume(row.session_id.clone()));
    }
    response.clicked().then_some(Picked::Select(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(summary: Option<&str>) -> Row {
        Row {
            session_id: "s".into(),
            cli_id: "claude".into(),
            title: "fix-login".into(),
            summary: summary.map(str::to_owned),
            branch: "agent/heron".into(),
            closed: "closed 2h ago".into(),
            busy: false,
        }
    }

    #[test]
    fn the_second_line_is_the_summary_else_the_branch() {
        assert_eq!(detail(&row(Some("Fix the login redirect"))), "Fix the login redirect");
        assert_eq!(detail(&row(None)), "agent/heron");
    }
}
