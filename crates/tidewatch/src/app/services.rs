//! The window's side of the repo services: the repos to the host each frame, the pill's and the
//! dialogs' actions, an agent's service tool call, the line each agent's next prompt carries, and
//! the Logs view (the service's live terminal in the place of the tab's, under a `Terminal` back
//! button, which is readable at the terminal's width where a list in a popover would not be).

use std::path::Path;
use std::time::Duration;

use egui::{Align, Layout, Rect, Sense, UiBuilder, pos2, vec2};
use tidewatch_core::services_mcp::{self, Call, ServiceCall};

use super::terminal::{TerminalGrid, show_shell};
use super::{Tidewatch, WAKE_SLACK_S};
use crate::brand;
use crate::layout;
use crate::repo::Workspace;
use crate::screen_kit::{Size, TEXT_SECONDARY, body, mono, quiet_button};
use crate::services_glue::{Begin, Command, Row};
use crate::services_view::{self, Action};

const BACK_LABEL: &str = "\u{2039} Terminal";
const HEADER_GAP: f32 = 12.0;

impl Tidewatch {
    fn repo_on_show(&self) -> Option<&Path> {
        match &self.workspaces[self.tabs.active_group()] {
            Workspace::Repo(repo) => Some(repo.root.as_path()),
            Workspace::Other { .. } => None,
        }
    }

    pub(super) fn services_rows(&self) -> Vec<Row> {
        self.repo_on_show().map(|repo| self.services.rows(repo)).unwrap_or_default()
    }

    pub(super) fn open_add_service(&mut self, ctx: &egui::Context) {
        if self.repo_on_show().is_some() {
            self.services_ui.open_add();
        } else {
            self.show_toast("Services belong to a git repo, and this folder is not one", ctx);
        }
    }

    pub(super) fn drive_services(&mut self, ctx: &egui::Context) {
        let repos: Vec<&Path> = self
            .workspaces
            .iter()
            .filter_map(|workspace| match workspace {
                Workspace::Repo(repo) => Some(repo.root.as_path()),
                Workspace::Other { .. } => None,
            })
            .collect();
        self.services.sync(&repos);
        if let Some(wait) = self.services.drive() {
            ctx.request_repaint_after(Duration::from_secs_f64(wait + WAKE_SLACK_S));
        }
        let agents = self.tabs.items().iter().filter_map(|tab| {
            tab.pane.agent.as_ref()?;
            match self.workspaces.get(tab.group)? {
                Workspace::Repo(repo) => Some((tab.id, repo.root.as_path())),
                Workspace::Other { .. } => None,
            }
        });
        if let Some(lines) = self.services.lines_for(agents)
            && let Some(listener) = self.hooks.listener()
        {
            listener.set_service_lines(lines);
        }
    }

    pub(super) fn show_service_dialog(&mut self, ctx: &egui::Context) {
        if !self.services_ui.dialog_open() {
            return;
        }
        let rows = self.services_rows();
        if let Some(action) = services_view::dialog(ctx, &mut self.services_ui, &rows) {
            self.apply_service_action(action, ctx);
        }
    }

    pub(super) fn apply_service_action(&mut self, action: Action, ctx: &egui::Context) {
        let Some(repo) = self.repo_on_show().map(Path::to_path_buf) else { return };
        let outcome = match action {
            Action::Logs(name) => {
                let tab = self.tabs.items()[self.tabs.active()].id;
                self.services_ui.show_logs(&name, tab);
                Ok(())
            }
            Action::Run(command, name) => self.services.command(&repo, &name, command).map(drop),
            Action::Save { editing: Some(old), service } => {
                self.services.edit(&repo, &old, &service).map_err(|found| found.join("; "))
            }
            Action::Save { editing: None, service } => {
                self.services.add(&repo, &service, Begin::Stopped).map_err(|found| found.join("; "))
            }
            Action::Remove(name) => {
                self.services.remove(&repo, &name).map_err(|found| found.join("; "))
            }
        };
        if let Err(reason) = outcome {
            self.show_toast(reason, ctx);
        }
        ctx.request_repaint();
    }

    /// What an agent's service tool call comes to, answered through the call's own channel.
    pub(super) fn serve_services(&mut self, tab: u64, served: ServiceCall, ctx: &egui::Context) {
        let repo = match self.workspace_of_tab(tab) {
            Workspace::Repo(repo) => Some(repo.root.clone()),
            Workspace::Other { .. } => None,
        };
        let answer = match repo {
            None => {
                Err("Services belong to a git repo, and this tab is in a plain folder.".to_owned())
            }
            Some(repo) => self.answer_service_call(&repo, served.call),
        };
        if served.reply.send(answer) {
            ctx.request_repaint();
        }
    }

    fn answer_service_call(&mut self, repo: &Path, call: Call) -> services_mcp::Answer {
        match call {
            Call::List => Ok(services_mcp::list_text(&self.services.listed(repo))),
            Call::Add(service) => {
                let name = service.name.clone();
                self.services.add(repo, &service, Begin::Now).map_err(|found| {
                    format!(
                        "Nothing was changed. Fix these and call add_service again:\n- {}",
                        found.join("\n- ")
                    )
                })?;
                Ok(format!("Saved and started `{name}`. list_services shows its state."))
            }
            Call::Restart(name) => self.services.command(repo, &name, Command::Restart),
            Call::Stop(name) => self.services.command(repo, &name, Command::Stop),
            Call::Log { name, lines } => self.services.log_text(repo, &name, lines),
        }
    }

    /// Whether the service named in the Logs view shows over the active tab.
    pub(super) fn service_logs_showing(&mut self) -> Option<String> {
        let tab = self.tabs.items()[self.tabs.active()].id;
        let name = self.services_ui.logs_over(tab)?.to_owned();
        let shown = self.repo_on_show().is_some_and(|repo| self.services.has(repo, &name));
        if !shown {
            self.services_ui.close_logs();
        }
        shown.then_some(name)
    }

    pub(super) fn show_service_logs(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        name: &str,
        keys_free: bool,
        modal_open: bool,
    ) {
        let Some(repo) = self.repo_on_show().map(Path::to_path_buf) else { return };
        let row = self.services.rows(&repo).into_iter().find(|row| row.name == name);
        let header = Rect::from_min_size(area.min, vec2(area.width(), layout::CHROME_H));
        let mut command = None;
        let mut back = false;
        ui.scope_builder(
            UiBuilder::new().max_rect(header).layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = HEADER_GAP;
                back = quiet_button(ui, BACK_LABEL, true, Size::Compact)
                    .on_hover_text("Back to the tab's terminal")
                    .clicked();
                let Some(row) = &row else { return };
                let dot = services_view::dot_color(row.word);
                let (dot_rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(dot_rect.center(), 4.0, dot);
                ui.label(egui::RichText::new(&row.name).font(body(14.0)).color(brand::TEXT));
                ui.label(egui::RichText::new(&row.line).size(TEXT_SECONDARY).color(brand::MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if services_view::is_running(row.word) {
                        if quiet_button(ui, "Stop", true, Size::Compact).clicked() {
                            command = Some(Command::Stop);
                        }
                        if quiet_button(ui, "Restart", true, Size::Compact).clicked() {
                            command = Some(Command::Restart);
                        }
                    } else if quiet_button(ui, "Start", true, Size::Compact).clicked() {
                        command = Some(Command::Start);
                    }
                });
            },
        );
        if back {
            self.services_ui.close_logs();
        }
        if let Some(command) = command {
            self.apply_service_action(Action::Run(command, name.to_owned()), ui.ctx());
        }
        let body_rect = Rect::from_min_max(pos2(area.left(), header.bottom()), area.max);
        if let Some(lines) = self.services.fake_lines(&repo, name) {
            paint_lines(ui, body_rect, &lines);
            return;
        }
        let grid = TerminalGrid::measure(ui, body_rect, self.terminal_font.in_use());
        let config = &self.terminal_config;
        match self.services.term_mut(&repo, name) {
            Some(term) => {
                grid.resize(term);
                show_shell(ui, term, &grid, config, keys_free, modal_open, &[]);
            }
            None => paint_lines(ui, body_rect, &[]),
        }
    }
}

fn paint_lines(ui: &egui::Ui, area: Rect, lines: &[String]) {
    let painter = ui.painter_at(area);
    let font = mono(13.0);
    let step = 18.0;
    if lines.is_empty() {
        painter.text(
            area.left_top() + vec2(layout::TERMINAL_PAD, layout::TERMINAL_PAD),
            egui::Align2::LEFT_TOP,
            "No output yet.",
            font,
            brand::FAINT,
        );
        return;
    }
    for (n, line) in lines.iter().enumerate() {
        painter.text(
            area.left_top() + vec2(layout::TERMINAL_PAD, layout::TERMINAL_PAD + step * n as f32),
            egui::Align2::LEFT_TOP,
            line,
            font.clone(),
            brand::SOFT,
        );
    }
}
