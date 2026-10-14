//! The terminal area of the active tab: shell grid, status messages and the exit notice.

use egui::FontId;
use stdusk_egui::colors;
use stdusk_egui::ui::{CursorStyle, GridStyle, collect_input, draw_pane_notice, render_grid};

use super::frame::FrameIntent;
use super::{FONT_SIZE, MIN_CONTRAST, Tidewatch};
use crate::brand;
use crate::commands::Command;
use crate::pane::{Body, Pane};
use crate::reader::Shown;
use crate::{layout, reader_glue};

pub(super) struct TerminalGrid {
    rect: egui::Rect,
    cols: usize,
    rows: usize,
    cell_w: f32,
    cell_h: f32,
    font: FontId,
}

impl TerminalGrid {
    pub(super) fn measure(ui: &egui::Ui, rect: egui::Rect) -> Self {
        let font = FontId::monospace(FONT_SIZE);
        let glyph = ui.painter().layout_no_wrap("M".to_owned(), font.clone(), colors::fg());
        let (cell_w, cell_h) = (glyph.size().x, glyph.size().y);
        let (cols, rows) = layout::grid_dims(rect, cell_w, cell_h);
        Self { rect, cols, rows, cell_w, cell_h, font }
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
                        None,
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
                    draw_pane_notice(ui, area, note);
                }
            }
            Body::Creating => {
                status_message(ui, area, "Creating worktree...", colors::dim());
            }
            Body::Removing => {
                status_message(ui, area, "Removing worktree...", colors::dim());
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
    ) {
        match shown {
            Some(Shown::Reader) => {
                let pane = self.tabs.active_pane_mut();
                let asked = reader_glue::draw(ui, terminal, pane, &mut self.dev_reader, now);
                let toast =
                    reader_glue::apply(asked, &mut self.tabs, active_id, &mut self.flow, ctx, now);
                if let Some(text) = toast {
                    self.show_toast(text, ctx);
                }
            }
            Some(Shown::Terminal(Some(note))) => reader_glue::note(ui, terminal, note),
            Some(Shown::Terminal(None)) | None => {}
        }
    }
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
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), colors::fg());
    let pill = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - 28.0),
        galley.size() + egui::vec2(24.0, 14.0),
    );
    painter.rect_filled(pill, 8.0, colors::elevated());
    painter.rect_stroke(
        pill,
        8.0,
        egui::Stroke::new(1.0, colors::border()),
        egui::StrokeKind::Inside,
    );
    painter.galley(pill.center() - galley.size() / 2.0, galley, colors::fg());
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
