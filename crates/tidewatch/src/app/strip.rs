//! The tab strip: workspace chip, leftovers chip, update pill, overview button, the tabs and their buttons.

use egui::{CornerRadius, Frame, Margin};
use stdusk_core::progress::Progress;
use stdusk_core::terminal::{CmdState, PtyTerm};
use stdusk_egui::colors;
use stdusk_egui::ui::{TAB_H, draw_tab, icons};
use stdusk_egui::widgets::{chip, icon_button};

use super::Tidewatch;
use super::frame::FrameIntent;
use crate::agent_frame;
use crate::agent_run::progress_fraction;
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
                    chip(ui, &self.workspace.chip_label(), true)
                        .on_hover_text(self.workspace.dir().display().to_string());
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
                        self.tabs.items().len(),
                        STRIP_GAP,
                    );
                    self.draw_tabs(ui, width, now, &mut frame.actions);
                    ui.add_space(6.0);
                    if icon_button(ui, icons::PLUS, "New tab (Cmd+T)").clicked() {
                        frame.actions.push(Action::New);
                    }
                    if icon_button(ui, icons::GEAR, "Profiles (Cmd+,)").clicked() {
                        frame.toggle_settings = true;
                    }
                });
            });
    }

    fn draw_tabs(&self, ui: &mut egui::Ui, width: f32, now: f64, actions: &mut Vec<Action>) {
        for (i, tab) in self.tabs.items().iter().enumerate() {
            let (pty_progress, cmd) = tab_signals(&tab.pane);
            let agent = tab.pane.agent.as_ref();
            let progress = if agent.is_some() { Progress::None } else { pty_progress };
            let cli = agent.and_then(|a| status_view::cli_of(&a.run.agent_id));
            let (resp, close) = draw_tab(
                ui,
                Some(i + 1),
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

fn tab_signals(pane: &Pane<PtyTerm>) -> (Progress, CmdState) {
    match &pane.body {
        Body::Shell(term) => (term.progress(), term.cmd_state()),
        _ => (Progress::default(), CmdState::default()),
    }
}
