//! The tab strip: overview button, repo chip (a menu of the repos, Open repo and Close repo), the Parked pill when something is parked, the repo's main checkout as an icon, leftovers chip, the agent tabs, new tab button, and at the right end the update chip and the settings button.

use egui::layers::ShapeIdx;
use egui::{CornerRadius, Frame, Margin, Shape, Stroke, StrokeKind, pos2, vec2};
use stdusk_core::progress::Progress;
use stdusk_core::terminal::{CmdState, PtyTerm};
use stdusk_egui::ui::{TAB_PAD_X, TAB_SLOT_W, TabMetrics, draw_tab_sized, icons};
use stdusk_egui::widgets::{IconButtonSize, chip, icon_button_sized, menu_item, style_menu};

use super::frame::FrameIntent;
use super::repos::RepoCommand;
use std::sync::Arc;

use tidewatch_core::avatar::Avatar;

use super::{Tidewatch, wall_ms};
use crate::agent_avatar::{self, Face};
use crate::amphetamine_view;
use crate::avatar_glue::meeseeks_avatar;
use crate::brand;
use crate::commands::Command;
use crate::job_model::{TabJob, tab_job};
use crate::pane::{Body, Pane};
use crate::park::Fits;
use crate::progress_model::{TabRing, tab_ring};
use crate::remote_pill::Popover;
use crate::tabs::{Action, Step, Tabs};
use crate::window_chrome::{self, Chrome};
use crate::{layout, overview_view, status_view};

mod main_icon;
pub(crate) mod pill;
mod repo_chip;

pub(crate) const STRIP_H: f32 = 48.0;
pub(crate) const TAB_H: f32 = 36.0;
pub(crate) const ICON_BOX: f32 = 36.0;
pub(crate) const ICON_GLYPH: f32 = 22.0;
const ROUND_RADIUS: f32 = 18.0;
const TAB_FONT_PX: f32 = 14.0;
const STRIP_VERTICAL_MARGIN: i8 = 6;
const STRIP_SIDE_MARGIN: i8 = 8;
const STRIP_GAP: f32 = 4.0;
const STRIP_CORNER: u8 = 10;
const TAB_RADIUS: u8 = 18;
const TAB_INSET: f32 = 14.0;
const AVATAR_SIZE: f32 = 24.0;
const AVATAR_GAP: f32 = 9.0;
const SLASH_SIZE: f32 = 22.0;
const SLASH_MARGIN: f32 = 6.0;
const TITLE_DOT_GAP: f32 = 8.0;
const PILL_DROP_SLACK: f32 = 6.0;
const POOFED_VEIL: f32 = 0.6;
const STATUS_DOT_TRAILING: f32 = TAB_PAD_X + TAB_SLOT_W / 2.0;

/// What an agent tab keeps besides its title: the ledger's and the leading job's words for its tooltip.
#[derive(Default)]
pub(crate) struct TabExtras {
    pub(crate) ring: Option<TabRing>,
    pub(crate) job: Option<TabJob>,
    pub(crate) avatar: Option<Arc<Avatar>>,
    pub(crate) can_draw: bool,
    pub(crate) mood: Mood,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Mood {
    pub(crate) struggling: bool,
}

impl Tidewatch {
    pub(super) fn show_tab_strip(&mut self, ui: &mut egui::Ui, now: f64, frame: &mut FrameIntent) {
        let waiting_by_repo: Vec<(String, usize)> = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(group, workspace)| {
                let waiting = self
                    .tabs
                    .items()
                    .iter()
                    .filter(|tab| tab.group == group && needs_you(&tab.pane))
                    .count();
                (workspace.chip_label(), waiting)
            })
            .collect();

        let chrome = self.shell.chrome();
        let drawn = chrome == Chrome::Drawn;
        let buttons_width = self.desktop.controls_width();
        let controls = if drawn && buttons_width > 0.0 { buttons_width + STRIP_GAP } else { 0.0 };
        let corner = if drawn { 0 } else { STRIP_CORNER };
        let margin = Margin {
            left: STRIP_SIDE_MARGIN,
            right: STRIP_SIDE_MARGIN,
            top: STRIP_VERTICAL_MARGIN,
            bottom: STRIP_VERTICAL_MARGIN,
        };
        egui::Panel::top("tabbar")
            .exact_size(STRIP_H)
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
                    if overview_view::button(ui, self.overview, &waiting_by_repo).clicked() {
                        frame.toggle_overview = true;
                    }
                    self.repo_chip(ui, frame);
                    let group = self.tabs.active_group();
                    let rows = self.parked_rows(group);
                    let parked_tail =
                        if rows.is_empty() { 0.0 } else { pill::width(ui, rows.len()) + STRIP_GAP };
                    let picked = lead(
                        ui,
                        (!rows.is_empty()).then_some((rows.as_slice(), self.dev_parked_popover)),
                        |ui| self.main_icon(ui, frame),
                    );
                    if let Some(row) = picked {
                        frame.actions.push(Action::Select(rows[row].index));
                    }
                    self.leftovers.chip(ui);
                    ui.add_space(8.0);
                    let pill_width = self.updates.pill_width(ui);
                    let tail = if pill_width > 0.0 { pill_width + STRIP_GAP } else { 0.0 };
                    let room = ui.available_width()
                        - ICON_BOX * 2.0
                        - 26.0
                        - tail
                        - controls
                        - amphetamine_view::SLOT_W
                        - STRIP_GAP;
                    let open = room + parked_tail;
                    self.parking.set_fits(Fits {
                        alone: layout::tab_capacity(open, STRIP_GAP),
                        with_pill: layout::tab_capacity(
                            open - pill::widest(ui) - STRIP_GAP,
                            STRIP_GAP,
                        ),
                    });
                    let width = layout::tab_width(
                        room,
                        self.tabs.shown_in(group).filter(|(_, tab)| !tab.pinned).count(),
                        STRIP_GAP,
                    );
                    self.draw_tabs(ui, width, now, &mut frame.actions);
                    ui.add_space(6.0);
                    let plus = strip_button(
                        ui,
                        icons::PLUS,
                        &format!("New tab ({})", Command::NewTab.keys()),
                        ROUND_RADIUS,
                    );
                    if plus.clicked() {
                        frame.actions.push(Action::New);
                    }
                    ui.add_space(
                        (ui.available_width()
                            - ICON_BOX
                            - tail
                            - controls
                            - amphetamine_view::SLOT_W
                            - STRIP_GAP)
                            .max(0.0),
                    );
                    self.amphetamine_button(
                        ui,
                        ui.next_widget_position().x - plus.rect.right(),
                        now,
                    );
                    self.updates.pill(ui);
                    if strip_button(
                        ui,
                        icons::GEAR,
                        &format!("Profiles ({})", Command::Settings.keys()),
                        ROUND_RADIUS,
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

    fn amphetamine_button(&mut self, ui: &mut egui::Ui, gap: f32, now: f64) {
        let wall = super::amphetamine::wall_secs();
        let input = amphetamine_view::Input {
            look: self.amphetamine.look(now),
            session: self.amphetamine.state(),
            wall,
            total: self.amphetamine.total(),
            lid: self.amphetamine.lid(),
            route: self.amphetamine.route(),
            gap,
            force_short: self.amphetamine.short(),
            pinned: self.amphetamine.pinned(),
        };
        let out = amphetamine_view::show(ui, &input, &mut self.amphetamine_view, now);
        if let Some(action) = out.action {
            self.apply_amphetamine(action, ui.ctx());
        }
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
        let gap = (ui.available_width() - ICON_BOX - controls).max(0.0);
        ui.add_space(gap);
        if strip_button(
            ui,
            icons::GEAR,
            &format!("Profiles ({})", Command::Settings.keys()),
            ROUND_RADIUS,
        )
        .clicked()
        {
            frame.toggle_settings = true;
        }
    }

    fn repo_chip(&self, ui: &mut egui::Ui, frame: &mut FrameIntent) {
        let group = self.tabs.active_group();
        let here = &self.workspaces[group];
        let several = self.workspaces.len() > 1;
        let resp = repo_chip::show(ui, &here.chip_label());
        let mut tip = format!("{}\n{}", here.chip_label(), here.dir().display());
        if several {
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
        let table = self.agent_table();
        let can_draw = |id| {
            self.tabs
                .items()
                .iter()
                .find(|tab| tab.id == id)
                .and_then(|tab| tab.pane.agent.as_ref())
                .and_then(|agent| table.get(&agent.run.agent_id))
                .is_some_and(|agent| tidewatch_core::avatar_ask::can_draw(agent.wiring))
        };
        let mood = |id| {
            let meeseeks = self
                .tabs
                .items()
                .iter()
                .find(|tab| tab.id == id)
                .and_then(|tab| tab.pane.agent.as_ref())
                .and_then(|agent| agent.meeseeks.as_ref());
            Mood { struggling: meeseeks.is_some_and(|m| m.struggle(now_ms).is_some()) }
        };
        let extras = |id| TabExtras {
            ring: self
                .progress
                .ledger(id)
                .map(|known| tab_ring(&known.ledger, self.ledger_stale(id, now))),
            job: self
                .job_live
                .board(id)
                .and_then(|board| tab_job(board, now_ms, self.job_agent(id))),
            avatar: self.avatar_live.get(id).cloned(),
            can_draw: can_draw(id),
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
                    None if matches!(tab.pane.body, Body::Poofed(_)) => {
                        Face::Picture(meeseeks_avatar())
                    }
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
                let name = crate::tab_title::label(tab.pane.agent.as_ref(), &tab.title);
                Some(pill::Row { index, name, face, line })
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
    face: AvatarMenu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AvatarMenu {
    None,
    Picture,
    Letter,
}

fn avatar_menu(has_picture: bool, can_draw: bool, locked: bool) -> AvatarMenu {
    match (locked, has_picture, can_draw) {
        (true, _, _) | (false, false, false) => AvatarMenu::None,
        (false, true, _) => AvatarMenu::Picture,
        (false, false, true) => AvatarMenu::Letter,
    }
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
        let poofed = matches!(tab.pane.body, Body::Poofed(_));
        let TabExtras { ring, job, avatar, can_draw, mood } =
            agent.map(|_| extras(tab.id)).unwrap_or_default();
        let face = agent
            .map(|agent| Face::of(&agent.profile_name, &agent.run.agent_id, avatar))
            .or_else(|| poofed.then(|| Face::Picture(meeseeks_avatar())));
        let label = crate::tab_title::label(agent, &tab.title);
        let (resp, close) = draw_tab_sized(
            ui,
            None,
            tab.id,
            "",
            i == tabs.active(),
            false,
            None,
            progress,
            cmd,
            &[],
            None,
            Some(width),
            TabMetrics { height: TAB_H, font_px: TAB_FONT_PX },
        );
        tab_look(&painter, under, resp.rect, i == tabs.active(), resp.hovered(), mood.struggling);
        let text = match (&face, draft) {
            (Some(_), false) => label.clone(),
            _ => format!("{}  {}", position + 1, tab.title),
        };
        let inset = if face.is_some() { AVATAR_SIZE + AVATAR_GAP } else { 0.0 };
        paint_title(&painter, resp.rect, &text, inset, (i == tabs.active(), draft));
        if let Some(face) = &face {
            let rect = face_rect(resp.rect);
            if agent_avatar::paint_face(ui.ctx(), ui.painter(), rect, face)
                && let Some(avatar) = face.avatar()
            {
                agent_avatar::hover(ui, rect, tab.id, avatar);
            }
            if poofed {
                painter.circle_filled(
                    rect.center(),
                    rect.width() / 2.0,
                    brand::SIDEBAR.gamma_multiply(POOFED_VEIL),
                );
            }
        }
        let menu = MenuView {
            parking: if draft {
                ParkMenu::Hidden
            } else {
                ParkMenu::Offered { parkable: !needs_you(&tab.pane), keep: tab.keep }
            },
            active: i == tabs.active(),
            face: avatar_menu(
                face.as_ref().is_some_and(|face| face.avatar().is_some()),
                can_draw,
                poofed || agent_avatar::is_locked(ui.ctx(), tab.id),
            ),
        };
        tab_menu(&resp, i, tab.id, &menu, actions);
        let resp = if agent.is_some() {
            let second = crate::tab_title::subtitle(agent, &tab.title);
            let mut tip = tab_tip(&label, &second, position + 1, job.as_ref());
            if let Some(ring) = &ring {
                tip = format!("{tip}\n{}", ring.tip);
            }
            resp.on_hover_text(tip)
        } else if let Body::Poofed(record) = &tab.pane.body {
            let second =
                crate::tab_title::task_line(&record.task).unwrap_or_else(|| tab.title.clone());
            resp.on_hover_text(format!("{}\nPoofed", tab_tip(&label, &second, position + 1, None)))
        } else {
            resp
        };
        if let Some(agent) = agent
            && !resp.contains_pointer()
        {
            let tone = agent.run.shown_tone();
            let center = status_view::dot_center(resp.rect, STATUS_DOT_TRAILING);
            status_view::paint_dot(ui, center, tone, now);
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
        overlay.rect_stroke(
            slots[from].rect,
            CornerRadius::same(TAB_RADIUS),
            Stroke::new(2.0, brand::TEAL),
            StrokeKind::Inside,
        );
        if let Some(step) = drag_step(&slots, from, x).filter(|_| !over_pill) {
            actions.push(Action::Move(slots[from].index, step));
        }
    }
    slots.iter().map(|slot| slot.rect).collect()
}

fn tab_tip(label: &str, name: &str, number: usize, job: Option<&TabJob>) -> String {
    let keys = number.checked_sub(1).map(Command::SelectTab).map(Command::keys).unwrap_or_default();
    let place = if number <= 9 && !keys.is_empty() {
        format!("Tab {number} ({keys})")
    } else {
        format!("Tab {number}")
    };
    let head = if label == name { label.to_owned() } else { format!("{label}\n{name}") };
    match job {
        Some(job) => format!("{head}\n{place}\n{}", job.tip),
        None => format!("{head}\n{place}"),
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
    if !offered && menu.face == AvatarMenu::None {
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
        if menu.face != AvatarMenu::None {
            if offered {
                ui.separator();
            }
            agent_avatar::menu_items(ui, tab, menu.face == AvatarMenu::Picture);
        }
    });
}

fn face_rect(tab: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        pos2(tab.left() + TAB_INSET, tab.center().y - AVATAR_SIZE / 2.0),
        vec2(AVATAR_SIZE, AVATAR_SIZE),
    )
}

fn paint_title(
    painter: &egui::Painter,
    tab: egui::Rect,
    text: &str,
    inset: f32,
    (active, draft): (bool, bool),
) {
    let x = tab.left() + TAB_INSET + inset;
    let (font, ink) = if active {
        (crate::screen_kit::heading(TAB_FONT_PX), brand::TEAL_TEXT)
    } else {
        (egui::FontId::proportional(TAB_FONT_PX), brand::SOFT)
    };
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, ink);
    job.sections[0].format.italics = draft;
    job.wrap = egui::text::TextWrapping::truncate_at_width(
        (tab.right() - x - TAB_PAD_X - TAB_SLOT_W - TITLE_DOT_GAP).max(1.0),
    );
    let galley = painter.layout_job(job);
    painter.galley(pos2(x, tab.center().y - galley.size().y / 2.0), galley, ink);
}

/// Gives a tab its round outline: teal when active, orange while the Meeseeks struggles, a
/// brighter line when hovered. `draw_tab` fills the active tab with its own grey, the shape right
/// after `under`; that fill is blanked in place, since no tab carries a colour fill.
fn tab_look(
    painter: &egui::Painter,
    under: ShapeIdx,
    tab: egui::Rect,
    active: bool,
    hovered: bool,
    struggling: bool,
) {
    if active {
        let fill = ShapeIdx(under.0 + 1);
        if is_fill_of(painter, fill, tab) {
            painter.set(fill, Shape::Noop);
        }
    }
    let line = if struggling {
        brand::ORANGE
    } else if active {
        brand::TEAL
    } else if hovered {
        brand::FIELD_LINE
    } else {
        brand::ITEM_LINE
    };
    painter.rect_stroke(
        tab,
        CornerRadius::same(TAB_RADIUS),
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
}

fn strip_button(ui: &mut egui::Ui, icon: &str, tip: &str, radius: f32) -> egui::Response {
    icon_button_sized(
        ui,
        icon,
        tip,
        IconButtonSize { width: ICON_BOX, height: ICON_BOX, glyph: ICON_GLYPH, radius },
    )
}

fn lead(
    ui: &mut egui::Ui,
    parked: Option<(&[pill::Row], Popover)>,
    main: impl FnOnce(&mut egui::Ui),
) -> Option<usize> {
    let slash = slash(ui);
    let picked = if let Some((rows, popover)) = parked {
        pill::show(ui, rows, popover).picked
    } else {
        let gap = ui.spacing().item_spacing.x;
        pill::drop_place(ui, pos2(slash.right() + gap, slash.center().y));
        None
    };
    main(ui);
    picked
}

fn slash(ui: &mut egui::Ui) -> egui::Rect {
    let font = egui::FontId::proportional(SLASH_SIZE);
    let galley = ui.painter().layout_no_wrap("/".to_owned(), font, brand::FIELD_LINE);
    let (rect, _) = ui.allocate_exact_size(
        vec2(galley.size().x + 2.0 * SLASH_MARGIN, TAB_H),
        egui::Sense::hover(),
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, brand::FIELD_LINE);
    rect
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
    pane.agent.as_ref().is_some_and(|agent| {
        matches!(agent.run.shown_tone(), status_view::Tone::Needs | status_view::Tone::Blocked)
    })
}

fn tab_signals(pane: &Pane<PtyTerm>) -> (Progress, CmdState) {
    match &pane.body {
        Body::Shell(term) => (term.progress(), term.cmd_state()),
        _ => (Progress::default(), CmdState::default()),
    }
}

#[cfg(test)]
mod tests;
