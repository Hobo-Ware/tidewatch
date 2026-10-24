//! The tab strip: overview button, repo chip (a menu of the repos, Open repo and Close repo), the repo's main checkout as an icon, leftovers chip, the agent tabs, new tab button, and at the right end the update chip and the settings button.

use egui::layers::ShapeIdx;
use egui::{CornerRadius, Frame, Margin, Shape, pos2, vec2};
use stdusk_core::progress::Progress;
use stdusk_core::terminal::{CmdState, PtyTerm};
use stdusk_egui::ui::{TAB_H, TAB_PAD_X, TAB_SLOT_W, draw_tab, ellipsize, icons};
use stdusk_egui::widgets::{chip, icon_button, menu_item, style_menu};

use super::frame::FrameIntent;
use super::repos::RepoCommand;
use std::sync::Arc;

use tidewatch_core::avatar::Avatar;

use super::{Tidewatch, wall_ms};
use crate::agent_avatar::{self, Face};
use crate::agent_frame;
use crate::agent_run::progress_fraction;
use crate::brand;
use crate::commands::Command;
use crate::job_model::{TabJob, tab_job};
use crate::pane::{Body, Pane};
use crate::park::Fits;
use crate::progress_model::{TabRing, tab_ring};
use crate::progress_ring;
use crate::tabs::{Action, Step, Tabs};
use crate::window_chrome::{self, Chrome};
use crate::{layout, overview_view, status_view};

mod main_icon;
pub(crate) mod pill;

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
const AVATAR_SIZE: f32 = 20.0;
const AVATAR_PAD_CHARS: usize = 4;
const PILL_DROP_SLACK: f32 = 6.0;
const JOB_BAR_H: f32 = 2.0;
const JOB_BAR_INSET: f32 = 6.0;

/// What an agent tab shows besides its title: the ledger's ring and the leading job's percent.
#[derive(Default)]
pub(crate) struct TabExtras {
    pub(crate) ring: Option<TabRing>,
    pub(crate) job: Option<TabJob>,
    pub(crate) avatar: Option<Arc<Avatar>>,
    pub(crate) mood: Mood,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Mood {
    pub(crate) struggling: bool,
    pub(crate) fade: Option<f32>,
}

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
                    if overview_view::button(ui, self.overview, needs_you).clicked() {
                        frame.toggle_overview = true;
                    }
                    self.repo_chip(ui, frame);
                    self.main_icon(ui, frame);
                    self.leftovers.chip(ui);
                    ui.add_space(8.0);
                    let pill_width = self.updates.pill_width(ui);
                    let tail = if pill_width > 0.0 { pill_width + STRIP_GAP } else { 0.0 };
                    let group = self.tabs.active_group();
                    let rows = self.parked_rows(group);
                    let parked_tail = if pill::wanted(ui.ctx(), rows.len()) {
                        pill::width(ui, rows.len()) + STRIP_GAP
                    } else {
                        0.0
                    };
                    let room = ui.available_width()
                        - stdusk_egui::widgets::ICON_BTN_W * 2.0
                        - overview_view::BUTTON_W
                        - 22.0
                        - tail
                        - controls;
                    self.parking.set_fits(Fits {
                        alone: layout::tab_capacity(room, STRIP_GAP),
                        with_pill: layout::tab_capacity(
                            room - pill::widest(ui) - STRIP_GAP,
                            STRIP_GAP,
                        ),
                    });
                    let width = layout::tab_width(
                        room - parked_tail,
                        self.tabs.shown_in(group).filter(|(_, tab)| !tab.pinned).count(),
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
                        (ui.available_width()
                            - stdusk_egui::widgets::ICON_BTN_W
                            - tail
                            - parked_tail
                            - controls)
                            .max(0.0),
                    );
                    if pill::wanted(ui.ctx(), rows.len()) {
                        let out = pill::show(ui, &rows, self.dev_parked_popover);
                        if let Some(row) = out.picked {
                            frame.actions.push(Action::Select(rows[row].index));
                        }
                    }
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
        let now_ms = wall_ms() as i64;
        let mood = |id| {
            let meeseeks = self
                .tabs
                .items()
                .iter()
                .find(|tab| tab.id == id)
                .and_then(|tab| tab.pane.agent.as_ref())
                .and_then(|agent| agent.meeseeks.as_ref());
            Mood {
                struggling: meeseeks.is_some_and(|m| m.struggle(now_ms).is_some()),
                fade: self.meeseeks.fade(id, now),
            }
        };
        let extras = |id| TabExtras {
            ring: self.progress.ledger(id).map(|known| tab_ring(&known.ledger)),
            job: self.job_live.board(id).and_then(|board| tab_job(board, now_ms)),
            avatar: self.avatar_live.get(id).cloned(),
            mood: mood(id),
        };
        tab_row(ui, &self.tabs, width, now, actions, (tab_signals, extras));
    }

    fn parked_rows(&self, group: usize) -> Vec<pill::Row> {
        let now_ms = wall_ms() as u64;
        let today = crate::progress_glue::today();
        self.tabs
            .parked_in(group)
            .filter_map(|(index, tab)| {
                let park = tab.park?;
                let face = match &tab.pane.agent {
                    Some(agent) => Face::of(
                        &agent.profile_name,
                        &agent.run.agent_id,
                        self.avatar_live.get(tab.id).cloned(),
                    ),
                    None => Face::of(&tab.title, "", None),
                };
                let line = self.parking.fake_line(tab.id).unwrap_or_else(|| {
                    let board = self.job_live.board(tab.id);
                    let job = crate::park::job_line(board, now_ms as i64);
                    let schedule =
                        self.progress.ledger(tab.id).map_or(crate::park::Schedule::None, |known| {
                            crate::park::schedule_of(&known.ledger, today)
                        });
                    crate::park::reason_line(park, now_ms, job, schedule)
                });
                Some(pill::Row { index, name: tab.title.clone(), face, line })
            })
            .collect()
    }
}

/// A drawn tab: its place in the whole list and where it sits on screen.
struct Slot {
    index: usize,
    rect: egui::Rect,
}

struct MenuView {
    parking: ParkMenu,
    active: bool,
    picture: bool,
}

#[derive(Clone, Copy)]
enum ParkMenu {
    Hidden,
    Offered { parkable: bool, keep: bool },
}

fn tab_row<T>(
    ui: &mut egui::Ui,
    tabs: &Tabs<Pane<T>>,
    width: f32,
    now: f64,
    actions: &mut Vec<Action>,
    (signals, extras): (impl Fn(&Pane<T>) -> (Progress, CmdState), impl Fn(u64) -> TabExtras),
) -> Vec<egui::Rect> {
    let mut slots = Vec::new();
    let mut drag = None;
    let pill = pill::rect(ui.ctx());
    let mut over_pill = false;
    for (position, (i, tab)) in tabs.shown_in(tabs.active_group()).enumerate() {
        if tab.pinned {
            continue;
        }
        let (pty_progress, cmd) = signals(&tab.pane);
        let agent = tab.pane.agent.as_ref();
        let progress = if agent.is_some() { Progress::None } else { pty_progress };
        let painter = ui.painter().clone();
        let under = painter.add(Shape::Noop);
        let draft = matches!(tab.pane.body, Body::Draft(_));
        let TabExtras { ring, job, avatar, mood } =
            agent.map(|_| extras(tab.id)).unwrap_or_default();
        let face = agent.map(|agent| Face::of(&agent.profile_name, &agent.run.agent_id, avatar));
        let title = match (&face, draft) {
            (_, true) => String::new(),
            (Some(_), false) => face_title(ui, &tab.title, width, ring.is_some()),
            (None, false) => tab.title.clone(),
        };
        let number = face.is_none().then_some(position + 1);
        let (resp, close) = draw_tab(
            ui,
            number,
            tab.id,
            &title,
            i == tabs.active(),
            false,
            None,
            progress,
            cmd,
            &[],
            None,
            Some(width),
        );
        nav_look(&painter, under, resp.rect, i == tabs.active(), resp.hovered());
        if mood.struggling {
            paint_struggle(&painter, resp.rect);
        }
        if draft {
            paint_draft_title(&painter, resp.rect, position + 1, &tab.title);
        }
        if let Some(ring) = &ring {
            paint_ring(ui, resp.rect, ring, tab.id);
        }
        if let Some(job) = &job {
            paint_job(ui, resp.rect, job);
        }
        if let Some(face) = &face {
            let rect = face_rect(resp.rect);
            if agent_avatar::paint_face(ui.ctx(), ui.painter(), rect, face)
                && let Some(avatar) = face.avatar()
            {
                agent_avatar::hover(ui, rect, tab.id, avatar);
            }
        }
        let menu = MenuView {
            parking: if draft {
                ParkMenu::Hidden
            } else {
                ParkMenu::Offered { parkable: !needs_you(&tab.pane), keep: tab.keep }
            },
            active: i == tabs.active(),
            picture: face.as_ref().is_some_and(|face| face.avatar().is_some())
                && !agent_avatar::is_locked(ui.ctx(), tab.id),
        };
        tab_menu(&resp, i, tab.id, &menu, actions);
        let resp = if agent.is_some() {
            resp.on_hover_text(tab_tip(&tab.title, position + 1, job.as_ref()))
        } else {
            resp
        };
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
        if resp.dragged()
            && ui.input(|input| input.pointer.is_decidedly_dragging())
            && let Some(pointer) = resp.interact_pointer_pos()
        {
            drag = Some((slots.len(), pointer.x));
            over_pill = pill.is_some_and(|pill| pill.expand(PILL_DROP_SLACK).contains(pointer));
        }
        if resp.drag_stopped()
            && let Some(pill) = pill
            && ui
                .input(|input| input.pointer.latest_pos())
                .is_some_and(|at| pill.expand(PILL_DROP_SLACK).contains(at))
        {
            actions.push(Action::Park(i));
        }
        if let Some(fade) = mood.fade {
            painter.rect_filled(
                resp.rect,
                CornerRadius::same(TAB_RADIUS),
                brand::SIDEBAR.gamma_multiply(fade),
            );
        }
        slots.push(Slot { index: i, rect: resp.rect });
    }
    pill::set_hot(ui.ctx(), over_pill);
    pill::set_dragging(ui.ctx(), drag.is_some());
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
        if let Some(step) = drag_step(&slots, from, x).filter(|_| !over_pill) {
            actions.push(Action::Move(slots[from].index, step));
        }
    }
    slots.iter().map(|slot| slot.rect).collect()
}

fn paint_struggle(painter: &egui::Painter, tab: egui::Rect) {
    let body = tab.shrink2(vec2(0.0, TAB_INSET_Y));
    painter.rect_filled(body, CornerRadius::same(TAB_RADIUS), brand::ORANGE.gamma_multiply(0.16));
    let bar = egui::Rect::from_min_size(
        body.left_top() + vec2(0.0, TAB_BAR_INSET),
        vec2(TAB_BAR_W, body.height() - 2.0 * TAB_BAR_INSET),
    );
    painter.rect_filled(bar, CornerRadius::same(1), brand::ORANGE);
}

fn tab_tip(title: &str, number: usize, job: Option<&TabJob>) -> String {
    let keys = number.checked_sub(1).map(Command::SelectTab).map(Command::keys).unwrap_or_default();
    let place = if number <= 9 && !keys.is_empty() {
        format!("Tab {number} ({keys})")
    } else {
        format!("Tab {number}")
    };
    match job {
        Some(job) => format!("{title}\n{place}\n{}", job.tip),
        None => format!("{title}\n{place}"),
    }
}

fn tab_menu(
    resp: &egui::Response,
    index: usize,
    tab: u64,
    menu: &MenuView,
    actions: &mut Vec<Action>,
) {
    let offered = matches!(menu.parking, ParkMenu::Offered { .. });
    if !offered && !menu.picture {
        return;
    }
    resp.context_menu(|ui| {
        style_menu(ui);
        if let ParkMenu::Offered { parkable, keep } = menu.parking {
            let keys = if menu.active { Command::ParkTab.keys() } else { String::new() };
            let park = ui.add_enabled_ui(parkable, |ui| menu_item(ui, "Park", &keys)).inner;
            if park.clicked() {
                actions.push(Action::Park(index));
                ui.close();
            }
            let keep = if keep { "Allow parking this tab" } else { "Never park this tab" };
            if menu_item(ui, keep, "").clicked() {
                actions.push(Action::ToggleKeep(index));
                ui.close();
            }
        }
        if menu.picture {
            if offered {
                ui.separator();
            }
            agent_avatar::menu_items(ui, tab);
        }
    });
}

pub(crate) fn title_budget(width: f32, char_w: f32, ring: bool) -> usize {
    let mut taken = 2.0 * TAB_PAD_X + TAB_SLOT_W + RING_GAP;
    if ring {
        taken += RING_SIZE + RING_GAP;
    }
    (((width - taken) / char_w).floor() as usize).max(1)
}

fn face_title(ui: &egui::Ui, title: &str, width: f32, ring: bool) -> String {
    let char_w = ui
        .painter()
        .layout_no_wrap("0".to_owned(), egui::FontId::monospace(12.0), brand::SOFT)
        .size()
        .x;
    let room = title_budget(width, char_w, ring);
    let name = ellipsize(title, room.saturating_sub(AVATAR_PAD_CHARS).max(1)).0;
    format!("{}{name}", " ".repeat(AVATAR_PAD_CHARS))
}

fn face_rect(tab: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        pos2(tab.left() + TAB_PAD_X, tab.center().y - AVATAR_SIZE / 2.0),
        vec2(AVATAR_SIZE, AVATAR_SIZE),
    )
}

fn ring_rect(tab: egui::Rect) -> egui::Rect {
    let right = tab.right() - TAB_PAD_X - TAB_SLOT_W - RING_GAP;
    egui::Rect::from_center_size(
        pos2(right - RING_SIZE / 2.0, tab.center().y),
        vec2(RING_SIZE, RING_SIZE),
    )
}

fn job_bar_rect(tab: egui::Rect, fraction: f32) -> egui::Rect {
    let floor = tab.bottom() - TAB_INSET_Y;
    let room = (tab.width() - 2.0 * JOB_BAR_INSET).max(0.0);
    egui::Rect::from_min_size(
        pos2(tab.left() + JOB_BAR_INSET, floor - JOB_BAR_H),
        vec2(room * fraction.clamp(0.0, 1.0), JOB_BAR_H),
    )
}

fn paint_job(ui: &mut egui::Ui, tab: egui::Rect, job: &TabJob) {
    let ink = if job.stalled { brand::ORANGE } else { brand::TEAL };
    ui.painter().rect_filled(job_bar_rect(tab, job.fraction), CornerRadius::same(1), ink);
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
    let after = slots.get(from + 1);
    let before = from.checked_sub(1).and_then(|n| slots.get(n));
    if after.is_some_and(|slot| pointer_x > slot.rect.center().x) {
        Some(Step::Right)
    } else if before.is_some_and(|slot| pointer_x < slot.rect.center().x) {
        Some(Step::Left)
    } else {
        None
    }
}

pub(super) fn needs_you<T>(pane: &Pane<T>) -> bool {
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
