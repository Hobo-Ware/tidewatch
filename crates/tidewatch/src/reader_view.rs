//! Draws the Reader: the transcript as rows (markdown for the agent, a small card for the user and
//! for each tool call), the permission strip and the reply box. Only rows near the viewport are
//! drawn; the others take their measured or estimated height. It also holds the small card and
//! field looks the other inline items of the left column share.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextWrapping};
use egui::style::ScrollAnimation;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Galley, Id, Layout, Margin, Painter, Rect,
    Response, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use jiff::tz::TimeZone;
use tidewatch_core::question::Reply;
use tidewatch_core::transcript::Source;

use crate::agent_avatar::Face;
use crate::agent_ready::{self, Ready};
use crate::brand;
use crate::composer::{self, Draft, Spec};
use crate::due_view::{self, Facts as DueFacts};
use crate::job_model::clock_label;
use crate::pr_card_model::CardAction;
use crate::pr_cards_view::{self, Data as PrData, Heights as PrHeights};
use crate::pr_timeline::{self, Place};
use crate::prompt_ghost::Ghost;
use crate::question_bulk;
use crate::question_card::{Card, Effect};
use crate::question_view::{self, ChipClick, Live, PermissionClick};
use crate::quick_reply::{self, Quick};
use crate::reader_chat_view::{self, Scene};
use crate::reader_keys::{self, Answer};
use crate::reader_rows::{Kind, Origin, Row, Rows};
use crate::reader_scroll::{self, Autoscroll, Heights};
use crate::reader_sent;
use crate::reader_turns::{self, Slot};
use crate::review::ReviewFacts;
use crate::review_row_view;
use crate::review_view::{self, ReviewAction};
use crate::screen_kit::{Size, body, primary_button, quiet_button};
use crate::slash::Entry as SlashEntry;
use crate::stopped_view;

/// The side inset every inline item of the left column keeps.
pub(crate) const INSET: f32 = 14.0;
const CARD_RADIUS: u8 = 8;
const OVERSCAN: f32 = 600.0;
const QUEUED_FRAME: std::time::Duration = std::time::Duration::from_millis(33);
const STRIP_H: f32 = 48.0;
const BAR_W: f32 = 2.0;
const BAR_INSET: f32 = 7.0;
const JUMP_W: f32 = 140.0;
const JUMP_H: f32 = 30.0;
const SCROLLBAR_GRAB_W: f32 = 16.0;
const OUTPUT_MAX_LINES: usize = 40;

pub(crate) struct ViewState {
    pub(crate) scroll: Autoscroll,
    pub(crate) heights: Heights,
    pr_heights: PrHeights,
    pub(crate) reply: String,
    pub(crate) pictures: Vec<PathBuf>,
    pub(crate) ghost: Ghost,
    review: review_view::CardState,
    focus_reply: bool,
    pub(crate) commands: Arc<Vec<SlashEntry>>,
    content_h: f32,
    body_h: f32,
    seen_rows: u64,
    stick_pending: bool,
    last_pass: Option<u64>,
    #[cfg(test)]
    settled_at: (f32, f32),
    live: Option<Live>,
    auto_left: Option<(u64, f64)>,
    quick: Option<(u64, Vec<Quick>)>,
    quick_used: Option<u64>,
    expanded: HashSet<u64>,
    turns: Option<(u64, usize, reader_turns::Layout)>,
    zone: TimeZone,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scroll: Autoscroll::default(),
            heights: Heights::default(),
            pr_heights: PrHeights::default(),
            reply: String::new(),
            pictures: Vec::new(),
            ghost: Ghost::default(),
            review: review_view::CardState::default(),
            focus_reply: true,
            commands: Arc::default(),
            content_h: 0.0,
            body_h: 0.0,
            seen_rows: 0,
            stick_pending: true,
            last_pass: None,
            #[cfg(test)]
            settled_at: (0.0, 0.0),
            live: None,
            auto_left: None,
            quick: None,
            quick_used: None,
            expanded: HashSet::new(),
            turns: None,
            zone: TimeZone::system(),
        }
    }
}

impl ViewState {
    /// What is left of the wait before Tidewatch answers question row `seq` itself.
    pub(crate) fn set_auto_left(&mut self, left: Option<(u64, f64)>) {
        self.auto_left = left;
    }

    pub(crate) fn live_mut(&mut self) -> Option<&mut Live> {
        self.live.as_mut()
    }

    /// Groups the rows into turns again when they changed since the last frame.
    fn refresh_turns(&mut self, rows: &Rows) {
        let stamp = (rows.revision(), rows.len());
        if self.turns.as_ref().map(|(revision, len, _)| (*revision, *len)) == Some(stamp) {
            return;
        }
        self.turns = Some((stamp.0, stamp.1, reader_turns::layout(rows)));
        let first = rows.iter().next().map_or(0, |row| row.seq);
        self.expanded.retain(|key| key & !reader_turns::NOTES_BIT >= first);
    }

    /// Opens the list of calls and the folded updates under every turn, as a click on a chip does;
    /// only the updates when `notes_only`.
    pub(crate) fn open_turns(&mut self, rows: &Rows, notes_only: bool) {
        self.refresh_turns(rows);
        if let Some((_, _, layout)) = &self.turns {
            let keys = layout.groups.iter().filter(|group| !group.calls.is_empty() && !notes_only);
            self.expanded.extend(keys.map(|group| group.key));
            let notes = layout.groups.iter().filter(|group| !group.notes().is_empty());
            self.expanded.extend(notes.map(reader_turns::Group::notes_key));
        }
    }

    /// The reply box takes the keyboard the next time it is drawn.
    pub(crate) fn request_reply_focus(&mut self) {
        self.focus_reply = true;
    }
}

/// Whether the agent is in the middle of a turn, so a menu may be open, or has finished it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Turn {
    Running,
    Finished,
}

/// What the agent is stuck on, when it waits for something typed into a menu rather than a reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prompt {
    /// A tool call waits for approval.
    Permission,
    /// The agent asked a question.
    Question,
}

/// What the Reader needs to know about the tab beyond its rows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    /// The tab's stable id: each tab keeps its own scroll position.
    pub(crate) tab: u64,
    pub(crate) source: Option<Source>,
    pub(crate) prompt: Option<Prompt>,
    /// Set while the tab waits for its first message: the empty state and the composer show.
    pub(crate) ready: Option<Ready<'a>>,
    /// The folder the agent works in: tool lines show paths relative to it.
    pub(crate) root: Option<&'a Path>,
    /// The tab's review: its draft above the chat box and the sent ones in the transcript.
    pub(crate) review: Option<ReviewFacts<'a>>,
    /// The tab's pull request cards, drawn between the rows by time.
    pub(crate) pr: Option<PrData<'a>>,
    /// An item due today in this tab, offered above the reply box.
    pub(crate) due: Option<DueFacts<'a>>,
    /// The profile's name, for "<Profile> asks".
    pub(crate) profile: &'a str,
    /// The tab's face, shown beside each of the agent's turns.
    pub(crate) face: Option<&'a Face>,
    pub(crate) turn: Turn,
    pub(crate) stopped: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Type this into the agent, then press Enter.
    Reply(String),
    /// Type the keys of this answer into the agent's prompt.
    Answer(Answer),
    SwitchToTerminal,
    Review(ReviewAction),
    /// A click on a pull request card.
    Pr(CardAction),
    /// The user answered, or skipped, the questions in row `seq`.
    Ask(u64, Reply),
    /// Answer every waiting question that has a pick with it.
    GoWithPicks,
    /// A click on the Due today card.
    Due(due_view::Action),
    Resume,
}

/// The look of an inline item: a card on the window ground.
pub(crate) fn item_frame() -> Frame {
    Frame::new()
        .fill(brand::ITEM)
        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
}

/// Paints `rect` as an inline item; a hot one (hovered) takes the stronger line.
pub(crate) fn paint_item(painter: &Painter, rect: Rect, hot: bool) {
    let line = if hot { brand::FIELD_LINE } else { brand::ITEM_LINE };
    let radius = CornerRadius::same(CARD_RADIUS);
    painter.rect_filled(rect, radius, brand::ITEM);
    painter.rect_stroke(rect, radius, Stroke::new(1.0, line), StrokeKind::Inside);
}

/// The short accent bar on the left edge of an inline item that needs attention.
pub(crate) fn paint_bar(painter: &Painter, rect: Rect, color: Color32) {
    let bar = Rect::from_min_max(
        rect.left_top() + vec2(0.0, BAR_INSET),
        rect.left_bottom() + vec2(BAR_W, -BAR_INSET),
    );
    painter.rect_filled(bar, CornerRadius::same(1), color);
}

/// One line of `text` cut with an ellipsis to fit `max_w`.
pub(crate) fn ellipsized(
    painter: &Painter,
    text: &str,
    font: FontId,
    ink: Color32,
    max_w: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, ink);
    job.wrap = TextWrapping::truncate_at_width(max_w.max(1.0));
    painter.layout_job(job)
}

/// A text input in the field look: ITEM fill, 1 px line, teal while it has the keyboard.
pub(crate) fn field_frame(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    field_frame_on(ui, brand::ITEM, false, add)
}

/// `field_frame` on another fill and, with `ring`, a faint teal ring around it while it has the
/// keyboard.
pub(crate) fn field_frame_on(
    ui: &mut Ui,
    fill: Color32,
    ring: bool,
    add: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    field_frame_shaped(ui, (fill, CARD_RADIUS, Margin::symmetric(10, 5)), ring, add)
}

/// The field look with its own fill, corner radius and inner margin.
pub(crate) fn field_frame_shaped(
    ui: &mut Ui,
    (fill, radius, margin): (Color32, u8, Margin),
    ring: bool,
    add: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    let mut prepared = Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(radius))
        .inner_margin(margin)
        .begin(ui);
    prepared.content_ui.set_width(prepared.content_ui.available_width());
    let response = add(&mut prepared.content_ui);
    let focused = response.has_focus();
    let line = if focused { brand::TEAL } else { brand::FIELD_LINE };
    prepared.frame.stroke = Stroke::new(1.0, line);
    let frame = prepared.end(ui);
    if ring && focused {
        ui.painter().rect_stroke(
            frame.rect.expand(1.5),
            CornerRadius::same(radius + 1),
            Stroke::new(3.0, brand::TEAL.gamma_multiply(0.10)),
            StrokeKind::Middle,
        );
    }
    response
}

/// The live card to show, if any: the oldest question waiting for an answer. Claude's own menu
/// counts only while the agent is mid-turn; a held `ask_user` call keeps its card at any time, so a
/// late answer can still be typed.
fn live_row(rows: &Rows, turn: Turn) -> Option<&Row> {
    rows.pending_question().filter(|row| match &row.kind {
        Kind::Question(q) => matches!(q.origin, Origin::Tidewatch(_)) || turn == Turn::Running,
        _ => false,
    })
}

/// Keeps `state.live` on the question being shown: a new card for a new question, none when there
/// is nothing to answer (the reply box then asks for the keyboard).
fn sync_live(state: &mut ViewState, shown: Option<&Row>) {
    let seq = shown.map(|row| row.seq);
    if state.live.as_ref().map(|live| live.seq) == seq {
        return;
    }
    let had = state.live.is_some();
    state.live = shown.and_then(|row| match &row.kind {
        Kind::Question(q) => Some(Live::new(row.seq, Card::new(q.questions.clone()))),
        _ => None,
    });
    if had && state.live.is_none() {
        state.focus_reply = true;
    }
}

/// The quick replies for the newest row when it is the agent's finished message.
fn quick_for<'a>(rows: &Rows, state: &'a mut ViewState, facts: &Facts<'_>) -> &'a [Quick] {
    let last = rows.iter().last();
    let wanted = last.filter(|row| row.kind == Kind::Assistant).filter(|row| {
        facts.turn == Turn::Finished && facts.prompt.is_none() && state.quick_used != Some(row.seq)
    });
    let Some(row) = wanted else {
        state.quick = None;
        return &[];
    };
    if state.quick.as_ref().map(|(seq, _)| *seq) != Some(row.seq) {
        state.quick = Some((row.seq, quick_reply::from_message(&row.text)));
    }
    state.quick.as_ref().map_or(&[][..], |(_, quick)| quick.as_slice())
}

/// The tool call waiting for permission as (tool, what it does), when the newest row is one.
fn permission_tool(rows: &Rows, root: Option<&Path>) -> Option<(String, String)> {
    let tool = rows.waiting_tool()?;
    let line = split_label(&tool.label, root);
    Some((line.name.to_owned(), format!("{}{}", line.dir, line.leaf)))
}

fn dock_height(facts: &Facts<'_>, rows: &Rows, live: Option<&Live>) -> f32 {
    match (facts.prompt, live) {
        (_, Some(live)) => live.height(),
        (Some(Prompt::Permission), None) => {
            let command =
                permission_tool(rows, facts.root).is_some_and(|(_, command)| !command.is_empty());
            question_view::PERMISSION_H
                + if command { question_view::PERMISSION_COMMAND_H } else { 0.0 }
        }
        (Some(Prompt::Question), None) => STRIP_H,
        (None, None) => 0.0,
    }
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    rows: &Rows,
    state: &mut ViewState,
    facts: Facts<'_>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    ui.painter().rect_filled(rect, 0.0, brand::GROUND);
    let pass = ui.ctx().cumulative_pass_nr();
    if reader_scroll::returned(state.last_pass, pass) && state.scroll.following() {
        state.stick_pending = true;
    }
    state.last_pass = Some(pass);
    let shown = live_row(rows, facts.turn);
    sync_live(state, shown);
    let live_seq = state.live.as_ref().map(|live| live.seq);
    let composing = facts.ready.filter(|_| rows.len() == 0 && facts.prompt.is_none());
    let input_h = if composing.is_some() {
        agent_ready::height(ui, &state.reply, state.pictures.len(), rect)
    } else {
        composer::height(ui, &state.reply, state.pictures.len(), rect)
    };
    let input = Rect::from_min_max(pos2(rect.left(), rect.bottom() - input_h), rect.max);
    let dock_h = dock_height(&facts, rows, state.live.as_ref());
    let review_h = facts
        .review
        .as_ref()
        .map_or(0.0, |review| review_view::height(review.draft, rect.height() - input_h - dock_h));
    let strip = Rect::from_min_max(
        pos2(rect.left(), input.top() - dock_h),
        pos2(rect.right(), input.top()),
    );
    let review_band = Rect::from_min_max(
        pos2(rect.left(), strip.top() - review_h),
        pos2(rect.right(), strip.top()),
    );
    let due_h = facts
        .due
        .map_or(0.0, |due| due_view::height(due.due, rect.height() - input_h - dock_h - review_h));
    let due_band = Rect::from_min_max(
        pos2(rect.left(), review_band.top() - due_h),
        pos2(rect.right(), review_band.top()),
    );
    let stopped_h = if facts.stopped && composing.is_none() { stopped_view::HEIGHT } else { 0.0 };
    let body = Rect::from_min_max(
        pos2(rect.left(), rect.top() + stopped_h),
        pos2(rect.right(), due_band.top()),
    );
    if stopped_h > 0.0 {
        let band = Rect::from_min_size(rect.min, vec2(rect.width(), stopped_h));
        if stopped_view::show(ui, band) {
            actions.push(Action::Resume);
        }
    }
    if let Some(due) = facts.due.filter(|_| due_h > 0.0) {
        actions.extend(due_view::show(ui, due_band, due).map(Action::Due));
    }
    if let Some(review) = facts.review.as_ref().filter(|_| review_h > 0.0) {
        let asked = review_view::show(ui, review_band, &mut state.review, review);
        actions.extend(asked.into_iter().map(Action::Review));
    }
    if let Some(ready) = composing {
        agent_ready::empty_state(ui, body, ready);
        let draft = Draft {
            text: &mut state.reply,
            pictures: &mut state.pictures,
            focus: &mut state.focus_reply,
        };
        if let Some(text) = agent_ready::composer(ui, input, draft, ready) {
            actions.push(Action::Reply(text));
        }
        return actions;
    }
    let quick = quick_for(rows, state, &facts).to_vec();
    let chips = transcript(
        ui,
        body,
        rows,
        state,
        (facts.tab, facts.root, facts.pr),
        facts.review.as_ref(),
        (live_seq, &quick, facts.profile),
        (facts.face, facts.turn == Turn::Running, facts.prompt == Some(Prompt::Permission)),
        &mut actions,
    );
    match chips {
        Some(ChipClick::Send(text)) => {
            state.quick_used = rows.newest_seq();
            actions.push(Action::Reply(text));
        }
        Some(ChipClick::Reply) => state.focus_reply = true,
        None => {}
    }
    let mut reply_enabled = facts.prompt.is_none();
    if let Some(live) = state.live.as_mut() {
        let tidewatch = shown.is_some_and(|row| {
            matches!(&row.kind, Kind::Question(q) if matches!(q.origin, Origin::Tidewatch(_)))
        });
        reply_enabled = tidewatch;
        let grab = state.reply.trim().is_empty();
        live.picks = question_bulk::plan(rows).count();
        live.auto_left = state.auto_left.filter(|(seq, _)| *seq == live.seq).map(|(_, left)| left);
        let dock = Rect::from_min_max(rect.min, pos2(rect.right(), input.top()));
        match question_view::live_card(ui, dock, live, facts.profile, grab) {
            Effect::Send(given) => actions.push(Action::Ask(live.seq, Reply::Answered(given))),
            Effect::Skip => actions.push(Action::Ask(live.seq, Reply::Skipped)),
            Effect::GoWithPicks => actions.push(Action::GoWithPicks),
            Effect::Nothing | Effect::TypeOther => {}
        }
    } else if let Some(prompt) = facts.prompt {
        actions.extend(prompt_strip(
            ui,
            strip,
            prompt,
            facts.source,
            rows,
            (facts.root, &state.zone),
        ));
    }
    let mut input_ui =
        ui.new_child(UiBuilder::new().max_rect(input.shrink2(vec2(INSET, composer::GAP_Y))));
    let draft = Draft {
        text: &mut state.reply,
        pictures: &mut state.pictures,
        focus: &mut state.focus_reply,
    };
    if let Some(text) =
        reply_box(&mut input_ui, draft, reply_enabled, &state.commands, state.ghost.text())
    {
        actions.push(Action::Reply(text));
    }
    actions
}

#[allow(clippy::too_many_arguments)]
fn transcript(
    ui: &mut Ui,
    body: Rect,
    rows: &Rows,
    state: &mut ViewState,
    (tab, root, pr): (u64, Option<&Path>, Option<PrData<'_>>),
    review: Option<&ReviewFacts<'_>>,
    (live_seq, quick, profile): (Option<u64>, &[Quick], &str),
    (face, running, waiting): (Option<&Face>, bool, bool),
    actions: &mut Vec<Action>,
) -> Option<ChipClick> {
    let mut chip_click = None;
    if rows.len() == 0 {
        ui.painter().text(
            body.center(),
            Align2::CENTER_CENTER,
            "Waiting for the agent's first message...",
            FontId::proportional(14.0),
            brand::MUTED,
        );
        return None;
    }
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let scrolled_up =
        ui.input(|i| i.smooth_scroll_delta.y > 0.0) && pointer.is_some_and(|p| body.contains(p));
    let grabbing_bar = ui.input(|i| i.pointer.primary_down())
        && pointer.is_some_and(|p| body.contains(p) && p.x > body.right() - SCROLLBAR_GRAB_W);
    if scrolled_up || grabbing_bar {
        state.scroll.release();
    }
    let width = body.width() - 2.0 * INSET;
    state.heights.for_width(width);
    state.pr_heights.for_width(width);
    let newest = rows.newest_seq().map_or(0, |seq| seq + 1) + rows.queued_len(now_ms()) as u64;
    if newest != state.seen_rows {
        state.seen_rows = newest;
        state.stick_pending = true;
    }
    if (body.height() - state.body_h).abs() > 0.5 {
        state.body_h = body.height();
        state.stick_pending = true;
    }
    let follow = state.scroll.following();
    let scroll_down = follow && std::mem::take(&mut state.stick_pending);
    state.refresh_turns(rows);
    let ViewState { heights, pr_heights, expanded, turns, zone, .. } = state;
    let (_, _, layout) = turns.as_ref()?;
    let scene = Scene { root, profile, face, zone, now_ms: now_ms(), running, waiting };
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body));
    let out = ScrollArea::vertical()
        .id_salt(("reader-scroll", tab))
        .auto_shrink([false, false])
        .show(&mut body_ui, |ui| {
            ui.add_space(8.0);
            if rows.dropped() > 0 {
                let text = format!("{} earlier entries are not shown", rows.dropped());
                ui.horizontal(|ui| {
                    ui.add_space(INSET);
                    ui.label(RichText::new(text).size(12.0).color(brand::MUTED));
                });
            }
            let clip = ui.clip_rect();
            let (list, stamps): (Vec<&Row>, Vec<Option<i64>>) = rows.stamped().unzip();
            let cards = pr.map_or(&[][..], |pr| pr.cards());
            for place in pr_timeline::places(rows, cards) {
                if let Place::Row(index) = place
                    && layout.slots[index] == Slot::Hidden
                {
                    continue;
                }
                let top = ui.next_widget_position().y;
                let (key, height) = match place {
                    Place::Row(index) => match layout.slots[index] {
                        Slot::Group(n) => {
                            let group = &layout.groups[n];
                            (
                                None,
                                heights.height_with(group.key, |w| {
                                    turn_guess(
                                        &list,
                                        group,
                                        (
                                            expanded.contains(&group.key),
                                            expanded.contains(&group.notes_key()),
                                        ),
                                        (running, w),
                                    )
                                }),
                            )
                        }
                        _ => (None, heights.height(list[index])),
                    },
                    Place::Card(index) => {
                        (Some(cards[index].id), pr_heights.height(cards[index].id))
                    }
                };
                if top + height < clip.top() - OVERSCAN || top > clip.bottom() + OVERSCAN {
                    ui.allocate_space(vec2(width, height));
                    continue;
                }
                match (place, pr) {
                    (Place::Row(index), _) => {
                        let row = list[index];
                        let slot = layout.slots[index];
                        let drawn = ui.horizontal(|ui| {
                            ui.add_space(INSET);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                match slot {
                                    Slot::Group(n) => reader_chat_view::turn(
                                        ui,
                                        &scene,
                                        &list,
                                        &layout.groups[n],
                                        expanded,
                                        width,
                                    ),
                                    _ if row.kind == Kind::User
                                        && rows.said_for(row.seq).is_some() =>
                                    {
                                        ui.add_space(6.0);
                                        if let Some(said) = rows.said_for(row.seq) {
                                            pr_cards_view::draw_bubble(
                                                ui,
                                                &reader_sent::bubble_of(said),
                                            );
                                        }
                                        ui.add_space(6.0);
                                    }
                                    _ if row.kind == Kind::User => reader_chat_view::user(
                                        ui,
                                        row,
                                        stamps[index],
                                        scene.zone,
                                        width,
                                    ),
                                    _ => {
                                        let asked = draw_row(
                                            ui,
                                            row,
                                            review,
                                            live_seq,
                                            (
                                                profile,
                                                stamps[index],
                                                scene.zone,
                                                rows.auto_picked(row.seq),
                                            ),
                                        );
                                        actions.extend(asked.into_iter().map(Action::Review));
                                    }
                                }
                            })
                        });
                        let key = match slot {
                            Slot::Group(n) => layout.groups[n].key,
                            _ => row.seq,
                        };
                        heights.record(key, drawn.response.rect.height());
                    }
                    (Place::Card(index), Some(pr)) => {
                        let drawn = ui.horizontal(|ui| {
                            ui.add_space(INSET);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                ui.add_space(6.0);
                                let clicked = pr_cards_view::card(ui, &pr, index);
                                ui.add_space(6.0);
                                clicked
                            })
                        });
                        if let Some(id) = key {
                            pr_heights.record(id, drawn.response.rect.height());
                        }
                        actions.extend(drawn.inner.inner.map(Action::Pr));
                    }
                    (Place::Card(_), None) => {}
                }
            }
            let time = ui.input(|i| i.time);
            for (n, queued) in rows.queued(now_ms()).enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(INSET);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        reader_chat_view::queued(
                            ui,
                            &queued.text,
                            u64::MAX - n as u64,
                            time,
                            width,
                        );
                    });
                });
                ui.ctx().request_repaint_after(QUEUED_FRAME);
            }
            if running && layout.awaiting {
                ui.horizontal(|ui| {
                    ui.add_space(INSET);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        reader_chat_view::pending(ui, &scene, width);
                    });
                });
            }
            if let Some(pr) = pr {
                let tail = ui.horizontal(|ui| {
                    ui.add_space(INSET);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        pr_cards_view::tail(ui, &pr)
                    })
                });
                actions.extend(tail.inner.inner.map(Action::Pr));
            }
            if !quick.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(INSET);
                    ui.vertical(|ui| chip_click = question_view::quick_chips(ui, quick));
                });
            }
            ui.add_space(12.0);
            if scroll_down {
                ui.scroll_to_cursor_animation(Some(Align::BOTTOM), ScrollAnimation::none());
            }
        });
    let max_offset = (out.content_size.y - out.inner_rect.height()).max(0.0);
    state.scroll.settle(out.state.offset.y, max_offset);
    #[cfg(test)]
    {
        state.settled_at = (out.state.offset.y, max_offset);
    }
    if (out.content_size.y - state.content_h).abs() > 0.5 {
        state.content_h = out.content_size.y;
        state.stick_pending = true;
        ui.ctx().request_repaint();
    }
    if !state.scroll.following() {
        let slot = Rect::from_center_size(
            pos2(body.center().x, body.bottom() - JUMP_H),
            vec2(JUMP_W, JUMP_H),
        );
        if reader_chat_view::jump_pill(ui, slot, "Jump to latest").clicked() {
            state.scroll.jump_to_latest();
            state.stick_pending = true;
            ui.ctx().request_repaint();
        }
    }
    chip_click
}

fn draw_row(
    ui: &mut Ui,
    row: &Row,
    review: Option<&ReviewFacts<'_>>,
    live_seq: Option<u64>,
    (profile, stamp, zone, auto): (&str, Option<i64>, &TimeZone, bool),
) -> Vec<ReviewAction> {
    match &row.kind {
        Kind::Review(ids) => return review_row_view::show(ui, ids, &row.text, review),
        Kind::Question(question) => {
            if live_seq != Some(row.seq) {
                let time = stamp.and_then(|at| clock_label(at, zone));
                question_view::summary(ui, question, profile, time.as_deref(), auto);
            }
        }
        Kind::User | Kind::Assistant | Kind::Tool(_) => {}
        Kind::Divider => draw_divider(ui, &row.text),
        Kind::Compacted => {
            let time = stamp.and_then(|at| clock_label(at, zone));
            draw_compaction(ui, time.as_deref());
        }
        Kind::SlashCommand => {
            let time = stamp.and_then(|at| clock_label(at, zone));
            reader_chat_view::slash_command(ui, &row.text, time.as_deref());
        }
        Kind::CommandOutput => reader_chat_view::command_output(ui, &row.text),
        Kind::Notice => {
            Frame::new()
                .fill(brand::RED_FILL)
                .stroke(Stroke::new(1.0, brand::RED_LINE))
                .corner_radius(CornerRadius::same(CARD_RADIUS))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&row.text).color(brand::RED));
                });
        }
    }
    Vec::new()
}

fn turn_guess(
    list: &[&Row],
    group: &reader_turns::Group,
    (listing, notes_listing): (bool, bool),
    (running, width): (bool, f32),
) -> f32 {
    let text_of = |index: &usize| list[*index].text.as_str();
    let answer: Vec<&str> = group.answer().iter().map(text_of).collect();
    let notes: Vec<&str> = group.notes().iter().map(text_of).collect();
    let live = running && !group.closed;
    reader_scroll::estimate_turn(
        &answer,
        group.calls.len(),
        (live, listing),
        (&notes, notes_listing),
        width,
    )
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(0))
}

fn draw_compaction(ui: &mut Ui, time: Option<&str>) {
    ui.add_space(10.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let painter = ui.painter();
    let text = match time {
        Some(time) => format!("Earlier messages summarised to free up room \u{b7} {time}"),
        None => "Earlier messages summarised to free up room".to_owned(),
    };
    let galley = painter.layout_no_wrap(text, body(12.0), brand::MUTED);
    let width = 13.0 + 8.0 + galley.size().x;
    let left = rect.center().x - width / 2.0;
    let mid = rect.center().y;
    let line = Stroke::new(1.0, brand::ITEM_LINE);
    painter.hline(rect.left()..=(left - 14.0).max(rect.left()), mid, line);
    painter.hline((left + width + 14.0).min(rect.right())..=rect.right(), mid, line);
    let icon = Stroke::new(1.3, brand::MUTED);
    for (y, from, to) in [(-4.5, 0.0, 13.0), (0.0, 2.0, 11.0), (4.5, 4.0, 9.0)] {
        painter.line_segment([pos2(left + from, mid + y), pos2(left + to, mid + y)], icon);
    }
    painter.galley(pos2(left + 21.0, mid - galley.size().y / 2.0), galley, brand::MUTED);
    ui.add_space(10.0);
}

fn draw_divider(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(divider_label(text), body(11.5), brand::MUTED);
    let half = galley.size().x / 2.0 + 10.0;
    let line = Stroke::new(1.0, brand::ITEM_LINE);
    painter.hline(rect.left()..=(rect.center().x - half).max(rect.left()), rect.center().y, line);
    painter.hline((rect.center().x + half).min(rect.right())..=rect.right(), rect.center().y, line);
    painter.galley(rect.center() - galley.size() / 2.0, galley, brand::MUTED);
    ui.add_space(6.0);
}

/// A stage divider's text as a quiet label: `Stage 2 \u{b7} Review \u{b7} gemini \u{b7} Read only`.
pub(crate) fn divider_label(text: &str) -> String {
    let text = text.trim_matches(|c: char| c == '-' || c.is_whitespace());
    text.replacen(": ", " \u{b7} ", 1)
        .replacen(" (", " \u{b7} ", 1)
        .replace(", ", " \u{b7} ")
        .trim_end_matches(')')
        .to_owned()
}

/// A tool row's label split for drawing: the tool, then the folder and leaf of its path.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ToolLine<'a> {
    pub(crate) name: &'a str,
    pub(crate) dir: String,
    pub(crate) leaf: String,
}

/// Splits a `Name  argument` label, with the worktree's own folder taken off every path.
pub(crate) fn split_label<'a>(label: &'a str, root: Option<&Path>) -> ToolLine<'a> {
    let (name, argument) = label.split_once("  ").unwrap_or((label, ""));
    let argument = root.and_then(Path::to_str).map_or_else(
        || argument.to_owned(),
        |root| {
            root_forms(root).iter().fold(argument.to_owned(), |text, root| {
                if text == *root { ".".to_owned() } else { text.replace(&format!("{root}/"), "") }
            })
        },
    );
    let is_path = argument.contains('/') && !argument.contains(char::is_whitespace);
    match argument.rfind('/').filter(|_| is_path) {
        Some(at) => {
            ToolLine { name, dir: argument[..=at].to_owned(), leaf: argument[at + 1..].to_owned() }
        }
        None => ToolLine { name, dir: String::new(), leaf: argument },
    }
}

/// A folder as written with and without macOS's `/private` in front, which `/tmp` and `/var` hide.
fn root_forms(root: &str) -> Vec<String> {
    let root = root.trim_end_matches('/');
    let other = match root.strip_prefix("/private") {
        Some(rest) if rest.starts_with('/') => rest.to_owned(),
        _ => format!("/private{root}"),
    };
    let mut forms = vec![root.to_owned(), other];
    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
    forms
}

/// The first lines of a tool's output, with a count of what was cut.
pub(crate) fn output_preview(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= OUTPUT_MAX_LINES {
        return output.to_owned();
    }
    let text = lines[..OUTPUT_MAX_LINES].join("\n");
    let hidden = lines.len() - OUTPUT_MAX_LINES;
    format!("{text}\n... {hidden} more lines")
}

fn prompt_strip(
    ui: &mut Ui,
    strip: Rect,
    prompt: Prompt,
    source: Option<Source>,
    rows: &Rows,
    (root, zone): (Option<&Path>, &TimeZone),
) -> Vec<Action> {
    let mut actions = Vec::new();
    if prompt == Prompt::Permission {
        let answers: Vec<Answer> =
            reader_keys::answers(source).iter().map(|(answer, _)| *answer).collect();
        let tool = permission_tool(rows, root);
        let tool = tool.as_ref().map(|(name, command)| (name.as_str(), command.as_str()));
        let strip = strip.shrink2(vec2(0.0, 4.0));
        let time = rows.stamped().last().and_then(|(_, at)| clock_label(at?, zone));
        match question_view::permission_card(ui, strip, tool, time.as_deref(), &answers) {
            Some(PermissionClick::Answer(answer)) => actions.push(Action::Answer(answer)),
            Some(PermissionClick::SwitchToTerminal) => actions.push(Action::SwitchToTerminal),
            None => {}
        }
        return actions;
    }
    let card = strip.shrink2(vec2(INSET, 4.0));
    paint_item(ui.painter(), card, false);
    paint_bar(ui.painter(), card, brand::ORANGE);
    let mut buttons = ui.new_child(
        UiBuilder::new()
            .max_rect(card.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    buttons.spacing_mut().item_spacing.x = 6.0;
    let answers = match prompt {
        Prompt::Permission => reader_keys::answers(source),
        Prompt::Question => &[],
    };
    if quiet_button(&mut buttons, "Switch to terminal", true, Size::Compact).clicked() {
        actions.push(Action::SwitchToTerminal);
    }
    for (answer, _) in answers.iter().rev() {
        let clicked = if *answer == Answer::Allow {
            primary_button(&mut buttons, answer.label(), true, Size::Compact)
        } else {
            quiet_button(&mut buttons, answer.label(), true, Size::Compact)
        }
        .clicked();
        if clicked {
            actions.push(Action::Answer(*answer));
        }
    }
    let left = card.left() + 16.0;
    let galley = ellipsized(
        ui.painter(),
        prompt_text(prompt, !answers.is_empty()),
        body(13.0),
        brand::TEXT,
        buttons.min_rect().left() - 12.0 - left,
    );
    ui.painter().galley(pos2(left, card.center().y - galley.size().y / 2.0), galley, brand::TEXT);
    actions
}

/// What the strip above the reply box says. A reply typed into a menu would pick its default
/// option on Enter, so the reply box is off while a prompt waits.
pub(crate) fn prompt_text(prompt: Prompt, has_buttons: bool) -> &'static str {
    match (prompt, has_buttons) {
        (Prompt::Permission, true) => "The agent is asking for permission",
        (Prompt::Permission, false) => {
            "The agent is asking for permission. Answer it in the terminal."
        }
        (Prompt::Question, _) => "The agent asked a question. Answer it in the terminal.",
    }
}

fn reply_id() -> Id {
    Id::new("reply-box")
}

pub(crate) fn reply_has_focus(ctx: &egui::Context) -> bool {
    ctx.memory(|memory| memory.has_focus(reply_id()))
}

/// The reply box: the shared composer with the Reader's hint.
pub(crate) fn reply_box(
    ui: &mut Ui,
    draft: Draft<'_>,
    enabled: bool,
    commands: &[SlashEntry],
    ghost: Option<&str>,
) -> Option<String> {
    let hint = if enabled { "Reply to the agent" } else { "Answer the prompt above first" };
    composer::show(ui, draft, &Spec { id: reply_id(), hint, enabled, commands, ghost })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bullets;
    use egui::{Event, Key, Modifiers, RawInput, TextEdit};

    const INPUT_H: f32 = composer::BOX_H + 2.0 * composer::GAP_Y;
    const REPLY_GAP_Y: f32 = composer::GAP_Y;
    const SEND: &str = "Send";

    fn reply_box(
        ui: &mut Ui,
        text: &mut String,
        focus: &mut bool,
        enabled: bool,
        commands: &[SlashEntry],
        ghost: Option<&str>,
    ) -> Option<String> {
        let draft = Draft { text, pictures: &mut Vec::new(), focus };
        super::reply_box(ui, draft, enabled, commands, ghost)
    }

    fn frame(
        ctx: &egui::Context,
        events: Vec<Event>,
        text: &mut String,
        focus: &mut bool,
    ) -> Option<String> {
        frame_with(ctx, events, text, focus, &[])
    }

    fn frame_with(
        ctx: &egui::Context,
        events: Vec<Event>,
        text: &mut String,
        focus: &mut bool,
        commands: &[SlashEntry],
    ) -> Option<String> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut sent = None;
        let _ = ctx.run_ui(raw, |ui| {
            sent = reply_box(ui, text, focus, true, commands, None);
        });
        sent
    }

    fn enter() -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn enter_in_the_reply_box_sends_the_text_and_clears_it() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        assert_eq!(frame(&ctx, vec![], &mut text, &mut focus), None);
        assert_eq!(
            frame(&ctx, vec![Event::Text("run the tests".into())], &mut text, &mut focus),
            None
        );
        assert_eq!(text, "run the tests");
        let sent = frame(&ctx, vec![enter()], &mut text, &mut focus);
        assert_eq!(sent.as_deref(), Some("run the tests"));
        assert_eq!(text, "");
    }

    fn flat(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(inner) => inner.iter().for_each(|shape| flat(shape, out)),
            other => out.push(other.clone()),
        }
    }

    struct Painted {
        boxed: Rect,
        text: Rect,
        send: Rect,
    }

    fn painted_reply(text: &mut String, height: f32) -> Painted {
        let ctx = egui::Context::default();
        let mut focus = true;
        let mut shapes = Vec::new();
        for events in [vec![], vec![]] {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                let room = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, height));
                ui.scope_builder(UiBuilder::new().max_rect(room), |ui| {
                    reply_box(ui, text, &mut focus, true, &[], None);
                });
            });
            shapes.clear();
            out.shapes.iter().for_each(|clipped| flat(&clipped.shape, &mut shapes));
        }
        let rect_of = |wanted: &dyn Fn(&egui::Shape) -> Option<Rect>| {
            shapes.iter().find_map(wanted).expect("the shape was painted")
        };
        let text_rect = |shown: &str| {
            rect_of(&|shape| match shape {
                egui::Shape::Text(t) if t.galley.text() == shown => {
                    Some(t.galley.rows[0].rect().translate(t.pos.to_vec2()))
                }
                _ => None,
            })
        };
        Painted {
            boxed: rect_of(&|shape| match shape {
                egui::Shape::Rect(r) if r.fill == brand::CARD => Some(r.rect),
                _ => None,
            }),
            text: text_rect(&bullets_drawn(text)),
            send: text_rect(SEND),
        }
    }

    fn bullets_drawn(text: &str) -> String {
        text.lines()
            .map(|line| match bullets::marker_of(line) {
                Some(_) => format!("\u{2022}{}", &line[1..]),
                None => line.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn one_line_of_text_is_centred_in_the_box_like_the_send_button() {
        let mut text = "/login".to_owned();
        let painted = painted_reply(&mut text, INPUT_H);
        let middle = painted.boxed.center().y;
        assert!(
            (painted.text.center().y - middle).abs() < 0.6,
            "text {painted:?}",
            painted = (painted.text, painted.boxed)
        );
        assert!((painted.send.center().y - middle).abs() < 0.6, "send");
    }

    #[test]
    fn one_line_is_centred_in_the_row_the_reader_gives_the_box() {
        let mut text = "/login".to_owned();
        let painted = painted_reply(&mut text, INPUT_H - 2.0 * REPLY_GAP_Y);
        let middle = painted.boxed.center().y;
        assert!((painted.text.center().y - middle).abs() < 0.6);
        assert!((painted.send.center().y - middle).abs() < 0.6);
    }

    #[test]
    fn several_lines_keep_growing_from_the_top() {
        let mut text = "one\ntwo\nthree".to_owned();
        let painted = painted_reply(&mut text, INPUT_H + 40.0);
        let first_row_gap = painted.text.top() - painted.boxed.top();
        assert!(first_row_gap < 16.0, "the first row sits at the top, not at {first_row_gap}");
    }

    fn typed_into(ctx: &egui::Context, typed: &str) -> (String, bool) {
        let (mut text, mut focus) = (String::new(), true);
        frame(ctx, vec![], &mut text, &mut focus);
        frame(ctx, vec![Event::Text(typed.into())], &mut text, &mut focus);
        (text, focus)
    }

    #[test]
    fn enter_on_a_bullet_line_continues_the_list_and_sends_nothing() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = typed_into(&ctx, "- one");
        assert_eq!(frame(&ctx, vec![enter()], &mut text, &mut focus), None);
        assert_eq!(text, "- one\n- ");
        frame(&ctx, vec![Event::Text("two".into())], &mut text, &mut focus);
        assert_eq!(text, "- one\n- two");
    }

    #[test]
    fn enter_on_an_empty_bullet_ends_the_list_and_the_next_enter_sends_what_was_typed() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = typed_into(&ctx, "- one");
        frame(&ctx, vec![enter()], &mut text, &mut focus);
        assert_eq!(frame(&ctx, vec![enter()], &mut text, &mut focus), None);
        assert_eq!(text, "- one\n");
        let sent = frame(&ctx, vec![enter()], &mut text, &mut focus);
        assert_eq!(sent.as_deref(), Some("- one\n"));
        assert_eq!(text, "");
    }

    #[test]
    fn shift_enter_on_a_bullet_line_is_a_plain_new_line() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = typed_into(&ctx, "* one");
        assert_eq!(frame(&ctx, vec![shift_enter()], &mut text, &mut focus), None);
        assert_eq!(text, "* one\n");
    }

    #[test]
    fn the_text_sent_with_the_send_button_is_the_typed_markers_untouched() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = typed_into(&ctx, "* a\n- b\nplain");
        let sent = frame(&ctx, vec![enter()], &mut text, &mut focus);
        assert_eq!(sent.as_deref(), Some("* a\n- b\nplain"));
    }

    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn slash_box(typed: &str) -> (egui::Context, String, bool) {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        let commands = crate::slash::builtins();
        frame_with(&ctx, vec![], &mut text, &mut focus, &commands);
        frame_with(&ctx, vec![Event::Text(typed.into())], &mut text, &mut focus, &commands);
        frame_with(&ctx, vec![], &mut text, &mut focus, &commands);
        (ctx, text, focus)
    }

    fn press(ctx: &egui::Context, text: &mut String, keys: &[Key]) -> Option<String> {
        let commands = crate::slash::builtins();
        let mut focus = false;
        let mut sent = None;
        for pressed in keys {
            sent = frame_with(ctx, vec![key(*pressed)], text, &mut focus, &commands);
        }
        sent
    }

    #[test]
    fn tab_completes_the_picked_command_and_down_moves_the_pick() {
        let (ctx, mut text, _) = slash_box("/lo");
        assert_eq!(press(&ctx, &mut text, &[Key::Tab]), None);
        assert_eq!(text, "/login ");
        let (ctx, mut text, _) = slash_box("/lo");
        press(&ctx, &mut text, &[Key::ArrowDown, Key::Tab]);
        assert_eq!(text, "/logout ");
    }

    #[test]
    fn enter_completes_part_of_a_command_name_but_sends_a_whole_one() {
        let (ctx, mut text, _) = slash_box("/lo");
        assert_eq!(press(&ctx, &mut text, &[Key::Enter]), None);
        assert_eq!(text, "/login ");
        let (ctx, mut text, _) = slash_box("/login");
        assert_eq!(press(&ctx, &mut text, &[Key::Enter]).as_deref(), Some("/login"));
    }

    #[test]
    fn esc_closes_the_list_and_keeps_the_text_and_the_keyboard() {
        let (ctx, mut text, _) = slash_box("/lo");
        press(&ctx, &mut text, &[Key::Escape]);
        assert_eq!(text, "/lo");
        assert!(ctx.memory(|memory| memory.has_focus(reply_id())));
        assert_eq!(press(&ctx, &mut text, &[Key::Tab]), None);
        assert_eq!(text, "/lo");
    }

    #[test]
    fn a_box_that_does_not_start_with_a_slash_has_no_list_and_keeps_its_keys() {
        let (ctx, mut text, _) = slash_box("fix /lo");
        assert_eq!(press(&ctx, &mut text, &[Key::Enter]).as_deref(), Some("fix /lo"));
    }

    #[test]
    fn enter_on_an_empty_or_blank_reply_sends_nothing() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        frame(&ctx, vec![], &mut text, &mut focus);
        frame(&ctx, vec![Event::Text("   ".into())], &mut text, &mut focus);
        assert_eq!(frame(&ctx, vec![enter()], &mut text, &mut focus), None);
    }

    const GHOST: &str = "run the whole test suite";

    fn ghost_frame(
        ctx: &egui::Context,
        events: Vec<Event>,
        text: &mut String,
        focus: &mut bool,
        ghost: Option<&str>,
    ) -> (Option<String>, Vec<(String, Color32)>) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut sent = None;
        let out = ctx.run_ui(raw, |ui| {
            sent = reply_box(ui, text, focus, true, &[], ghost);
        });
        let mut shapes = Vec::new();
        out.shapes.iter().for_each(|clipped| flat(&clipped.shape, &mut shapes));
        let painted = shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.job.sections.first().map_or(Color32::TRANSPARENT, |s| s.format.color),
                )),
                _ => None,
            })
            .collect();
        (sent, painted)
    }

    fn ghost_box() -> (egui::Context, String, bool) {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        ghost_frame(&ctx, vec![], &mut text, &mut focus, Some(GHOST));
        ghost_frame(&ctx, vec![], &mut text, &mut focus, Some(GHOST));
        (ctx, text, focus)
    }

    #[test]
    fn the_suggestion_shows_grey_in_an_empty_box_and_not_in_a_typed_one() {
        let (ctx, mut text, mut focus) = ghost_box();
        let (_, painted) = ghost_frame(&ctx, vec![], &mut text, &mut focus, Some(GHOST));
        assert_eq!(
            painted.iter().find(|(shown, _)| shown == GHOST).map(|(_, ink)| *ink),
            Some(brand::MUTED)
        );
        let (_, none) = ghost_frame(&ctx, vec![], &mut text, &mut focus, None);
        assert_eq!(none.iter().filter(|(shown, _)| shown == GHOST).count(), 0);
        ghost_frame(&ctx, vec![Event::Text("x".into())], &mut text, &mut focus, Some(GHOST));
        let (_, typed) = ghost_frame(&ctx, vec![], &mut text, &mut focus, Some(GHOST));
        assert_eq!(text, "x");
        assert_eq!(typed.iter().filter(|(shown, _)| shown == GHOST).count(), 0);
    }

    #[test]
    fn tab_turns_the_suggestion_into_real_text_and_sends_nothing() {
        let (ctx, mut text, mut focus) = ghost_box();
        let (sent, _) = ghost_frame(&ctx, vec![key(Key::Tab)], &mut text, &mut focus, Some(GHOST));
        assert_eq!((sent, text.as_str()), (None, GHOST));
        let (_, painted) = ghost_frame(&ctx, vec![], &mut text, &mut focus, Some(GHOST));
        assert_eq!(painted.iter().filter(|(shown, _)| shown == GHOST).count(), 1);
        let at = TextEdit::load_state(&ctx, reply_id()).and_then(|s| s.cursor.char_range());
        assert_eq!(at.map(|range| range.primary.index.0), Some(GHOST.chars().count()));
    }

    #[test]
    fn typing_then_tab_keeps_the_typed_text() {
        let (ctx, mut text, mut focus) = ghost_box();
        ghost_frame(&ctx, vec![Event::Text("go".into())], &mut text, &mut focus, Some(GHOST));
        ghost_frame(&ctx, vec![key(Key::Tab)], &mut text, &mut focus, Some(GHOST));
        assert_eq!(text, "go");
    }

    #[test]
    fn enter_on_an_empty_box_never_sends_the_suggestion() {
        let (ctx, mut text, mut focus) = ghost_box();
        let (sent, _) = ghost_frame(&ctx, vec![enter()], &mut text, &mut focus, Some(GHOST));
        assert_eq!((sent, text.as_str()), (None, ""));
    }

    #[test]
    fn tab_without_a_suggestion_moves_focus_as_before() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        ghost_frame(&ctx, vec![], &mut text, &mut focus, None);
        ghost_frame(&ctx, vec![], &mut text, &mut focus, None);
        ghost_frame(&ctx, vec![key(Key::Tab)], &mut text, &mut focus, None);
        assert_eq!(text, "");
    }

    #[test]
    fn focus_is_asked_for_once_and_again_after_a_send() {
        let ctx = egui::Context::default();
        let (mut text, mut focus) = (String::new(), true);
        frame(&ctx, vec![], &mut text, &mut focus);
        assert!(!focus);
        frame(&ctx, vec![Event::Text("hi".into())], &mut text, &mut focus);
        frame(&ctx, vec![enter()], &mut text, &mut focus);
        assert!(focus);
    }

    fn line(name: &'static str, dir: &str, leaf: &str) -> ToolLine<'static> {
        ToolLine { name, dir: dir.into(), leaf: leaf.into() }
    }

    #[test]
    fn a_tool_path_is_shown_from_the_worktree_with_its_folder_split_off() {
        let root = Path::new("/h/.tidewatch/acme/upbeat-harbor");
        let label = "Edit  /h/.tidewatch/acme/upbeat-harbor/src/auth.rs";
        assert_eq!(split_label(label, Some(root)), line("Edit", "src/", "auth.rs"));
        assert_eq!(split_label(label, None).dir, "/h/.tidewatch/acme/upbeat-harbor/src/");
        assert_eq!(split_label("Read  src/a/b.rs", Some(root)), line("Read", "src/a/", "b.rs"));
        assert_eq!(split_label("Read  Cargo.toml", Some(root)), line("Read", "", "Cargo.toml"));
    }

    #[test]
    fn a_worktree_is_found_with_or_without_the_private_prefix() {
        for (root, label) in [
            ("/tmp/w/tree", "Grep  /private/tmp/w/tree"),
            ("/private/tmp/w/tree", "Grep  /tmp/w/tree"),
            ("/private/tmp/w/tree", "Grep  /private/tmp/w/tree"),
        ] {
            assert_eq!(split_label(label, Some(Path::new(root))), line("Grep", "", "."), "{root}");
        }
        let root = Some(Path::new("/tmp/w/tree"));
        assert_eq!(
            split_label("Edit  /private/tmp/w/tree/src/a.rs", root),
            line("Edit", "src/", "a.rs")
        );
        assert_eq!(
            split_label("Edit  /private/tmp/w/other/a.rs", root).dir,
            "/private/tmp/w/other/"
        );
    }

    #[test]
    fn a_stage_divider_reads_as_a_quiet_label() {
        assert_eq!(
            divider_label("--- Stage 1: Work (claude opus, Auto edit) ---"),
            "Stage 1 \u{b7} Work \u{b7} claude opus \u{b7} Auto edit"
        );
        assert_eq!(
            divider_label("--- Stage 2: Review (gemini, Read only, round 2) ---"),
            "Stage 2 \u{b7} Review \u{b7} gemini \u{b7} Read only \u{b7} round 2"
        );
        assert_eq!(divider_label("Stage 3"), "Stage 3");
    }

    #[test]
    fn a_command_stays_whole_and_loses_only_the_worktree_prefix() {
        let root = Path::new("/w/tree");
        assert_eq!(split_label("Bash  git status", Some(root)), line("Bash", "", "git status"));
        assert_eq!(
            split_label("Bash  cat /w/tree/src/a.rs | wc", Some(root)),
            line("Bash", "", "cat src/a.rs | wc")
        );
        assert_eq!(split_label("TodoWrite", Some(root)), line("TodoWrite", "", ""));
    }

    #[test]
    fn long_output_is_cut_with_a_count() {
        let output = (0..60).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let preview = output_preview(&output);
        assert_eq!(preview.lines().count(), OUTPUT_MAX_LINES + 1);
        assert!(preview.ends_with("... 20 more lines"));
        assert_eq!(output_preview("a\nb"), "a\nb");
    }

    fn repaint_delays(rows: &Rows, frames: usize) -> Vec<std::time::Duration> {
        let ctx = crate::fonts::test_context();
        let mut state = ViewState::default();
        let mut delays = Vec::new();
        for frame in 0..frames {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0))),
                focused: true,
                time: Some(frame as f64 * 0.016),
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                show(
                    ui,
                    rect,
                    rows,
                    &mut state,
                    Facts {
                        tab: 1,
                        source: None,
                        prompt: None,
                        ready: None,
                        root: None,
                        review: None,
                        pr: None,
                        due: None,
                        profile: "Builder",
                        face: None,
                        turn: Turn::Finished,
                        stopped: false,
                    },
                );
            });
            delays.extend(out.viewport_output.values().map(|v| v.repaint_delay));
        }
        delays
    }

    #[test]
    fn a_settled_reader_asks_for_no_more_frames_than_the_cursor_blink() {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        for n in 0..60 {
            let text = format!("# Title {n}\n\n- a\n- b\n\n```rust\nfn x() {{}}\n```\n\nwords");
            rows.push(Entry { role: Role::Assistant, text, tool: None, ts: None });
        }
        let delays = repaint_delays(&rows, 24);
        let busy = delays.iter().rev().take(8).filter(|d| d.is_zero()).count();
        assert_eq!(busy, 0, "{delays:?}");
    }

    #[test]
    fn the_reader_view_draws_every_kind_of_row_headless() {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        rows.push_all(vec![
            Entry { role: Role::User, text: "hi".into(), tool: None, ts: None },
            Entry {
                role: Role::Assistant,
                text: "# Title\n\n- a\n- b\n\n```rust\nfn x() {}\n```".into(),
                tool: None,
                ts: None,
            },
            Entry { role: Role::System, text: "boom".into(), tool: None, ts: None },
        ]);
        let ctx = crate::fonts::test_context();
        let mut state = ViewState::default();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0))),
            focused: true,
            ..RawInput::default()
        };
        for _ in 0..2 {
            let _ = ctx.run_ui(raw.clone(), |ui| {
                let rect = ui.max_rect();
                let facts = Facts {
                    tab: 1,
                    source: Some(Source::Claude),
                    prompt: Some(Prompt::Permission),
                    ready: None,
                    root: None,
                    review: None,
                    pr: None,
                    due: None,
                    profile: "Builder",
                    face: None,
                    turn: Turn::Running,
                    stopped: false,
                };
                let actions = show(ui, rect, &rows, &mut state, facts);
                assert_eq!(actions, Vec::<Action>::new());
            });
        }
        assert_eq!(state.heights.measured_count(), 3);
    }

    fn cmd_enter() -> egui::Event {
        egui::Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }
    }

    #[test]
    fn the_review_card_sits_above_the_reply_box_and_the_composer_and_command_enter_sends_it() {
        use kelp_core::review::{Anchor, Comment, Side};
        use tidewatch_core::notes::Notes;
        use tidewatch_core::transcript::{Entry, Role};

        let mut notes = Notes::default();
        let anchor = Anchor {
            side: Side::New,
            line_hint: 4,
            text: "x".into(),
            before: vec![],
            after: vec![],
            span: 0,
            block: vec![],
        };
        notes.add("a.rs", None, anchor, Comment { author: "v".into(), time: 1, body: "n".into() });
        let draft = crate::review::Draft::of(&notes, "claude", "v");
        let mut populated = Rows::default();
        populated.push(Entry { role: Role::User, text: "hi".into(), tool: None, ts: None });
        let cases = [
            (&populated, None),
            (&Rows::default(), Some(Ready { profile: "Builder", agent_id: "claude" })),
        ];
        for (rows, ready) in cases {
            let ctx = egui::Context::default();
            crate::fonts::install(&ctx);
            let mut state = ViewState::default();
            state.review.message = "go".into();
            let mut run = |events: Vec<egui::Event>| {
                let raw = RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
                    events,
                    focused: true,
                    ..RawInput::default()
                };
                let mut actions = Vec::new();
                let _ = ctx.run_ui(raw, |ui| {
                    let review = ReviewFacts {
                        draft: &draft,
                        send: crate::review::SendState::Ready,
                        notes: &notes,
                        agent_id: "claude",
                        profile: "Builder",
                        working: false,
                        keys_free: true,
                    };
                    let facts = Facts {
                        tab: 1,
                        source: None,
                        prompt: None,
                        ready,
                        root: None,
                        review: Some(review),
                        pr: None,
                        due: None,
                        profile: "Builder",
                        face: None,
                        turn: Turn::Finished,
                        stopped: false,
                    };
                    let rect = ui.max_rect();
                    actions = show(ui, rect, rows, &mut state, facts);
                });
                actions
            };
            run(vec![]);
            run(vec![]);
            let sent = run(vec![cmd_enter()]);
            assert_eq!(sent, vec![Action::Review(ReviewAction::Send("go".into()))]);
            assert_eq!(run(vec![]), vec![]);
        }
    }

    #[test]
    fn the_due_card_sits_above_the_reply_box_and_its_buttons_reach_the_caller() {
        use crate::due::Due;
        use tidewatch_core::progress::Mark;
        use tidewatch_core::transcript::{Entry, Role};

        fn texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => out.push((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }

        let due = Due {
            what: "Switch off the old webhook".into(),
            days: 0,
            steps: vec![(Mark::Todo, "Merge the switch".into())],
        };
        let mut rows = Rows::default();
        rows.push(Entry { role: Role::User, text: "hi".into(), tool: None, ts: None });
        let ctx = crate::fonts::test_context();
        let mut state = ViewState::default();
        let mut run = |events: Vec<egui::Event>| {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 700.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let mut actions = Vec::new();
            let output = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                let mut facts = facts(None, Turn::Finished);
                facts.due =
                    Some(DueFacts { due: &due, more: 0, profile: "Builder", blocked: None });
                actions = show(ui, rect, &rows, &mut state, facts);
            });
            let mut found = Vec::new();
            output.shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut found));
            (actions, found)
        };
        run(vec![]);
        let (_, found) = run(vec![]);
        let at = |text: &str| found.iter().find(|(drawn, _)| drawn == text).unwrap().1.center();
        assert!(at("Ask Builder to start").y < at("Reply to the agent").y);
        let ask = at("Ask Builder to start");
        let button = |pressed| egui::Event::PointerButton {
            pos: ask,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(vec![egui::Event::PointerMoved(ask)]);
        run(vec![button(true)]);
        let (actions, _) = run(vec![button(false)]);
        let [Action::Due(crate::due_view::Action::Ask(text))] = actions.as_slice() else {
            panic!("{actions:?}")
        };
        assert!(text.starts_with("Due today: Switch off the old webhook."));
    }

    #[test]
    fn a_stopped_agent_gets_a_bar_over_the_old_conversation_and_resume_reaches_the_caller() {
        use tidewatch_core::transcript::{Entry, Role};

        fn texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => out.push((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }

        let mut rows = Rows::default();
        rows.push(Entry { role: Role::User, text: "old question".into(), tool: None, ts: None });
        let ctx = crate::fonts::test_context();
        let mut state = ViewState::default();
        let mut run = |stopped: bool, events: Vec<egui::Event>| {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 700.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let mut actions = Vec::new();
            let output = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                let mut facts = facts(None, Turn::Finished);
                facts.stopped = stopped;
                actions = show(ui, rect, &rows, &mut state, facts);
            });
            let mut found = Vec::new();
            output.shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut found));
            (actions, found)
        };
        run(false, vec![]);
        let (_, running) = run(false, vec![]);
        assert!(!running.iter().any(|(text, _)| text == "Agent is not running"));
        run(true, vec![]);
        let (_, found) = run(true, vec![]);
        let at = |text: &str| found.iter().find(|(drawn, _)| drawn == text).unwrap().1;
        assert!(at("Agent is not running").bottom() < at("old question").top());
        let resume = at("Resume").center();
        let button = |pressed| egui::Event::PointerButton {
            pos: resume,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(true, vec![egui::Event::PointerMoved(resume)]);
        run(true, vec![button(true)]);
        let (actions, _) = run(true, vec![button(false)]);
        assert_eq!(actions, vec![Action::Resume]);
    }

    #[test]
    fn a_pasted_review_is_drawn_as_a_card_and_its_raw_text_never_reaches_the_screen() {
        use kelp_core::review::{Anchor, Comment, Side};
        use tidewatch_core::notes::Notes;
        use tidewatch_core::transcript::{Entry, Role};

        let mut notes = Notes::default();
        let anchor = Anchor {
            side: Side::New,
            line_hint: 4,
            text: "x".into(),
            before: vec![],
            after: vec![],
            span: 0,
            block: vec![],
        };
        let id = notes.add(
            "src/a.rs",
            None,
            anchor,
            Comment { author: "v".into(), time: 1, body: "rename it".into() },
        );
        notes.mark_sent(&[id], 5);
        let pasted = tidewatch_core::note_paste::format_review(&[&notes.threads[0]], "Start here");
        let mut rows = Rows::default();
        rows.push(Entry { role: Role::User, text: pasted, tool: None, ts: None });
        let draft = crate::review::Draft::default();
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut state = ViewState::default();
        let mut shapes = String::new();
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0))),
                focused: true,
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                let review = ReviewFacts {
                    draft: &draft,
                    send: crate::review::SendState::Ready,
                    notes: &notes,
                    agent_id: "claude",
                    profile: "Builder",
                    working: false,
                    keys_free: true,
                };
                let facts = Facts {
                    tab: 1,
                    source: None,
                    prompt: None,
                    ready: None,
                    root: None,
                    review: Some(review),
                    pr: None,
                    due: None,
                    profile: "Builder",
                    face: None,
                    turn: Turn::Finished,
                    stopped: false,
                };
                let rect = ui.max_rect();
                show(ui, rect, &rows, &mut state, facts);
            });
            shapes = format!("{:?}", out.shapes);
        }
        assert!(shapes.contains("Review"));
        assert!(shapes.contains("rename it"));
        assert!(!shapes.contains("Tidewatch review notes"));
        assert_eq!(state.heights.measured_count(), 1);
    }

    fn facts<'a>(prompt: Option<Prompt>, turn: Turn) -> Facts<'a> {
        Facts {
            tab: 1,
            source: Some(Source::Claude),
            prompt,
            ready: None,
            root: None,
            review: None,
            pr: None,
            due: None,
            profile: "Builder",
            face: None,
            turn,
            stopped: false,
        }
    }

    fn draw(rows: &Rows, state: &mut ViewState, facts: Facts<'_>) -> Vec<Action> {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 700.0))),
            focused: true,
            ..RawInput::default()
        };
        let mut actions = Vec::new();
        for _ in 0..3 {
            let _ = ctx.run_ui(raw.clone(), |ui| {
                let rect = ui.max_rect();
                actions = show(ui, rect, rows, state, facts);
            });
        }
        actions
    }

    fn question_entry(
        phase: tidewatch_core::transcript::ToolPhase,
    ) -> tidewatch_core::transcript::Entry {
        use tidewatch_core::question::{Question, QuestionOption, Select};
        use tidewatch_core::transcript::{AskPart, Entry, ToolCall};
        Entry {
            role: tidewatch_core::transcript::Role::Tool,
            text: String::new(),
            tool: Some(ToolCall {
                name: "AskUserQuestion".into(),
                summary: String::new(),
                id: Some("q1".into()),
                phase,
                ask: Some(AskPart::Asked(vec![Question {
                    text: "Where?".into(),
                    header: None,
                    decision: None,
                    select: Select::Single,
                    options: vec![QuestionOption::new("Stage"), QuestionOption::new("Run")],
                }])),
            }),
            ts: None,
        }
    }

    #[test]
    fn a_waiting_claude_question_shows_a_live_card_only_while_the_agent_is_mid_turn() {
        use tidewatch_core::transcript::ToolPhase;
        let mut rows = Rows::default();
        rows.push(question_entry(ToolPhase::Call));
        let mut state = ViewState::default();
        draw(&rows, &mut state, facts(Some(Prompt::Question), Turn::Running));
        assert_eq!(state.live.as_ref().map(|live| live.seq), Some(0));
        let mut finished = ViewState::default();
        draw(&rows, &mut finished, facts(None, Turn::Finished));
        assert!(finished.live.is_none(), "a finished agent has no menu open");
    }

    #[test]
    fn the_card_goes_away_when_the_question_is_answered_and_the_reply_box_asks_for_the_keyboard() {
        use crate::reader_rows::Asked;
        let mut rows = Rows::default();
        rows.push(question_entry(tidewatch_core::transcript::ToolPhase::Call));
        let mut state = ViewState::default();
        draw(&rows, &mut state, facts(None, Turn::Running));
        state.focus_reply = false;
        rows.set_asked(0, Asked::Declined);
        draw(&rows, &mut state, facts(None, Turn::Running));
        assert!(state.live.is_none());
    }

    #[test]
    fn a_finished_message_that_ends_with_a_yes_or_no_question_gets_quick_replies_once() {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::Assistant,
            text: "Tests pass. Should I also update the README?".into(),
            tool: None,
            ts: None,
        });
        let mut state = ViewState::default();
        draw(&rows, &mut state, facts(None, Turn::Finished));
        let labels: Vec<&str> =
            state.quick.as_ref().unwrap().1.iter().map(|q| q.label.as_str()).collect();
        assert_eq!(labels, ["Yes", "No"]);
        let mut working = ViewState::default();
        draw(&rows, &mut working, facts(None, Turn::Running));
        assert!(working.quick.is_none(), "no chips while the agent works");
        state.quick_used = rows.newest_seq();
        draw(&rows, &mut state, facts(None, Turn::Finished));
        assert!(state.quick.is_none(), "a used chip row does not come back");
    }

    fn shape_texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(text) => out.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| shape_texts(shape, out)),
            _ => {}
        }
    }

    struct Frames {
        ctx: egui::Context,
        state: ViewState,
        shapes: Vec<(String, Rect)>,
    }

    impl Frames {
        fn new() -> Self {
            Self {
                ctx: crate::fonts::test_context(),
                state: ViewState::default(),
                shapes: Vec::new(),
            }
        }

        fn run(&mut self, rows: &Rows, turn: Turn, events: Vec<Event>) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let state = &mut self.state;
            let output = self.ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                show(ui, rect, rows, state, facts(None, turn));
            });
            self.shapes.clear();
            output.shapes.iter().for_each(|clipped| shape_texts(&clipped.shape, &mut self.shapes));
        }

        fn at(&self, text: &str) -> Option<egui::Pos2> {
            self.shapes.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| rect.center())
        }

        fn click(&mut self, rows: &Rows, at: egui::Pos2) {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.run(rows, Turn::Finished, vec![Event::PointerMoved(at)]);
            self.run(rows, Turn::Finished, vec![button(true)]);
            self.run(rows, Turn::Finished, vec![button(false)]);
            self.run(rows, Turn::Finished, vec![]);
        }
    }

    fn turn_rows() -> Rows {
        use tidewatch_core::transcript::{Entry, Role, ToolCall, ToolPhase};
        let call = |id: &str, name: &str, summary: &str, phase| Entry {
            role: Role::Tool,
            text: String::new(),
            tool: Some(ToolCall {
                name: name.into(),
                summary: summary.into(),
                id: Some(id.into()),
                phase,
                ask: None,
            }),
            ts: Some(1_700_000_000_000),
        };
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::User,
            text: "tidy up".into(),
            tool: None,
            ts: Some(1_700_000_000_000),
        });
        for (id, name, summary, error) in [
            ("a", "Bash", "git status", false),
            ("b", "Bash", "bun test", true),
            ("c", "Read", "src/auth.rs", false),
        ] {
            rows.push(call(id, name, summary, ToolPhase::Call));
            rows.push(call(id, name, summary, ToolPhase::Result { error }));
        }
        rows.push(Entry {
            role: Role::Assistant,
            text: "All tidy.".into(),
            tool: None,
            ts: Some(1_700_000_060_000),
        });
        rows
    }

    #[test]
    fn a_turn_is_one_bubble_and_one_chip_row_with_no_line_per_call() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        frames.run(&rows, Turn::Finished, vec![]);
        frames.run(&rows, Turn::Finished, vec![]);
        for want in ["All tidy.", "3 steps", "2 commands \u{b7} 1 read", "\u{b7} 1 failed", "1m"] {
            assert!(frames.at(want).is_some(), "{want}: {:?}", frames.shapes);
        }
        assert!(frames.shapes.iter().all(|(text, _)| !text.contains("git status")));
        assert!(frames.shapes.iter().all(|(text, _)| !text.starts_with("Bash")));
        assert_eq!(frames.state.heights.measured_count(), 2);
    }

    #[test]
    fn a_click_on_a_chip_opens_the_list_of_calls_and_a_second_click_folds_it() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        frames.run(&rows, Turn::Finished, vec![]);
        frames.run(&rows, Turn::Finished, vec![]);
        let chip = frames.at("3 steps").unwrap();
        frames.click(&rows, chip);
        for want in ["$ git status", "$ bun test", "Read 1 file", "src/auth.rs"] {
            assert!(frames.at(want).is_some(), "{want}: {:?}", frames.shapes);
        }
        let again = frames.at("3 steps").unwrap();
        frames.click(&rows, again);
        assert!(frames.at("$ git status").is_none());
        assert_eq!(frames.state.expanded.len(), 0);
    }

    fn notes_turn(notes: &[&str], answer: &str, running: bool) -> Rows {
        use tidewatch_core::transcript::{Entry, Role, ToolCall, ToolPhase};
        let at = Some(1_700_000_000_000);
        let say = |role, text: &str| Entry { role, text: text.into(), tool: None, ts: at };
        let call = |id: &str, phase| Entry {
            role: Role::Tool,
            text: String::new(),
            tool: Some(ToolCall {
                name: "Read".into(),
                summary: "src/auth.rs".into(),
                id: Some(id.into()),
                phase,
                ask: None,
            }),
            ts: at,
        };
        let mut rows = Rows::default();
        rows.push(say(Role::User, "tidy up"));
        rows.push(call("start", ToolPhase::Call));
        rows.push(call("start", ToolPhase::Result { error: false }));
        for (n, note) in notes.iter().enumerate() {
            rows.push(say(Role::Assistant, note));
            rows.push(call(&format!("n{n}"), ToolPhase::Call));
            rows.push(call(&format!("n{n}"), ToolPhase::Result { error: false }));
        }
        if running {
            rows.push(call("last", ToolPhase::Call));
        }
        rows.push(say(Role::Assistant, answer));
        if !running {
            rows.push(say(Role::User, "thanks"));
        }
        rows
    }

    fn turn_height(frames: &Frames) -> f32 {
        let (_, _, layout) = frames.state.turns.as_ref().unwrap();
        frames.state.heights.measured(layout.groups[0].key).unwrap()
    }

    #[test]
    fn a_finished_turn_draws_only_its_last_text_and_folds_the_notes_behind_one_chip() {
        let notes = ["Reading the page.", "Wiring the table in now.", "Tests pass (154)."];
        let long = notes_turn(&notes, "All tidy.", false);
        let plain = notes_turn(&[], "All tidy.", false);
        let (mut folded, mut single) = (Frames::new(), Frames::new());
        for _ in 0..2 {
            folded.run(&long, Turn::Finished, vec![]);
            single.run(&plain, Turn::Finished, vec![]);
        }
        assert!(folded.at("All tidy.").is_some(), "{:?}", folded.shapes);
        assert!(folded.at("3 updates").is_some());
        assert!(single.shapes.iter().all(|(text, _)| !text.contains("update")));
        for note in notes {
            assert!(folded.at(note).is_none(), "{note} must stay folded");
        }
        let chip_row = reader_chat_view::CHIP_H + reader_chat_view::CHIP_GAP;
        let extra = turn_height(&folded) - turn_height(&single);
        assert!((extra - chip_row).abs() < 0.5, "the bubble is the last text only: {extra}");
    }

    #[test]
    fn a_click_on_the_updates_chip_lists_the_notes_in_order_and_a_second_click_folds_them() {
        let notes = ["Reading the page.", "Wiring the table in now."];
        let rows = notes_turn(&notes, "All tidy.", false);
        let mut frames = Frames::new();
        frames.run(&rows, Turn::Finished, vec![]);
        frames.run(&rows, Turn::Finished, vec![]);
        let chip = frames.at("2 updates").unwrap();
        frames.click(&rows, chip);
        let (first, second) = (frames.at(notes[0]).unwrap(), frames.at(notes[1]).unwrap());
        assert!(first.y < second.y);
        assert!(frames.at("3 reads").is_some(), "the call chips are untouched");
        assert!(frames.at("read  src/auth.rs").is_none());
        let again = frames.at("2 updates").unwrap();
        frames.click(&rows, again);
        assert!(frames.at(notes[0]).is_none());
        assert_eq!(frames.state.expanded.len(), 0);
    }

    #[test]
    fn the_dev_switch_opens_the_updates_alone_or_with_the_calls() {
        let rows = notes_turn(&["Reading the page."], "All tidy.", false);
        let mut both = ViewState::default();
        both.open_turns(&rows, false);
        let mut notes = ViewState::default();
        notes.open_turns(&rows, true);
        assert_eq!((both.expanded.len(), notes.expanded.len()), (2, 1));
        let (_, _, layout) = notes.turns.as_ref().unwrap();
        assert!(notes.expanded.contains(&layout.groups[0].notes_key()));
    }

    #[test]
    fn a_turn_in_progress_shows_its_latest_text_as_the_only_text_with_the_live_pill() {
        let rows =
            notes_turn(&["Reading the page.", "Wiring the table in now."], "Tests pass.", true);
        let mut frames = Frames::new();
        frames.run(&rows, Turn::Running, vec![]);
        frames.run(&rows, Turn::Running, vec![]);
        assert!(frames.at("Tests pass.").is_some(), "{:?}", frames.shapes);
        assert!(frames.at("2 updates").is_some());
        assert!(
            frames.at("Reading the page.").is_none()
                && frames.at("Wiring the table in now.").is_none()
        );
        assert!(frames.at("Reading").is_some());
    }

    #[test]
    fn a_turn_in_progress_shows_a_live_pill_and_no_bubble_until_text_arrives() {
        use tidewatch_core::transcript::{Entry, Role, ToolCall, ToolPhase};
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::User,
            text: "run it".into(),
            tool: None,
            ts: Some(1_700_000_000_000),
        });
        rows.push(Entry {
            role: Role::Tool,
            text: String::new(),
            tool: Some(ToolCall {
                name: "Bash".into(),
                summary: "bun test e2e/auth".into(),
                id: Some("t".into()),
                phase: ToolPhase::Call,
                ask: None,
            }),
            ts: Some(1_700_000_000_000),
        });
        let mut frames = Frames::new();
        frames.run(&rows, Turn::Running, vec![]);
        frames.run(&rows, Turn::Running, vec![]);
        assert!(frames.at("Running").is_some(), "{:?}", frames.shapes);
        assert!(frames.at("bun test e2e/auth").is_some());
        assert!(frames.shapes.iter().any(|(text, _)| text.ends_with("1 command so far")));
        assert!(
            frames
                .state
                .turns
                .as_ref()
                .is_some_and(|(_, _, layout)| layout.groups[0].texts.is_empty())
        );
    }

    #[test]
    fn asking_for_the_reply_box_makes_it_take_the_keyboard() {
        let mut state = ViewState { focus_reply: false, ..ViewState::default() };
        state.request_reply_focus();
        assert!(state.focus_reply);
    }

    #[test]
    fn a_permission_prompt_draws_its_card_and_hides_nothing_else() {
        use tidewatch_core::transcript::{Entry, Role, ToolPhase};
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::Tool,
            text: String::new(),
            tool: Some(tidewatch_core::transcript::ToolCall {
                name: "Bash".into(),
                summary: "cargo test".into(),
                id: Some("b1".into()),
                phase: ToolPhase::Call,
                ask: None,
            }),
            ts: None,
        });
        assert_eq!(
            permission_tool(&rows, None),
            Some(("Bash".to_owned(), "cargo test".to_owned()))
        );
        let mut state = ViewState::default();
        let actions = draw(&rows, &mut state, facts(Some(Prompt::Permission), Turn::Running));
        assert_eq!(actions, Vec::<Action>::new());
    }

    fn conversation(turns: usize, words: usize) -> Rows {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        for n in 0..turns {
            let ask = format!("question {n}");
            rows.push(Entry { role: Role::User, text: ask, tool: None, ts: None });
            let text = format!("answer {n}: {}", "word ".repeat(words + n % 7 * 30));
            rows.push(Entry { role: Role::Assistant, text, tool: None, ts: None });
        }
        rows
    }

    struct Readers {
        ctx: egui::Context,
        time: f64,
    }

    impl Readers {
        fn new() -> Self {
            Self { ctx: crate::fonts::test_context(), time: 0.0 }
        }

        fn frame(&mut self, tab: u64, rows: &Rows, state: &mut ViewState, events: Vec<Event>) {
            self.time += 0.016;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0))),
                events,
                focused: true,
                time: Some(self.time),
                ..RawInput::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                let facts = Facts { tab, ..facts(None, Turn::Finished) };
                let _ = show(ui, rect, rows, state, facts);
            });
        }

        fn hidden(&mut self) {
            self.time += 0.016;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0))),
                focused: true,
                time: Some(self.time),
                ..RawInput::default()
            };
            let _ = self.ctx.run_ui(raw, |_| {});
        }

        fn show(&mut self, tab: u64, rows: &Rows, state: &mut ViewState) {
            for _ in 0..12 {
                self.frame(tab, rows, state, vec![]);
            }
        }

        fn scroll_up(&mut self, tab: u64, rows: &Rows, state: &mut ViewState) {
            self.frame(tab, rows, state, vec![Event::PointerMoved(pos2(300.0, 200.0))]);
            let wheel = Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 400.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            };
            self.frame(tab, rows, state, vec![wheel]);
            for _ in 0..4 {
                self.show(tab, rows, state);
            }
            self.frame(tab, rows, state, vec![Event::PointerGone]);
        }
    }

    fn at_bottom(state: &ViewState) -> bool {
        let (offset, max) = state.settled_at;
        max > 0.0 && (max - offset).abs() < 1.0
    }

    #[test]
    fn a_tab_opens_at_its_newest_message_when_you_switch_to_it() {
        let mut readers = Readers::new();
        let (short, long) = (conversation(8, 40), conversation(30, 60));
        let (mut a, mut b) = (ViewState::default(), ViewState::default());
        readers.show(1, &short, &mut a);
        readers.show(2, &long, &mut b);
        assert!(at_bottom(&b), "{:?}", b.settled_at);
        readers.show(1, &short, &mut a);
        assert!(at_bottom(&a), "{:?}", a.settled_at);
        readers.show(2, &long, &mut b);
        assert!(at_bottom(&b), "{:?}", b.settled_at);
        assert!(b.scroll.following());
        readers.show(1, &short, &mut a);
        assert!(at_bottom(&a), "{:?}", a.settled_at);
    }

    #[test]
    fn a_tab_scrolled_up_keeps_its_place_when_you_switch_away_and_back() {
        let mut readers = Readers::new();
        let (first, second) = (conversation(30, 60), conversation(12, 50));
        let (mut a, mut b) = (ViewState::default(), ViewState::default());
        readers.show(1, &first, &mut a);
        readers.scroll_up(1, &first, &mut a);
        assert!(!a.scroll.following());
        let (kept, max) = a.settled_at;
        assert!(kept < max - 100.0, "{:?}", a.settled_at);
        readers.show(2, &second, &mut b);
        assert!(at_bottom(&b), "{:?}", b.settled_at);
        readers.show(1, &first, &mut a);
        assert!(!a.scroll.following());
        assert_eq!(a.settled_at.0, kept);
    }

    #[test]
    fn a_following_tab_is_at_the_bottom_after_the_reader_was_hidden_for_a_while() {
        let mut readers = Readers::new();
        let (rows, mut state) = (conversation(20, 60), ViewState::default());
        readers.show(1, &rows, &mut state);
        assert!(at_bottom(&state), "{:?}", state.settled_at);
        for _ in 0..5 {
            readers.hidden();
        }
        readers.frame(1, &rows, &mut state, vec![]);
        assert!(at_bottom(&state), "{:?}", state.settled_at);
        assert!(state.scroll.following());
    }

    fn shift_enter() -> Event {
        Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers { shift: true, ..Modifiers::NONE },
        }
    }

    struct Seen {
        column: Rect,
        status_top: f32,
        boxed: Option<Rect>,
        marker: Option<(Rect, Rect)>,
        newest: Option<Rect>,
        send: Option<Rect>,
        actions: Vec<Action>,
    }

    fn texts_in(shape: &egui::Shape, clip: Rect, out: &mut Vec<(Arc<Galley>, egui::Pos2, Rect)>) {
        match shape {
            egui::Shape::Text(text) => out.push((text.galley.clone(), text.pos, clip)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts_in(s, clip, out)),
            _ => {}
        }
    }

    fn long_chat() -> Rows {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        for n in 1..=14 {
            let role = if n % 2 == 0 { Role::Assistant } else { Role::User };
            let text = format!("message {n}: {}", "words ".repeat(30));
            rows.push(Entry { role, text, tool: None, ts: None });
        }
        rows.push(Entry {
            role: Role::Assistant,
            text: "the newest message".into(),
            tool: None,
            ts: None,
        });
        rows
    }

    fn reply_column(
        ctx: &egui::Context,
        (rows, state): (&Rows, &mut ViewState),
        events: Vec<Event>,
        h: f32,
    ) -> Seen {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, h))),
            events,
            focused: true,
            time: Some(ctx.cumulative_pass_nr() as f64 * 0.1),
            ..RawInput::default()
        };
        let mut column = Rect::NOTHING;
        let mut actions = Vec::new();
        let out = ctx.run_ui(raw, |ui| {
            egui::Panel::bottom("statusbar")
                .exact_size(crate::status_bar::HEIGHT)
                .frame(Frame::NONE)
                .show(ui, |_| {});
            egui::CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
                column = ui.max_rect();
                actions = show(ui, column, rows, state, facts(None, Turn::Finished));
            });
        });
        let mut texts = Vec::new();
        out.shapes
            .iter()
            .for_each(|clipped| texts_in(&clipped.shape, clipped.clip_rect, &mut texts));
        let boxed = out
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect)
                    if rect.fill == brand::CARD
                        && (rect.rect.width() - (column.width() - 2.0 * INSET)).abs() < 4.0 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .max_by(|a, b| a.bottom().total_cmp(&b.bottom()));
        let marker = texts.iter().find(|(galley, ..)| galley.text().contains("END")).and_then(
            |(galley, pos, clip)| {
                let row = galley.rows.last()?.rect().translate(pos.to_vec2());
                Some((row, *clip))
            },
        );
        let rect_of = |(galley, pos, _): &(Arc<Galley>, egui::Pos2, Rect)| {
            galley.rect.translate(pos.to_vec2())
        };
        let newest = texts.iter().find(|(g, ..)| g.text() == "the newest message").map(rect_of);
        let send = texts.iter().find(|(g, ..)| g.text() == "Send").map(rect_of);
        Seen {
            column,
            status_top: h - crate::status_bar::HEIGHT,
            boxed,
            marker,
            newest,
            send,
            actions,
        }
    }

    fn settled(
        ctx: &egui::Context,
        pair: (&Rows, &mut ViewState),
        events: Vec<Event>,
        h: f32,
    ) -> Seen {
        let (rows, state) = pair;
        reply_column(ctx, (rows, &mut *state), events, h);
        for _ in 0..5 {
            reply_column(ctx, (rows, &mut *state), vec![], h);
        }
        reply_column(ctx, (rows, state), vec![], h)
    }

    fn assert_reply_box_in_place(seen: &Seen, what: &str) {
        let boxed = seen.boxed.unwrap_or_else(|| panic!("{what}: the reply box is drawn"));
        assert!(boxed.bottom() <= seen.status_top, "{what}: {boxed:?} below the status bar");
        assert!(boxed.top() >= seen.column.top(), "{what}: {boxed:?} above the column");
        assert!(
            boxed.height() <= (seen.column.height() * 0.5).max(INPUT_H) + 0.5,
            "{what}: {boxed:?} past half the column"
        );
        assert!(boxed.left() >= seen.column.left() && boxed.right() <= seen.column.right());
        let newest = seen.newest.unwrap_or_else(|| panic!("{what}: the newest message is drawn"));
        assert!(newest.bottom() <= boxed.top(), "{what}: {newest:?} under the box {boxed:?}");
        let send = seen.send.unwrap_or_else(|| panic!("{what}: Send is drawn"));
        assert!(boxed.contains_rect(send), "{what}: Send {send:?} outside {boxed:?}");
        assert!(send.bottom() > boxed.bottom() - 26.0, "{what}: Send {send:?} not at the bottom");
        assert!(send.right() > boxed.right() - 30.0, "{what}: Send {send:?} not at the right");
    }

    fn assert_cursor_row_visible(seen: &Seen, what: &str) {
        let (row, clip) = seen.marker.unwrap_or_else(|| panic!("{what}: the text end is drawn"));
        let boxed = seen.boxed.expect("the reply box is drawn");
        assert!(clip.contains_rect(row.shrink(0.5)), "{what}: cursor row {row:?} outside {clip:?}");
        assert!(boxed.contains_rect(clip), "{what}: visible {clip:?} outside the box {boxed:?}");
    }

    fn typing_in_the_reply_box(window_h: f32, lines: bool) {
        let ctx = crate::fonts::test_context();
        let rows = long_chat();
        let mut state = ViewState::default();
        let mut first = settled(&ctx, (&rows, &mut state), vec![], window_h);
        assert_eq!(first.boxed.map(|b| b.height()), Some(INPUT_H - 2.0 * REPLY_GAP_Y));
        let mut last_height = first.boxed.unwrap().height();
        if lines {
            for line in 1..=60 {
                let typed = vec![Event::Text(format!("line {line}")), shift_enter()];
                first = settled(&ctx, (&rows, &mut state), typed, window_h);
                assert_reply_box_in_place(&first, &format!("line {line}"));
                let height = first.boxed.unwrap().height();
                assert!(height >= last_height - 0.5, "line {line}: the box shrank");
                last_height = height;
            }
            assert!(state.reply.ends_with("line 60\n"), "every line was typed");
            let end = vec![Event::Text("the END".into())];
            first = settled(&ctx, (&rows, &mut state), end, window_h);
        } else {
            let long = format!("{}END", "a long reply that keeps going and going ".repeat(50));
            assert!(long.len() > 2000);
            first = settled(&ctx, (&rows, &mut state), vec![Event::Text(long)], window_h);
            assert!(!state.reply.contains('\n'));
        }
        assert_reply_box_in_place(&first, "after typing");
        assert_cursor_row_visible(&first, "after typing");
        assert!(first.boxed.unwrap().height() > INPUT_H, "the box grew");
        assert_eq!(first.actions.len(), 0);
    }

    #[test]
    fn sixty_typed_lines_grow_the_reply_box_then_scroll_inside_it() {
        typing_in_the_reply_box(600.0, true);
    }

    #[test]
    fn sixty_typed_lines_in_a_short_window_keep_the_reply_box_above_the_status_bar() {
        typing_in_the_reply_box(360.0, true);
    }

    #[test]
    fn a_2000_character_line_wraps_in_the_reply_box() {
        typing_in_the_reply_box(600.0, false);
    }

    #[test]
    fn a_2000_character_line_wraps_in_a_short_window() {
        typing_in_the_reply_box(360.0, false);
    }

    #[test]
    fn enter_sends_the_reply_and_shift_enter_adds_a_line() {
        let ctx = crate::fonts::test_context();
        let rows = long_chat();
        let mut state = ViewState::default();
        settled(&ctx, (&rows, &mut state), vec![], 600.0);
        settled(&ctx, (&rows, &mut state), vec![Event::Text("one".into())], 600.0);
        let seen = settled(&ctx, (&rows, &mut state), vec![shift_enter()], 600.0);
        assert_eq!(seen.actions, Vec::<Action>::new());
        settled(&ctx, (&rows, &mut state), vec![Event::Text("two".into())], 600.0);
        assert_eq!(state.reply, "one\ntwo");
        let seen = reply_column(&ctx, (&rows, &mut state), vec![enter()], 600.0);
        assert_eq!(seen.actions, vec![Action::Reply("one\ntwo".into())]);
        assert_eq!(state.reply, "");
        assert_eq!(
            settled(&ctx, (&rows, &mut state), vec![], 600.0).boxed.map(|b| b.height()),
            Some(INPUT_H - 2.0 * REPLY_GAP_Y)
        );
    }

    #[test]
    fn a_pasted_text_keeps_its_newlines() {
        let ctx = crate::fonts::test_context();
        let rows = long_chat();
        let mut state = ViewState::default();
        settled(&ctx, (&rows, &mut state), vec![], 600.0);
        let pasted = Event::Paste("first\nsecond\n\nfourth".into());
        let seen = settled(&ctx, (&rows, &mut state), vec![pasted], 600.0);
        assert_eq!(state.reply, "first\nsecond\n\nfourth");
        assert!(seen.boxed.unwrap().height() > INPUT_H, "four rows are taller than one");
    }

    #[test]
    fn the_reply_box_is_one_row_tall_when_empty_and_grows_by_each_row() {
        let ctx = crate::fonts::test_context();
        let column = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let mut heights = Vec::new();
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            for text in ["", "one", "one\ntwo", "one\ntwo\nthree", &"row\n".repeat(200)] {
                heights.push(composer::height(ui, text, 0, column));
            }
        });
        assert_eq!(heights[0], INPUT_H);
        assert_eq!(heights[1], INPUT_H);
        let row = heights[2] - INPUT_H;
        assert!(row > 10.0, "{heights:?}");
        assert!((heights[3] - INPUT_H - 2.0 * row).abs() < 0.5, "{heights:?}");
        assert_eq!(heights[4], 600.0 * composer::SHARE + 2.0 * composer::GAP_Y);
    }

    #[test]
    fn the_due_card_docks_above_a_grown_reply_box() {
        use crate::due::Due;
        use tidewatch_core::progress::Mark;

        let due = Due {
            what: "Switch off the old webhook".into(),
            days: 0,
            steps: vec![(Mark::Todo, "Merge the switch".into())],
        };
        let ctx = crate::fonts::test_context();
        let rows = long_chat();
        let mut state = ViewState { reply: "a\n".repeat(30), ..ViewState::default() };
        let mut card_and_box = || {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 700.0))),
                focused: true,
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                let mut facts = facts(None, Turn::Finished);
                facts.due =
                    Some(DueFacts { due: &due, more: 0, profile: "Builder", blocked: None });
                show(ui, rect, &rows, &mut state, facts);
            });
            let mut texts = Vec::new();
            out.shapes.iter().for_each(|c| texts_in(&c.shape, c.clip_rect, &mut texts));
            let card = texts
                .iter()
                .find(|(g, ..)| g.text() == "Ask Builder to start")
                .map(|(g, pos, _)| g.rect.translate(pos.to_vec2()));
            let boxed = out.shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Rect(rect)
                    if rect.fill == brand::CARD && rect.rect.width() > 700.0 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            });
            (card, boxed)
        };
        card_and_box();
        let (card, boxed) = card_and_box();
        let (card, boxed) = (card.expect("the card is drawn"), boxed.expect("the box is drawn"));
        assert!(boxed.height() > INPUT_H);
        assert!(card.bottom() <= boxed.top(), "{card:?} over {boxed:?}");
    }

    fn drawn_texts(rows: &Rows, width: f32) -> Vec<String> {
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let ctx = crate::fonts::test_context();
        let mut state = ViewState::default();
        let mut found = Vec::new();
        for _ in 0..3 {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 700.0))),
                focused: true,
                ..RawInput::default()
            };
            let output = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                show(ui, rect, rows, &mut state, facts(None, Turn::Running));
            });
            found.clear();
            output.shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut found));
        }
        found
    }

    #[test]
    fn a_message_tidewatch_typed_is_one_quiet_card_and_never_a_you_bubble() {
        use tidewatch_core::transcript::{Entry, Role};
        let prompt = "@codex commented on PR #7. Answer it.\n\n> <!-- codex-review-state:v1 -->\n> | a | b |\n> |---|---|\n> <details>";
        let mut rows = Rows::default();
        rows.tidewatch_said(prompt, "Pull request", 0);
        rows.push(Entry { role: Role::User, text: prompt.into(), tool: None, ts: Some(5_000) });
        let found = drawn_texts(&rows, 900.0);
        assert!(found.iter().any(|text| text.starts_with("TidewatchPull request")), "{found:?}");
        assert!(found.iter().any(|text| text.starts_with("@codex commented")), "{found:?}");
        assert!(found.iter().all(|text| !text.starts_with("You")), "{found:?}");
        assert!(found.iter().all(|text| !text.contains("codex-review-state")), "{found:?}");
    }

    #[test]
    fn a_message_sent_while_the_agent_is_busy_shows_queued_until_the_transcript_has_it() {
        use tidewatch_core::transcript::{Entry, Role};
        let now = now_ms();
        let mut rows = Rows::default();
        rows.push(Entry { role: Role::User, text: "start".into(), tool: None, ts: Some(now) });
        rows.queue_user("then also this", now);
        let found = drawn_texts(&rows, 900.0);
        assert!(found.iter().any(|text| text == "Queued"), "{found:?}");
        assert!(found.iter().any(|text| text == "then also this"), "{found:?}");
        rows.push(Entry {
            role: Role::User,
            text: "then also this".into(),
            tool: None,
            ts: Some(now + 1),
        });
        let found = drawn_texts(&rows, 900.0);
        assert!(found.iter().all(|text| text != "Queued"), "{found:?}");
        assert_eq!(found.iter().filter(|text| *text == "then also this").count(), 1, "{found:?}");
    }
}
