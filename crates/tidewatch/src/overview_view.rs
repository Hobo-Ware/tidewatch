//! The Overview screen (Cmd+0): it replaces the terminal area with a header (title, summary, repo
//! chips), a main column of agent cards (Needs you, Working, Idle) and a right rail (Coming up,
//! Poofed today, Running elsewhere). A click on a card asks to jump to its tab. Also the icon
//! button in the tab strip that opens it, with a badge counting the tabs that need you.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Painter, Pos2, Rect, ScrollArea, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use stdusk_egui::colors;

use crate::agenda::Day;
use crate::agent_avatar::{self, Face};
use crate::app::strip::{ICON_BOX, ICON_GLYPH};
use crate::brand;
use crate::commands::Command;
use crate::mascot::{self, Pose};
use crate::overview::{
    Board, Overview, PoofedRow, RepoChip, Row, WORKING_CAP, ask_button, ask_tag, capped, card_view,
    fmt_elapsed, idle_line, repo_ink,
};
use crate::overview_rail;
use crate::progress_ring;
use crate::progress_view::flow;
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, button_width, heading, primary_button};
use crate::status_view::{self, Tone};

const BUTTON_H: f32 = ICON_BOX;
const SIDE: f32 = crate::layout::EDGE;
const RAIL_W: f32 = 380.0;
const COLUMN_GAP: f32 = 16.0;
const STACK_BELOW: f32 = 900.0;
pub(crate) const SECTION_GAP: f32 = 16.0;
pub(crate) const SECTION_PAD: f32 = 14.0;
pub(crate) const SECTION_HEAD_H: f32 = 22.0;
pub(crate) const HEAD_GAP: f32 = 12.0;
pub(crate) const SECTION_RADIUS: u8 = 14;
const CARD_GAP: f32 = 10.0;
const CARD_PAD: f32 = 12.0;
const CARD_RADIUS: u8 = 10;
const ROW_H: f32 = 40.0;
const FACE: f32 = 36.0;
const FACE_IN_RING: f32 = 28.0;
const RING: f32 = 40.0;
const FACE_GAP: f32 = 10.0;
const ASK_ROW_GAP: f32 = 8.0;
const BAR_GAP: f32 = 10.0;
const BAR_H: f32 = 3.0;
const NEEDS_H: f32 = CARD_PAD * 2.0 + ROW_H + ASK_ROW_GAP + 26.0;
const WORKING_H: f32 = CARD_PAD * 2.0 + ROW_H + BAR_GAP + BAR_H;
const IDLE_H: f32 = CARD_PAD * 2.0 + ROW_H;
pub(crate) const SWATCH: f32 = 7.0;
const PILL_H: f32 = 18.0;
const CHIP_H: f32 = 28.0;
const CHIP_GAP: f32 = 8.0;
const CHIP_CHARS: usize = 18;
const TITLE_SIZE: f32 = 22.0;
const MASCOT_H: f32 = 120.0;
const BADGE_H: f32 = 16.0;
const GRID_ICON: &str = "\u{E464}";

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
            Stroke::new(1.0, brand::TEAL),
            StrokeKind::Inside,
        );
    } else if hovered {
        ui.painter().rect_filled(rect, 8.0, colors::hover());
    }
    let ink = if active {
        brand::TEAL
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
    let font = FontId::monospace(11.0);
    let galley = ui.painter().layout_no_wrap(text, font, colors::bg());
    let width = (galley.size().x + 8.0).max(BADGE_H);
    let badge =
        Rect::from_min_size(pos2(button.right() - width, button.top() + 1.0), vec2(width, BADGE_H));
    let fill = status_view::color(Tone::Needs).unwrap_or(Color32::YELLOW);
    ui.painter().rect_filled(badge.expand(2.0), BADGE_H, colors::titlebar());
    ui.painter().rect_filled(badge, BADGE_H, fill);
    ui.painter().galley(badge.center() - galley.size() / 2.0, galley, colors::bg());
}

/// What the Overview keeps while the app runs, and forgets with it: nothing is saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    /// The repo chip picked, `None` for All.
    pub(crate) repo: Option<String>,
    pub(crate) working_all: bool,
    pub(crate) poofed_all: bool,
}

/// What the rail lists.
#[derive(Default)]
pub(crate) struct Sections<'a> {
    pub(crate) agenda: &'a [Day],
    pub(crate) poofed: &'a [PoofedRow],
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Out {
    /// The tab whose card was clicked.
    pub(crate) jump: Option<u64>,
    /// The row of Coming up that was clicked, or whose Decide was.
    pub(crate) agenda: Option<usize>,
    /// The row of Poofed today that was clicked.
    pub(crate) poofed: Option<usize>,
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
    ui.painter().rect_filled(area, 0.0, brand::GROUND);
    let board = overview.board(state.repo.as_deref());
    let head_h = header(ui, area, &board, state);
    let board = overview.board(state.repo.as_deref());
    let agenda = crate::agenda::narrow(sections.agenda, state.repo.as_deref());
    let poofed: Vec<PoofedRow> = sections
        .poofed
        .iter()
        .filter(|row| state.repo.as_deref().is_none_or(|repo| row.place == repo))
        .cloned()
        .collect();
    let mut out = Out::default();
    let body = Rect::from_min_max(
        pos2(area.left() + SIDE, area.top() + head_h),
        pos2(area.right() - SIDE, area.bottom() - 14.0),
    );
    let mut content = ui.new_child(UiBuilder::new().max_rect(body));
    ScrollArea::vertical().auto_shrink([false, false]).show(&mut content, |ui| {
        let width = ui.available_width();
        let stacked = width < STACK_BELOW;
        let main_w = if stacked { width } else { width - RAIL_W - COLUMN_GAP };
        let top = ui.cursor().min;
        let mut column = |at: Pos2, w: f32| {
            let rect = Rect::from_min_size(at, vec2(w, 100_000.0));
            let mut child =
                ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
            child.spacing_mut().item_spacing = vec2(0.0, 0.0);
            child
        };
        let mut main = column(top, main_w);
        main_column(&mut main, main_w, &board, state, &mut out);
        let rail_at = if stacked {
            pos2(top.x, main.min_rect().bottom() + SECTION_GAP)
        } else {
            pos2(top.x + main_w + COLUMN_GAP, top.y)
        };
        let rail_w = if stacked { width } else { RAIL_W };
        let mut rail = column(rail_at, rail_w);
        overview_rail::show(&mut rail, rail_w, &agenda, &poofed, state, &mut out);
        let gap = if poofed.is_empty() && agenda.is_empty() { 0.0 } else { SECTION_GAP };
        rail.add_space(gap);
        below(&mut rail);
        let used = main.min_rect().union(rail.min_rect());
        ui.allocate_rect(used, Sense::hover());
    });
    out
}

fn chip_width(ui: &Ui, chip: Option<&RepoChip>, active_all: usize) -> f32 {
    let (label, count, swatch) = match chip {
        Some(chip) => (chip_label(&chip.label), chip.count, SWATCH + 6.0),
        None => ("All".to_owned(), active_all, 0.0),
    };
    let font = FontId::proportional(13.0);
    let text = ui.painter().layout_no_wrap(label, font.clone(), brand::SOFT).size().x;
    let count = ui.painter().layout_no_wrap(count.to_string(), font, brand::FAINT).size().x;
    24.0 + swatch + text + 6.0 + count
}

fn chip_label(label: &str) -> String {
    stdusk_egui::ui::ellipsize(label, CHIP_CHARS).0
}

fn header(ui: &mut Ui, area: Rect, board: &Board, state: &mut State) -> f32 {
    let (left, right) = (area.left() + SIDE, area.right() - SIDE);
    let painter = ui.painter().clone();
    let title = painter.layout_no_wrap("Overview".to_owned(), heading(TITLE_SIZE), brand::TEXT);
    let summary = painter.layout_no_wrap(board.summary(), FontId::proportional(13.0), brand::SOFT);
    let all: usize = board.chips.iter().map(|chip| chip.count).sum();
    let mut chips: Vec<Option<&RepoChip>> = vec![None];
    chips.extend(board.chips.iter().map(Some));
    let widths: Vec<f32> = chips.iter().map(|chip| chip_width(ui, *chip, all)).collect();
    let total: f32 = widths.iter().sum::<f32>() + CHIP_GAP * widths.len().saturating_sub(1) as f32;
    let row_top = area.top() + 16.0;
    let mid = row_top + CHIP_H / 2.0;
    let room = right - left - title.size().x - 14.0 - summary.size().x - 24.0;
    let inline = total <= room;
    painter.galley(pos2(left, mid - title.size().y / 2.0), title.clone(), brand::TEXT);
    painter.galley(
        pos2(left + title.size().x + 14.0, mid - summary.size().y / 2.0 + 2.0),
        summary,
        brand::SOFT,
    );
    let (places, height) = if inline {
        let mut x = right - total;
        let places: Vec<(f32, f32)> = widths
            .iter()
            .map(|width| {
                let at = (x - left, 0.0);
                x += width + CHIP_GAP;
                at
            })
            .collect();
        (places, CHIP_H)
    } else {
        flow(&widths, right - left, CHIP_H, CHIP_GAP)
    };
    let origin = if inline { pos2(left, row_top) } else { pos2(left, row_top + CHIP_H + 10.0) };
    let active: Option<String> =
        state.repo.clone().filter(|repo| board.chips.iter().any(|c| c.label == *repo));
    for ((chip, width), (x, y)) in chips.iter().zip(&widths).zip(places) {
        let rect = Rect::from_min_size(origin + vec2(x, y), vec2(*width, CHIP_H));
        let selected = match chip {
            Some(chip) => active.as_deref() == Some(chip.label.as_str()),
            None => active.is_none(),
        };
        let id = ui.id().with(("overview-chip", chip.map(|c| c.label.as_str())));
        let resp = ui.interact(rect, id, Sense::click());
        let (line, ink) = if selected {
            (brand::TEAL, brand::TEAL_TEXT)
        } else if resp.hovered() {
            (brand::FAINT, brand::TEXT)
        } else {
            (brand::ITEM_LINE, brand::SOFT)
        };
        painter.rect_stroke(
            rect,
            CornerRadius::same(255),
            Stroke::new(1.0, line),
            StrokeKind::Inside,
        );
        let mut x = rect.left() + 12.0;
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
            None => ("All".to_owned(), all),
        };
        let font = FontId::proportional(13.0);
        let text = painter.layout_no_wrap(label, font.clone(), ink);
        let count = painter.layout_no_wrap(count.to_string(), font, brand::FAINT);
        painter.galley(pos2(x, mid_of(rect) - text.size().y / 2.0), text.clone(), ink);
        painter.galley(
            pos2(x + text.size().x + 6.0, mid_of(rect) - count.size().y / 2.0),
            count,
            brand::FAINT,
        );
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            state.repo = chip.map(|chip| chip.label.clone());
            ui.ctx().request_repaint();
        }
    }
    16.0 + height + if inline { 0.0 } else { 10.0 } + 14.0
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
            .size(13.0)
            .color(brand::MUTED),
        );
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Needs,
    Working,
    Idle,
}

impl Lane {
    fn title(self) -> &'static str {
        match self {
            Self::Needs => "Needs you",
            Self::Working => "Working",
            Self::Idle => "Idle",
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Self::Idle => "nothing asked",
            _ => "",
        }
    }

    fn dot(self) -> Color32 {
        match self {
            Self::Needs => brand::NEEDS,
            Self::Working => brand::WORKING,
            Self::Idle => brand::FAINT,
        }
    }

    fn card_h(self) -> f32 {
        match self {
            Self::Needs => NEEDS_H,
            Self::Working => WORKING_H,
            Self::Idle => IDLE_H,
        }
    }

    fn columns(self, width: f32) -> usize {
        let (min, max) = match self {
            Self::Needs | Self::Working => (250.0, 3),
            Self::Idle => (300.0, 2),
        };
        columns(width, min, max)
    }
}

fn columns(width: f32, min: f32, max: usize) -> usize {
    (((width + CARD_GAP) / (min + CARD_GAP)).floor() as usize).clamp(1, max)
}

fn main_column(ui: &mut Ui, width: f32, board: &Board, state: &mut State, out: &mut Out) {
    if board.total() == 0 {
        empty(ui);
        return;
    }
    if !board.needs.is_empty() {
        out.jump = out.jump.or(grid_section(
            ui,
            width,
            Lane::Needs,
            &board.needs,
            board.needs.len(),
            None,
        )
        .0);
        ui.add_space(SECTION_GAP);
    }
    if !board.working.is_empty() {
        let (shown, _) = capped(&board.working, WORKING_CAP, state.working_all);
        let more = (board.working.len() > WORKING_CAP).then(|| {
            if state.working_all {
                "Show fewer".to_owned()
            } else {
                format!("Show all {}", board.working.len())
            }
        });
        let (jump, toggled) =
            grid_section(ui, width, Lane::Working, shown, board.working.len(), more.as_deref());
        out.jump = out.jump.or(jump);
        if toggled {
            state.working_all = !state.working_all;
        }
        ui.add_space(SECTION_GAP);
    }
    if !board.idle.is_empty() {
        out.jump =
            out.jump.or(grid_section(ui, width, Lane::Idle, &board.idle, board.idle.len(), None).0);
        ui.add_space(SECTION_GAP);
    }
}

pub(crate) fn section_frame(painter: &Painter, rect: Rect, needs: bool) {
    let (fill, line) = if needs {
        (brand::NEEDS_FILL, brand::STRUGGLE_LINE)
    } else {
        (brand::CARD, brand::CARD_LINE)
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
        painter.text(pos2(x, mid), Align2::LEFT_CENTER, head.title, heading(15.0), head.ink);
    if !head.caption.is_empty() {
        painter.text(
            pos2(title.right() + 8.0, mid + 1.0),
            Align2::LEFT_CENTER,
            head.caption,
            FontId::proportional(12.0),
            brand::MUTED,
        );
    }
    painter.text(
        pos2(section.right() - SECTION_PAD, mid),
        Align2::RIGHT_CENTER,
        head.count,
        FontId::proportional(13.0),
        brand::MUTED,
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

fn grid_section(
    ui: &mut Ui,
    width: f32,
    lane: Lane,
    rows: &[Row],
    total: usize,
    more: Option<&str>,
) -> (Option<u64>, bool) {
    let inner_w = width - SECTION_PAD * 2.0;
    let columns = lane.columns(inner_w);
    let card_w = (inner_w - CARD_GAP * (columns - 1) as f32) / columns as f32;
    let lines = rows.len().div_ceil(columns);
    let grid_h = lines as f32 * lane.card_h() + lines.saturating_sub(1) as f32 * CARD_GAP;
    let more_h = if more.is_some() { CARD_GAP + Size::Compact.height() } else { 0.0 };
    let height = SECTION_PAD * 2.0 + SECTION_HEAD_H + HEAD_GAP + grid_h + more_h;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    section_frame(&painter, rect, lane == Lane::Needs);
    let ink = if lane == Lane::Needs { brand::NEEDS } else { brand::TEXT };
    section_head(
        &painter,
        rect,
        &Head {
            title: lane.title(),
            caption: lane.caption(),
            count: &total.to_string(),
            dot: Some(lane.dot()),
            ink,
        },
    );
    let top = rect.top() + SECTION_PAD + SECTION_HEAD_H + HEAD_GAP;
    let mut jump = None;
    for (index, row) in rows.iter().enumerate() {
        let (line, column) = (index / columns, index % columns);
        let at = pos2(
            rect.left() + SECTION_PAD + column as f32 * (card_w + CARD_GAP),
            top + line as f32 * (lane.card_h() + CARD_GAP),
        );
        let card_rect = Rect::from_min_size(at, vec2(card_w, lane.card_h()));
        if card(ui, card_rect, row, lane) {
            jump = Some(row.tab_id);
        }
    }
    let toggled = more.is_some_and(|label| {
        ghost(ui, pos2(rect.left() + SECTION_PAD, top + grid_h + CARD_GAP), label)
    });
    (jump, toggled)
}

/// An outline pill whose left edge is at `left`, centered on `mid`. Returns its right edge.
fn kind_pill(painter: &Painter, left: f32, mid: f32, text: &str, ink: Color32) -> f32 {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.0), ink);
    let pill =
        Rect::from_min_size(pos2(left, mid - PILL_H / 2.0), vec2(galley.size().x + 14.0, PILL_H));
    painter.rect_stroke(
        pill,
        CornerRadius::same((PILL_H / 2.0) as u8),
        Stroke::new(1.0, ink.gamma_multiply(0.33)),
        StrokeKind::Inside,
    );
    painter.galley(pill.center() - galley.size() / 2.0, galley, ink);
    pill.right()
}

/// The repo's square and name, its left edge at `left` and centered on `mid`. Returns the right edge.
pub(crate) fn repo_label(painter: &Painter, left: f32, mid: f32, name: &str, max_w: f32) -> f32 {
    let square = Rect::from_center_size(pos2(left + SWATCH / 2.0, mid), vec2(SWATCH, SWATCH));
    painter.rect_filled(square, CornerRadius::same(2), repo_ink(name));
    let text =
        ellipsized(painter, name, FontId::proportional(12.0), brand::SOFT, max_w - SWATCH - 4.0);
    let right = left + SWATCH + 4.0 + text.size().x;
    painter.galley(pos2(left + SWATCH + 4.0, mid - text.size().y / 2.0), text, brand::SOFT);
    right
}

fn card(ui: &mut Ui, rect: Rect, row: &Row, lane: Lane) -> bool {
    let resp = ui.interact(rect, ui.id().with(("overview_card", row.tab_id)), Sense::click());
    let view = card_view(row);
    let painter = ui.painter_at(rect);
    let line = match (lane, resp.hovered()) {
        (Lane::Needs, true) => brand::ORANGE_SOFT,
        (Lane::Needs, false) => brand::STRUGGLE_LINE,
        (_, true) => brand::FIELD_LINE,
        (_, false) => brand::ITEM_LINE,
    };
    painter.rect(
        rect,
        CornerRadius::same(CARD_RADIUS),
        brand::ITEM,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let inner = rect.shrink(CARD_PAD);
    let center = pos2(inner.left() + RING / 2.0, inner.top() + ROW_H / 2.0);
    let ringed = lane == Lane::Working;
    if ringed {
        let track = brand::ITEM_LINE;
        if view.fraction > 0.0 {
            progress_ring::paint(&painter, center, RING, view.fraction, brand::WORKING, track);
        } else {
            let radius = RING / 2.0 - progress_ring::stroke_of(RING) / 2.0;
            painter.circle_stroke(
                center,
                radius,
                Stroke::new(progress_ring::stroke_of(RING), track),
            );
        }
    }
    let tile = Rect::from_center_size(
        center,
        vec2(if ringed { FACE_IN_RING } else { FACE }, if ringed { FACE_IN_RING } else { FACE }),
    );
    let face = Face::of(&row.profile, &row.agent_id, row.avatar.clone());
    if agent_avatar::paint_face(ui.ctx(), &painter, tile, &face)
        && let Some(avatar) = face.avatar()
    {
        agent_avatar::hover(ui, tile, row.tab_id, avatar);
    }
    if row.avatar.is_some() {
        agent_avatar::menu(&resp, row.tab_id);
    }
    let text_x = inner.left() + RING + FACE_GAP;
    let clock = (lane == Lane::Working).then(|| {
        painter.layout_no_wrap(fmt_elapsed(row.elapsed), FontId::monospace(12.0), brand::MUTED)
    });
    let clock_w = clock.as_ref().map_or(0.0, |clock| clock.size().x + 8.0);
    let full = (inner.right() - text_x).max(1.0);
    let room = (full - clock_w).max(1.0);
    let tag = row.kind.label();
    let pill_w =
        painter.layout_no_wrap(tag.clone(), FontId::proportional(11.0), brand::SOFT).size().x
            + 14.0;
    let name_ink = if lane == Lane::Idle { brand::SOFT } else { brand::TEXT };
    let name =
        ellipsized(&painter, &row.title, heading(14.0), name_ink, (room - pill_w - 6.0).max(20.0));
    let top = center.y - 16.0;
    let name_mid = top + 8.0;
    let name_right = text_x + name.size().x;
    painter.galley(pos2(text_x, name_mid - name.size().y / 2.0), name, name_ink);
    if name_right + 6.0 + pill_w <= text_x + room + 1.0 {
        kind_pill(&painter, name_right + 6.0, name_mid, &tag, row.kind.ink());
    }
    if let Some(clock) = clock {
        painter.galley(
            pos2(inner.right() - clock.size().x, name_mid - clock.size().y / 2.0),
            clock,
            brand::MUTED,
        );
    }
    let line_mid = top + 8.0 + 8.0 + 8.0;
    let (text, ink) = second_line(row, lane, &view);
    let repo_right = repo_label(&painter, text_x, line_mid, &row.place, (full * 0.45).max(60.0));
    let rest = ellipsized(
        &painter,
        &format!("\u{b7} {text}"),
        FontId::proportional(12.0),
        ink,
        (inner.right() - repo_right - 6.0).max(1.0),
    );
    painter.galley(pos2(repo_right + 6.0, line_mid - rest.size().y / 2.0), rest, ink);
    let mut clicked = false;
    match lane {
        Lane::Needs => clicked = ask_row(ui, &painter, inner, row, &view.tag),
        Lane::Working => {
            let bar = Rect::from_min_size(
                pos2(inner.left(), inner.bottom() - BAR_H),
                vec2(inner.width(), BAR_H),
            );
            painter.rect_filled(bar, CornerRadius::same(2), brand::ITEM_LINE);
            if view.fraction > 0.0 {
                let fill =
                    Rect::from_min_size(bar.min, vec2(bar.width() * view.fraction.min(1.0), BAR_H));
                painter.rect_filled(fill, CornerRadius::same(2), brand::WORKING);
            }
        }
        Lane::Idle => {}
    }
    let tip = status_view::describe(&row.status);
    clicked | resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked()
}

fn second_line(row: &Row, lane: Lane, view: &crate::overview::CardView) -> (String, Color32) {
    match lane {
        Lane::Needs => {
            let ask = if view.line.is_empty() {
                status_view::describe(&row.status)
            } else {
                view.line.clone()
            };
            (ask, brand::NEEDS)
        }
        Lane::Working => (view.line.clone(), brand::MUTED),
        Lane::Idle => {
            let failed = status_view::tone(&row.status) == Tone::Failed;
            (idle_line(row), if failed { brand::RED } else { brand::MUTED })
        }
    }
}

fn ask_row(ui: &mut Ui, painter: &Painter, inner: Rect, row: &Row, tag: &str) -> bool {
    let button = ask_button(row);
    let width = button_width(ui, button, Size::Compact);
    let rect = Rect::from_min_size(
        pos2(inner.right() - width, inner.bottom() - Size::Compact.height()),
        vec2(width, Size::Compact.height()),
    );
    let (text, ink) = match &row.auto_pick {
        Some(line) => (line.clone(), brand::MUTED),
        None => (ask_tag(row, tag), brand::SOFT),
    };
    let galley = ellipsized(
        painter,
        &text,
        FontId::proportional(12.0),
        ink,
        (inner.width() - width - 8.0).max(1.0),
    );
    painter.galley(pos2(inner.left(), rect.center().y - galley.size().y / 2.0), galley, ink);
    in_rect(ui, rect, |ui| primary_button(ui, button, true, Size::Compact).clicked())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::{Item, Mark};
    use crate::overview::{AgentKind, PlaceInput};
    use egui::{Event, PointerButton, RawInput};
    use tidewatch_core::status::{AgentStatus, Reason};

    #[test]
    fn the_ring_clears_the_corners_of_the_avatar_tile() {
        let corner = (FACE_IN_RING * 0.28).floor();
        let corner_reach = (FACE_IN_RING / 2.0 - corner) * std::f32::consts::SQRT_2 + corner;
        let inner_edge = RING / 2.0 - progress_ring::stroke_of(RING);
        assert!(
            inner_edge >= corner_reach,
            "ring starts at {inner_edge}, tile reaches {corner_reach}"
        );
    }

    #[test]
    fn the_button_tip_says_what_the_badge_counts() {
        let repo = |name: &str, count: usize| (name.to_owned(), count);
        let cases = [
            (vec![], "Overview (K)"),
            (vec![repo("acme-web", 0)], "Overview (K)"),
            (vec![repo("acme-web", 1)], "Overview (K)\n1 agent needs you in acme-web"),
            (vec![repo("acme-web", 2)], "Overview (K)\n2 agents need you in acme-web"),
            (
                vec![repo("acme-web", 3), repo("acme-api", 0), repo("acme-docs", 1)],
                "Overview (K)\n4 agents need you: 3 in acme-web, 1 in acme-docs",
            ),
        ];
        for (needs, want) in cases {
            assert_eq!(button_tip("K", &needs), want, "{needs:?}");
        }
    }

    fn row(id: u64, title: &str, place: &str, status: AgentStatus) -> Row {
        Row {
            tab_id: id,
            title: title.into(),
            place: place.into(),
            profile: "Builder".into(),
            workflow: None,
            agent_id: "claude".into(),
            branch: format!("agent/{title}"),
            status,
            detail: Some("Bash: touch hello.txt".into()),
            end_block: None,
            elapsed: 242.0,
            pr: None,
            done_check: None,
            look: None,
            job_line: None,
            avatar: None,
            kind: AgentKind::Cli("claude".into()),
            auto_pick: None,
            idle_secs: 600.0,
            next: None,
        }
    }

    fn overview(rows: Vec<Row>) -> Overview {
        let mut places: Vec<PlaceInput> = Vec::new();
        for row in rows {
            match places.iter_mut().find(|place| place.label == row.place) {
                Some(place) => place.rows.push(row),
                None => places.push(PlaceInput {
                    label: row.place.clone(),
                    path: format!("/work/{}", row.place),
                    rows: vec![row],
                }),
            }
        }
        crate::overview::build(places)
    }

    fn walk(shape: &egui::Shape, found: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(text) if !text.galley.text().trim().is_empty() => found.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, found)),
            _ => {}
        }
    }

    struct Screen {
        ctx: egui::Context,
        data: Overview,
        state: State,
        agenda: Vec<Day>,
        poofed: Vec<PoofedRow>,
        width: f32,
    }

    impl Screen {
        fn new(data: Overview) -> Self {
            Self {
                ctx: crate::fonts::test_context(),
                data,
                state: State::default(),
                agenda: Vec::new(),
                poofed: Vec::new(),
                width: 1200.0,
            }
        }

        fn frame(&mut self, events: Vec<Event>) -> (Out, Vec<(String, Rect)>) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, 900.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let mut out = Out::default();
            let sections = Sections { agenda: &self.agenda, poofed: &self.poofed };
            let output = self.ctx.run_ui(raw, |ui| {
                let area = ui.max_rect();
                out = show(ui, area, &self.data, &mut self.state, &sections, |_| {});
            });
            let mut found = Vec::new();
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut found);
            }
            (out, found)
        }

        fn texts(&mut self) -> Vec<String> {
            self.frame(vec![]);
            self.frame(vec![]).1.into_iter().map(|(text, _)| text).collect()
        }

        fn click(&mut self, label: &str) -> Out {
            self.frame(vec![]);
            let found = self.frame(vec![]).1;
            let at = found.iter().find(|(text, _)| text == label).expect(label).1.center();
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]).0
        }
    }

    fn has(texts: &[String], want: &str) -> bool {
        texts.iter().any(|text| text == want)
    }

    fn two_repos() -> Overview {
        overview(vec![
            row(1, "web-working", "acme-web", AgentStatus::Working),
            row(2, "web-asks", "acme-web", AgentStatus::NeedsYou(Reason::Question)),
            row(3, "api-working", "acme-api", AgentStatus::Working),
            row(4, "api-idle", "acme-api", AgentStatus::Done),
        ])
    }

    #[test]
    fn a_quiet_agent_is_listed_under_idle_and_nothing_needs_you() {
        let mut screen = Screen::new(overview(vec![
            row(1, "went-quiet", "acme-web", AgentStatus::NeedsYou(Reason::Quiet)),
            row(2, "busy", "acme-web", AgentStatus::Working),
        ]));
        let texts = screen.texts();
        assert!(has(&texts, "Idle"), "{texts:?}");
        assert!(!has(&texts, "Needs you"), "{texts:?}");
        assert!(has(&texts, "2 agents \u{b7} 0 need you \u{b7} 1 working \u{b7} 1 idle"));
        assert!(has(&texts, "went-quiet"));
    }

    #[test]
    fn a_repo_chip_narrows_every_card_and_all_brings_them_back() {
        let mut screen = Screen::new(two_repos());
        let texts = screen.texts();
        for name in ["web-working", "web-asks", "api-working", "api-idle"] {
            assert!(has(&texts, name), "{name}");
        }
        assert!(has(&texts, "All") && has(&texts, "acme-api") && has(&texts, "acme-web"));
        screen.click("acme-api");
        assert_eq!(screen.state.repo.as_deref(), Some("acme-api"));
        let texts = screen.texts();
        assert!(has(&texts, "api-working") && has(&texts, "api-idle"));
        assert!(!has(&texts, "web-working") && !has(&texts, "web-asks"));
        assert!(!has(&texts, "Needs you"));
        assert!(has(&texts, "2 agents \u{b7} 0 need you \u{b7} 1 working \u{b7} 1 idle"));
        screen.click("All");
        assert_eq!(screen.state.repo, None);
        assert!(has(&screen.texts(), "web-asks"));
    }

    #[test]
    fn a_repo_chip_narrows_the_rail_too() {
        let mut screen = Screen::new(two_repos());
        let item = |row, what: &str, place: &str| Item {
            row,
            what: what.into(),
            place: place.into(),
            tab: "t".into(),
            note: None,
            mark: Mark::Plain,
            tip: String::new(),
        };
        screen.agenda = vec![Day {
            label: "TODAY \u{b7} MON 26".into(),
            today: true,
            items: vec![item(0, "web chore", "acme-web"), item(1, "api chore", "acme-api")],
        }];
        let poofed = |index, name: &str, place: &str| PoofedRow {
            index,
            name: name.into(),
            outcome: "answered".into(),
            took: "2m".into(),
            summary: String::new(),
            place: place.into(),
            secs: 120,
        };
        screen.poofed = vec![poofed(0, "web poof", "acme-web"), poofed(1, "api poof", "acme-api")];
        let all = screen.texts();
        assert!(has(&all, "web chore") && has(&all, "api chore") && has(&all, "2 scheduled"));
        assert!(has(&all, "web poof") && has(&all, "api poof") && has(&all, "2 \u{b7} 4m"));
        screen.click("acme-web");
        let web = screen.texts();
        assert!(has(&web, "web chore") && !has(&web, "api chore") && has(&web, "1 scheduled"));
        assert!(has(&web, "web poof") && !has(&web, "api poof") && has(&web, "1 \u{b7} 2m"));
    }

    fn eight_working() -> Overview {
        overview(
            (0..8)
                .map(|n| row(n, &format!("worker-{n}"), "acme-web", AgentStatus::Working))
                .collect(),
        )
    }

    #[test]
    fn working_shows_two_rows_then_show_all_expands_in_place() {
        let mut screen = Screen::new(eight_working());
        let texts = screen.texts();
        let shown =
            |texts: &[String]| (0..8).filter(|n| has(texts, &format!("worker-{n}"))).count();
        assert_eq!(shown(&texts), 6);
        assert!(has(&texts, "Show all 8"));
        screen.click("Show all 8");
        let texts = screen.texts();
        assert_eq!(shown(&texts), 8);
        assert!(has(&texts, "Show fewer") && !has(&texts, "Show all 8"));
        screen.click("Show fewer");
        assert_eq!(shown(&screen.texts()), 6);
    }

    #[test]
    fn six_working_agents_have_no_show_all() {
        let mut screen = Screen::new(overview(
            (0..6)
                .map(|n| row(n, &format!("worker-{n}"), "acme-web", AgentStatus::Working))
                .collect(),
        ));
        assert!(!screen.texts().iter().any(|text| text.starts_with("Show")));
    }

    #[test]
    fn poofed_today_shows_three_and_show_all_expands_in_place() {
        let mut screen = Screen::new(two_repos());
        screen.poofed = (0..5)
            .map(|n| PoofedRow {
                index: n,
                name: format!("poof-{n}"),
                outcome: "PR #1".into(),
                took: "1m".into(),
                summary: String::new(),
                place: "acme-web".into(),
                secs: 60,
            })
            .collect();
        let texts = screen.texts();
        assert_eq!((0..5).filter(|n| has(&texts, &format!("poof-{n}"))).count(), 3);
        assert!(has(&texts, "Show all 5") && has(&texts, "5 \u{b7} 5m"));
        screen.click("Show all 5");
        let texts = screen.texts();
        assert!((0..5).all(|n| has(&texts, &format!("poof-{n}"))));
        assert_eq!(screen.click("poof-4").poofed, Some(4));
    }

    #[test]
    fn a_needs_you_card_names_its_ask_and_its_button_opens_the_tab() {
        let mut waiting =
            row(7, "asks-a-thing", "acme-web", AgentStatus::NeedsYou(Reason::Permission));
        waiting.kind = AgentKind::Meeseeks;
        waiting.auto_pick = Some("Picks the recommended answer in 3:40".into());
        let mut screen = Screen::new(overview(vec![waiting]));
        let texts = screen.texts();
        for want in [
            "Needs you",
            "asks-a-thing",
            "Meeseeks",
            "acme-web",
            "Review",
            "Picks the recommended answer in 3:40",
        ] {
            assert!(has(&texts, want), "{want} in {texts:?}");
        }
        assert_eq!(screen.click("Review").jump, Some(7));
    }

    #[test]
    fn an_idle_card_reads_its_idle_line() {
        let mut idle = row(1, "wakes-later", "acme-web", AgentStatus::NeedsYou(Reason::Quiet));
        idle.idle_secs = 3.0 * 3600.0;
        let mut screen = Screen::new(overview(vec![idle]));
        assert!(
            screen.texts().iter().any(|text| text == "\u{b7} Idle 3h \u{b7} nothing scheduled")
        );
    }

    #[test]
    fn the_overview_has_no_checklist() {
        let mut screen = Screen::new(two_repos());
        let texts = screen.texts();
        assert!(!texts.iter().any(|text| text.contains("Checklist") || text == "Edit"));
        assert!(!texts.iter().any(|text| text.contains("Four stages")));
    }

    #[test]
    fn a_decide_button_opens_the_row_of_its_item() {
        let mut screen = Screen::new(two_repos());
        screen.agenda = vec![Day {
            label: "NO DATE".into(),
            today: false,
            items: vec![Item {
                row: 3,
                what: "Install the bot".into(),
                place: "acme-web".into(),
                tab: "heron".into(),
                note: None,
                mark: Mark::Decide,
                tip: String::new(),
            }],
        }];
        assert!(has(&screen.texts(), "NO DATE"));
        let out = screen.click("Decide");
        assert_eq!((out.agenda, out.jump), (Some(3), None));
    }

    #[test]
    fn clicking_a_card_asks_for_its_tab_and_empty_space_asks_for_none() {
        let mut screen = Screen::new(two_repos());
        assert_eq!(screen.click("web-working").jump, Some(1));
        assert_eq!(screen.click("Overview").jump, None);
    }

    #[test]
    fn an_empty_overview_has_nothing_to_click() {
        let mut screen = Screen::new(overview(vec![]));
        assert_eq!(screen.click("Overview"), Out::default());
    }

    #[test]
    fn no_text_overlaps_at_any_window_width() {
        let mut with_ledger = row(
            4,
            "needs-you-with-a-long-name-here-that-goes-on",
            "acme-web",
            AgentStatus::NeedsYou(Reason::Permission),
        );
        with_ledger.look = Some(crate::progress_model::look(
            &tidewatch_core::progress::Ledger::parse(
                "# Goal\nx\n\n## Status\nphase: 2 Build\nstate: working\nnote: a rather long note about what is happening right now in the build\n\n## Checklist\n### 2. Build\n- [~] a\n\n## Needs you\n- Pick\n",
            ),
            false,
        ));
        let mut working = row(
            9,
            "busy-with-an-equally-long-name-for-the-working-lane",
            "acme-api",
            AgentStatus::Working,
        );
        working.elapsed = 7_300.0;
        let idle = row(10, "quiet-one", "acme-api", AgentStatus::NeedsYou(Reason::Quiet));
        let mut screen = Screen::new(overview(vec![with_ledger, working, idle]));
        screen.poofed = vec![PoofedRow {
            index: 0,
            name: "a-long-poofed-name-that-needs-an-ellipsis-to-fit".into(),
            outcome: "PR #2340".into(),
            took: "1h 02m".into(),
            summary: String::new(),
            place: "acme-web".into(),
            secs: 3720,
        }];
        for width in [360.0, 700.0, 1000.0, 1440.0] {
            screen.width = width;
            screen.frame(vec![]);
            let found = screen.frame(vec![]).1;
            for (index, (a, a_rect)) in found.iter().enumerate() {
                for (b, b_rect) in &found[index + 1..] {
                    assert!(
                        !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                        "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                    );
                }
            }
        }
    }

    #[test]
    fn cards_flow_into_columns_by_width() {
        let cases = [(240.0, 1), (520.0, 2), (800.0, 3), (2000.0, 3)];
        for (width, want) in cases {
            assert_eq!(Lane::Working.columns(width), want, "{width}");
        }
        assert_eq!(Lane::Idle.columns(1000.0), 2);
        assert_eq!(Lane::Idle.columns(400.0), 1);
    }
}
