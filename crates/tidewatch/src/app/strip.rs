//! The tab strip: repo chip (a menu of the repos, Open repo and Close repo), leftovers chip, overview button, the tabs, new tab button, and at the right end the update chip and the settings button.

use egui::layers::ShapeIdx;
use egui::{CornerRadius, Frame, Margin, Shape, pos2, vec2};
use stdusk_core::progress::Progress;
use stdusk_core::terminal::{CmdState, PtyTerm};
use stdusk_egui::ui::{TAB_H, TAB_PAD_X, TAB_SLOT_W, draw_tab, ellipsize, icons};
use stdusk_egui::widgets::{chip, icon_button, menu_item, style_menu};

use super::Tidewatch;
use super::frame::FrameIntent;
use super::repos::RepoCommand;
use crate::agent_frame;
use crate::agent_run::progress_fraction;
use crate::brand;
use crate::commands::Command;
use crate::pane::{Body, Pane};
use crate::progress_model::{TabRing, tab_ring};
use crate::progress_ring;
use crate::tabs::{Action, Step, Tabs};
use crate::{layout, overview_view, status_view};

const STRIP_TOP_MARGIN: i8 = 6;
const STRIP_SIDE_MARGIN: i8 = 8;
const STRIP_GAP: f32 = 4.0;
const STRIP_CORNER: u8 = 10;
const TAB_RADIUS: u8 = 8;
const TAB_INSET_Y: f32 = 3.0;
const TAB_BAR_W: f32 = 2.0;
const TAB_BAR_INSET: f32 = 8.0;
const RING_SIZE: f32 = 16.0;
const RING_GAP: f32 = 6.0;

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
                    .fill(brand::SIDEBAR)
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
                            - tail,
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
                        (ui.available_width() - stdusk_egui::widgets::ICON_BTN_W - tail).max(0.0),
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
        let rings = |id| self.progress.ledger(id).map(|known| tab_ring(&known.ledger));
        tab_row(ui, &self.tabs, width, now, actions, (tab_signals, rings));
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
    (signals, rings): (impl Fn(&Pane<T>) -> (Progress, CmdState), impl Fn(u64) -> Option<TabRing>),
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
        let draft = matches!(tab.pane.body, Body::Draft(_));
        let ring = agent.and_then(|_| rings(tab.id));
        let title = match (&ring, draft) {
            (_, true) => String::new(),
            (Some(_), false) => ring_title(ui, &tab.title, position + 1, width),
            (None, false) => tab.title.clone(),
        };
        let (resp, close) = draw_tab(
            ui,
            Some(position + 1),
            tab.id,
            &title,
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
        if draft {
            paint_draft_title(&painter, resp.rect, position + 1, &tab.title);
        }
        if let Some(ring) = &ring {
            paint_ring(ui, resp.rect, ring, tab.id);
        }
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

/// How many characters of a tab's title fit in a tab `width` wide that also holds a ring: the
/// tab's padding, the trailing slot and the ring come off, then the number before the title.
pub(crate) fn title_budget(width: f32, char_w: f32, number: usize) -> usize {
    let taken = 2.0 * TAB_PAD_X + TAB_SLOT_W + RING_GAP + RING_SIZE + RING_GAP;
    let prefix = number.to_string().len() + 2;
    (((width - taken) / char_w).floor() as usize).saturating_sub(prefix).max(1)
}

fn ring_title(ui: &egui::Ui, title: &str, number: usize, width: f32) -> String {
    let char_w = ui
        .painter()
        .layout_no_wrap("0".to_owned(), egui::FontId::monospace(12.0), brand::SOFT)
        .size()
        .x;
    ellipsize(title, title_budget(width, char_w, number)).0
}

fn ring_rect(tab: egui::Rect) -> egui::Rect {
    let right = tab.right() - TAB_PAD_X - TAB_SLOT_W - RING_GAP;
    egui::Rect::from_center_size(
        pos2(right - RING_SIZE / 2.0, tab.center().y),
        vec2(RING_SIZE, RING_SIZE),
    )
}

fn paint_ring(ui: &mut egui::Ui, tab: egui::Rect, ring: &TabRing, id: u64) {
    let rect = ring_rect(tab);
    progress_ring::paint(
        ui.painter(),
        rect.center(),
        RING_SIZE,
        ring.ring.fraction,
        progress_ring::tone_color(ring.ring.tone),
        brand::ITEM_LINE,
    );
    ui.interact(rect.expand(3.0), ui.id().with(("tab-ring", id)), egui::Sense::hover())
        .on_hover_text(&ring.tip);
}

/// The label of a draft tab, in italics after the tab's number: `draw_tab` has no italic title, so
/// it is given none and this one is laid over the same place.
fn paint_draft_title(painter: &egui::Painter, tab: egui::Rect, number: usize, title: &str) {
    let digits = painter.layout_no_wrap("0".to_owned(), egui::FontId::monospace(12.0), brand::SOFT);
    let prefix = (number.to_string().len() + 2) as f32 * digits.size().x;
    let x = tab.left() + TAB_PAD_X + prefix;
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        egui::FontId::proportional(13.0),
        brand::SOFT,
    );
    job.sections[0].format.italics = true;
    job.wrap = egui::text::TextWrapping::truncate_at_width(
        (tab.right() - x - TAB_PAD_X - TAB_SLOT_W - 6.0).max(1.0),
    );
    let galley = painter.layout_job(job);
    painter.galley(pos2(x, tab.center().y - galley.size().y / 2.0), galley, brand::SOFT);
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
