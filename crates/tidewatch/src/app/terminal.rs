//! The terminal area of the active tab: shell grid, status messages and the exit notice.

use egui::FontId;
use stdusk_egui::ui::{CursorStyle, GridStyle, collect_input, render_grid};

use super::frame::FrameIntent;
use super::{FONT_SIZE, MIN_CONTRAST, Tidewatch};
use crate::brand;
use crate::commands::Command;
use crate::mascot::{self, Pose};
use crate::pane::{Body, Pane};
use crate::reader::Shown;
use crate::reader_view::Action;
use crate::screen_kit::paint_pill_card;
use crate::terminal_font::InUse;
use crate::{layout, reader_glue};

const CAUGHT_H: f32 = 72.0;
const CAUGHT_GAP: f32 = 6.0;

pub(super) struct TerminalGrid {
    rect: egui::Rect,
    cols: usize,
    rows: usize,
    cell_w: f32,
    cell_h: f32,
    font: FontId,
    bold: Option<FontId>,
}

impl TerminalGrid {
    pub(super) fn measure(ui: &egui::Ui, rect: egui::Rect, in_use: &InUse) -> Self {
        let (font, bold) = in_use.font_ids(FONT_SIZE);
        let glyph = ui.painter().layout_no_wrap("M".to_owned(), font.clone(), brand::TEXT);
        let (cell_w, cell_h) = (glyph.size().x, glyph.size().y);
        let (cols, rows) = layout::grid_dims(rect, cell_w, cell_h);
        Self { rect, cols, rows, cell_w, cell_h, font, bold }
    }
}

impl Tidewatch {
    pub(super) fn show_terminal(
        &mut self,
        ui: &mut egui::Ui,
        grid: &TerminalGrid,
        agent_name: Option<&str>,
        reader_on: bool,
        frame: &FrameIntent,
    ) {
        let area = grid.rect;
        let Pane { body, agent, .. } = self.tabs.active_pane_mut();
        match body {
            Body::Shell(term) => {
                term.resize(grid.cols, grid.rows);
                if !reader_on {
                    let typed = collect_input(ui, false, false, term.app_cursor());
                    if !typed.is_empty() && !frame.keys.modal_open && !frame.keys.typing_in_a_panel
                    {
                        term.send(&typed);
                        if let Some(agent) = agent {
                            agent.resume_note = None;
                        }
                    }
                    let snap = term.grid_snapshot();
                    render_grid(
                        ui,
                        &[],
                        area,
                        term,
                        &snap,
                        grid.cell_w,
                        grid.cell_h,
                        &grid.font,
                        grid.bold.as_ref(),
                        GridStyle {
                            cursor: CursorStyle::Block,
                            dimmed: false,
                            link_active: false,
                            blink: false,
                            ligatures: false,
                            min_contrast: MIN_CONTRAST,
                            app_mouse: false,
                        },
                        &[],
                    );
                }
                if let (Some(name), Some(exit)) = (agent_name, term.exited()) {
                    exit_notice(ui, area, &exit_notice_text(name, exit.code));
                } else if let Some(note) = agent.as_ref().and_then(|agent| agent.resume_note) {
                    let pill = pane_notice(ui, area, note);
                    caught_above(ui, area, pill);
                }
            }
            Body::Creating => {
                status_message(ui, area, "Creating worktree...", brand::MUTED);
            }
            Body::Removing => {
                status_message(ui, area, "Removing worktree...", brand::MUTED);
            }
            Body::Failed(message) => status_message(ui, area, message, brand::RED),
        }
    }

    pub(super) fn show_reader(
        &mut self,
        ui: &mut egui::Ui,
        terminal: egui::Rect,
        shown: Option<Shown>,
        ctx: &egui::Context,
        now: f64,
        active_id: u64,
        keys_free: bool,
    ) {
        match shown {
            Some(Shown::Reader) => {
                let parts = self.review_parts(active_id);
                let notes = self.views.get(&active_id).map(|view| &view.notes);
                let review =
                    parts.as_ref().zip(notes).map(|(parts, notes)| parts.facts(notes, keys_free));
                let pane = self.tabs.active_pane_mut();
                let asked =
                    reader_glue::draw(ui, terminal, pane, &mut self.dev_reader, now, review);
                let (reviews, asked): (Vec<_>, Vec<_>) =
                    asked.into_iter().partition(|action| matches!(action, Action::Review(_)));
                let toast =
                    reader_glue::apply(asked, &mut self.tabs, active_id, &mut self.flow, ctx, now);
                if let Some(text) = toast {
                    self.show_toast(text, ctx);
                }
                let reviews = reviews
                    .into_iter()
                    .filter_map(|action| match action {
                        Action::Review(review) => Some(review),
                        _ => None,
                    })
                    .collect();
                self.apply_review(reviews, active_id, ctx);
            }
            Some(Shown::Terminal(Some(note))) => reader_glue::note(ui, terminal, note),
            Some(Shown::Terminal(None)) | None => {}
        }
    }
}

fn caught_above(ui: &egui::Ui, area: egui::Rect, pill: egui::Rect) {
    let room = egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, pill.top() - CAUGHT_GAP));
    if room.height() < CAUGHT_H * 2.0 {
        return;
    }
    let size = egui::vec2(mascot::width_at(Pose::Caught, CAUGHT_H), CAUGHT_H);
    let at = egui::Rect::from_center_size(
        egui::pos2(pill.center().x, room.bottom() - size.y / 2.0),
        size,
    );
    mascot::show(ui, at, Pose::Caught);
}

fn status_message(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    let painter = ui.painter_at(rect);
    let galley = painter.layout(
        text.to_owned(),
        FontId::monospace(FONT_SIZE),
        color,
        (rect.width() * 0.8).max(1.0),
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}

fn exit_notice_text(profile_name: &str, code: i32) -> String {
    format!("{profile_name} exited with code {code}. {} closes the tab.", Command::CloseTab.keys())
}

fn exit_notice(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    let painter = ui.painter_at(rect);
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), brand::TEXT);
    let pill = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - 28.0),
        galley.size() + egui::vec2(24.0, 14.0),
    );
    paint_pill_card(&painter, pill);
    painter.galley(pill.center() - galley.size() / 2.0, galley, brand::TEXT);
}

/// A small card along the bottom edge of the pane, drawn over the grid and never sent to the pty.
/// Long text wraps. Returns the card's rectangle.
fn pane_notice(ui: &egui::Ui, pane: egui::Rect, text: &str) -> egui::Rect {
    let (pad, margin) = (egui::vec2(10.0, 5.0), 8.0);
    let wrap_width = (pane.width() - 2.0 * (margin + pad.x)).max(60.0);
    let galley =
        ui.painter().layout(text.to_owned(), FontId::proportional(12.0), brand::MUTED, wrap_width);
    let size = galley.size() + pad * 2.0;
    let pill = egui::Rect::from_min_size(
        egui::pos2(pane.center().x - size.x / 2.0, pane.bottom() - margin - size.y),
        size,
    );
    let painter = ui.painter_at(pane);
    paint_pill_card(&painter, pill);
    painter.galley(pill.min + pad, galley, brand::MUTED);
    pill
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exit_notice_names_the_profile_and_the_code() {
        assert_eq!(
            exit_notice_text("Builder", 130),
            format!("Builder exited with code 130. {} closes the tab.", Command::CloseTab.keys())
        );
    }
}
