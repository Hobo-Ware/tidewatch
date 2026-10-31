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
use crate::answer_chips::{self, Pick};
use crate::app::strip::{ICON_BOX, ICON_GLYPH};
use crate::brand;
use crate::commands::Command;
use crate::mascot::{self, Pose};
use crate::overview::{
    Board, Overview, PoofedRow, RepoChip, Row, WORKING_CAP, ask_button, capped, card_view,
    fmt_elapsed, idle_line, needs_line, repo_ink, waiting_line,
};
use crate::overview_rail;
use crate::panel_blocks;
use crate::progress_model::Ask;
use crate::progress_ring;
use crate::progress_view::flow;
use crate::reader_view::ellipsized;
use crate::screen_kit::TEXT_TAG;
use crate::screen_kit::{
    Size, TEXT_PAGE, TEXT_SECONDARY, TEXT_TITLE, TagFill, body, button_width, heading, paint_tag,
    primary_button, tag_width,
};
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
const FACE: f32 = 44.0;
const FACE_IN_RING: f32 = 38.0;
const RING: f32 = 48.0;
const FACE_GAP: f32 = 10.0;
const ASK_ROW_GAP: f32 = 8.0;
const WHY_H: f32 = 18.0;
const ONE_BUTTON_GAP: f32 = 12.0;
const STAGE_ROOM: f32 = 70.0;
const MIN_STAGE_W: f32 = 24.0;
const AGE_GAP: f32 = 10.0;
const ONE_BUTTON_MID: f32 = f32::midpoint(NAME_MID, LINE_MID);
const NAME_MID: f32 = 10.0;
const LINE_MID: f32 = 31.0;
const ASK_CARD_MIN_W: f32 = 420.0;
const BAR_GAP: f32 = 10.0;
const BAR_H: f32 = 3.0;
pub(crate) const SWATCH: f32 = 7.0;
const CHIP_H: f32 = Size::Compact.height();
const CHIP_GAP: f32 = 8.0;
const CHIP_CHARS: usize = 18;
const CHIP_ROWS_MAX: usize = 3;
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
    /// The tab whose Needs you card had a ready answer chosen, and the choice.
    pub(crate) answer: Option<(u64, Pick)>,
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
    let font = FontId::proportional(TEXT_SECONDARY);
    let text = ui.painter().layout_no_wrap(label, font.clone(), brand::SOFT).size().x;
    let count = ui.painter().layout_no_wrap(count.to_string(), font, brand::FAINT).size().x;
    2.0 * Size::Compact.pad() + swatch + text + 6.0 + count
}

fn chip_label(label: &str) -> String {
    stdusk_egui::ui::ellipsize(label, CHIP_CHARS).0
}

fn header(ui: &mut Ui, area: Rect, board: &Board, state: &mut State) -> f32 {
    let (left, right) = (area.left() + SIDE, area.right() - SIDE);
    let painter = ui.painter().clone();
    let title = painter.layout_no_wrap("Overview".to_owned(), heading(TEXT_PAGE), brand::TEXT);
    let summary =
        painter.layout_no_wrap(board.summary(), FontId::proportional(TEXT_SECONDARY), brand::SOFT);
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
            (brand::TEAL, brand::TEAL_TEXT)
        } else if resp.hovered() {
            (brand::FAINT, brand::TEXT)
        } else {
            (brand::ITEM_LINE, brand::SOFT)
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
            .color(brand::MUTED),
        );
    });
}

struct Plan {
    text: f32,
    action: f32,
    ask: Option<AskLayout>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Needs,
    Working,
    Waiting,
    Idle,
}

impl Lane {
    fn title(self) -> &'static str {
        match self {
            Self::Needs => "Needs you",
            Self::Working => "Working",
            Self::Waiting => "Waiting",
            Self::Idle => "Idle",
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Self::Waiting => "soonest first",
            Self::Idle => "nothing asked",
            _ => "",
        }
    }

    fn dot(self) -> Color32 {
        match self {
            Self::Needs => brand::NEEDS,
            Self::Working => brand::WORKING,
            Self::Waiting => brand::WAITING,
            Self::Idle => brand::FAINT,
        }
    }

    fn action_gap(self) -> f32 {
        match self {
            Self::Needs => ASK_ROW_GAP,
            Self::Working => BAR_GAP,
            Self::Waiting | Self::Idle => 0.0,
        }
    }

    fn plan(self, painter: &Painter, row: &Row, card_w: f32) -> Plan {
        let counting = f32::from(u8::from(row.auto_pick.is_some()));
        match (self, ask_of(row)) {
            (Self::Needs, Some(ask)) => {
                let ask = AskLayout::new(painter, ask, card_w);
                let lines = f32::from(u8::from(ask.has_why)) + counting;
                let (text, action) = (ROW_H + WHY_H * lines, ask.row.height());
                Plan { text, action, ask: Some(ask) }
            }
            (Self::Needs, None) => Plan { text: ROW_H + WHY_H * counting, action: 0.0, ask: None },
            (Self::Working, _) => Plan { text: ROW_H, action: BAR_H, ask: None },
            (Self::Waiting | Self::Idle, _) => Plan { text: ROW_H, action: 0.0, ask: None },
        }
    }

    /// The one height every card of a line takes: the tallest text above and the tallest action
    /// area below. Each card's own action area ends at the bottom padding.
    fn line_height(self, plans: &[Plan]) -> f32 {
        let fold = |part: fn(&Plan) -> f32| plans.iter().map(part).fold(0.0, f32::max);
        let (text, action) = (fold(|plan| plan.text), fold(|plan| plan.action));
        let gap = if action > 0.0 { self.action_gap() } else { 0.0 };
        CARD_PAD * 2.0 + text + gap + action
    }

    fn columns(self, width: f32, answering: bool) -> usize {
        let (min, max) = match self {
            Self::Needs if answering => (ASK_CARD_MIN_W, 3),
            Self::Needs | Self::Working | Self::Waiting => (250.0, 3),
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
        let drawn = grid_section(ui, width, Lane::Needs, &board.needs, board.needs.len(), None);
        out.jump = out.jump.or(drawn.jump);
        out.answer = out.answer.take().or(drawn.answer);
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
        let drawn =
            grid_section(ui, width, Lane::Working, shown, board.working.len(), more.as_deref());
        out.jump = out.jump.or(drawn.jump);
        if drawn.toggled {
            state.working_all = !state.working_all;
        }
        ui.add_space(SECTION_GAP);
    }
    if !board.waiting.is_empty() {
        let drawn =
            grid_section(ui, width, Lane::Waiting, &board.waiting, board.waiting.len(), None);
        out.jump = out.jump.or(drawn.jump);
        ui.add_space(SECTION_GAP);
    }
    if !board.idle.is_empty() {
        let drawn = grid_section(ui, width, Lane::Idle, &board.idle, board.idle.len(), None);
        out.jump = out.jump.or(drawn.jump);
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
        painter.text(pos2(x, mid), Align2::LEFT_CENTER, head.title, heading(TEXT_TITLE), head.ink);
    if !head.caption.is_empty() {
        painter.text(
            pos2(title.right() + 8.0, mid + 1.0),
            Align2::LEFT_CENTER,
            head.caption,
            FontId::proportional(TEXT_SECONDARY),
            brand::MUTED,
        );
    }
    painter.text(
        pos2(section.right() - SECTION_PAD, mid),
        Align2::RIGHT_CENTER,
        head.count,
        FontId::proportional(TEXT_SECONDARY),
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

#[derive(Default)]
struct GridOut {
    jump: Option<u64>,
    toggled: bool,
    answer: Option<(u64, Pick)>,
}

fn grid_section(
    ui: &mut Ui,
    width: f32,
    lane: Lane,
    rows: &[Row],
    total: usize,
    more: Option<&str>,
) -> GridOut {
    let inner_w = width - SECTION_PAD * 2.0;
    let answering = lane == Lane::Needs && rows.iter().any(|row| ask_of(row).is_some());
    let columns = lane.columns(inner_w, answering);
    let card_w = (inner_w - CARD_GAP * (columns - 1) as f32) / columns as f32;
    let painter = ui.painter().clone();
    let plans: Vec<Plan> = rows.iter().map(|row| lane.plan(&painter, row, card_w)).collect();
    let lines: Vec<f32> = plans.chunks(columns).map(|line| lane.line_height(line)).collect();
    let grid_h = lines.iter().sum::<f32>() + lines.len().saturating_sub(1) as f32 * CARD_GAP;
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
    let mut out = GridOut::default();
    let mut line_top = top;
    for (line, cards) in rows.chunks(columns).enumerate() {
        let height = lines[line];
        for (column, row) in cards.iter().enumerate() {
            let at =
                pos2(rect.left() + SECTION_PAD + column as f32 * (card_w + CARD_GAP), line_top);
            let plan = &plans[line * columns + column];
            let own = lane == Lane::Needs && plan.ask.is_none();
            let card_rect = Rect::from_min_size(
                at,
                vec2(
                    card_w,
                    if own { lane.line_height(std::slice::from_ref(plan)) } else { height },
                ),
            );
            let drawn =
                card(ui, card_rect, row, lane, (plan, card_rect.bottom() - CARD_PAD - plan.action));
            if drawn.clicked {
                out.jump = Some(row.tab_id);
            }
            if out.answer.is_none() {
                out.answer = drawn.answer.map(|pick| (row.tab_id, pick));
            }
        }
        line_top += height + CARD_GAP;
    }
    out.toggled = more.is_some_and(|label| {
        ghost(ui, pos2(rect.left() + SECTION_PAD, top + grid_h + CARD_GAP), label)
    });
    out
}

/// An outline pill whose left edge is at `left`, centered on `mid`. Returns its right edge.
fn kind_pill(painter: &Painter, left: f32, mid: f32, text: &str, ink: Color32) -> f32 {
    paint_tag(painter, (left, mid), text, ink, TagFill::Line(ink.gamma_multiply(0.33))).right()
}

/// The repo's square and name, its left edge at `left` and centered on `mid`. Returns the right edge.
pub(crate) fn repo_label(painter: &Painter, left: f32, mid: f32, name: &str, max_w: f32) -> f32 {
    let square = Rect::from_center_size(pos2(left + SWATCH / 2.0, mid), vec2(SWATCH, SWATCH));
    painter.rect_filled(square, CornerRadius::same(2), repo_ink(name));
    let text = ellipsized(
        painter,
        name,
        FontId::proportional(TEXT_SECONDARY),
        brand::SOFT,
        max_w - SWATCH - 4.0,
    );
    let right = left + SWATCH + 4.0 + text.size().x;
    painter.galley(pos2(left + SWATCH + 4.0, mid - text.size().y / 2.0), text, brand::SOFT);
    right
}

#[derive(Default)]
struct CardOut {
    clicked: bool,
    answer: Option<Pick>,
}

fn ask_of(row: &Row) -> Option<&Ask> {
    row.look.as_ref()?.ask.as_ref()
}

/// The answer chips of a Needs you card, laid out to the room beside its face.
struct AskLayout {
    chips: Vec<answer_chips::Chip>,
    row: answer_chips::Row,
    room: f32,
    has_why: bool,
}

impl AskLayout {
    fn new(painter: &Painter, ask: &Ask, card_w: f32) -> Self {
        let room = (card_w - CARD_PAD * 2.0 - RING - FACE_GAP).max(40.0);
        let chips = answer_chips::chips(&ask.options);
        let row = answer_chips::Row::new(painter, &chips, room);
        Self { chips, row, room, has_why: ask.why.is_some() }
    }
}

fn card(
    ui: &mut Ui,
    rect: Rect,
    row: &Row,
    lane: Lane,
    (plan, action_top): (&Plan, f32),
) -> CardOut {
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
    let ask = ask_of(row).filter(|_| lane == Lane::Needs);
    let one_button = lane == Lane::Needs && ask.is_none();
    let button_w = if one_button { button_width(ui, ask_button(row), Size::Compact) } else { 0.0 };
    let button_room = if one_button { button_w + ONE_BUTTON_GAP } else { 0.0 };
    let clock = (lane == Lane::Working).then(|| {
        painter.layout_no_wrap(
            fmt_elapsed(row.elapsed),
            FontId::monospace(TEXT_SECONDARY),
            brand::FAINT,
        )
    });
    let full = (inner.right() - text_x - button_room).max(1.0);
    let tag = row.kind.label();
    let pill_w = if lane == Lane::Working { 0.0 } else { tag_width(&painter, &tag) };
    let name_ink = if lane == Lane::Idle { brand::SOFT } else { brand::TEXT };
    let flag_w =
        row.stamp.out_of_date.map_or(0.0, |_| panel_blocks::out_of_date_width(&painter) + 6.0);
    let name = ellipsized(
        &painter,
        &row.title,
        heading(TEXT_TITLE),
        name_ink,
        (full - flag_w - if pill_w > 0.0 { pill_w + 6.0 } else { 0.0 }).max(20.0),
    );
    let name_mid = inner.top() + NAME_MID;
    let mut used = text_x + name.size().x;
    painter.galley(pos2(text_x, name_mid - name.size().y / 2.0), name, name_ink);
    let right_edge = text_x + full;
    let mut flag_up = false;
    if let Some(why) = row.stamp.out_of_date
        && used + flag_w <= right_edge + 1.0
    {
        used = panel_blocks::paint_out_of_date(ui, used + 6.0, name_mid, why).right();
        flag_up = true;
    }
    let age = row
        .stamp
        .age
        .as_deref()
        .map(|text| (text, panel_blocks::age_width(&painter, text)))
        .filter(|(_, width)| right_edge - width >= used + AGE_GAP);
    let pill_limit = age.map_or(right_edge, |(_, width)| right_edge - width - AGE_GAP);
    if pill_w > 0.0 && used + 6.0 + pill_w <= pill_limit + 1.0 {
        kind_pill(&painter, used + 6.0, name_mid, &tag, row.kind.ink());
    }
    if let Some((text, _)) = age {
        panel_blocks::paint_age(&painter, right_edge, name_mid, text);
    }
    let lost_flag = row.stamp.out_of_date.filter(|_| !flag_up);
    let lost_quiet = row
        .stamp
        .age
        .as_deref()
        .filter(|text| text.starts_with(panel_blocks::QUIET_PREFIX) && age.is_none());
    let marks_below = lost_flag.is_some() || lost_quiet.is_some();
    let clock = clock.filter(|_| !marks_below);
    let clock_w = clock.as_ref().map_or(0.0, |clock| clock.size().x + 8.0);
    let line_mid = inner.top() + LINE_MID;
    let (text, ink) = match ask {
        Some(ask) => (ask.text.clone(), brand::NEEDS),
        None => second_line(row, lane, &view),
    };
    let mut line_left = text_x;
    if lane == Lane::Working {
        let kind =
            painter.layout_no_wrap(tag, FontId::proportional(TEXT_SECONDARY), row.kind.ink());
        let kind_size = kind.size();
        painter.galley(pos2(line_left, line_mid - kind_size.y / 2.0), kind, row.kind.ink());
        line_left += kind_size.x + 6.0;
        let dot = painter.layout_no_wrap(
            "\u{b7}".to_owned(),
            FontId::proportional(TEXT_SECONDARY),
            brand::MUTED,
        );
        painter.galley(pos2(line_left, line_mid - dot.size().y / 2.0), dot.clone(), brand::MUTED);
        line_left += dot.size().x + 6.0;
    }
    let line_room = (inner.right() - line_left - button_room - clock_w).max(1.0);
    let marks_w = lost_flag.map_or(0.0, |_| panel_blocks::out_of_date_width(&painter) + 6.0)
        + lost_quiet.map_or(0.0, |text| panel_blocks::age_width(&painter, text) + 6.0);
    let repo_fits = !marks_below || line_room - marks_w >= MIN_STAGE_W + 6.0;
    let repo_right = if repo_fits {
        let repo_room = if marks_below {
            line_room - marks_w - 6.0
        } else {
            (line_room - STAGE_ROOM).max(line_room * 0.5)
        };
        repo_label(&painter, line_left, line_mid, &row.place, repo_room)
    } else {
        line_left - 6.0
    };
    let rest_room = line_left + line_room - repo_right - 6.0;
    if marks_below {
        let mut x = repo_right + 6.0;
        if let Some(why) = lost_flag {
            x = panel_blocks::paint_out_of_date(ui, x, line_mid, why).right() + 6.0;
        }
        if let Some(text) = lost_quiet {
            let room = (line_left + line_room - x).max(20.0);
            let galley = ellipsized(&painter, text, body(TEXT_TAG), brand::MUTED, room);
            painter.galley(pos2(x, line_mid - galley.size().y / 2.0), galley, brand::MUTED);
        }
    } else if rest_room >= MIN_STAGE_W {
        let rest = ellipsized(
            &painter,
            &format!("\u{b7} {text}"),
            FontId::proportional(TEXT_SECONDARY),
            ink,
            rest_room,
        );
        painter.galley(pos2(repo_right + 6.0, line_mid - rest.size().y / 2.0), rest, ink);
    }
    if let Some(clock) = clock {
        painter.galley(
            pos2(inner.right() - clock.size().x, line_mid - clock.size().y / 2.0),
            clock,
            brand::FAINT,
        );
    }
    let mut out = CardOut::default();
    match lane {
        Lane::Needs => {
            if let (Some(ask), Some(layout)) = (ask, &plan.ask) {
                out.answer = ask_chips(ui, &painter, (rect, &resp), (ask, row, layout), action_top);
            } else {
                let button = Rect::from_min_size(
                    pos2(
                        inner.right() - button_w,
                        inner.top() + ONE_BUTTON_MID - Size::Compact.height() / 2.0,
                    ),
                    vec2(button_w, Size::Compact.height()),
                );
                out.clicked = ask_row(ui, &painter, rect, row, button);
            }
        }
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
        Lane::Waiting | Lane::Idle => {}
    }
    let tip = match ask {
        Some(ask) => ask.why.as_deref().map_or_else(
            || ask.text.clone(),
            |why| format!("{}\n{}", ask.text, answer_chips::sentence(why)),
        ),
        None => match row.stamp.age.as_deref() {
            Some(age) => format!("{} \u{b7} {age}", status_view::describe(&row.status)),
            None => status_view::describe(&row.status),
        },
    };
    let card_clicked =
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked();
    out.clicked |= card_clicked || out.answer == Some(Pick::Other);
    out
}

fn ask_chips(
    ui: &mut Ui,
    painter: &Painter,
    (rect, resp): (Rect, &egui::Response),
    (ask, row, layout): (&Ask, &Row, &AskLayout),
    action_top: f32,
) -> Option<Pick> {
    let text_x = rect.left() + CARD_PAD + RING + FACE_GAP;
    let why = ask.why.as_deref().map(answer_chips::sentence);
    let lines = why.iter().chain(row.auto_pick.iter());
    for (index, line) in lines.enumerate() {
        let galley = ellipsized(
            painter,
            line,
            FontId::proportional(TEXT_SECONDARY),
            brand::MUTED,
            layout.room,
        );
        let at = pos2(text_x, rect.top() + CARD_PAD + ROW_H + 1.0 + WHY_H * index as f32);
        painter.galley(at, galley, brand::MUTED);
    }
    let origin = pos2(text_x, action_top);
    let clicked = layout.row.show(ui, resp.id, origin, &layout.chips);
    let card = answer_chips::Card { id: resp.id, rect, terminal_hidden: true };
    let picked = clicked.or_else(|| answer_chips::key_pick(ui, &card, &layout.chips))?;
    answer_chips::pick_of(&layout.chips, picked)
}

fn second_line(row: &Row, lane: Lane, view: &crate::overview::CardView) -> (String, Color32) {
    match lane {
        Lane::Needs => (needs_line(row, &view.line), brand::NEEDS),
        Lane::Working => (view.line.clone(), brand::MUTED),
        Lane::Waiting => (waiting_line(row, &view.line), brand::WAITING),
        Lane::Idle => {
            let failed = row.tone == Tone::Failed;
            (idle_line(row), if failed { brand::RED } else { brand::MUTED })
        }
    }
}

fn ask_row(ui: &mut Ui, painter: &Painter, rect: Rect, row: &Row, button: Rect) -> bool {
    let text_x = rect.left() + CARD_PAD + RING + FACE_GAP;
    if let Some(line) = &row.auto_pick {
        let room = (rect.right() - CARD_PAD - text_x).max(1.0);
        let galley =
            ellipsized(painter, line, FontId::proportional(TEXT_SECONDARY), brand::MUTED, room);
        painter.galley(pos2(text_x, rect.top() + CARD_PAD + ROW_H + 1.0), galley, brand::MUTED);
    }
    in_rect(ui, button, |ui| primary_button(ui, ask_button(row), true, Size::Compact).clicked())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::{Item, Mark};
    use crate::overview::{AgentKind, PlaceInput};
    use egui::{Event, PointerButton, RawInput};
    use tidewatch_core::status::{AgentStatus, Reason};

    #[test]
    fn the_ring_clears_the_round_face() {
        let inner_edge = RING / 2.0 - progress_ring::stroke_of(RING);
        assert!(
            inner_edge >= FACE_IN_RING / 2.0,
            "ring starts at {inner_edge}, face reaches {}",
            FACE_IN_RING / 2.0
        );
    }

    fn walk_discs(shape: &egui::Shape, found: &mut Vec<(egui::Pos2, f32)>) {
        match shape {
            egui::Shape::Circle(circle) if circle.fill != Color32::TRANSPARENT => {
                found.push((circle.center, circle.radius));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk_discs(shape, found)),
            _ => {}
        }
    }

    #[test]
    fn the_faces_are_44_plain_and_38_inside_the_48_ring() {
        assert_eq!((FACE, FACE_IN_RING, RING), (44.0, 38.0, 48.0));
        assert_eq!(CARD_PAD * 2.0 + ROW_H, 64.0);
    }

    #[test]
    fn a_card_keeps_its_height_and_its_face_is_centered_on_the_title_and_the_line() {
        let mut screen = Screen::new(overview(vec![
            row(1, "working-one", "acme-web", AgentStatus::Working),
            row(2, "idle-one", "acme-web", AgentStatus::Done),
        ]));
        screen.width = 2000.0;
        screen.frame(vec![]);
        let ctx = screen.ctx.clone();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(2000.0, 900.0))),
            focused: true,
            ..RawInput::default()
        };
        let sections = Sections { agenda: &screen.agenda, poofed: &screen.poofed };
        let output = ctx.run_ui(raw, |ui| {
            let area = ui.max_rect();
            show(ui, area, &screen.data, &mut screen.state, &sections, |_| {});
        });
        let (mut found, mut boxes, mut discs) = (Vec::new(), Vec::new(), Vec::new());
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut found);
            walk_boxes(&clipped.shape, &mut boxes);
            walk_discs(&clipped.shape, &mut discs);
        }
        for (title, size, below) in
            [("working-one", FACE_IN_RING, BAR_GAP + BAR_H), ("idle-one", FACE, 0.0)]
        {
            let name = rect_of(&found, title);
            let card = card_around(&boxes, &name);
            assert_eq!(card.height(), CARD_PAD * 2.0 + ROW_H + below, "{title}");
            let face = discs
                .iter()
                .find(|(center, radius)| {
                    (*radius - size / 2.0).abs() < 0.01
                        && card.contains(*center)
                        && center.x < name.left()
                })
                .unwrap_or_else(|| panic!("{title} has no {size} face in {discs:?}"));
            let line = found
                .iter()
                .find(|(text, rect)| {
                    text.contains("acme-web")
                        && (rect.top() - name.top()).abs() < 30.0
                        && card.contains_rect(*rect)
                })
                .unwrap_or_else(|| panic!("{title} has no second line"))
                .1;
            let block_middle = f32::midpoint(name.top(), line.bottom());
            assert!(
                (face.0.y - block_middle).abs() <= 1.0,
                "{title}: face centre {} against text block centre {block_middle}",
                face.0.y
            );
            assert!(face.0.x + size / 2.0 < name.left(), "{title}: the face reaches the text");
            assert_eq!(name.left(), card.left() + CARD_PAD + RING + FACE_GAP, "{title}");
            let room = CARD_PAD + ROW_H / 2.0 - RING / 2.0;
            assert!(room >= 8.0, "{title}: the ring leaves {room} above it");
            let row_middle = card.top() + CARD_PAD + ROW_H / 2.0;
            assert!((face.0.y - row_middle).abs() <= 0.5, "{title}: {} {row_middle}", face.0.y);
        }
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
            agent_id: "claude".into(),
            branch: format!("agent/{title}"),
            tone: crate::status_view::tone(&status),
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
            stamp: crate::overview::Stamp::default(),
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

    fn walk_boxes(shape: &egui::Shape, found: &mut Vec<Rect>) {
        match shape {
            egui::Shape::Rect(rect) => found.push(rect.rect),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk_boxes(shape, found)),
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
            let (out, found, _) = self.frame_drawn(events);
            (out, found)
        }

        fn frame_drawn(&mut self, events: Vec<Event>) -> (Out, Vec<(String, Rect)>, Vec<Rect>) {
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
            let mut boxes = Vec::new();
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut found);
                walk_boxes(&clipped.shape, &mut boxes);
            }
            (out, found, boxes)
        }

        fn drawn(&mut self) -> (Vec<(String, Rect)>, Vec<Rect>) {
            self.frame(vec![]);
            let (_, found, boxes) = self.frame_drawn(vec![]);
            (found, boxes)
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
        assert!(has(&texts, "2 agents \u{b7} 1 working \u{b7} 1 idle"));
        assert!(has(&texts, "went-quiet"));
    }

    fn reported_row(id: u64, title: &str, state: &str, extra: &serde_json::Value) -> Row {
        let mut args = serde_json::json!({
            "goal": "g", "state": state, "now": "doing it",
            "stages": [
                { "name": "Main", "kind": "ship", "status": "active" },
                { "name": "Verify", "kind": "verify", "status": "todo" }
            ],
            "steps": [{ "stage": "Main", "text": "a step", "status": "active" }]
        });
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let ledger =
            tidewatch_core::progress_report::ProgressReport::check(args.as_object().unwrap(), None)
                .unwrap()
                .to_ledger();
        let mut row = row(id, title, "acme-web", AgentStatus::Done);
        let reported = ledger.shown_state();
        row.tone = crate::status_view::shown(&row.status, Some(reported));
        row.look = Some(crate::progress_model::look_as(&ledger, false, reported));
        row
    }

    #[test]
    fn waiting_agents_have_their_own_section_and_blocked_ones_sit_with_what_needs_you() {
        let mut screen = Screen::new(overview(vec![
            reported_row(
                1,
                "waits-on-pr",
                "waiting",
                &serde_json::json!({ "waiting_on": { "pr": 2340 } }),
            ),
            reported_row(
                2,
                "waits-on-date",
                "waiting",
                &serde_json::json!({ "waiting_on": { "date": "2026-10-28" } }),
            ),
            reported_row(3, "is-stuck", "blocked", &serde_json::json!({})),
            row(4, "busy", "acme-web", AgentStatus::Working),
        ]));
        let texts = screen.texts();
        for want in [
            "Waiting",
            "soonest first",
            "\u{b7} Waiting on PR #2340",
            "\u{b7} Waiting until Wed 28",
            "Needs you",
            "is-stuck",
            "Review",
        ] {
            assert!(has(&texts, want), "{want:?} not drawn in {texts:?}");
        }
        assert!(has(&texts, "4 agents \u{b7} 1 need you \u{b7} 1 working \u{b7} 2 waiting"));
        let order = |name: &str| texts.iter().position(|text| text == name).unwrap();
        assert!(order("waits-on-date") < order("waits-on-pr"), "the dated wait is soonest");
        assert_eq!(screen.click("is-stuck").jump, Some(3));
    }

    #[test]
    fn a_picked_repo_that_loses_its_last_agent_falls_back_to_all_and_the_chips_agree() {
        let mut screen = Screen::new(two_repos());
        screen.click("acme-api");
        assert_eq!(screen.state.repo.as_deref(), Some("acme-api"));
        screen.data = crate::overview::build(vec![
            PlaceInput {
                label: "acme-web".into(),
                path: "/work/acme-web".into(),
                rows: vec![
                    row(1, "web-working", "acme-web", AgentStatus::Working),
                    row(2, "web-asks", "acme-web", AgentStatus::NeedsYou(Reason::Question)),
                ],
            },
            PlaceInput { label: "acme-api".into(), path: "/work/acme-api".into(), rows: vec![] },
        ]);
        let texts = screen.texts();
        assert_eq!(screen.state.repo, None);
        assert!(has(&texts, "web-working") && has(&texts, "web-asks"), "{texts:?}");
        assert!(!has(&texts, "Nothing running. Start an agent with"), "{texts:?}");
        assert!(has(&texts, "2 agents \u{b7} 1 need you \u{b7} 1 working"));
    }

    fn poofed(count: usize) -> Vec<PoofedRow> {
        (0..count)
            .map(|n| PoofedRow {
                index: n,
                name: format!("poof-{n}"),
                outcome: "PR #1".into(),
                took: "1m".into(),
                summary: String::new(),
                place: "acme-web".into(),
                secs: 60,
            })
            .collect()
    }

    fn rail_right(screen: &mut Screen) -> f32 {
        let (_, boxes) = screen.drawn();
        boxes
            .iter()
            .filter(|rect| rect.width() > 100.0 && rect.top() > 56.0)
            .map(egui::Rect::right)
            .filter(|right| *right < screen.width - 1.0)
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_rail_reaches_the_header_edge_until_the_body_scrolls_and_then_leaves_the_bar_room() {
        let mut screen = Screen::new(two_repos());
        brand::tint_visuals(&screen.ctx);
        screen.poofed = poofed(2);
        assert_eq!(rail_right(&mut screen), screen.width - SIDE);
        let rows: Vec<Row> =
            (0..40).map(|n| row(n, &format!("agent-{n}"), "acme-web", AgentStatus::Done)).collect();
        let mut busy = Screen::new(overview(rows));
        brand::tint_visuals(&busy.ctx);
        busy.poofed = poofed(2);
        let right = rail_right(&mut busy);
        assert_eq!(right, busy.width - SIDE - brand::SCROLL_ROOM);
    }

    #[test]
    fn wrapped_chips_show_three_rows_and_scroll_past_them() {
        let cap = 3.0 * CHIP_H + 2.0 * CHIP_GAP;
        let cases = [(3, 500.0, false), (8, 500.0, false), (40, 500.0, true), (200, 900.0, true)];
        for (count, width, scrolls) in cases {
            let wrapped = wrap_chips(&vec![110.0; count], width);
            assert_eq!(wrapped.scrolls, scrolls, "{count} chips in {width}");
            assert!(wrapped.shown <= cap, "{count} chips in {width}");
            assert_eq!(wrapped.at.len(), count);
            if scrolls {
                let right = wrapped.at.iter().map(|(x, _)| x + 110.0).fold(0.0, f32::max);
                assert!(right <= width - brand::SCROLL_ROOM, "the bar keeps its room");
                assert!(wrapped.full > cap);
            }
        }
    }

    #[test]
    fn many_repos_in_a_narrow_window_leave_the_cards_on_screen() {
        let rows: Vec<Row> = (0..80)
            .map(|n| row(n, &format!("agent-{n}"), &format!("repo-{n:02}"), AgentStatus::Working))
            .collect();
        let mut screen = Screen::new(overview(rows));
        screen.width = 600.0;
        let (found, _) = screen.drawn();
        let card = found.iter().find(|(text, _)| text == "agent-0").map(|(_, rect)| *rect);
        let card = card.expect("the first card is drawn");
        assert!(card.top() < 320.0, "the chips pushed the cards down to {}", card.top());
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
        assert!(has(&texts, "2 agents \u{b7} 1 working \u{b7} 1 idle"));
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
            about: false,
            tip: String::new(),
        };
        screen.agenda = vec![Day {
            label: "TODAY \u{b7} MON 26".into(),
            today: true,
            month: None,
            starts_month: false,
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
            month: None,
            starts_month: false,
            items: vec![Item {
                row: 3,
                what: "Install the bot".into(),
                place: "acme-web".into(),
                tab: "heron".into(),
                note: None,
                mark: Mark::Decide,
                about: false,
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

    const ASKING: &str = "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Post the Slack follow-up -- the owner wanted it out today -- options: Post it | Skip it* | Draft only | Ask Sam\n";
    const ASKING_PLAIN: &str = "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Should the filters apply to archived items?\n";

    fn asking(id: u64, ledger: &str) -> Row {
        let mut asking = row(id, "asker", "acme-web", AgentStatus::NeedsYou(Reason::Question));
        asking.look = Some(crate::progress_model::look(
            &tidewatch_core::progress::Ledger::parse(ledger),
            false,
        ));
        asking
    }

    fn key(number: egui::Key) -> Event {
        Event::Key {
            key: number,
            physical_key: Some(number),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn box_around(boxes: &[Rect], text: &Rect) -> Rect {
        boxes
            .iter()
            .filter(|rect| rect.contains_rect(*text) && rect.height() < 60.0)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .copied()
            .expect("a box around the text")
    }

    fn rect_of(found: &[(String, Rect)], label: &str) -> Rect {
        found.iter().find(|(text, _)| text == label).unwrap_or_else(|| panic!("{label}")).1
    }

    #[test]
    fn a_one_button_card_puts_its_button_on_the_title_row_and_has_no_empty_band() {
        let mut screen = Screen::new(overview(vec![row(
            2,
            "web-asks",
            "acme-web",
            AgentStatus::NeedsYou(Reason::Question),
        )]));
        let (found, boxes) = screen.drawn();
        let button = box_around(&boxes, &rect_of(&found, "Answer"));
        let tag = box_around(&boxes, &rect_of(&found, "Claude"));
        let title = rect_of(&found, "web-asks");
        let card = card_around(&boxes, &title);
        let second = found
            .iter()
            .find(|(text, _)| text.contains("Asked a question"))
            .map(|(_, rect)| *rect)
            .expect("the second line");
        assert_eq!(button.height(), Size::Compact.height());
        assert_eq!(tag.height(), Size::Tag.height());
        assert!((button.right() + CARD_PAD - card.right()).abs() < 1.5, "{button:?} {card:?}");
        assert!(button.left() > tag.right(), "{button:?} {tag:?}");
        let block_middle = f32::midpoint(title.top(), second.bottom());
        assert!((button.center().y - block_middle).abs() <= 3.0, "{button:?} {title:?} {second:?}");
        assert_eq!(card.height(), CARD_PAD * 2.0 + ROW_H, "no empty band");
        assert!(second.right() <= button.left(), "{second:?} {button:?}");
        let texts: Vec<String> = found.into_iter().map(|(text, _)| text).collect();
        assert_eq!(texts.iter().filter(|text| *text == "Needs you").count(), 1, "{texts:?}");
        assert!(!has(&texts, "question"), "{texts:?}");
    }

    fn stamped(mut row: Row, age: &str, why: Option<tidewatch_core::panel_view::Why>) -> Row {
        row.stamp = crate::overview::Stamp { age: Some(age.to_owned()), out_of_date: why };
        row
    }

    #[test]
    fn every_lane_puts_the_age_at_the_right_of_the_title_row_and_the_tag_right_after_the_title() {
        use tidewatch_core::panel_view::Why;
        let cases = [
            ("working", AgentStatus::Working),
            ("idle", AgentStatus::NeedsYou(Reason::Quiet)),
            ("asks", AgentStatus::NeedsYou(Reason::Question)),
        ];
        for (title, status) in cases {
            let row =
                stamped(row(2, title, "acme-web", status), "2m ago", Some(Why::OlderThanTurn));
            let mut screen = Screen::new(overview(vec![row]));
            let (found, boxes) = screen.drawn();
            let name = rect_of(&found, title);
            let age = rect_of(&found, "2m ago");
            let tag = found
                .iter()
                .find(|(text, _)| text == "out of date")
                .unwrap_or_else(|| panic!("{title}: {found:?}"))
                .1;
            let card = card_around(&boxes, &name);
            assert!((name.center().y - age.center().y).abs() < 3.0, "{title}: {name:?} {age:?}");
            assert!((name.center().y - tag.center().y).abs() < 4.0, "{title}: {name:?} {tag:?}");
            assert!(tag.left() >= name.right(), "{title}: {name:?} {tag:?}");
            assert!(age.left() >= tag.right(), "{title}: {tag:?} {age:?}");
            assert!(age.right() <= card.right() - CARD_PAD + 1.0, "{title}: {age:?} {card:?}");
            if let Some((_, answer)) = found.iter().find(|(text, _)| text == "Answer") {
                let button = box_around(&boxes, answer);
                assert!(age.right() <= button.left(), "{title}: {age:?} {button:?}");
            }
        }
    }

    #[test]
    fn a_card_with_no_age_and_a_fresh_report_draws_neither() {
        let mut screen =
            Screen::new(overview(vec![row(2, "plain", "acme-web", AgentStatus::Working)]));
        let texts = screen.texts();
        assert!(!has(&texts, "out of date") && !texts.iter().any(|text| text.ends_with("ago")));
    }

    #[test]
    fn no_text_overlaps_with_an_age_and_a_tag_at_any_window_width() {
        use tidewatch_core::panel_view::Why;
        let rows = vec![
            stamped(
                row(
                    4,
                    "needs-you-with-a-long-name-here-that-goes-on",
                    "acme-web",
                    AgentStatus::NeedsYou(Reason::Permission),
                ),
                "no word since 14:02",
                Some(Why::NeedsYouNoQuestion),
            ),
            stamped(
                row(
                    9,
                    "busy-with-an-equally-long-name-for-the-working-lane",
                    "acme-api",
                    AgentStatus::Working,
                ),
                "11h ago",
                Some(Why::WorkingButTurnEnded),
            ),
            stamped(
                row(10, "quiet-one", "acme-api", AgentStatus::NeedsYou(Reason::Quiet)),
                "3d ago",
                None,
            ),
        ];
        let mut screen = Screen::new(overview(rows));
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
    fn a_narrow_working_card_keeps_its_quiet_text_and_tag_and_cuts_the_subtitle_and_time() {
        use tidewatch_core::panel_view::Why;
        let rows: Vec<Row> = (1..=3)
            .map(|id| {
                let mut working = row(
                    id,
                    &format!("reproduce-the-flaky-board-{id}"),
                    "acme-web",
                    AgentStatus::Working,
                );
                working.elapsed = 7.0;
                working.detail = Some("Build \u{b7} reproducing the failure".into());
                stamped(working, "no word since 14:02", Some(Why::WorkingButTurnEnded))
            })
            .collect();
        let mut screen = Screen::new(overview(rows));
        screen.width = 1000.0;
        let (found, boxes) = screen.drawn();
        let quiet: Vec<Rect> = found
            .iter()
            .filter(|(text, _)| text == "no word since 14:02")
            .map(|(_, rect)| *rect)
            .collect();
        let tags: Vec<Rect> =
            found.iter().filter(|(text, _)| text == "out of date").map(|(_, rect)| *rect).collect();
        assert_eq!((quiet.len(), tags.len()), (3, 3), "{found:?}");
        let title = rect_of(&found, "reproduce-the-flaky-board-1");
        let card = card_around(&boxes, &title);
        assert!(card.width() < 330.0, "not the narrow card the test means: {card:?}");
        for mark in quiet.iter().chain(&tags) {
            assert!(
                mark.right() <= card.right() + 0.5 || mark.left() > card.right(),
                "{mark:?} {card:?}"
            );
        }
        assert!(quiet.iter().any(|mark| card.contains_rect(*mark)), "{quiet:?} {card:?}");
        assert!(tags.iter().any(|mark| card.contains_rect(*mark)), "{tags:?} {card:?}");
        for (index, (a, a_rect)) in found.iter().enumerate() {
            for (b, b_rect) in &found[index + 1..] {
                assert!(
                    !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                    "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?}"
                );
            }
        }
        assert!(!found.iter().any(|(text, _)| text == "7s"), "the time is cut first: {found:?}");
    }

    #[test]
    fn a_working_card_gives_the_title_the_whole_line_and_shares_the_second_with_kind_repo_and_time()
    {
        let mut working = row(1, "soundtrack-matcher", "acme-workers", AgentStatus::Working);
        working.elapsed = 720.0;
        let mut screen = Screen::new(overview(vec![working]));
        screen.width = 2000.0;
        let (found, boxes) = screen.drawn();
        let title = rect_of(&found, "soundtrack-matcher");
        let card = card_around(&boxes, &title);
        let in_card = |label: &str| {
            found
                .iter()
                .find(|(text, rect)| text == label && card.contains_rect(*rect))
                .map_or_else(|| panic!("{label} in the card {card:?}"), |(_, rect)| *rect)
        };
        let (kind, repo, time) = (in_card("Claude"), in_card("acme-workers"), in_card("12m"));
        assert!(kind.top() >= title.bottom() - 4.0, "{kind:?} is on the second line");
        assert!((kind.center().y - time.center().y).abs() < 2.0, "{kind:?} {time:?}");
        assert!((kind.center().y - repo.center().y).abs() < 4.0, "{kind:?} {repo:?}");
        assert_eq!(kind.left(), title.left());
        assert!(kind.right() < repo.left() && repo.right() < time.left());
        assert!((time.right() + CARD_PAD - card.right()).abs() < 1.5, "{time:?} {card:?}");
        assert!(
            boxes
                .iter()
                .all(|rect| rect.height() != Size::Tag.height() || !rect.contains_rect(kind)),
            "the kind is plain text on a working card, not a pill"
        );
        assert!(found.iter().all(|(text, _)| text != "12m 00s"));
    }

    #[test]
    fn a_working_title_is_not_cut_for_a_pill_or_a_clock() {
        let long = "soundtrack-matcher-for-the-long-running-migration";
        let mut screen =
            Screen::new(overview(vec![row(1, long, "acme-workers", AgentStatus::Working)]));
        screen.width = 2000.0;
        let (found, boxes) = screen.drawn();
        let title = rect_of(&found, long);
        let card = card_around(&boxes, &title);
        let time = found
            .iter()
            .find(|(text, _)| text.ends_with('s') || text.ends_with('m'))
            .map(|(_, rect)| *rect);
        assert!(title.right() <= card.right() - CARD_PAD, "{title:?} {card:?} {time:?}");
        assert!(title.width() > 280.0, "the whole name is laid out: {title:?}");
    }

    #[test]
    fn answer_chips_are_compact_and_sit_in_one_row_under_the_title_column() {
        let mut screen = Screen::new(overview(vec![asking(7, ASKING)]));
        let (found, boxes) = screen.drawn();
        let title = rect_of(&found, "asker");
        let chips: Vec<Rect> = ["Post it", "Skip it", "Draft only"]
            .iter()
            .map(|label| box_around(&boxes, &rect_of(&found, label)))
            .collect();
        for chip in &chips {
            assert_eq!(chip.height(), Size::Compact.height(), "{chip:?}");
        }
        assert!((chips[0].left() - title.left()).abs() < 1.0, "{:?} {title:?}", chips[0]);
        assert!(chips.windows(2).all(|pair| pair[0].center().y == pair[1].center().y));
    }

    #[test]
    fn a_needs_you_question_draws_the_question_the_why_and_its_chips_in_place_of_answer() {
        let mut screen = Screen::new(overview(vec![asking(7, ASKING)]));
        let texts = screen.texts();
        for want in ["Post it", "Skip it", "Draft only", "Ask Sam", "Something else", "1", "4"] {
            assert!(has(&texts, want), "{want:?} in {texts:?}");
        }
        assert!(texts.iter().any(|text| text.contains("Post the Slack follow-up")), "{texts:?}");
        assert!(has(&texts, "The owner wanted it out today"), "{texts:?}");
        assert!(!has(&texts, "Answer"));
    }

    #[test]
    fn a_question_with_no_options_offers_done_not_now_and_something_else() {
        let mut screen = Screen::new(overview(vec![asking(7, ASKING_PLAIN)]));
        let texts = screen.texts();
        for want in ["Done", "Not now", "Something else"] {
            assert!(has(&texts, want), "{want:?} in {texts:?}");
        }
    }

    #[test]
    fn clicking_an_option_answers_that_tab_with_its_exact_text_and_does_not_open_it() {
        let mut screen = Screen::new(overview(vec![asking(7, ASKING)]));
        let out = screen.click("Skip it");
        assert_eq!(out.answer, Some((7, Pick::Answer("Skip it".to_owned()))));
        assert_eq!(out.jump, None);
    }

    #[test]
    fn something_else_opens_the_tab_and_asks_for_its_reply_box() {
        let mut screen = Screen::new(overview(vec![asking(7, ASKING)]));
        let out = screen.click("Something else");
        assert_eq!((out.answer, out.jump), (Some((7, Pick::Other)), Some(7)));
    }

    #[test]
    fn a_digit_answers_the_card_under_the_pointer_and_only_that_one() {
        let mut screen = Screen::new(overview(vec![asking(7, ASKING), asking(8, ASKING_PLAIN)]));
        screen.frame(vec![]);
        let found = screen.frame(vec![]).1;
        let rects: Vec<Rect> = found
            .iter()
            .filter(|(text, _)| text == "Something else")
            .map(|(_, rect)| *rect)
            .collect();
        assert_eq!(rects.len(), 2);
        let (first, second) = (rects[0].center(), rects[1].center());
        let mut hits = Vec::new();
        for over in [first, second] {
            screen.frame(vec![Event::PointerMoved(over)]);
            hits.push(screen.frame(vec![key(egui::Key::Num2)]).0.answer);
        }
        let want = |tab: u64, text: &str| Some((tab, Pick::Answer(text.to_owned())));
        let mut expected = vec![want(7, "Skip it"), want(8, "Not now")];
        expected.sort_by_key(|hit| hit.as_ref().map(|(tab, _)| *tab));
        hits.sort_by_key(|hit| hit.as_ref().map(|(tab, _)| *tab));
        assert_eq!(hits, expected);
        screen.frame(vec![Event::PointerMoved(pos2(1.0, 890.0))]);
        assert_eq!(screen.frame(vec![key(egui::Key::Num1)]).0.answer, None, "pointer over no card");
    }

    #[test]
    fn a_question_card_keeps_the_auto_pick_countdown_of_an_agent_that_runs_on_its_own() {
        let mut meeseeks = asking(7, ASKING);
        meeseeks.auto_pick = Some("Picks the recommended answer in 3:40".to_owned());
        let mut screen = Screen::new(overview(vec![meeseeks]));
        let texts = screen.texts();
        assert!(has(&texts, "Picks the recommended answer in 3:40"), "{texts:?}");
        assert!(has(&texts, "The owner wanted it out today"));
    }

    #[test]
    fn a_working_agent_never_shows_chips() {
        let mut working = row(3, "busy", "acme-web", AgentStatus::Working);
        working.look = asking(3, ASKING).look;
        let mut screen = Screen::new(overview(vec![working]));
        let texts = screen.texts();
        assert!(!has(&texts, "Something else") && !has(&texts, "Skip it"), "{texts:?}");
    }

    #[test]
    fn no_text_overlaps_in_a_question_card_at_any_window_width() {
        let mut screen = Screen::new(overview(vec![asking(7, ASKING), asking(8, ASKING_PLAIN)]));
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

    const ASKING_LONG: &str = "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Which loss should the recommendation model keep after the long benchmark run? -- Run 14 beat the others on held-out data and the replica agrees -- options: Keep the new loss and rerun the whole benchmark on the replica before the rollout window opens* | Keep the old loss | Blend both losses | Rerun the benchmark\n";

    fn titled(mut row: Row, title: &str) -> Row {
        row.title = title.into();
        row
    }

    fn uneven_row() -> Overview {
        overview(vec![
            titled(asking(7, ASKING_LONG), "long-answers"),
            titled(asking(8, ASKING_PLAIN), "plain-question"),
            row(9, "button-only", "acme-web", AgentStatus::NeedsYou(Reason::Question)),
        ])
    }

    fn card_around(boxes: &[Rect], title: &Rect) -> Rect {
        boxes
            .iter()
            .filter(|rect| rect.contains_rect(*title) && rect.height() > 60.0)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .copied()
            .expect("a card around the title")
    }

    fn last_action_bottom(boxes: &[Rect], card: &Rect) -> f32 {
        boxes
            .iter()
            .filter(|rect| card.contains_rect(**rect) && rect.height() == Size::Compact.height())
            .map(Rect::bottom)
            .fold(f32::NEG_INFINITY, f32::max)
    }

    #[test]
    fn answer_cards_of_very_different_content_share_one_height_and_end_their_actions_on_one_line() {
        let mut screen = Screen::new(uneven_row());
        screen.width = 2000.0;
        let (found, boxes) = screen.drawn();
        let cards: Vec<Rect> = ["long-answers", "plain-question"]
            .iter()
            .map(|title| card_around(&boxes, &rect_of(&found, title)))
            .collect();
        for card in &cards {
            assert_eq!(card.top(), cards[0].top(), "one row: {cards:?}");
            assert_eq!(card.height(), cards[0].height(), "one height: {cards:?}");
        }
        let actions: Vec<f32> = cards.iter().map(|card| last_action_bottom(&boxes, card)).collect();
        for (card, bottom) in cards.iter().zip(&actions) {
            assert!((bottom - actions[0]).abs() < 0.5, "actions end on one line: {actions:?}");
            assert!((bottom + CARD_PAD - card.bottom()).abs() < 1.5, "{card:?} {bottom}");
        }
    }

    #[test]
    fn a_one_button_card_keeps_its_own_height_beside_taller_answer_cards() {
        let mut screen = Screen::new(uneven_row());
        screen.width = 2000.0;
        let (found, boxes) = screen.drawn();
        let card = card_around(&boxes, &rect_of(&found, "button-only"));
        let tallest = card_around(&boxes, &rect_of(&found, "long-answers"));
        let button = box_around(&boxes, &rect_of(&found, "Answer"));
        assert_eq!(card.top(), tallest.top());
        assert_eq!(card.height(), CARD_PAD * 2.0 + ROW_H);
        assert!(tallest.height() > card.height() + 30.0, "{card:?} {tallest:?}");
        assert!(button.top() < card.top() + CARD_PAD + ROW_H / 2.0, "{button:?} {card:?}");
        assert!(card.contains_rect(button), "{button:?} {card:?}");
    }

    #[test]
    fn working_waiting_and_idle_cards_of_a_row_share_one_height() {
        let waiting = |id: u64, title: &str| {
            let mut waiting = row(id, title, "acme-web", AgentStatus::Done);
            waiting.look = Some(crate::progress_model::look_as(
                &tidewatch_core::progress::Ledger::parse(
                    "# Goal\nx\n\n## Status\nstate: waiting\nnote: Waiting for a long reply from the partner about the feed\n",
                ),
                false,
                tidewatch_core::progress::State::Waiting,
            ));
            waiting
        };
        let mut screen = Screen::new(overview(vec![
            row(1, "working-a", "acme-web", AgentStatus::Working),
            titled(
                row(2, "working-b", "acme-web", AgentStatus::Working),
                "w-with-a-very-long-name-that-is-cut",
            ),
            row(3, "idle-a", "acme-web", AgentStatus::Done),
            row(4, "idle-b", "acme-web", AgentStatus::Done),
            waiting(5, "waiting-a"),
            waiting(6, "waiting-b"),
        ]));
        screen.width = 2000.0;
        let (found, boxes) = screen.drawn();
        for titles in [
            ["working-a", "w-with-a-very-long-name-that-is-cut"],
            ["idle-a", "idle-b"],
            ["waiting-a", "waiting-b"],
        ] {
            let cards: Vec<Rect> =
                titles.iter().map(|title| card_around(&boxes, &rect_of(&found, title))).collect();
            assert_eq!(cards[0].height(), cards[1].height(), "{titles:?} {cards:?}");
            assert_eq!(cards[0].top(), cards[1].top(), "{titles:?} {cards:?}");
        }
    }

    #[test]
    fn no_text_overlaps_and_every_action_stays_in_its_card_in_uneven_rows_at_narrow_widths() {
        let mut screen = Screen::new(uneven_row());
        for width in [360.0, 520.0, 700.0, 1000.0, 1440.0, 2000.0] {
            screen.width = width;
            screen.frame(vec![]);
            let (_, found, boxes) = screen.frame_drawn(vec![]);
            for (index, (a, a_rect)) in found.iter().enumerate() {
                for (b, b_rect) in &found[index + 1..] {
                    assert!(
                        !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                        "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                    );
                }
            }
            for title in ["long-answers", "plain-question", "button-only"] {
                let card = card_around(&boxes, &rect_of(&found, title));
                let inside = boxes
                    .iter()
                    .filter(|rect| {
                        rect.height() == Size::Compact.height()
                            && rect.center().y > card.top()
                            && rect.center().y < card.bottom() + Size::Compact.height()
                            && rect.center().x > card.left()
                            && rect.center().x < card.right()
                    })
                    .all(|rect| card.contains_rect(*rect));
                assert!(inside, "an action leaves {title} at {width}: {card:?}");
            }
        }
    }

    #[test]
    fn cards_flow_into_columns_by_width() {
        let cases = [(240.0, 1), (520.0, 2), (800.0, 3), (2000.0, 3)];
        for (width, want) in cases {
            assert_eq!(Lane::Working.columns(width, false), want, "{width}");
        }
        assert_eq!(Lane::Idle.columns(1000.0, false), 2);
        assert_eq!(Lane::Idle.columns(400.0, false), 1);
    }
}
