//! The window's side of the repo services: the repos to the host each frame, the pill's and the
//! dialogs' actions, an agent's service tool call, the line each agent's next prompt carries, and
//! the Logs view (the service's live terminal in the place of the tab's, under a `Terminal` back
//! button, which is readable at the terminal's width where a list in a popover would not be).

use std::path::{Path, PathBuf};

use egui::{Align, Layout, Rect, Sense, UiBuilder, pos2, vec2};
use tidewatch_core::services::{Key, Service};
use tidewatch_core::services_mcp::{self, Call, ServiceCall};

use super::Tidewatch;
use super::terminal::{TerminalGrid, show_shell};
use crate::brand;
use crate::layout;
use crate::repo::Workspace;
use crate::screen_kit::{Glyph, Size, TEXT_SECONDARY, body, glyph_button, mono, quiet_button};
use crate::services_glue::{AgentPlace, Begin, Command, Row};
use crate::services_view::{self, Action};
use crate::wake::request_wake;

const BACK_LABEL: &str = "\u{2039} Terminal";
const HEADER_GAP: f32 = 12.0;

impl Tidewatch {
    fn repo_on_show(&self) -> Option<&Path> {
        match &self.workspaces[self.tabs.active_group()] {
            Workspace::Repo(repo) => Some(repo.root.as_path()),
            Workspace::Other { .. } => None,
        }
    }

    /// The repo's services, each tab service under its tab's title when that tab is open.
    pub(super) fn services_rows(&self) -> Vec<Row> {
        let Some(repo) = self.repo_on_show() else { return Vec::new() };
        let mut rows = self.services.rows(repo);
        for row in &mut rows {
            let Some(folder) = &row.key.tab else { continue };
            if let Some(tab) = self.tabs.items().iter().find(|tab| {
                self.tab_folder(tab.id).is_some_and(|dir| dir.components().eq(folder.components()))
            }) {
                row.owner = Some(tab.title.clone());
            }
        }
        rows
    }

    /// The folder a tab runs in: its worktree, the folder its agent was started in, or the repo.
    pub(super) fn tab_folder(&self, id: u64) -> Option<PathBuf> {
        let tab = self.tabs.get(id)?;
        if let Some(worktree) = &tab.pane.worktree {
            return Some(worktree.path.clone());
        }
        let launched = tab.pane.agent.as_ref().and_then(|agent| agent.launched.as_ref());
        if let Some(dir) = launched.and_then(|launched| launched.dir.clone()) {
            return Some(dir);
        }
        match self.workspaces.get(tab.group)? {
            Workspace::Repo(repo) => Some(repo.root.clone()),
            Workspace::Other { .. } => None,
        }
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
            request_wake(ctx, wait);
        }
        let folders: Vec<(u64, PathBuf, PathBuf)> = self
            .tabs
            .items()
            .iter()
            .filter_map(|tab| {
                tab.pane.agent.as_ref()?;
                let Workspace::Repo(repo) = self.workspaces.get(tab.group)? else { return None };
                Some((tab.id, repo.root.clone(), self.tab_folder(tab.id)?))
            })
            .collect();
        let agents = folders.iter().map(|(id, repo, folder)| AgentPlace { id: *id, repo, folder });
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
            Action::Logs(key) => {
                let tab = self.tabs.items()[self.tabs.active()].id;
                self.services_ui.show_logs(&key, tab);
                Ok(())
            }
            Action::Run(command, key) => self.services.command(&repo, &key, command).map(drop),
            Action::Share(key) => {
                self.services.share(&repo, &key).map_err(|found| found.join("; "))
            }
            Action::Save { editing: Some(old), service } => {
                self.services.edit(&repo, &old, &service).map_err(|found| found.join("; "))
            }
            Action::Save { editing: None, service } => {
                self.services.add(&repo, &service, Begin::Stopped).map_err(|found| found.join("; "))
            }
            Action::Remove(key) => {
                self.services.remove(&repo, &key).map_err(|found| found.join("; "))
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
        let answer = match repo.zip(self.tab_folder(tab)) {
            None => {
                Err("Services belong to a git repo, and this tab is in a plain folder.".to_owned())
            }
            Some((repo, folder)) => self.answer_service_call(&repo, &folder, served.call),
        };
        if served.reply.send(answer) {
            ctx.request_repaint();
        }
    }

    fn answer_service_call(
        &mut self,
        repo: &Path,
        folder: &Path,
        call: Call,
    ) -> services_mcp::Answer {
        let named = |host: &crate::services_glue::Host, name: &str| {
            host.resolve(repo, folder, name).ok_or_else(|| host.unknown(repo, folder, name))
        };
        match call {
            Call::List => Ok(services_mcp::list_text(&self.services.listed(repo, folder))),
            Call::Add { service, shared } => self.add_for_agent(repo, folder, service, shared),
            Call::Restart(name) => {
                let key = named(&self.services, &name)?;
                self.services.command(repo, &key, Command::Restart)
            }
            Call::Stop(name) => {
                let key = named(&self.services, &name)?;
                self.services.command(repo, &key, Command::Stop)
            }
            Call::Log { name, lines } => {
                self.services.log_text(repo, &named(&self.services, &name)?, lines)
            }
        }
    }

    fn add_for_agent(
        &mut self,
        repo: &Path,
        folder: &Path,
        service: Service,
        shared: bool,
    ) -> services_mcp::Answer {
        let service = service.in_tab((!shared).then(|| folder.to_path_buf()));
        let name = service.name.clone();
        self.services.add(repo, &service, Begin::Now).map_err(|found| {
            format!(
                "Nothing was changed. Fix these and call add_service again:\n- {}",
                found.join("\n- ")
            )
        })?;
        Ok(if shared {
            format!(
                "Saved and started `{name}` as a shared service, in the repo's main checkout. list_services shows its state."
            )
        } else {
            format!(
                "Saved and started `{name}` as your tab's own service, in your worktree. It is stopped and removed when your tab closes. list_services shows its state."
            )
        })
    }

    /// Whether the service named in the Logs view shows over the active tab.
    pub(super) fn service_logs_showing(&mut self) -> Option<Key> {
        let tab = self.tabs.items()[self.tabs.active()].id;
        let key = self.services_ui.logs_over(tab)?.clone();
        let shown = self.repo_on_show().is_some_and(|repo| self.services.has(repo, &key));
        if !shown {
            self.services_ui.close_logs();
        }
        shown.then_some(key)
    }

    pub(super) fn show_service_logs(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        key: &Key,
        keys_free: bool,
        modal_open: bool,
    ) {
        let Some(repo) = self.repo_on_show().map(Path::to_path_buf) else { return };
        let area = logs_area(area);
        let row = self.services.rows(&repo).into_iter().find(|row| &row.key == key);
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
                        if glyph_button(ui, Glyph::Stop, "Stop", Size::Compact).clicked() {
                            command = Some(Command::Stop);
                        }
                        if glyph_button(ui, Glyph::Restart, "Restart", Size::Compact).clicked() {
                            command = Some(Command::Restart);
                        }
                    } else if glyph_button(ui, Glyph::Play, "Start", Size::Compact).clicked() {
                        command = Some(Command::Start);
                    }
                });
            },
        );
        if back {
            self.services_ui.close_logs();
        }
        if let Some(command) = command {
            self.apply_service_action(Action::Run(command, key.clone()), ui.ctx());
        }
        let body_rect = Rect::from_min_max(pos2(area.left(), header.bottom()), area.max);
        if let Some(lines) = self.services.fake_lines(&repo, key) {
            paint_lines(ui, body_rect, &lines);
            return;
        }
        let grid = TerminalGrid::measure(ui, body_rect, self.terminal_font.in_use());
        let config = &self.terminal_config;
        match self.services.term_mut(&repo, key) {
            Some(term) => {
                grid.resize(term);
                show_shell(ui, term, &grid, config, keys_free, modal_open, &[]);
            }
            None => paint_lines(ui, body_rect, &[]),
        }
    }
}

fn logs_area(terminal: Rect) -> Rect {
    layout::agent_grid(terminal)
}

fn paint_lines(ui: &egui::Ui, area: Rect, lines: &[String]) {
    let painter = ui.painter_at(area);
    let font = mono(13.0);
    let step = 18.0;
    if lines.is_empty() {
        painter.text(
            area.left_top() + vec2(0.0, layout::TERMINAL_PAD),
            egui::Align2::LEFT_TOP,
            "No output yet.",
            font,
            brand::FAINT,
        );
        return;
    }
    for (n, line) in lines.iter().enumerate() {
        painter.text(
            area.left_top() + vec2(0.0, layout::TERMINAL_PAD + step * n as f32),
            egui::Align2::LEFT_TOP,
            line,
            font.clone(),
            brand::SOFT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logs_view_starts_at_the_agent_tabs_left_edge() {
        let window = Rect::from_min_max(pos2(0.0, 40.0), pos2(1000.0, 840.0));
        let terminal = layout::split_side(window, layout::SIDE_FRACTION).terminal;
        let logs = logs_area(terminal);
        assert_eq!(logs.left(), layout::EDGE);
        assert_eq!((logs.top(), logs.max), (terminal.top(), terminal.max));
    }
}
