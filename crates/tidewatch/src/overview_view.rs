//! The Overview screen (Cmd+0): it replaces the terminal area with a header (title, summary, repo
//! chips), a main column (the Needs you inbox, the Agents panel with a full-width row per agent,
//! Finished today) and a right rail (Coming up, Poofed today, Running elsewhere). A click on a row
//! jumps to its tab. Also the icon button in the tab strip that opens it, with a badge counting
//! the tabs that need you.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Painter, Pos2, Rect, ScrollArea, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use jiff::tz::TimeZone;
use stdusk_egui::colors;

use crate::agenda::{self, Day};
use crate::app::strip::{ICON_BOX, ICON_GLYPH};
use crate::brand;
use crate::commands::Command;
use crate::mascot::{self, Pose};
use crate::overview::{Board, Overview, PoofedRow, RepoChip, WORKING_CAP, capped, repo_ink};
use crate::overview_agents::{Density, Lane, density, finished_shown, finished_today};
use crate::overview_agents_view::{self, Group};
use crate::overview_inbox::{self, Entry};
use crate::overview_inbox_view;
use crate::overview_rail;
use crate::progress_view::flow;
use crate::question_compact::Click;
use crate::reader_view::ellipsized;
use crate::screen_kit::TEXT_TAG;
use crate::screen_kit::{Size, TEXT_PAGE, TEXT_SECONDARY, TEXT_TITLE, button_width, heading};
use crate::status_view;
use tidewatch_core::panel_view::Tone;

const BUTTON_H: f32 = ICON_BOX;
const SIDE: f32 = crate::layout::EDGE;
const COLUMN_GAP: f32 = 16.0;
const STACK_BELOW: f32 = 1000.0;
pub(crate) const SECTION_GAP: f32 = 16.0;
pub(crate) const SECTION_PAD: f32 = 14.0;
pub(crate) const SECTION_HEAD_H: f32 = 22.0;
pub(crate) const HEAD_GAP: f32 = 12.0;
pub(crate) const SECTION_RADIUS: u8 = 14;
pub(crate) const SWATCH: f32 = 7.0;
const CHIP_H: f32 = Size::Compact.height();
const CHIP_GAP: f32 = 8.0;
const CHIP_CHARS: usize = 18;
const CHIP_ROWS_MAX: usize = 3;
const MASCOT_H: f32 = 120.0;
const BADGE_H: f32 = 16.0;
const GRID_ICON: &str = "\u{E464}";

/// The right rail's width: 340 from a 1400 point window up, 290 below that.
fn rail_width(total: f32) -> f32 {
    if total >= 1300.0 { 340.0 } else { 290.0 }
}

pub(crate) const BUTTON_W: f32 = ICON_BOX;

/// The tab strip's Overview button. `needs` is how many tabs wait on you in each repo; none draws no
/// badge.
pub(crate) fn button(ui: &mut egui::Ui, active: bool, needs: &[(String, usize)]) -> egui::Response {
    let total: usize = needs.iter().map(|(_, count)| count).sum();
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON_W, BUTTON_H), Sense::click());
    let hovered = resp.hovered();
    if active {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0, brand::teal()),
            StrokeKind::Inside,
        );
    } else if hovered {
        ui.painter().rect_filled(rect, 8.0, colors::hover());
    }
    let ink = if active {
        brand::teal()
    } else if hovered {
        colors::fg()
    } else {
        colors::dim()
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        GRID_ICON,
        FontId::proportional(ICON_GLYPH),
        ink,
    );
    if total > 0 {
        paint_badge(ui, rect, total);
    }
    resp.on_hover_text(button_tip(&Command::Overview.keys(), needs))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn button_tip(keys: &str, needs: &[(String, usize)]) -> String {
    let waiting: Vec<&(String, usize)> = needs.iter().filter(|(_, count)| *count > 0).collect();
    let total: usize = waiting.iter().map(|(_, count)| count).sum();
    let who = match total {
        1 => "1 agent needs".to_owned(),
        n => format!("{n} agents need"),
    };
    match waiting.as_slice() {
        [] => format!("Overview ({keys})"),
        [(repo, _)] => format!("Overview ({keys})\n{who} you in {repo}"),
        several => {
            let split: Vec<String> =
                several.iter().map(|(repo, count)| format!("{count} in {repo}")).collect();
            format!("Overview ({keys})\n{who} you: {}", split.join(", "))
        }
    }
}

fn paint_badge(ui: &egui::Ui, button: Rect, count: usize) {
    let text = if count > 9 { "9+".to_owned() } else { count.to_string() };
    let font = FontId::monospace(TEXT_TAG);
    let galley = ui.painter().layout_no_wrap(text, font, colors::bg());
    let width = (galley.size().x + 8.0).max(BADGE_H);
    let badge =
        Rect::from_min_size(pos2(button.right() - width, button.top() + 1.0), vec2(width, BADGE_H));
    let fill = status_view::color(Tone::Needs).unwrap_or(Color32::YELLOW);
    ui.painter().rect_filled(badge.expand(2.0), BADGE_H, colors::titlebar());
    ui.painter().rect_filled(badge, BADGE_H, fill);
    ui.painter().galley(badge.center() - galley.size() / 2.0, galley, colors::bg());
}

/// A part of the Overview that folds its rest in or out in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fold {
    Working,
    Poofed,
    Finished,
    Events,
    Parked,
}

/// What the Overview keeps while the app runs, and forgets with it: nothing is saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    /// The repo chip picked, `None` for All.
    pub(crate) repo: Option<String>,
    pub(crate) opened: Vec<Fold>,
}

impl State {
    pub(crate) fn is_open(&self, fold: Fold) -> bool {
        self.opened.contains(&fold)
    }

    pub(crate) fn toggle(&mut self, fold: Fold) {
        if self.is_open(fold) {
            self.opened.retain(|open| *open != fold);
        } else {
            self.opened.push(fold);
        }
    }
}

/// What the rail lists.
#[derive(Default)]
pub(crate) struct Sections<'a> {
    pub(crate) agenda: &'a [Day],
    pub(crate) poofed: &'a [PoofedRow],
    /// The wall clock in unix seconds: what Finished today counts as today.
    pub(crate) now_secs: i64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Out {
    /// The tab whose card was clicked.
    pub(crate) jump: Option<u64>,
    /// The row of Coming up that was clicked, or whose Decide was.
    pub(crate) agenda: Option<usize>,
    /// The row of Poofed today that was clicked.
    pub(crate) poofed: Option<usize>,
    /// The tab whose Needs you card had a ready answer chosen, and the choice.
    pub(crate) answer: Option<(u64, Click)>,
}

/// Draws the Overview in `area` and returns what the user clicked. `below` draws last in the rail.
pub(crate) fn show(
    ui: &mut Ui,
    area: Rect,
    overview: &Overview,
    state: &mut State,
    sections: &Sections<'_>,
    below: impl FnOnce(&mut Ui),
) -> Out {
    ui.painter().rect_filled(area, 0.0, brand::ground());
    let board = overview.board(state.repo.as_deref());
    state.repo = state.repo.take().filter(|repo| board.chips.iter().any(|c| c.label == *repo));
    let head_h = header(ui, area, &board, state);
    let board = overview.board(state.repo.as_deref());
    let agenda = crate::agenda::narrow(sections.agenda, state.repo.as_deref());
    let poofed: Vec<PoofedRow> = sections
        .poofed
        .iter()
        .filter(|row| state.repo.as_deref().is_none_or(|repo| row.place == repo))
        .cloned()
        .collect();
    let entries = overview_inbox::build(&board, &agenda);
    let rail = agenda::rail(&agenda);
    let zone = TimeZone::system();
    let mut out = Out::default();
    let body = Rect::from_min_max(
        pos2(area.left() + SIDE, area.top() + head_h),
        pos2(area.right() - SIDE, area.bottom() - 14.0),
    );
    let mut content = ui.new_child(UiBuilder::new().max_rect(body));
    ScrollArea::vertical().auto_shrink([false, false]).show(&mut content, |ui| {
        let width = ui.available_width();
        let stacked = width < STACK_BELOW;
        let rail_w = rail_width(width);
        let main_w = if stacked { width } else { width - rail_w - COLUMN_GAP };
        let top = ui.cursor().min;
        let mut column = |at: Pos2, w: f32| {
            let rect = Rect::from_min_size(at, vec2(w, 100_000.0));
            let mut child =
                ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
            child.spacing_mut().item_spacing = vec2(0.0, 0.0);
            child
        };
        let mut main = column(top, main_w);
        main_column(
            &mut main,
            (main_w, body.height()),
            (&board, &entries),
            (state, (sections.now_secs, &zone)),
            &mut out,
        );
        let rail_at = if stacked {
            pos2(top.x, main.min_rect().bottom() + SECTION_GAP)
        } else {
            pos2(top.x + main_w + COLUMN_GAP, top.y)
        };
        let rail_w = if stacked { width } else { rail_w };
        let mut rail_ui = column(rail_at, rail_w);
        overview_rail::show(&mut rail_ui, rail_w, &rail, &poofed, state, &mut out);
        let empty_rail = poofed.is_empty()
            && agenda::count(&rail.dated) + rail.events.len() + rail.parked.len() == 0;
        let gap = if empty_rail { 0.0 } else { SECTION_GAP };
        rail_ui.add_space(gap);
        below(&mut rail_ui);
        let used = main.min_rect().union(rail_ui.min_rect());
        ui.allocate_rect(used, Sense::hover());
    });
    out
}

fn chip_width(ui: &Ui, chip: Option<&RepoChip>, active_all: usize) -> f32 {
    let (label, count, swatch) = match chip {
        Some(chip) => (chip_label(&chip.label), chip.count, SWATCH + 6.0),
        None => ("All".to_owned(), active_all, 0.0),
    };
    let font = FontId::proportional(TEXT_SECONDARY);
    let text = ui.painter().layout_no_wrap(label, font.clone(), brand::soft()).size().x;
    let count = ui.painter().layout_no_wrap(count.to_string(), font, brand::faint()).size().x;
    2.0 * Size::Compact.pad() + swatch + text + 6.0 + count
}

fn chip_label(label: &str) -> String {
    stdusk_egui::ui::ellipsize(label, CHIP_CHARS).0
}

fn header(ui: &mut Ui, area: Rect, board: &Board, state: &mut State) -> f32 {
    let (left, right) = (area.left() + SIDE, area.right() - SIDE);
    let painter = ui.painter().clone();
    let title = painter.layout_no_wrap("Overview".to_owned(), heading(TEXT_PAGE), brand::text());
    let summary = painter.layout_no_wrap(
        board.summary(),
        FontId::proportional(TEXT_SECONDARY),
        brand::soft(),
    );
    let all: usize = board.chips.iter().map(|chip| chip.count).sum();
    let mut chips: Vec<Option<&RepoChip>> = vec![None];
    chips.extend(board.chips.iter().map(Some));
    let widths: Vec<f32> = chips.iter().map(|chip| chip_width(ui, *chip, all)).collect();
    let total: f32 = widths.iter().sum::<f32>() + CHIP_GAP * widths.len().saturating_sub(1) as f32;
    let row_top = area.top() + 16.0;
    let mid = row_top + CHIP_H / 2.0;
    let room = right - left - title.size().x - 14.0 - summary.size().x - 24.0;
    let inline = total <= room;
    painter.galley(pos2(left, mid - title.size().y / 2.0), title.clone(), brand::text());
    painter.galley(
        pos2(left + title.size().x + 14.0, mid - summary.size().y / 2.0 + 2.0),
        summary,
        brand::soft(),
    );
    let wide = right - left;
    let wrapped = if inline {
        let mut x = right - total;
        let at = widths
            .iter()
            .map(|width| {
                let at = (x - left, 0.0);
                x += width + CHIP_GAP;
                at
            })
            .collect();
        Wrapped { at, full: CHIP_H, shown: CHIP_H, scrolls: false }
    } else {
        wrap_chips(&widths, wide)
    };
    let origin = if inline { pos2(left, row_top) } else { pos2(left, row_top + CHIP_H + 10.0) };
    let active: Option<String> =
        state.repo.clone().filter(|repo| board.chips.iter().any(|c| c.label == *repo));
    let laid = Laid { chips: &chips, widths: &widths, at: &wrapped.at, all };
    if wrapped.scrolls {
        let area = Rect::from_min_size(origin, vec2(wide, wrapped.shown));
        let mut inner = ui.new_child(UiBuilder::new().max_rect(area));
        ScrollArea::vertical().id_salt("overview-chips").auto_shrink([false, false]).show(
            &mut inner,
            |ui| {
                let size = vec2(wide - brand::SCROLL_ROOM, wrapped.full);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                paint_chips(ui, rect.min, &laid, active.as_deref(), state);
            },
        );
    } else {
        paint_chips(ui, origin, &laid, active.as_deref(), state);
    }
    let height = wrapped.shown;
    16.0 + height + if inline { 0.0 } else { 10.0 } + 14.0
}

struct Wrapped {
    at: Vec<(f32, f32)>,
    full: f32,
    shown: f32,
    scrolls: bool,
}

struct Laid<'a> {
    chips: &'a [Option<&'a RepoChip>],
    widths: &'a [f32],
    at: &'a [(f32, f32)],
    all: usize,
}

fn wrap_chips(widths: &[f32], width: f32) -> Wrapped {
    let cap = CHIP_ROWS_MAX as f32 * CHIP_H + (CHIP_ROWS_MAX - 1) as f32 * CHIP_GAP;
    let (at, full) = flow(widths, width, CHIP_H, CHIP_GAP);
    if full <= cap {
        return Wrapped { at, full, shown: full, scrolls: false };
    }
    let (at, full) = flow(widths, width - brand::SCROLL_ROOM, CHIP_H, CHIP_GAP);
    Wrapped { at, full, shown: cap, scrolls: true }
}

fn paint_chips(
    ui: &mut Ui,
    origin: Pos2,
    laid: &Laid<'_>,
    active: Option<&str>,
    state: &mut State,
) {
    let painter = ui.painter().clone();
    for ((chip, width), (x, y)) in laid.chips.iter().zip(laid.widths).zip(laid.at) {
        let rect = Rect::from_min_size(origin + vec2(*x, *y), vec2(*width, CHIP_H));
        let selected = match chip {
            Some(chip) => active == Some(chip.label.as_str()),
            None => active.is_none(),
        };
        let id = ui.id().with(("overview-chip", chip.map(|c| c.label.as_str())));
        let resp = ui.interact(rect, id, Sense::click());
        let (line, ink) = if selected {
            (brand::teal(), brand::teal_text())
        } else if resp.hovered() {
            (brand::faint(), brand::text())
        } else {
            (brand::item_line(), brand::soft())
        };
        painter.rect_stroke(
            rect,
            CornerRadius::same(Size::Compact.radius()),
            Stroke::new(1.0, line),
            StrokeKind::Inside,
        );
        let mut x = rect.left() + Size::Compact.pad();
        let (label, count) = match chip {
            Some(chip) => {
                let swatch = Rect::from_center_size(
                    pos2(x + SWATCH / 2.0, mid_of(rect)),
                    vec2(SWATCH, SWATCH),
                );
                painter.rect_filled(swatch, CornerRadius::same(2), repo_ink(&chip.label));
                x += SWATCH + 6.0;
                (chip_label(&chip.label), chip.count)
            }
            None => ("All".to_owned(), laid.all),
        };
        let font = FontId::proportional(TEXT_SECONDARY);
        let text = painter.layout_no_wrap(label, font.clone(), ink);
        let count = painter.layout_no_wrap(count.to_string(), font, brand::faint());
        painter.galley(pos2(x, mid_of(rect) - text.size().y / 2.0), text.clone(), ink);
        painter.galley(
            pos2(x + text.size().x + 6.0, mid_of(rect) - count.size().y / 2.0),
            count,
            brand::faint(),
        );
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            state.repo = chip.map(|chip| chip.label.clone());
            ui.ctx().request_repaint();
        }
    }
}

fn mid_of(rect: Rect) -> f32 {
    rect.center().y
}

fn empty(ui: &mut Ui) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        let size = vec2(mascot::width_at(Pose::Asleep, MASCOT_H), MASCOT_H);
        let (area, _) = ui.allocate_exact_size(size, Sense::hover());
        mascot::show(ui, area, Pose::Asleep);
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(format!(
                "Nothing running. Start an agent with {}.",
                Command::NewTab.keys()
            ))
            .size(TEXT_SECONDARY)
            .color(brand::muted()),
        );
    });
}

/// What the main column draws, top to bottom: the Needs you inbox, the Agents panel (rows show up
/// to four step lines when everything fits the `viewport` height, else two) and Finished today.
fn main_column(
    ui: &mut Ui,
    (width, viewport): (f32, f32),
    (board, entries): (&Board, &[Entry]),
    (state, when): (&mut State, (i64, &TimeZone)),
    out: &mut Out,
) {
    if board.total() == 0 && entries.is_empty() {
        empty(ui);
        return;
    }
    let mut used = 0.0;
    if !entries.is_empty() {
        overview_inbox_view::show(ui, width, entries, board, out);
        used += overview_inbox_view::height(entries.len()) + SECTION_GAP;
        ui.add_space(SECTION_GAP);
    }
    if board.total() == 0 {
        return;
    }
    let (working, _) = capped(&board.working, WORKING_CAP, state.is_open(Fold::Working));
    let more = (board.working.len() > WORKING_CAP).then(|| {
        if state.is_open(Fold::Working) {
            "Show fewer".to_owned()
        } else {
            format!("Show all {}", board.working.len())
        }
    });
    let group = |lane, rows, total, more| Group { lane, rows, total, more };
    let groups: Vec<Group<'_>> = [
        group(Lane::Needs, board.needs.as_slice(), board.needs.len(), None),
        group(Lane::Working, working, board.working.len(), more),
        group(Lane::Waiting, board.waiting.as_slice(), board.waiting.len(), None),
        group(Lane::Idle, board.idle.as_slice(), board.idle.len(), None),
    ]
    .into_iter()
    .filter(|group| !group.rows.is_empty())
    .collect();
    let density = density(used, overview_agents_view::height(&groups, Density::Expanded), viewport);
    let drawn = overview_agents_view::show(ui, width, &groups, density, board.total());
    out.jump = out.jump.or(drawn.jump);
    if drawn.toggled {
        state.toggle(Fold::Working);
    }
    used += overview_agents_view::height(&groups, density) + SECTION_GAP;
    ui.add_space(SECTION_GAP);
    let rows = board.needs.iter().chain(&board.working).chain(&board.waiting).chain(&board.idle);
    let done = finished_today(rows, when.0, when.1);
    if done.is_empty() {
        return;
    }
    let room = viewport - used - overview_agents_view::finished_chrome();
    let shown = finished_shown(done.len(), room, state.is_open(Fold::Finished));
    let (jump, toggled) = overview_agents_view::finished(
        ui,
        width,
        &done,
        (shown, state.is_open(Fold::Finished)),
        when.1,
    );
    out.jump = out.jump.or(jump);
    if toggled {
        state.toggle(Fold::Finished);
    }
    ui.add_space(SECTION_GAP);
}
pub(crate) fn section_frame(painter: &Painter, rect: Rect, needs: bool) {
    let (fill, line) = if needs {
        (brand::needs_fill(), brand::struggle_line())
    } else {
        (brand::card(), brand::card_line())
    };
    painter.rect(
        rect,
        CornerRadius::same(SECTION_RADIUS),
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
}

pub(crate) struct Head<'a> {
    pub(crate) title: &'a str,
    pub(crate) caption: &'a str,
    pub(crate) count: &'a str,
    pub(crate) dot: Option<Color32>,
    pub(crate) ink: Color32,
}

pub(crate) fn section_head(painter: &Painter, section: Rect, head: &Head<'_>) {
    let mid = section.top() + SECTION_PAD + SECTION_HEAD_H / 2.0;
    let mut x = section.left() + SECTION_PAD;
    if let Some(dot) = head.dot {
        painter.circle_filled(pos2(x + 4.0, mid), 4.0, dot);
        x += 16.0;
    }
    let title =
        painter.text(pos2(x, mid), Align2::LEFT_CENTER, head.title, heading(TEXT_TITLE), head.ink);
    if !head.caption.is_empty() {
        painter.text(
            pos2(title.right() + 8.0, mid + 1.0),
            Align2::LEFT_CENTER,
            head.caption,
            FontId::proportional(TEXT_SECONDARY),
            brand::muted(),
        );
    }
    painter.text(
        pos2(section.right() - SECTION_PAD, mid),
        Align2::RIGHT_CENTER,
        head.count,
        FontId::proportional(TEXT_SECONDARY),
        brand::muted(),
    );
}

/// Draws `add` in `rect` without taking room in `ui`: a scope would pull its cursor back up.
pub(crate) fn in_rect<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    add(&mut child)
}

/// A ghost button of `label` at the left of `rect`'s top; true when clicked.
pub(crate) fn ghost(ui: &mut Ui, at: Pos2, label: &str) -> bool {
    let width = button_width(ui, label, Size::Compact);
    let rect = Rect::from_min_size(at, vec2(width, Size::Compact.height()));
    in_rect(ui, rect, |ui| {
        crate::screen_kit::quiet_button(ui, label, true, Size::Compact).clicked()
    })
}

/// The repo's square and name, its left edge at `left` and centered on `mid`. Returns the right edge.
pub(crate) fn repo_label(painter: &Painter, left: f32, mid: f32, name: &str, max_w: f32) -> f32 {
    let square = Rect::from_center_size(pos2(left + SWATCH / 2.0, mid), vec2(SWATCH, SWATCH));
    painter.rect_filled(square, CornerRadius::same(2), repo_ink(name));
    let text = ellipsized(
        painter,
        name,
        FontId::proportional(TEXT_SECONDARY),
        brand::soft(),
        max_w - SWATCH - 4.0,
    );
    let right = left + SWATCH + 4.0 + text.size().x;
    painter.galley(pos2(left + SWATCH + 4.0, mid - text.size().y / 2.0), text, brand::soft());
    right
}

#[cfg(test)]
mod tests;
