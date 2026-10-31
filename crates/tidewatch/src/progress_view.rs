//! The Progress panel in the History card, top to bottom: the goal, Waiting on you, Now, Waiting
//! for, the stage bars and steps, Coming up, the lists, the done check and the links. Which side a
//! tab shows is kept here.

use std::collections::HashMap;
use std::time::Duration;
use tidewatch_core::prefs::RightPanel;

use egui::{
    CornerRadius, Frame, Margin, Rect, ScrollArea, Sense, Shape, Stroke, Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::link_board::LinkBoard;
use tidewatch_core::panel_rules::fresh;
use tidewatch_core::panel_state::{CheckSeen, MarkSeen};
use tidewatch_core::panel_view::Why;
use tidewatch_core::progress::{Ledger, State};

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
use crate::progress_blocks::{self as blocks_view, Column, Steps};
use crate::progress_model::{
    self as model, Bar, BarKind, StageShape, TAGS, Tag, Tone, entries, stage_rows, stages_shown,
};
use crate::progress_state::TabLedger;
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, TEXT_SECONDARY, body, heading};

const WIDE_PAD: f32 = 22.0;
const NARROW_PAD: f32 = 14.0;
const NARROW_BELOW: f32 = 330.0;
/// Below this panel width a row puts its label over its value.
pub(crate) const STACK_BELOW: f32 = 360.0;
const GAP: f32 = 22.0;
const BAR_H: f32 = 5.0;
const BAR_MIN_W: f32 = 52.0;
const BAR_MAX_W: f32 = 120.0;
const BAR_GAP: f32 = 6.0;
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
    all_entries: [bool; 3],
    verified_open: bool,
    all_rows: bool,
    links: (bool, bool),
}

impl TabState {
    /// Whether the list of `tag` shows all its entries.
    pub(crate) fn shows_all(self, tag: Tag) -> bool {
        self.all_entries[tag.index()]
    }

    pub(crate) fn toggle_entries(&mut self, tag: Tag) {
        self.all_entries[tag.index()] ^= true;
    }

    pub(crate) fn verified_open(self) -> bool {
        self.verified_open
    }

    pub(crate) fn toggle_verified(&mut self) {
        self.verified_open ^= true;
    }

    /// Whether the panel has something new for the owner that it has not shown yet.
    pub(crate) fn unseen(self, attention: Option<u64>) -> bool {
        attention.is_some() && self.seen != attention
    }

    pub(crate) fn show_all_rows(self) -> bool {
        self.all_rows
    }

    pub(crate) fn toggle_rows(&mut self) {
        self.all_rows ^= true;
    }

    fn forget_short_folds(&mut self, counts: [usize; 3]) {
        for tag in TAGS {
            if counts[tag.index()] <= blocks_view::LIST_LIMIT {
                self.all_entries[tag.index()] = false;
            }
        }
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
    /// The checks Tidewatch ran and saw finish.
    pub(crate) checks: &'a [CheckSeen],
}

impl Input<'_> {
    /// The ledger as the panel reads it: the report with its question and wait taken out when the
    /// facts contradict it.
    fn ledger_shown(&self) -> Option<std::borrow::Cow<'_, Ledger>> {
        self.ledger.map(|known| fresh(&known.ledger, self.stale))
    }

    /// The questions the owner is asked: the report's, else the Needs you items of the last message.
    fn asked(&self) -> Option<model::Asked> {
        let fallback: &[String] = self.last_turn.as_ref().map_or(&[], |turn| &turn.needs);
        match self.ledger_shown() {
            Some(ledger) => model::asked(&ledger, fallback),
            None => model::asked(&Ledger::default(), fallback),
        }
    }

    /// A number for what wants the owner's eye: a question and the last turn's done check. It
    /// changes whenever either does, which is what makes them new.
    pub(crate) fn attention(&self) -> Option<u64> {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let turn = self.last_turn.as_ref().and_then(LastTurn::attention);
        let asked = self.asked();
        if turn.is_none() && asked.is_none() {
            return None;
        }
        let mut hasher = DefaultHasher::new();
        (turn, asked).hash(&mut hasher);
        Some(hasher.finish())
    }

    /// Whether the report on show was saved before the turn that runs now.
    fn holds_last_turn(&self) -> bool {
        matches!(self.blocks.out_of_date, Some(Why::DoneButTurnRunning | Why::OlderThanTurn))
    }
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
    state.seen = input.attention();
    let stack = rect.width() < STACK_BELOW;
    ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
        Frame::new().inner_margin(Margin::symmetric(pad, pad)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            action = content(ui, input, state, stack);
        });
    });
    action
}

/// The space between two blocks: none before the first.
struct Gaps(bool);

impl Gaps {
    fn before(&mut self, ui: &mut Ui) {
        if std::mem::replace(&mut self.0, true) {
            ui.add_space(GAP);
        }
    }
}

fn content(ui: &mut Ui, input: &Input<'_>, state: &mut TabState, stack: bool) -> Option<Action> {
    let width = ui.available_width();
    let mut action = None;
    let mut gaps = Gaps(false);
    let old = input.holds_last_turn();
    let column = Column { width, stack, dim: false };
    if let Some(card) = input.meeseeks {
        gaps.before(ui);
        action = meeseeks_view::show(ui, width, card).map(|clicked| match clicked {
            meeseeks_view::Action::TakeOver => Action::TakeOver,
            meeseeks_view::Action::GiveUp => Action::GiveUp,
        });
    }
    let ledger = input.ledger_shown();
    let ledger = ledger.as_deref();
    let shape = input.meeseeks.map_or_else(StageShape::default, |card| card.shape);
    let tone = match (input.shown, ledger) {
        (Some(state), _) => model::tone_of(state),
        (None, Some(ledger)) => model::tone(ledger),
        (None, None) => Tone::Working,
    };
    if let Some(ledger) = ledger {
        gaps.before(ui);
        header(ui, width, ledger, &input.blocks);
    }
    if let Some(asked) = input.asked() {
        gaps.before(ui);
        blocks_view::asked_block(ui, column, &asked);
    }
    let mut lead = None;
    if let Some(ledger) = ledger {
        gaps.before(ui);
        let now = model::now_line(ledger, tone, input.jobs, old);
        blocks_view::now_block(ui, column, &now, &input.blocks);
        lead = now.job.map(|job| job.id);
        if let Some(what) = model::waiting_for(ledger, tone) {
            ui.add_space(8.0);
            blocks_view::waiting_block(ui, column, &what);
        }
    }
    let rest: Vec<Card> = input
        .jobs
        .iter()
        .filter(|card| !matches!(card, Card::Bar(bar) if lead == Some(bar.id)))
        .cloned()
        .collect();
    if !rest.is_empty() {
        gaps.before(ui);
        let picked = jobs_block(ui, width, stack, &rest, input.blocks.jobs.as_deref());
        action = action.or(picked);
    }
    let mut bars = ledger.map_or_else(Vec::new, |ledger| model::bars(ledger, shape));
    bars.extend(input.meeseeks.and_then(crate::meeseeks_model::Card::checks_bar));
    if !bars.is_empty() {
        gaps.before(ui);
        phase_bars(ui, width, &bars, tone);
    }
    let mut heading_shown = !old;
    if let Some(ledger) = ledger {
        let report = Column { dim: old, ..column };
        report_blocks(ui, (input, ledger), (&mut gaps, &mut heading_shown), report, state);
    }
    if let Some(turn) = input.last_turn.as_ref().filter(|turn| turn.check.is_some()) {
        gaps.before(ui);
        let picked = last_turn_view::show_check(ui, width, turn, input.blocks.turn.as_deref());
        action = picked.or(action);
    }
    if let Some(board) = input.links.filter(|board| !board.is_empty()) {
        gaps.before(ui);
        action = link_card(ui, width, stack, board, input, state).or(action);
    }
    action
}

fn old_heading(ui: &mut Ui, column: Column, shown: &mut bool) {
    if !std::mem::replace(shown, true) {
        blocks_view::section_label(ui, Column { dim: false, ..column }, "From the last turn", None);
        ui.add_space(4.0);
    }
}

/// The blocks that come from the report itself: the steps, Coming up and the lists. A report from
/// the last turn shows them dimmed under one heading.
fn report_blocks(
    ui: &mut Ui,
    (input, ledger): (&Input<'_>, &Ledger),
    (gaps, heading_shown): (&mut Gaps, &mut bool),
    column: Column,
    state: &mut TabState,
) {
    let shape = input.meeseeks.map_or_else(StageShape::default, |card| card.shape);
    let stages = stage_rows(ledger, shape.kind);
    if !stages.is_empty() {
        gaps.before(ui);
        old_heading(ui, column, heading_shown);
        steps_block(ui, column, &stages, input, state);
    }
    let coming = model::coming_up(ledger);
    if !coming.is_empty() {
        gaps.before(ui);
        old_heading(ui, column, heading_shown);
        blocks_view::coming_block(ui, column, &coming);
    }
    state.forget_short_folds(TAGS.map(|tag| entries(ledger, tag).len()));
    for tag in [Tag::Decisions, Tag::NotVerified, Tag::Verified] {
        let list = entries(ledger, tag);
        if list.is_empty() {
            continue;
        }
        gaps.before(ui);
        old_heading(ui, column, heading_shown);
        if tag == Tag::Verified && !state.verified_open() {
            let text = format!("Show {} verified", list.len());
            if blocks_view::fold_link(ui, &text, "verified") {
                state.toggle_verified();
            }
            continue;
        }
        if blocks_view::list_block(ui, column, (tag, &list), state.shows_all(tag)) {
            state.toggle_entries(tag);
        }
        if tag == Tag::Verified && blocks_view::fold_link(ui, "Hide verified", "verified-hide") {
            state.toggle_verified();
        }
    }
}

fn link_card(
    ui: &mut Ui,
    width: f32,
    stack: bool,
    board: &LinkBoard,
    input: &Input<'_>,
    state: &mut TabState,
) -> Option<Action> {
    let copied = link_view::show(
        ui,
        width,
        stack,
        board,
        input.now_ms,
        (&mut state.links.0, &mut state.links.1),
        input.blocks.links.as_deref(),
    );
    copied.map(Action::CopyLink)
}

/// The goal as the panel's title, wrapped, with the mark of a report the facts contradict beside
/// it or, when the title leaves no room, under it.
fn header(ui: &mut Ui, width: f32, ledger: &Ledger, blocks: &Blocks) {
    let narrow = width < NARROW_BELOW - 2.0 * NARROW_PAD;
    let title_size = if narrow { 15.0 } else { 17.0 };
    let painter = ui.painter().clone();
    let title =
        if ledger.title.trim().is_empty() { ledger.goal.trim() } else { ledger.title.trim() };
    let flag_w = blocks.out_of_date.map(|_| panel_blocks::out_of_date_width(&painter));
    let mut text = Text::new();
    text.run(title, &egui::epaint::text::TextFormat::simple(heading(title_size), brand::TEXT));
    let title = text.wrapped(width.max(40.0), 3).lay_out(&painter);
    let size = title.size();
    let single = size.y < title_size * 1.8;
    let beside = flag_w.is_some_and(|flag| single && size.x + 8.0 + flag <= width);
    let below = flag_w.is_some() && !beside;
    let extra = if below { Size::Tag.height() + 6.0 } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(width, size.y + extra), Sense::hover());
    title
        .paint_selectable(
            ui,
            ui.id().with("progress-goal-text"),
            pos2(rect.left(), rect.top()),
            brand::MUTED,
        )
        .on_hover_text(ledger.goal.trim());
    if let Some(why) = blocks.out_of_date {
        let (left, mid) = if beside {
            (rect.left() + size.x + 8.0, rect.top() + size.y / 2.0)
        } else {
            (rect.left(), rect.top() + size.y + 6.0 + Size::Tag.height() / 2.0)
        };
        panel_blocks::paint_out_of_date(ui, left, mid, why);
    }
}

/// The job cards under a caption row that carries the block's age, when it has one.
fn jobs_block(
    ui: &mut Ui,
    width: f32,
    stack: bool,
    cards: &[Card],
    age: Option<&str>,
) -> Option<Action> {
    if let Some(age) = age.filter(|_| !cards.is_empty()) {
        let painter = ui.painter().clone();
        let galley =
            painter.layout_no_wrap("Jobs".to_owned(), heading(TEXT_SECONDARY), brand::SOFT);
        let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y + 8.0), Sense::hover());
        let mid = rect.top() + galley.size().y / 2.0;
        painter.galley(pos2(rect.left(), rect.top()), galley, brand::SOFT);
        panel_blocks::paint_age(&painter, rect.right(), mid, age);
    }
    job_view::show(ui, width, stack, cards).map(|(job, message)| Action::Ask { job, message })
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

/// How many bars fit in a row of `width` when none may be narrower than `min`, and so how the
/// phases wrap.
pub(crate) fn bar_columns(width: f32, count: usize, min: f32) -> usize {
    let fit = ((width + BAR_GAP) / (min.max(BAR_MIN_W) + BAR_GAP)).floor() as usize;
    fit.clamp(1, count.max(1))
}

fn phase_bars(ui: &mut Ui, width: f32, bars: &[Bar], tone: Tone) {
    let painter = ui.painter().clone();
    let labels: Vec<_> = bars
        .iter()
        .map(|bar| painter.layout_no_wrap(bar.label.clone(), body(12.0), brand::MUTED))
        .collect();
    let widest = labels.iter().map(|galley| galley.size().x).fold(0.0, f32::max);
    let columns = bar_columns(width, bars.len(), widest.min(BAR_MAX_W));
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
    let label = ellipsized(painter, &bar.label, body(12.0), ink, column.width());
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

fn steps_block(
    ui: &mut Ui,
    column: Column,
    stages: &[model::StageRows<'_>],
    input: &Input<'_>,
    state: &mut TabState,
) {
    let all = state.show_all_rows();
    let shown = stages_shown(stages, all);
    let hidden = stages.len() - shown.len();
    let folds = stages.iter().any(|stage| stage.over) && stages.iter().any(|stage| !stage.over);
    let steps =
        Steps { stages: &shown, marks: input.marks, checks: input.checks, time: input.time };
    let spinning = blocks_view::steps_block(ui, column, &steps);
    if spinning && input.animate {
        ui.ctx().request_repaint_after(SPIN_EVERY);
    }
    if !folds {
        state.all_rows = false;
        return;
    }
    let text = if all {
        "Show less".to_owned()
    } else {
        format!("Show {hidden} finished stage{}", if hidden == 1 { "" } else { "s" })
    };
    ui.add_space(4.0);
    if blocks_view::fold_link(ui, &text, "stages") {
        state.toggle_rows();
    }
}

#[cfg(test)]
mod tests;
