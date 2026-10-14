//! The tab strip: repo chip (a menu of the repos, Open repo and Close repo), leftovers chip, update pill, overview button, the tabs and their buttons.

use egui::{CornerRadius, Frame, Margin};
use stdusk_core::progress::Progress;
use stdusk_core::terminal::{CmdState, PtyTerm};
use stdusk_egui::colors;
use stdusk_egui::ui::{TAB_H, draw_tab, icons};
use stdusk_egui::widgets::{chip, icon_button, menu_item, style_menu};

use super::Tidewatch;
use super::frame::FrameIntent;
use super::repos::RepoCommand;
use crate::agent_frame;
use crate::agent_run::progress_fraction;
use crate::brand;
use crate::commands::Command;
use crate::pane::{Body, Pane};
use crate::tabs::Action;
use crate::{layout, overview_view, status_view};

const STRIP_TOP_MARGIN: i8 = 6;
const STRIP_SIDE_MARGIN: i8 = 8;
const STRIP_GAP: f32 = 4.0;
const STRIP_CORNER: u8 = 10;

impl Tidewatch {
    pub(super) fn show_tab_strip(&mut self, ui: &mut egui::Ui, now: f64, frame: &mut FrameIntent) {
        let needs_you = agent_frame::tones(&self.tabs)
            .into_iter()
            .filter(|tone| *tone == status_view::Tone::Needs)
            .count();

        egui::Panel::top("tabbar")
            .exact_size(TAB_H + f32::from(STRIP_TOP_MARGIN))
            .frame(
                Frame::new()
                    .fill(colors::titlebar())
                    .corner_radius(CornerRadius {
                        nw: STRIP_CORNER,
                        ne: STRIP_CORNER,
                        sw: 0,
                        se: 0,
                    })
                    .inner_margin(Margin {
                        left: STRIP_SIDE_MARGIN,
                        right: STRIP_SIDE_MARGIN,
                        top: STRIP_TOP_MARGIN,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(TAB_H);
                    ui.spacing_mut().item_spacing.x = STRIP_GAP;
                    ui.add_space(4.0);
                    if self.welcome.is_some() {
                        Self::welcome_strip(ui, frame);
                        return;
                    }
                    self.repo_chip(ui, frame);
                    self.leftovers.chip(ui);
                    self.updates.pill(ui);
                    ui.add_space(4.0);
                    if overview_view::button(ui, self.overview, needs_you).clicked() {
                        frame.toggle_overview = true;
                    }
                    ui.add_space(8.0);
                    let width = layout::tab_width(
                        ui.available_width()
                            - stdusk_egui::widgets::ICON_BTN_W * 2.0
                            - overview_view::BUTTON_W
                            - 22.0,
                        self.tabs.in_group(self.tabs.active_group()).count(),
                        STRIP_GAP,
                    );
                    self.draw_tabs(ui, width, now, &mut frame.actions);
                    ui.add_space(6.0);
                    if icon_button(
                        ui,
                        icons::PLUS,
                        &format!("New tab ({})", Command::NewTab.keys()),
                    )
                    .clicked()
                    {
                        frame.actions.push(Action::New);
                    }
                    if icon_button(
                        ui,
                        icons::GEAR,
                        &format!("Profiles ({})", Command::Settings.keys()),
                    )
                    .clicked()
                    {
                        frame.toggle_settings = true;
                    }
                });
            });
    }

    fn welcome_strip(ui: &mut egui::Ui, frame: &mut FrameIntent) {
        let resp = chip(ui, "No repo", false);
        egui::Popup::menu(&resp).show(|ui| {
            style_menu(ui);
            if menu_item(ui, "Open repo...", &Command::OpenRepo.keys()).clicked() {
                frame.repo_command = Some(RepoCommand::Open);
                ui.close();
            }
        });
        let gap = (ui.available_width() - stdusk_egui::widgets::ICON_BTN_W).max(0.0);
        ui.add_space(gap);
        if icon_button(ui, icons::GEAR, &format!("Profiles ({})", Command::Settings.keys()))
            .clicked()
        {
            frame.toggle_settings = true;
        }
    }

    fn repo_chip(&self, ui: &mut egui::Ui, frame: &mut FrameIntent) {
        let group = self.tabs.active_group();
        let here = &self.workspaces[group];
        let several = self.workspaces.len() > 1;
        let resp = chip(ui, &here.chip_label(), true);
        let mut tip = here.dir().display().to_string();
        if several {
            if self.tabs.items().iter().any(|tab| tab.group != group && needs_you(&tab.pane)) {
                let dot = resp.rect.right_top() + egui::vec2(-2.0, 2.0);
                let painter = ui.painter();
                painter.circle_filled(dot, 5.5, colors::titlebar());
                painter.circle_filled(dot, 4.0, brand::NEEDS);
            }
            tip = format!(
                "{tip}\nSwitch repo ({} / {})",
                Command::NextRepo.keys(),
                Command::PrevRepo.keys()
            );
        }
        let resp = resp.on_hover_text(tip);
        egui::Popup::menu(&resp).show(|ui| {
            style_menu(ui);
            if several {
                for (i, workspace) in self.workspaces.iter().enumerate() {
                    let tabs = self.tabs.in_group(i).filter(|(_, tab)| !tab.pinned).count();
                    let detail = egui::RichText::new(format!("{tabs} tabs")).color(colors::dim());
                    let row = egui::Button::selectable(i == group, workspace.chip_label())
                        .shortcut_text(detail)
                        .min_size(egui::vec2(ui.available_width(), 0.0));
                    if ui.add(row).on_hover_text(workspace.dir().display().to_string()).clicked() {
                        frame.actions.push(Action::ShowGroup(i));
                        ui.close();
                    }
                }
                ui.separator();
            }
            if menu_item(ui, "Open repo...", "").clicked() {
                frame.repo_command = Some(RepoCommand::Open);
                ui.close();
            }
            let close = ui.add_enabled_ui(several, |ui| menu_item(ui, "Close repo", "")).inner;
            if close.clicked() {
                frame.repo_command = Some(RepoCommand::Close);
                ui.close();
            }
        });
    }

    fn draw_tabs(&self, ui: &mut egui::Ui, width: f32, now: f64, actions: &mut Vec<Action>) {
        for (position, (i, tab)) in self.tabs.in_group(self.tabs.active_group()).enumerate() {
            let (pty_progress, cmd) = tab_signals(&tab.pane);
            let agent = tab.pane.agent.as_ref();
            let progress = if agent.is_some() { Progress::None } else { pty_progress };
            let cli = agent.and_then(|a| status_view::cli_of(&a.run.agent_id));
            let (resp, close) = draw_tab(
                ui,
                Some(position + 1),
                tab.id,
                &tab.title,
                i == self.tabs.active(),
                tab.pinned,
                None,
                progress,
                cmd,
                &[],
                cli,
                Some(width),
            );
            if let Some(agent) = agent {
                let tone = status_view::tone(agent.run.status());
                let fraction = progress_fraction(pty_progress);
                status_view::paint_bar(ui, resp.rect, tone, now, fraction);
            }
            if close {
                actions.push(Action::Close(i));
            } else if resp.clicked() {
                actions.push(Action::Select(i));
            }
        }
    }
}

fn needs_you<T>(pane: &Pane<T>) -> bool {
    pane.agent
        .as_ref()
        .is_some_and(|agent| status_view::tone(agent.run.status()) == status_view::Tone::Needs)
}

fn tab_signals(pane: &Pane<PtyTerm>) -> (Progress, CmdState) {
    match &pane.body {
        Body::Shell(term) => (term.progress(), term.cmd_state()),
        _ => (Progress::default(), CmdState::default()),
    }
}
