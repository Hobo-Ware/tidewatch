//! The tab strip: repo chip (a menu of the repos, Open repo and Close repo), leftovers chip, overview button, the tabs, new tab button, and at the right end the update chip and the settings button.

use egui::layers::ShapeIdx;
use egui::{CornerRadius, Frame, Margin, Shape, vec2};
use stdusk_core::progress::Progress;
use stdusk_core::terminal::{CmdState, PtyTerm};
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
use crate::tabs::{Action, Step, Tabs};
use crate::window_chrome::{self, Chrome};
use crate::{layout, overview_view, status_view};

const STRIP_TOP_MARGIN: i8 = 6;
const STRIP_SIDE_MARGIN: i8 = 8;
const STRIP_GAP: f32 = 4.0;
const STRIP_CORNER: u8 = 10;
const TAB_RADIUS: u8 = 8;
const TAB_INSET_Y: f32 = 3.0;
const TAB_BAR_W: f32 = 2.0;
const TAB_BAR_INSET: f32 = 8.0;

impl Tidewatch {
    pub(super) fn show_tab_strip(&mut self, ui: &mut egui::Ui, now: f64, frame: &mut FrameIntent) {
        let needs_you = agent_frame::tones(&self.tabs)
            .into_iter()
            .filter(|tone| *tone == status_view::Tone::Needs)
            .count();

        let chrome = self.shell.chrome();
        let drawn = chrome == Chrome::Drawn;
        let buttons_width = self.desktop.controls_width();
        let controls = if drawn && buttons_width > 0.0 { buttons_width + STRIP_GAP } else { 0.0 };
        let corner = if drawn { 0 } else { STRIP_CORNER };
        let margin = Margin {
            left: STRIP_SIDE_MARGIN,
            right: STRIP_SIDE_MARGIN,
            top: STRIP_TOP_MARGIN,
            bottom: 0,
        };
        egui::Panel::top("tabbar")
            .exact_size(TAB_H + f32::from(STRIP_TOP_MARGIN))
            .frame(
                Frame::new()
                    .fill(brand::SIDEBAR)
                    .corner_radius(CornerRadius { nw: corner, ne: corner, sw: 0, se: 0 })
                    .inner_margin(margin),
            )
            .show(ui, |ui| {
                if drawn {
                    window_chrome::title_bar(ui, ui.max_rect() + margin, &self.desktop);
                }
                ui.horizontal(|ui| {
                    ui.set_min_height(TAB_H);
                    ui.spacing_mut().item_spacing.x = STRIP_GAP;
                    ui.add_space(4.0);
                    if self.welcome.is_some() {
                        Self::welcome_strip(ui, frame, controls);
                        if drawn {
                            window_chrome::controls(ui, &self.desktop);
                        }
                        return;
                    }
                    self.repo_chip(ui, frame);
                    self.leftovers.chip(ui);
                    ui.add_space(4.0);
                    if overview_view::button(ui, self.overview, needs_you).clicked() {
                        frame.toggle_overview = true;
                    }
                    ui.add_space(8.0);
                    let pill_width = self.updates.pill_width(ui);
                    let tail = if pill_width > 0.0 { pill_width + STRIP_GAP } else { 0.0 };
                    let width = layout::tab_width(
                        ui.available_width()
                            - stdusk_egui::widgets::ICON_BTN_W * 2.0
                            - overview_view::BUTTON_W
                            - 22.0
                            - tail
                            - controls,
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
                    ui.add_space(
                        (ui.available_width() - stdusk_egui::widgets::ICON_BTN_W - tail - controls)
                            .max(0.0),
                    );
                    self.updates.pill(ui);
                    if icon_button(
                        ui,
                        icons::GEAR,
                        &format!("Profiles ({})", Command::Settings.keys()),
                    )
                    .clicked()
                    {
                        frame.toggle_settings = true;
                    }
                    if drawn {
                        window_chrome::controls(ui, &self.desktop);
                    }
                });
            });
    }

    fn welcome_strip(ui: &mut egui::Ui, frame: &mut FrameIntent, controls: f32) {
        let resp = chip(ui, "No repo", false);
        egui::Popup::menu(&resp).show(|ui| {
            style_menu(ui);
            if menu_item(ui, "Open repo...", &Command::OpenRepo.keys()).clicked() {
                frame.repo_command = Some(RepoCommand::Open);
                ui.close();
            }
        });
        let gap = (ui.available_width() - stdusk_egui::widgets::ICON_BTN_W - controls).max(0.0);
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
                painter.circle_filled(dot, 5.5, brand::SIDEBAR);
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
                    let detail = egui::RichText::new(format!("{tabs} tabs")).color(brand::MUTED);
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
        tab_row(ui, &self.tabs, width, now, actions, tab_signals);
    }
}

/// A drawn tab: its place in the whole list and where it sits on screen.
struct Slot {
    index: usize,
    rect: egui::Rect,
    pinned: bool,
}

/// Draws the visible group's tabs and reports clicks, closes and drag-reorders as actions. Returns
/// the tabs' rects in order.
fn tab_row<T>(
    ui: &mut egui::Ui,
    tabs: &Tabs<Pane<T>>,
    width: f32,
    now: f64,
    actions: &mut Vec<Action>,
    signals: impl Fn(&Pane<T>) -> (Progress, CmdState),
) -> Vec<egui::Rect> {
    let mut slots = Vec::new();
    let mut drag = None;
    for (position, (i, tab)) in tabs.in_group(tabs.active_group()).enumerate() {
        let (pty_progress, cmd) = signals(&tab.pane);
        let agent = tab.pane.agent.as_ref();
        let progress = if agent.is_some() { Progress::None } else { pty_progress };
        let cli = agent.and_then(|a| status_view::cli_of(&a.run.agent_id));
        let painter = ui.painter().clone();
        let under = painter.add(Shape::Noop);
        let (resp, close) = draw_tab(
            ui,
            Some(position + 1),
            tab.id,
            &tab.title,
            i == tabs.active(),
            tab.pinned,
            None,
            progress,
            cmd,
            &[],
            cli,
            Some(width),
        );
        nav_look(&painter, under, resp.rect, i == tabs.active(), resp.hovered());
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
        if !tab.pinned
            && resp.dragged()
            && ui.input(|input| input.pointer.is_decidedly_dragging())
            && let Some(pointer) = resp.interact_pointer_pos()
        {
            drag = Some((position, pointer.x));
        }
        slots.push(Slot { index: i, rect: resp.rect, pinned: tab.pinned });
    }
    if let Some((from, x)) = drag {
        ui.output_mut(|output| output.cursor_icon = egui::CursorIcon::Grabbing);
        let overlay = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("tab_drag_overlay"),
        ));
        overlay.rect_filled(
            slots[from].rect.shrink2(vec2(0.0, TAB_INSET_Y)),
            CornerRadius::same(TAB_RADIUS),
            brand::TEAL.gamma_multiply(0.35),
        );
        if let Some(step) = drag_step(&slots, from, x) {
            actions.push(Action::Move(slots[from].index, step));
        }
    }
    slots.iter().map(|slot| slot.rect).collect()
}

/// Gives a tab the nav look: a teal tint, rounded, with a short bar on the left when active, a
/// fainter tint when hovered. `draw_tab` fills the active tab with its own grey and has no
/// styling, so that fill, the shape right after `under`, is swapped for the tint in place.
fn nav_look(
    painter: &egui::Painter,
    under: ShapeIdx,
    tab: egui::Rect,
    active: bool,
    hovered: bool,
) {
    let rect = tab.shrink2(vec2(0.0, TAB_INSET_Y));
    let radius = CornerRadius::same(TAB_RADIUS);
    if active {
        let fill = ShapeIdx(under.0 + 1);
        if is_fill_of(painter, fill, tab) {
            painter.set(fill, Shape::rect_filled(rect, radius, brand::TEAL_NAV));
        } else {
            painter.rect_filled(rect, radius, brand::TEAL_NAV);
        }
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.top() + TAB_BAR_INSET),
            egui::pos2(rect.left() + TAB_BAR_W, rect.bottom() - TAB_BAR_INSET),
        );
        painter.rect_filled(bar, CornerRadius::same(1), brand::TEAL);
    } else if hovered {
        painter.set(under, Shape::rect_filled(rect, radius, brand::TEAL_NAV.gamma_multiply(0.6)));
    }
}

fn is_fill_of(painter: &egui::Painter, idx: ShapeIdx, tab: egui::Rect) -> bool {
    painter.ctx().graphics(|graphics| {
        graphics
            .get(painter.layer_id())
            .and_then(|list| list.all_entries().nth(idx.0))
            .is_some_and(|clipped| matches!(&clipped.shape, Shape::Rect(rect) if rect.rect == tab))
    })
}

/// The swap a drag asks for: the pointer past the middle of the next tab on either side. Asked
/// again every frame, so a long drag moves the tab one place at a time.
fn drag_step(slots: &[Slot], from: usize, pointer_x: f32) -> Option<Step> {
    let after = slots.get(from + 1).filter(|slot| !slot.pinned);
    let before = from.checked_sub(1).and_then(|n| slots.get(n)).filter(|slot| !slot.pinned);
    if after.is_some_and(|slot| pointer_x > slot.rect.center().x) {
        Some(Step::Right)
    } else if before.is_some_and(|slot| pointer_x < slot.rect.center().x) {
        Some(Step::Left)
    } else {
        None
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

#[cfg(test)]
mod tests;
