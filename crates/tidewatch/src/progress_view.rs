//! The Progress panel in the History card: ring and goal, a bar per phase, the question, the last
//! turn's summary, the current phase's rows and the tags that open the lists. Which side a tab shows is kept here.

use std::collections::HashMap;
use std::time::Duration;
use tidewatch_core::prefs::RightPanel;

use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, Frame, Margin, Rect, ScrollArea, Sense, Shape, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::link_board::LinkBoard;
use tidewatch_core::panel_rules::fresh;
use tidewatch_core::panel_state::MarkSeen;
use tidewatch_core::progress::{Entry, Ledger, Mark, State};

use crate::brand;
use crate::job_model::Card;
use crate::job_view;
use crate::last_turn::LastTurn;
use crate::last_turn_view;
use crate::link_text::Text;
use crate::link_view;
use crate::meeseeks_model::Card as MeeseeksCard;
use crate::meeseeks_view;
use crate::panel_blocks::{self, Blocks};
use crate::progress_model::{
    self as model, Bar, BarKind, ItemRow, Now, StageRows, StageShape, TAGS, Tag, Tone, entries,
    stage_rows, stages_shown,
};
use crate::progress_ring::{paint_mark, tone_color};
use crate::progress_state::TabLedger;
use crate::reader_view::{ellipsized, paint_item};
use crate::screen_kit::{Size, body, heading, mono};
use crate::screen_kit::{TEXT_BODY, TEXT_SECONDARY, TEXT_TAG};
use crate::selectable;

const WIDE_PAD: f32 = 22.0;
const NARROW_PAD: f32 = 14.0;
const NARROW_BELOW: f32 = 330.0;
const GAP: f32 = 22.0;
const ROW_H: f32 = 30.0;
const MARK_W: f32 = 18.0;
const BAR_H: f32 = 5.0;
const BAR_MIN_W: f32 = 52.0;
const BAR_GAP: f32 = 6.0;
const TAG_H: f32 = Size::Compact.height();
const TAG_GAP: f32 = 8.0;
const LIST_LIMIT: usize = 6;
const SPIN_EVERY: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Progress,
    Changes,
    History,
}

impl Side {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Progress => "Progress",
            Self::Changes => "Changes",
            Self::History => "History",
        }
    }
}

pub(crate) const fn sides(has_progress: bool) -> &'static [Side] {
    if has_progress {
        &[Side::Progress, Side::Changes, Side::History]
    } else {
        &[Side::Changes, Side::History]
    }
}

impl From<RightPanel> for Side {
    fn from(panel: RightPanel) -> Self {
        match panel {
            RightPanel::Progress => Self::Progress,
            RightPanel::History => Self::History,
        }
    }
}

/// The side the card shows: Changes whenever the user chose it, else History until a ledger
/// exists, then the user's choice, else `first` (the Settings default).
pub(crate) fn side_shown(chosen: Option<Side>, has_ledger: bool, first: Side) -> Side {
    match (has_ledger, chosen) {
        (_, Some(Side::Changes)) => Side::Changes,
        (false, _) => Side::History,
        (true, Some(side)) => side,
        (true, None) => first,
    }
}

/// The switch of a folder tab with no repo: History has nothing to show there.
pub(crate) const fn folder_sides(has_progress: bool) -> &'static [Side] {
    if has_progress { &[Side::Progress, Side::Changes] } else { &[Side::Changes] }
}

pub(crate) fn folder_side_shown(chosen: Option<Side>, has_ledger: bool, first: Side) -> Side {
    match side_shown(chosen, has_ledger, first) {
        Side::History => Side::Changes,
        side => side,
    }
}

/// What the panel remembers of one tab while the app runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TabState {
    pub(crate) chosen: Option<Side>,
    pub(crate) shown: Option<Side>,
    seen: Option<u64>,
    open: [bool; 3],
    all_rows: bool,
    all_links: bool,
    archive_links: bool,
}

impl TabState {
    pub(crate) fn toggle(&mut self, tag: Tag) {
        self.open[tag.index()] ^= true;
    }

    pub(crate) fn is_open(self, tag: Tag) -> bool {
        self.open[tag.index()]
    }

    /// Whether the last turn has Needs you items or a done check the panel has not shown yet.
    pub(crate) fn unseen(self, attention: Option<u64>) -> bool {
        attention.is_some() && self.seen != attention
    }

    pub(crate) fn show_all_rows(self) -> bool {
        self.all_rows
    }

    pub(crate) fn toggle_rows(&mut self) {
        self.all_rows ^= true;
    }
}

#[derive(Default)]
pub(crate) struct Memory {
    tabs: HashMap<u64, TabState>,
}

impl Memory {
    pub(crate) fn tab(&mut self, id: u64) -> &mut TabState {
        self.tabs.entry(id).or_default()
    }

    pub(crate) fn shown(&self, id: u64) -> Option<Side> {
        self.tabs.get(&id).and_then(|state| state.shown)
    }

    pub(crate) fn forget_except(&mut self, open: impl Fn(u64) -> bool) {
        self.tabs.retain(|id, _| open(*id));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Ask { job: u64, message: String },
    TakeOver,
    GiveUp,
    DoneCheck(crate::done_check_view::Action),
    CopyLink(String),
}

pub(crate) struct Input<'a> {
    pub(crate) ledger: Option<&'a TabLedger>,
    /// The agent works on after the ledger asked the owner something (`panel_rules.rs` in core).
    pub(crate) stale: bool,
    pub(crate) jobs: &'a [Card],
    pub(crate) now_ms: i64,
    pub(crate) time: f64,
    /// Whether things that turn may ask for frames.
    pub(crate) animate: bool,
    /// The side shown first once a ledger exists.
    pub(crate) first: Side,
    pub(crate) meeseeks: Option<&'a MeeseeksCard>,
    pub(crate) last_turn: Option<LastTurn>,
    /// The state the tab shows everywhere, when the agent reported one that still stands.
    pub(crate) shown: Option<State>,
    /// What the agent shared and Tidewatch found, kept for the life of the tab.
    pub(crate) links: Option<&'a LinkBoard>,
    /// The age of each block, and whether the facts contradict the report.
    pub(crate) blocks: Blocks,
    /// The steps Tidewatch ticked itself.
    pub(crate) marks: &'a [MarkSeen],
}

fn pad_of(width: f32) -> f32 {
    if width < NARROW_BELOW { NARROW_PAD } else { WIDE_PAD }
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    input: &Input<'_>,
    state: &mut TabState,
) -> Option<Action> {
    let mut action = None;
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).id_salt("progress-panel"));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let pad = pad_of(rect.width()) as i8;
    state.seen = input.last_turn.as_ref().and_then(LastTurn::attention);
    ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
        Frame::new().inner_margin(Margin::symmetric(pad, pad)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            action = content(ui, input, state);
        });
    });
    action
}

fn content(ui: &mut Ui, input: &Input<'_>, state: &mut TabState) -> Option<Action> {
    let width = ui.available_width();
    let mut action = None;
    let links = input.links.filter(|board| !board.is_empty());
    if let Some(card) = input.meeseeks {
        action = meeseeks_view::show(ui, width, card).map(|clicked| match clicked {
            meeseeks_view::Action::TakeOver => Action::TakeOver,
            meeseeks_view::Action::GiveUp => Action::GiveUp,
        });
        if !input.jobs.is_empty()
            || input.ledger.is_some()
            || input.last_turn.is_some()
            || links.is_some()
        {
            ui.add_space(GAP);
        }
    }
    let Some(known) = input.ledger else {
        if let Some(board) = links {
            action = link_card(ui, width, board, input, state).or(action);
            if !input.jobs.is_empty() || input.last_turn.is_some() {
                ui.add_space(GAP);
            }
        }
        let asked = jobs_block(ui, width, input.jobs, input.blocks.jobs.as_deref());
        action = action.or(asked);
        if let Some(turn) = &input.last_turn {
            if !input.jobs.is_empty() {
                ui.add_space(GAP);
            }
            let picked = last_turn_view::show(ui, width, turn, input.blocks.turn.as_deref());
            action = action.or(picked);
        }
        return action;
    };
    let ledger = &*fresh(&known.ledger, input.stale);
    let shape = input.meeseeks.map_or_else(StageShape::default, |card| card.shape);
    let tone = input.shown.map_or_else(|| model::tone(ledger), model::tone_of);
    header(ui, width, ledger, tone, &input.blocks);
    if let Some(board) = links {
        ui.add_space(GAP);
        action = link_card(ui, width, board, input, state).or(action);
    }
    let now = model::now_line(ledger, tone, input.jobs);
    if let Some(now) = &now {
        ui.add_space(GAP);
        now_block(ui, width, now);
    }
    let rest: Vec<Card> = input
        .jobs
        .iter()
        .filter(|card| !matches!(card, Card::Bar(bar) if now.as_ref().and_then(|now| now.job) == Some(bar.id)))
        .cloned()
        .collect();
    if !rest.is_empty() {
        ui.add_space(GAP);
        let picked = jobs_block(ui, width, &rest, input.blocks.jobs.as_deref());
        action = action.or(picked);
    }
    ui.add_space(GAP);
    phase_bars(ui, width, ledger, tone, shape);
    if let Some(turn) = &input.last_turn {
        ui.add_space(GAP);
        let picked = last_turn_view::show(ui, width, turn, input.blocks.turn.as_deref());
        action = picked.or(action);
    }
    let stages = stage_rows(ledger, shape.kind);
    if !stages.is_empty() {
        ui.add_space(GAP);
        steps_block(ui, width, &stages, input, state);
    }
    if TAGS.iter().any(|tag| !entries(ledger, *tag).is_empty()) {
        ui.add_space(GAP);
        tags(ui, width, ledger, state);
    }
    action
}

fn link_card(
    ui: &mut Ui,
    width: f32,
    board: &LinkBoard,
    input: &Input<'_>,
    state: &mut TabState,
) -> Option<Action> {
    let copied = link_view::show(
        ui,
        width,
        board,
        input.now_ms,
        (&mut state.all_links, &mut state.archive_links),
        input.blocks.links.as_deref(),
    );
    copied.map(Action::CopyLink)
}

fn single_line(
    painter: &egui::Painter,
    text: &str,
    font: egui::FontId,
    ink: Color32,
    w: f32,
) -> std::sync::Arc<egui::Galley> {
    ellipsized(painter, text, font, ink, w)
}

fn header(ui: &mut Ui, width: f32, ledger: &Ledger, tone: Tone, blocks: &Blocks) {
    let narrow = width < NARROW_BELOW - 2.0 * NARROW_PAD;
    let title_size = if narrow { 15.0 } else { 17.0 };
    let painter = ui.painter().clone();
    let title =
        if ledger.title.trim().is_empty() { ledger.goal.trim() } else { ledger.title.trim() };
    let flag_w =
        blocks.out_of_date.map_or(0.0, |_| panel_blocks::out_of_date_width(&painter) + 8.0);
    let title = Text::line(title, heading(title_size), brand::TEXT, (width - flag_w).max(40.0))
        .lay_out(&painter);
    let age_w = blocks.card_age().map_or(0.0, |text| panel_blocks::age_width(&painter, text) + 8.0);
    let meta = meta_line(&painter, tone, (width - age_w).max(40.0));
    let since = blocks
        .since
        .as_deref()
        .map(|text| painter.layout_no_wrap(text.to_owned(), body(TEXT_SECONDARY), brand::MUTED));
    let since_h = since.as_ref().map_or(0.0, |galley| galley.size().y + 4.0);
    let block = title.size().y + meta.size().y + 4.0 + since_h;
    let (rect, _) = ui.allocate_exact_size(vec2(width, block), Sense::hover());
    let title_h = title.size().y;
    let title_w = title.size().x;
    title
        .paint_selectable(
            ui,
            ui.id().with("progress-goal-text"),
            pos2(rect.left(), rect.top()),
            brand::MUTED,
        )
        .on_hover_text(ledger.goal.trim());
    if let Some(why) = blocks.out_of_date {
        panel_blocks::paint_out_of_date(
            ui,
            rect.left() + title_w + 8.0,
            rect.top() + title_h / 2.0,
            why,
        );
    }
    let meta_top = rect.top() + title_h + 4.0;
    let meta_mid = meta_top + meta.size().y / 2.0;
    let meta_h = paint_row(ui, rect.left(), meta_top, meta);
    if let Some(text) = blocks.card_age() {
        panel_blocks::paint_age(&painter, rect.right(), meta_mid, text);
    }
    if let Some(galley) = since {
        selectable::galley(ui, pos2(rect.left(), meta_top + meta_h), galley, brand::MUTED);
    }
}

fn paint_row(ui: &Ui, x: f32, y: f32, galley: std::sync::Arc<egui::Galley>) -> f32 {
    let height = galley.size().y;
    selectable::galley(ui, pos2(x, y), galley, brand::MUTED);
    height + 4.0
}

fn meta_line(painter: &egui::Painter, tone: Tone, room: f32) -> std::sync::Arc<egui::Galley> {
    let ink = tone_color(tone);
    let mut job = LayoutJob::default();
    job.append("\u{25cf} ", 0.0, TextFormat::simple(body(TEXT_SECONDARY), ink));
    job.append(tone.word(), 0.0, TextFormat::simple(body(TEXT_SECONDARY), ink));
    job.wrap = TextWrapping::truncate_at_width(room.max(1.0));
    painter.layout_job(job)
}

/// The job cards under a caption row that carries the block's age, when it has one.
fn jobs_block(ui: &mut Ui, width: f32, cards: &[Card], age: Option<&str>) -> Option<Action> {
    if let Some(age) = age.filter(|_| !cards.is_empty()) {
        let painter = ui.painter().clone();
        let galley = painter.layout_no_wrap("JOBS".to_owned(), body(12.0), brand::MUTED);
        let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 8.0), Sense::hover());
        let mid = rect.top() + galley.size().y / 2.0;
        painter.galley(pos2(rect.left(), rect.top()), galley, brand::MUTED);
        panel_blocks::paint_age(&painter, rect.right(), mid, age);
    }
    job_view::show(ui, width, cards).map(|(job, message)| Action::Ask { job, message })
}

/// How many bars fit in a row of `width`, and so how the phases wrap.
pub(crate) fn bar_columns(width: f32, count: usize) -> usize {
    let fit = ((width + BAR_GAP) / (BAR_MIN_W + BAR_GAP)).floor() as usize;
    fit.clamp(1, count.max(1))
}

fn phase_bars(ui: &mut Ui, width: f32, ledger: &Ledger, tone: Tone, shape: StageShape) {
    let bars = model::bars(ledger, shape);
    if bars.is_empty() {
        return;
    }
    let columns = bar_columns(width, bars.len());
    let col_w = (width - BAR_GAP * (columns - 1) as f32) / columns as f32;
    let label_h = 16.0;
    let row_h = BAR_H + 7.0 + label_h;
    for (line, chunk) in bars.chunks(columns).enumerate() {
        if line > 0 {
            ui.add_space(12.0);
        }
        let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
        for (index, bar) in chunk.iter().enumerate() {
            let left = rect.left() + (col_w + BAR_GAP) * index as f32;
            let column = Rect::from_min_size(pos2(left, rect.top()), vec2(col_w, row_h));
            paint_bar(ui, column, bar, tone, line * columns + index);
        }
    }
}

fn paint_bar(ui: &mut Ui, column: Rect, bar: &Bar, tone: Tone, index: usize) {
    let painter = ui.painter();
    let track = Rect::from_min_size(column.min, vec2(column.width(), BAR_H));
    let radius = CornerRadius::same((BAR_H / 2.0) as u8);
    match bar.kind {
        BarKind::Skipped => {
            let y = track.center().y;
            painter.extend(Shape::dashed_line(
                &[pos2(track.left() + 2.0, y), pos2(track.right() - 2.0, y)],
                Stroke::new(1.5, brand::FAINT.gamma_multiply(0.6)),
                4.0,
                3.0,
            ));
        }
        BarKind::Plain => {
            painter.rect_filled(track, radius, brand::FIELD_LINE);
            if bar.fill > 0.0 {
                let fill = Rect::from_min_size(track.min, vec2(track.width() * bar.fill, BAR_H));
                let ink = if tone == Tone::Done { brand::DONE } else { brand::TEAL };
                painter.rect_filled(fill, radius, ink);
            }
        }
    }
    let ink = if bar.current { brand::TEXT } else { brand::MUTED };
    let label = single_line(painter, &bar.label, body(12.0), ink, column.width());
    let at = pos2(column.left(), track.bottom() + 7.0);
    painter.galley(at, label, ink);
    let area = Rect::from_min_size(column.min, vec2(column.width(), column.height()));
    let tip = if bar.total == 0 {
        bar.label.clone()
    } else {
        format!("{}: {}/{}", bar.label, bar.done, bar.total)
    };
    ui.interact(area, ui.id().with(("progress-bar", index)), Sense::hover()).on_hover_text(tip);
}

fn now_block(ui: &mut Ui, width: f32, now: &Now) {
    const LINE_H: f32 = 22.0;
    const BAR_GAP: f32 = 6.0;
    const BAR_HEIGHT: f32 = 6.0;
    const MIN_TEXT_W: f32 = 90.0;
    let painter = ui.painter().clone();
    let label = painter.layout_no_wrap(now.label.to_owned(), body(14.0), brand::MUTED);
    let right = now
        .right
        .as_ref()
        .map(|text| painter.layout_no_wrap(text.clone(), body(14.0), brand::MUTED));
    let right_w = right.as_ref().map_or(0.0, |galley| galley.size().x + 12.0);
    let text_x = label.size().x + 8.0;
    let stacked = right.is_some() && width - text_x - right_w < MIN_TEXT_W;
    let text_room = if stacked { width - text_x } else { width - text_x - right_w };
    let text = ellipsized(&painter, &now.text, body(14.0), brand::TEXT, text_room.max(20.0));
    let lines = if stacked { 2.0 } else { 1.0 };
    let bar_h = if now.fraction.is_some() { BAR_GAP + BAR_HEIGHT } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(width, LINE_H * lines + bar_h), Sense::hover());
    let mid = rect.top() + LINE_H / 2.0;
    painter.galley(pos2(rect.left(), mid - label.size().y / 2.0), label, brand::MUTED);
    selectable::galley(
        ui,
        pos2(rect.left() + text_x, mid - text.size().y / 2.0),
        text,
        brand::TEXT,
    );
    if let (Some(right), Some(full)) = (right, now.right.as_deref()) {
        if stacked {
            let fitted = ellipsized(&painter, full, body(14.0), brand::MUTED, width);
            let at = pos2(rect.left(), mid + LINE_H - fitted.size().y / 2.0);
            painter.galley(at, fitted, brand::MUTED);
        } else {
            let at = pos2(rect.right() - right.size().x, mid - right.size().y / 2.0);
            painter.galley(at, right, brand::MUTED);
        }
    }
    if let Some(fraction) = now.fraction {
        let track = Rect::from_min_size(
            pos2(rect.left(), rect.top() + LINE_H * lines + BAR_GAP),
            vec2(rect.width(), BAR_HEIGHT),
        );
        let radius = CornerRadius::same(3);
        painter.rect_filled(track, radius, brand::ITEM_LINE);
        if fraction > 0.0 {
            let fill = Rect::from_min_size(track.min, vec2(track.width() * fraction, BAR_HEIGHT));
            painter.rect_filled(fill, radius, brand::TEAL);
        }
    }
}

fn steps_block(
    ui: &mut Ui,
    width: f32,
    stages: &[StageRows<'_>],
    input: &Input<'_>,
    state: &mut TabState,
) {
    let shown = stages_shown(stages, state.show_all_rows());
    let hidden = stages.len() - shown.len();
    let mut spinning = false;
    for (index, stage) in shown.iter().enumerate() {
        if index > 0 {
            ui.add_space(12.0);
        }
        stage_heading(ui, width, &stage.title);
        for row in &stage.rows {
            let (mark, by_tidewatch) = panel_blocks::tick(
                row.mark,
                panel_blocks::ticked_by_tidewatch(input.marks, row.text),
            );
            spinning |= mark == Mark::Doing;
            item_row(ui, width, (row, mark, by_tidewatch), input.time);
        }
    }
    if spinning && input.animate {
        ui.ctx().request_repaint_after(SPIN_EVERY);
    }
    if hidden > 0 || state.show_all_rows() {
        let text = if state.show_all_rows() {
            "Show less".to_owned()
        } else {
            format!("Show {hidden} finished stage{}", if hidden == 1 { "" } else { "s" })
        };
        ui.add_space(4.0);
        if link(ui, &text) {
            state.toggle_rows();
        }
    }
}

fn stage_heading(ui: &mut Ui, width: f32, title: &str) {
    let painter = ui.painter().clone();
    let galley = ellipsized(&painter, &title.to_uppercase(), body(12.0), brand::MUTED, width);
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 8.0), Sense::hover());
    selectable::galley(ui, pos2(rect.left(), rect.top()), galley, brand::MUTED);
}

fn item_row(
    ui: &mut Ui,
    width: f32,
    (row, mark, by_tidewatch): (&ItemRow<'_>, Mark, bool),
    time: f64,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    let painter = ui.painter();
    paint_mark(painter, pos2(rect.left() + MARK_W / 2.0, rect.center().y), 14.0, mark, time);
    let text_x = rect.left() + MARK_W + 12.0;
    let muted = matches!(mark, Mark::Skipped);
    let ink = if muted { brand::FAINT } else { brand::TEXT };
    let room = (rect.right() - text_x).max(1.0);
    let why_ink = if mark == Mark::Blocked { brand::NEEDS } else { brand::FAINT };
    let why_text = panel_blocks::why_with_tick(row.why, by_tidewatch);
    let why = why_text.as_deref().map(|why| {
        Text::line(why, body(TEXT_SECONDARY), why_ink, (room * 0.42).max(40.0)).lay_out(painter)
    });
    let why_w = why.as_ref().map_or(0.0, |galley| galley.size().x + 12.0);
    let label =
        Text::line(row.text, body(TEXT_BODY), ink, (room - why_w).max(1.0)).lay_out(painter);
    let at = pos2(text_x, rect.center().y - label.size().y / 2.0);
    let tip = match why_text.as_deref() {
        Some(why) => format!("{} - {why}", row.text),
        None => row.text.to_owned(),
    };
    label.paint_selectable(ui, response.id.with("label"), at, ink).on_hover_text(&tip);
    if let Some(galley) = why {
        let at = pos2(rect.right() - galley.size().x, rect.center().y - galley.size().y / 2.0);
        galley.paint_selectable(ui, response.id.with("why"), at, why_ink).on_hover_text(&tip);
    }
    response.on_hover_text(tip);
}

fn link(ui: &mut Ui, text: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), body(TEXT_SECONDARY), brand::MUTED);
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 8.0, 22.0), Sense::click());
    let ink = if response.hovered() { brand::TEXT } else { brand::MUTED };
    ui.painter().galley(rect.left_center() + vec2(0.0, -galley.size().y / 2.0), galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Where pills of `row_h` tall, `gap` apart, sit when they flow in rows of `width`: the top left
/// of each, and the height of everything.
pub(crate) fn flow(widths: &[f32], width: f32, row_h: f32, gap: f32) -> (Vec<(f32, f32)>, f32) {
    let mut at = Vec::with_capacity(widths.len());
    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    for pill in widths {
        if x > 0.0 && x + pill > width {
            x = 0.0;
            y += row_h + gap;
        }
        at.push((x, y));
        x += pill + gap;
    }
    (at, if widths.is_empty() { 0.0 } else { y + row_h })
}

fn tags(ui: &mut Ui, width: f32, ledger: &Ledger, state: &mut TabState) {
    let shown: Vec<(Tag, usize)> = TAGS
        .iter()
        .map(|tag| (*tag, entries(ledger, *tag).len()))
        .filter(|(_, count)| *count > 0)
        .collect();
    let labels: Vec<_> = shown.iter().map(|(tag, count)| tag.label(*count)).collect();
    let icon_w = |tag: Tag| if tag == Tag::Decisions { 0.0 } else { 16.0 };
    let widths: Vec<f32> = shown
        .iter()
        .zip(&labels)
        .map(|((tag, _), label)| {
            let text =
                ui.painter().layout_no_wrap(label.clone(), body(TEXT_SECONDARY), brand::SOFT);
            (text.size().x + 22.0 + icon_w(*tag)).min(width)
        })
        .collect();
    let (places, height) = flow(&widths, width, TAG_H, TAG_GAP);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    for (index, ((tag, _), label)) in shown.iter().zip(&labels).enumerate() {
        let (x, y) = places[index];
        let pill = Rect::from_min_size(rect.min + vec2(x, y), vec2(widths[index], TAG_H));
        let response =
            ui.interact(pill, ui.id().with(("progress-tag", tag.index())), Sense::click());
        let open = state.is_open(*tag);
        let painter = ui.painter();
        let line = if open {
            brand::TEAL.gamma_multiply(0.6)
        } else if response.hovered() {
            brand::FIELD_LINE
        } else {
            brand::ITEM_LINE
        };
        painter.rect(
            pill,
            CornerRadius::same(Size::Compact.radius()),
            Color32::TRANSPARENT,
            Stroke::new(1.0, line),
            StrokeKind::Inside,
        );
        let ink = if open { brand::TEXT } else { brand::SOFT };
        let mut left = pill.left() + 11.0;
        match tag {
            Tag::Verified => {
                paint_mark(painter, pos2(left + 5.0, pill.center().y), 12.0, Mark::Done, 0.0);
                left += 16.0;
            }
            Tag::NotVerified => {
                paint_mark(painter, pos2(left + 5.0, pill.center().y), 12.0, Mark::Todo, 0.0);
                left += 16.0;
            }
            Tag::Decisions => {}
        }
        let text = single_line(
            painter,
            label,
            body(TEXT_SECONDARY),
            ink,
            (pill.right() - 11.0 - left).max(1.0),
        );
        painter.galley(pos2(left, pill.center().y - text.size().y / 2.0), text, ink);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            state.toggle(*tag);
        }
    }
    for (tag, _) in &shown {
        if state.is_open(*tag) {
            ui.add_space(14.0);
            entry_list(ui, width, *tag, &entries(ledger, *tag));
        }
    }
}

fn entry_list(ui: &mut Ui, width: f32, tag: Tag, list: &[&Entry]) {
    const PAD: f32 = 10.0;
    const BULLET: f32 = 16.0;
    let painter = ui.painter().clone();
    let caption = painter.layout_no_wrap(tag.caption().to_owned(), body(TEXT_TAG), brand::FAINT);
    let (cap_rect, _) = ui.allocate_exact_size(vec2(width, caption.size().y + 6.0), Sense::hover());
    selectable::galley(ui, cap_rect.left_top(), caption, brand::FAINT);
    let text_w = (width - 2.0 * PAD - BULLET).max(40.0);
    let galleys: Vec<_> = list
        .iter()
        .take(LIST_LIMIT)
        .map(|entry| {
            let mut text = Text::new();
            text.run(entry.text.trim(), &TextFormat::simple(body(TEXT_SECONDARY), brand::SOFT));
            if let Some(why) = entry.why.as_deref().map(str::trim).filter(|why| !why.is_empty()) {
                text.run(&format!("  {why}"), &TextFormat::simple(body(12.0), brand::FAINT));
            }
            text.wrapped(text_w, 3).lay_out(&painter)
        })
        .collect();
    let more = list.len().saturating_sub(LIST_LIMIT);
    let more_h = if more > 0 { 20.0 } else { 0.0 };
    let body_h: f32 = galleys.iter().map(|galley| galley.size().y + 8.0).sum::<f32>() - 8.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, body_h + more_h + 2.0 * PAD), Sense::hover());
    paint_item(&painter, rect, false);
    let mut y = rect.top() + PAD;
    for (index, galley) in galleys.into_iter().enumerate() {
        let center = pos2(rect.left() + PAD + 5.0, y + 8.0);
        match tag {
            Tag::Verified => paint_mark(&painter, center, 12.0, Mark::Done, 0.0),
            Tag::NotVerified => paint_mark(&painter, center, 12.0, Mark::Todo, 0.0),
            Tag::Decisions => {
                painter.circle_filled(center, 2.0, brand::FAINT);
            }
        }
        let height = galley.size().y;
        let line_id = ui.id().with(("entry-text", tag.caption(), index));
        galley.paint_selectable(ui, line_id, pos2(rect.left() + PAD + BULLET, y), brand::SOFT);
        y += height + if index + 1 < LIST_LIMIT { 8.0 } else { 0.0 };
    }
    if more > 0 {
        painter.text(
            pos2(rect.left() + PAD + BULLET, rect.bottom() - PAD - 8.0),
            Align2::LEFT_CENTER,
            format!("+{more} more"),
            mono(TEXT_TAG),
            brand::FAINT,
        );
    }
}

#[cfg(test)]
mod tests;
