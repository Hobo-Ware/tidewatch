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
use tidewatch_core::clock::now_ms;
use tidewatch_core::question::Reply;
use tidewatch_core::transcript::Source;

use crate::agent_avatar::Face;
use crate::agent_ready::{self, Ready};
use crate::answer_chips::Pick;
use crate::ask_model::Ask;
use crate::ask_view;
use crate::at_files;
use crate::brand;
use crate::bubble_toolbar::BubbleAction;
use crate::composer::{self, Draft, Outcome, Spec};
use crate::due_view::{self, Facts as DueFacts};
use crate::held_view;
use crate::job_model::clock_label;
use crate::pr_card_model::CardAction;
use crate::pr_cards_view::{self, Data as PrData, Heights as PrHeights};
use crate::pr_timeline::{self, Place};
use crate::prompt_ghost::Ghost;
use crate::question_bulk;
use crate::question_card::{Card, Effect};
use crate::question_view::{self, ChipClick, Live, PermissionClick};
use crate::quick_reply::{self, Quick};
use crate::quiet_notes::{self, Entry};
use crate::reader_chat_view::{self, Queue, Scene};
use crate::reader_keys::{self, Answer};
use crate::reader_rows::{Kind, Origin, Row, Rows};
use crate::reader_scroll::{self, Autoscroll, Heights};
use crate::reader_turns::{self, Slot};
use crate::recall::Recall;
use crate::reply_reason;
use crate::review::ReviewFacts;
use crate::review_row_view;
use crate::review_view::{self, ReviewAction};
use crate::screen_kit::{Size, body, primary_button, quiet_button};
use crate::send_line::SendLine;
use crate::slash::Entry as SlashEntry;
use crate::tidewatch_view::{self, Click, Clicked};
use crate::tokens;
use tidewatch_core::sent::Record;

/// The side inset every inline item of the left column keeps.
pub(crate) const INSET: f32 = 14.0;
const CARD_RADIUS: u8 = 8;
const OVERSCAN: f32 = 600.0;
const QUEUED_FRAME: std::time::Duration = std::time::Duration::from_millis(33);
const STRIP_H: f32 = 48.0;
const BAR_W: f32 = 2.0;
const BAR_INSET: f32 = 7.0;
const JUMP_W: f32 = 140.0;
const JUMP_H: f32 = Size::Compact.height();
const JUMP_STRIP: f32 = JUMP_H + 12.0;
const SCROLLBAR_GRAB_W: f32 = 16.0;
const OUTPUT_MAX_LINES: usize = 40;

#[allow(clippy::struct_excessive_bools)] // independent marks of one tab's view, not a mode
pub(crate) struct ViewState {
    pub(crate) scroll: Autoscroll,
    pub(crate) heights: Heights,
    pr_heights: PrHeights,
    pub(crate) reply: String,
    pub(crate) files: Arc<Vec<String>>,
    pub(crate) files_root: PathBuf,
    composing: bool,
    pub(crate) recall: Recall,
    pub(crate) line: SendLine,
    held_edit: Option<(u64, String)>,
    focus_held_edit: bool,
    pub(crate) pastes: Option<PathBuf>,
    pub(crate) ghost: Ghost,
    review: review_view::CardState,
    focus_reply: bool,
    pub(crate) commands: Arc<Vec<SlashEntry>>,
    content_h: f32,
    dev_up: Option<f32>,
    dev_hold: bool,
    strip_h: f32,
    strip_shift: f32,
    scroll_id: Option<Id>,
    body_h: f32,
    seen_rows: u64,
    pub(crate) fresh_after_ms: Option<i64>,
    stick_pending: bool,
    last_pass: Option<u64>,
    #[cfg(test)]
    settled_at: (f32, f32),
    live: Option<Live>,
    auto_left: Option<(u64, f64)>,
    quick: Option<(u64, Vec<Quick>)>,
    quick_used: Option<u64>,
    expanded: HashSet<u64>,
    tidewatch_open: HashSet<u64>,
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
            files: Arc::default(),
            files_root: PathBuf::new(),
            composing: false,
            recall: Recall::default(),
            line: SendLine::default(),
            held_edit: None,
            focus_held_edit: false,
            pastes: None,
            ghost: Ghost::default(),
            review: review_view::CardState::default(),
            focus_reply: true,
            commands: Arc::default(),
            content_h: 0.0,
            dev_up: None,
            dev_hold: false,
            strip_h: 0.0,
            strip_shift: 0.0,
            scroll_id: None,
            body_h: 0.0,
            seen_rows: 0,
            fresh_after_ms: None,
            stick_pending: true,
            last_pass: None,
            #[cfg(test)]
            settled_at: (0.0, 0.0),
            live: None,
            auto_left: None,
            quick: None,
            quick_used: None,
            expanded: HashSet::new(),
            tidewatch_open: HashSet::new(),
            turns: None,
            zone: TimeZone::system(),
        }
    }
}

impl ViewState {
    /// A view of a tab that started at `started_ms`: what arrives later is new to it.
    pub(crate) fn started_at(started_ms: i64) -> Self {
        Self { fresh_after_ms: Some(started_ms), ..Self::default() }
    }

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

    /// Opens the list of calls and the folded updates under every turn, and every Tidewatch message,
    /// as a click on a chip does; only the updates when `notes_only`.
    pub(crate) fn open_turns(&mut self, rows: &Rows, notes_only: bool) {
        self.refresh_turns(rows);
        if let Some((_, _, layout)) = &self.turns {
            let keys = layout.groups.iter().filter(|group| !group.calls.is_empty() && !notes_only);
            self.expanded.extend(keys.map(|group| group.key));
            let notes = layout.groups.iter().filter(|group| !group.notes().is_empty());
            self.expanded.extend(notes.map(reader_turns::Group::notes_key));
        }
        if !notes_only {
            self.tidewatch_open.extend(rows.told_ids());
        }
    }

    /// Scrolls up from the bottom by `pixels` at the next draw, for captures.
    pub(crate) fn scroll_up_by(&mut self, pixels: f32) {
        self.dev_up = Some(pixels);
    }

    /// The reply box takes the keyboard the next time it is drawn.
    pub(crate) fn request_reply_focus(&mut self) {
        self.focus_reply = true;
    }

    /// Puts `text` in the box with the cursor at its `|`, for a capture.
    pub(crate) fn set_sentence(&mut self, ctx: &egui::Context, text: &str) {
        let (before, after) = text.split_once('|').unwrap_or((text, ""));
        self.reply = format!("{before}{after}");
        let id = if self.composing { agent_ready::composer_id() } else { reply_id() };
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let at = egui::text::CCursor::new(before.chars().count());
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(at)));
        state.store(ctx, id);
        self.focus_reply = true;
    }

    /// Puts files into the sentence at the box's cursor, or at its end when the box is not the one typed in.
    pub(crate) fn insert_files(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let id = if self.composing { agent_ready::composer_id() } else { reply_id() };
        let targets: Vec<tokens::Target> = paths.into_iter().map(tokens::Target::File).collect();
        composer::insert_targets(ctx, id, &mut self.reply, &targets);
        self.focus_reply = true;
    }

    /// The held message whose row is open for editing.
    pub(crate) fn editing_held(&self) -> Option<u64> {
        self.held_edit.as_ref().map(|(id, _)| *id)
    }

    /// Follows the newest message again, as after the owner sends one.
    pub(crate) fn jump_to_latest(&mut self) {
        self.scroll.jump_to_latest();
        self.stick_pending = true;
    }

    /// Gives `text` back to the owner's box, ahead of what is already typed there.
    pub(crate) fn keep_in_box(&mut self, text: &str) {
        if self.reply.trim().is_empty() {
            text.clone_into(&mut self.reply);
        } else {
            self.reply = format!("{text}\n\n{}", self.reply);
        }
        self.focus_reply = true;
    }

    pub(crate) fn begin_edit_held(&mut self, id: u64) {
        if let Some(text) = self.line.text_of(id) {
            self.held_edit = Some((id, text.to_owned()));
            self.focus_held_edit = true;
        }
    }

    pub(crate) fn apply_held(&mut self, out: held_view::Out, actions: &mut Vec<Action>) {
        match out {
            held_view::Out::Edit(id) => self.begin_edit_held(id),
            held_view::Out::Save(id, text) => {
                self.line.set_text(id, &text);
                self.held_edit = None;
                self.focus_reply = true;
            }
            held_view::Out::Cancel => {
                self.held_edit = None;
                self.focus_reply = true;
            }
            held_view::Out::Delete(id) => {
                self.line.delete(id);
            }
            held_view::Out::Shift(id, way) => {
                self.line.shift(id, way);
            }
            held_view::Out::SendNow(id) => actions.push(Action::SendNow(id)),
        }
    }

    fn apply_bubble(
        &mut self,
        click: (BubbleAction, String, Option<u64>),
        actions: &mut Vec<Action>,
    ) {
        match click {
            (BubbleAction::SendAgain, text, forget) => {
                actions.push(Action::SendAgain { text, forget });
            }
            (BubbleAction::Dismiss, _, Some(id)) => actions.push(Action::Forget(id)),
            (BubbleAction::EditAndSend, text, _) => {
                self.recall.load_message(&text, &mut self.reply);
                self.focus_reply = true;
            }
            (BubbleAction::Dismiss, _, None) => {}
        }
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
    /// The ledger's question waiting for the owner, drawn right above the reply box.
    pub(crate) ask: Option<&'a Ask>,
    /// The profile's name, for "<Profile> asks".
    pub(crate) profile: &'a str,
    /// The tab's face, shown beside each of the agent's turns.
    pub(crate) face: Option<&'a Face>,
    pub(crate) turn: Turn,
    /// The agent is in the middle of a turn that Stop can end.
    pub(crate) working: bool,
    /// Why the reply box is shut because the agent is not at its prompt.
    pub(crate) reply_blocked: Option<Blocked>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blocked {
    Stopped,
    Failed,
    SettingUp,
    Shell,
}

impl Blocked {
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Stopped => "Resume first",
            Self::Failed => "The agent failed, resume first",
            Self::SettingUp => "The agent is starting up",
            Self::Shell => "No agent is running here",
        }
    }
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
    /// A button under a row Tidewatch said: its workflow, or the setting that stops its kind.
    Tidewatch(Click),
    /// The user answered, or skipped, the questions in row `seq`.
    Ask(u64, Reply),
    /// Answer every waiting question that has a pick with it.
    GoWithPicks,
    /// A click on the Due today card.
    Due(due_view::Action),
    /// A ready answer picked on the Needs you card.
    Pick(String),
    Resume,
    /// Stop the agent's turn.
    Stop,
    /// Stop the turn and type the held message `id` now.
    SendNow(u64),
    /// Send `text` again; `forget` is the queued message it replaces.
    SendAgain {
        text: String,
        forget: Option<u64>,
    },
    /// Stop showing the queued message `id`.
    Forget(u64),
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
    facts: &Facts<'_>,
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
        agent_ready::height(ui, &state.reply, rect)
    } else {
        composer::height_with_stop(ui, &state.reply, rect, facts.working)
    };
    let input = Rect::from_min_max(pos2(rect.left(), rect.bottom() - input_h), rect.max);
    let reason = facts.reply_blocked.filter(|_| composing.is_none());
    let reason_h = if reason.is_some() { reply_reason::HEIGHT } else { 0.0 };
    let above = input.top() - reason_h;
    let dock_h = dock_height(facts, rows, state.live.as_ref());
    let strip = Rect::from_min_max(pos2(rect.left(), above - dock_h), pos2(rect.right(), above));
    let asked = facts
        .ask
        .filter(|_| dock_h == 0.0)
        .map(|ask| (ask, ask_view::lay_out(ui, rect.width(), ask)));
    let ask_h =
        asked.as_ref().map_or(0.0, |(_, laid)| laid.band(rect.height() - input_h - reason_h));
    let ask_band =
        Rect::from_min_max(pos2(rect.left(), strip.top() - ask_h), pos2(rect.right(), strip.top()));
    let review_h = facts.review.as_ref().map_or(0.0, |review| {
        review_view::height(review.draft, rect.height() - input_h - reason_h - dock_h - ask_h)
    });
    let review_band = Rect::from_min_max(
        pos2(rect.left(), ask_band.top() - review_h),
        pos2(rect.right(), ask_band.top()),
    );
    let due_h = facts.due.map_or(0.0, |due| {
        due_view::height(due.due, rect.height() - input_h - reason_h - dock_h - ask_h - review_h)
    });
    let due_band = Rect::from_min_max(
        pos2(rect.left(), review_band.top() - due_h),
        pos2(rect.right(), review_band.top()),
    );
    if state.held_edit.as_ref().is_some_and(|(id, _)| state.line.text_of(*id).is_none()) {
        state.held_edit = None;
    }
    let free_h = rect.height() - input_h - reason_h - dock_h - ask_h - review_h - due_h;
    let held_h = held_view::height(ui, rect.width(), (state.line.len(), state.held_edit.is_some()))
        .min((free_h / 2.0).max(0.0));
    let held_band = Rect::from_min_max(
        pos2(rect.left(), due_band.top() - held_h),
        pos2(rect.right(), due_band.top()),
    );
    let body = Rect::from_min_max(rect.min, pos2(rect.right(), held_band.top()));
    if let Some(blocked) = reason {
        let band = Rect::from_min_max(pos2(rect.left(), above), pos2(rect.right(), input.top()));
        if reply_reason::show(ui, band, blocked) {
            actions.push(Action::Resume);
        }
    }
    if held_h > 0.0 {
        let note = if facts.working {
            "Held by Tidewatch \u{b7} sent when the agent finishes its turn"
        } else if reason.is_some() {
            "Held by Tidewatch \u{b7} sent when the agent is running again"
        } else {
            "Held by Tidewatch \u{b7} sent as soon as the agent is free"
        };
        let shown = held_view::Shown {
            items: state.line.items(),
            editing: state.held_edit.as_mut().map(|(id, buffer)| (*id, buffer)),
            focus_edit: &mut state.focus_held_edit,
            note,
        };
        for out in held_view::show(ui, held_band, shown) {
            state.apply_held(out, &mut actions);
        }
    }
    if let Some(due) = facts.due.filter(|_| due_h > 0.0) {
        actions.extend(due_view::show(ui, due_band, due).map(Action::Due));
    }
    if let Some(review) = facts.review.as_ref().filter(|_| review_h > 0.0) {
        let asked = review_view::show(ui, review_band, &mut state.review, review);
        actions.extend(asked.into_iter().map(Action::Review));
    }
    if let Some((ask, laid)) = asked.as_ref().filter(|_| ask_h > 0.0) {
        let reply = if composing.is_some() { agent_ready::composer_id() } else { reply_id() };
        let empty = state.reply.trim().is_empty();
        match ask_view::show(ui, ask_band, laid, ask, (reply, empty)) {
            Some(Pick::Answer(text)) => actions.push(Action::Pick(text)),
            Some(Pick::Other) => state.focus_reply = true,
            None => {}
        }
    }
    state.composing = composing.is_some();
    if let Some(ready) = composing {
        agent_ready::empty_state(ui, body, ready);
        let draft = Draft { text: &mut state.reply, focus: &mut state.focus_reply, recall: None };
        let files = (!state.files_root.as_os_str().is_empty())
            .then_some(at_files::Source { root: &state.files_root, paths: &state.files });
        if let Some(text) = agent_ready::composer(ui, input, draft, (ready, files)) {
            actions.push(Action::Reply(text));
            state.jump_to_latest();
        }
        return actions;
    }
    let quick = quick_for(rows, state, facts).to_vec();
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
            state.jump_to_latest();
        }
        Some(ChipClick::Reply) => state.focus_reply = true,
        None => {}
    }
    let mut reply_enabled = facts.prompt.is_none() && facts.reply_blocked.is_none();
    if let Some(live) = state.live.as_mut() {
        let tidewatch = shown.is_some_and(|row| {
            matches!(&row.kind, Kind::Question(q) if matches!(q.origin, Origin::Tidewatch(_)))
        });
        reply_enabled = tidewatch && facts.reply_blocked.is_none();
        let grab = state.reply.trim().is_empty();
        live.picks = question_bulk::plan(rows).count();
        live.auto_left = state.auto_left.filter(|(seq, _)| *seq == live.seq).map(|(_, left)| left);
        let dock = Rect::from_min_max(rect.min, pos2(rect.right(), above));
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
        focus: &mut state.focus_reply,
        recall: Some(&mut state.recall),
    };
    let files = (!state.files_root.as_os_str().is_empty())
        .then_some(at_files::Source { root: &state.files_root, paths: &state.files });
    let outcome = reply_box(
        &mut input_ui,
        draft,
        (reply_enabled, facts.reply_blocked),
        (&state.commands, state.ghost.text(), files),
        (facts.working, state.line.len()),
    );
    if let Some(text) = outcome.sent {
        actions.push(Action::Reply(text));
        state.jump_to_latest();
    }
    if outcome.stop {
        actions.push(Action::Stop);
    }
    if outcome.edit_held
        && let Some(id) = state.line.newest_id()
    {
        state.begin_edit_held(id);
    }
    actions
}

/// The part of `body` the rows scroll in and, while the view is not following, the slot of the
/// "Jump to latest" pill. The pill gets a strip of its own under the rows, so it never covers the
/// last row; a body too short to spare the strip lets the pill float over it.
fn jump_split(body: Rect, following: bool) -> (Rect, Option<Rect>) {
    if following {
        return (body, None);
    }
    let strip = if body.height() >= 4.0 * JUMP_STRIP { JUMP_STRIP } else { JUMP_H };
    let slot = Rect::from_center_size(
        pos2(body.center().x, body.bottom() - strip / 2.0),
        vec2(JUMP_W, JUMP_H),
    );
    let rows = if strip == JUMP_STRIP {
        Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - strip))
    } else {
        body
    };
    (rows, Some(slot))
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
    let dev_up = if state.content_h > 0.0 { state.dev_up.take() } else { None };
    state.dev_hold |= dev_up.is_some();
    if state.dev_hold {
        state.scroll.release();
    }
    let (view, jump) = jump_split(body, state.scroll.following());
    let strip_h = body.height() - view.height();
    let shift = std::mem::take(&mut state.strip_shift);
    if std::mem::replace(&mut state.strip_h, strip_h) < strip_h {
        state.strip_shift = strip_h;
    }
    let width = view.width() - 2.0 * INSET;
    state.heights.for_width(width);
    state.pr_heights.for_width(width);
    let newest = rows.newest_seq().map_or(0, |seq| seq + 1) + rows.queued_len() as u64;
    if newest != state.seen_rows {
        state.seen_rows = newest;
        state.stick_pending = true;
    }
    if (view.height() - state.body_h).abs() > 0.5 {
        state.body_h = view.height();
        state.stick_pending = true;
    }
    let follow = state.scroll.following();
    let scroll_down = follow && std::mem::take(&mut state.stick_pending);
    state.refresh_turns(rows);
    let ViewState {
        heights,
        pr_heights,
        expanded,
        tidewatch_open,
        turns,
        zone,
        pastes,
        fresh_after_ms,
        ..
    } = state;
    let (_, _, layout) = turns.as_ref()?;
    let scene = Scene {
        root,
        pastes: pastes.as_deref(),
        profile,
        face,
        zone,
        now_ms: now_ms(),
        fresh_after_ms: *fresh_after_ms,
        running,
        waiting,
    };
    let mut bubbles: Vec<(BubbleAction, String, Option<u64>)> = Vec::new();
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(view));
    if let Some(id) = state.scroll_id.filter(|_| shift > 0.0)
        && let Some(mut saved) = egui::scroll_area::State::load(ui.ctx(), id)
    {
        saved.offset.y += shift;
        saved.store(ui.ctx(), id);
    }
    let out = ScrollArea::vertical()
        .id_salt(("reader-scroll", tab))
        .auto_shrink([false, false])
        .show(&mut body_ui, |ui| {
            if let Some(pixels) = dev_up {
                ui.scroll_with_delta(vec2(0.0, pixels));
            }
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
            let told = rows.timed();
            let gone = |place: &Place| match *place {
                Place::Row(index) => {
                    layout.slots[index] == Slot::Hidden
                        || (matches!(list[index].kind, Kind::Tidewatch(_))
                            && rows.said_for(list[index].seq).is_some_and(Record::dropped))
                }
                Place::Said(n) => told[n].dropped(),
                Place::Card(_) => false,
            };
            let shown = pr_timeline::places(rows, cards).into_iter().filter(|place| !gone(place));
            let quiet_of = |place: Place| match place {
                Place::Said(n) => Some(told[n]).filter(|record| record.quiet()),
                Place::Row(index) if matches!(list[index].kind, Kind::Tidewatch(_)) => {
                    rows.said_for(list[index].seq).filter(|record| record.quiet())
                }
                _ => None,
            };
            for entry in quiet_notes::fold(shown, quiet_of) {
                let top = ui.next_widget_position().y;
                let place = match entry {
                    Entry::Quiet(run) => {
                        let id = quiet_notes::key(&run);
                        let height = pr_heights.height_or(id, tidewatch_view::QUIET_GUESS_H);
                        if top + height < clip.top() - OVERSCAN || top > clip.bottom() + OVERSCAN {
                            ui.allocate_space(vec2(width, height));
                            continue;
                        }
                        let drawn = ui.horizontal(|ui| {
                            ui.add_space(INSET);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                let shown = tidewatch_view::Quiet {
                                    run: &run,
                                    zone: scene.zone,
                                    open: tidewatch_open.contains(&id),
                                };
                                tidewatch_view::quiet(ui, &shown)
                            })
                        });
                        pr_heights.record(id, drawn.response.rect.height());
                        let Clicked { toggled, click } = drawn.inner.inner;
                        if toggled {
                            reader_turns::toggle(tidewatch_open, id);
                        }
                        actions.extend(click.map(Action::Tidewatch));
                        continue;
                    }
                    Entry::Place(place) => place,
                };
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
                    Place::Said(index) => (Some(told[index].id), pr_heights.height(told[index].id)),
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
                                    _ if matches!(row.kind, Kind::Tidewatch(_)) => {
                                        if let Some(record) = rows.said_for(row.seq) {
                                            let shown = tidewatch_view::Shown {
                                                record,
                                                stamp: stamps[index],
                                                zone: scene.zone,
                                                open: tidewatch_open.contains(&record.id),
                                            };
                                            let Clicked { toggled, click } =
                                                tidewatch_view::show(ui, &shown);
                                            if toggled {
                                                reader_turns::toggle(tidewatch_open, record.id);
                                            }
                                            actions.extend(click.map(Action::Tidewatch));
                                        }
                                    }
                                    _ if row.kind == Kind::User => {
                                        let fresh = reader_chat_view::is_fresh(
                                            stamps[index],
                                            scene.fresh_after_ms,
                                        );
                                        let shown = (width, scene.pastes);
                                        let click = reader_chat_view::user(
                                            ui,
                                            row,
                                            stamps[index],
                                            scene.zone,
                                            shown,
                                            fresh,
                                        );
                                        if let Some(click) = click {
                                            bubbles.push((click, row.text.clone(), None));
                                        }
                                    }
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
                    (Place::Said(index), _) => {
                        let record = told[index];
                        let drawn = ui.horizontal(|ui| {
                            ui.add_space(INSET);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                let shown = tidewatch_view::Shown {
                                    record,
                                    stamp: Some(record.at_ms),
                                    zone: scene.zone,
                                    open: tidewatch_open.contains(&record.id),
                                };
                                tidewatch_view::show(ui, &shown)
                            })
                        });
                        if let Some(id) = key {
                            pr_heights.record(id, drawn.response.rect.height());
                        }
                        let Clicked { toggled, click } = drawn.inner.inner;
                        if toggled {
                            reader_turns::toggle(tidewatch_open, record.id);
                        }
                        actions.extend(click.map(Action::Tidewatch));
                    }
                }
            }
            let time = ui.input(|i| i.time);
            for (n, queued) in rows.queued().enumerate() {
                let stale = queued.not_seen(now_ms());
                let queue = if stale { Queue::NotSeen } else { Queue::Waiting(time) };
                let drawn = ui.horizontal(|ui| {
                    ui.add_space(INSET);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        reader_chat_view::queued(
                            ui,
                            &queued.text,
                            u64::MAX - n as u64,
                            queue,
                            width,
                        )
                    })
                });
                if let Some(click) = drawn.inner.inner {
                    bubbles.push((click, queued.text.clone(), Some(queued.id)));
                }
                if !stale {
                    ui.ctx().request_repaint_after(QUEUED_FRAME);
                }
            }
            if rows.auto_paused() {
                tidewatch_view::paused_line(ui, width);
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
    for click in bubbles {
        state.apply_bubble(click, actions);
    }
    let max_offset = (out.content_size.y - out.inner_rect.height()).max(0.0);
    state.scroll.settle(out.state.offset.y, max_offset);
    state.scroll_id = Some(out.id);
    if state.dev_hold {
        state.scroll.release();
    }
    #[cfg(test)]
    {
        state.settled_at = (out.state.offset.y, max_offset);
    }
    if (out.content_size.y - state.content_h).abs() > 0.5 {
        state.content_h = out.content_size.y;
        state.stick_pending = true;
        ui.ctx().request_repaint();
    }
    if let Some(slot) = jump.filter(|_| !state.scroll.following())
        && reader_chat_view::jump_pill(ui, slot, "Jump to latest").clicked()
    {
        state.scroll.jump_to_latest();
        state.stick_pending = true;
        ui.ctx().request_repaint();
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
        Kind::User | Kind::Tidewatch(_) | Kind::Assistant | Kind::Tool(_) => {}
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
    let answer: Vec<&str> = group.answers().iter().map(text_of).collect();
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

pub(crate) const REPLY_BLOCKED: &str = "Answer the prompt above first";

/// The reply box: the shared composer with the Reader's hint.
pub(crate) fn reply_box(
    ui: &mut Ui,
    draft: Draft<'_>,
    (enabled, blocked): (bool, Option<Blocked>),
    (commands, ghost, files): (&[SlashEntry], Option<&str>, Option<at_files::Source<'_>>),
    (working, held): (bool, usize),
) -> Outcome {
    let hint = match (enabled, blocked) {
        (_, Some(blocked)) => blocked.hint(),
        (true, None) => "Reply to the agent",
        (false, None) => REPLY_BLOCKED,
    };
    let mode = composer::Mode::Reply;
    let spec =
        Spec { id: reply_id(), hint, enabled, commands, ghost, mode, stop: working, held, files };
    composer::show(ui, draft, &spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bubble_toolbar::{Placed, Tool, placed_id, tool_id};
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
        let draft = Draft { text, focus, recall: None };
        super::reply_box(ui, draft, (enabled, None), (commands, ghost, None), (false, 0)).sent
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
            text: text_rect(text),
            send: text_rect(SEND),
        }
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
                    &Facts {
                        tab: 1,
                        source: None,
                        prompt: None,
                        ready: None,
                        root: None,
                        review: None,
                        pr: None,
                        due: None,
                        ask: None,
                        profile: "Builder",
                        face: None,
                        turn: Turn::Finished,
                        working: false,
                        reply_blocked: None,
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
            rows.push(Entry {
                role: Role::Assistant,
                text,
                tool: None,
                ts: None,
                images: Vec::new(),
            });
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
            Entry { role: Role::User, text: "hi".into(), tool: None, ts: None, images: Vec::new() },
            Entry {
                role: Role::Assistant,
                text: "# Title\n\n- a\n- b\n\n```rust\nfn x() {}\n```".into(),
                tool: None,
                ts: None,
                images: Vec::new(),
            },
            Entry {
                role: Role::System,
                text: "boom".into(),
                tool: None,
                ts: None,
                images: Vec::new(),
            },
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
                    ask: None,
                    profile: "Builder",
                    face: None,
                    turn: Turn::Running,
                    working: false,
                    reply_blocked: None,
                };
                let actions = show(ui, rect, &rows, &mut state, &facts);
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
        populated.push(Entry {
            role: Role::User,
            text: "hi".into(),
            tool: None,
            ts: None,
            images: Vec::new(),
        });
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
                        ask: None,
                        profile: "Builder",
                        face: None,
                        turn: Turn::Finished,
                        working: false,
                        reply_blocked: None,
                    };
                    let rect = ui.max_rect();
                    actions = show(ui, rect, rows, &mut state, &facts);
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
        rows.push(Entry {
            role: Role::User,
            text: "hi".into(),
            tool: None,
            ts: None,
            images: Vec::new(),
        });
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
                actions = show(ui, rect, &rows, &mut state, &facts);
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
    fn a_reply_box_of_an_agent_that_is_not_at_its_prompt_says_why_and_takes_no_text() {
        use tidewatch_core::transcript::{Entry, Role};

        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }

        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::User,
            text: "old question".into(),
            tool: None,
            ts: None,
            images: Vec::new(),
        });
        let ctx = crate::fonts::test_context();
        let mut state = ViewState::default();
        let mut hint = |blocked: Option<Blocked>| {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 700.0))),
                focused: true,
                ..RawInput::default()
            };
            let output = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                let mut facts = facts(None, Turn::Finished);
                facts.reply_blocked = blocked;
                show(ui, rect, &rows, &mut state, &facts);
            });
            let mut found = Vec::new();
            output.shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut found));
            found
        };
        hint(None);
        assert!(hint(None).contains(&"Reply to the agent".to_owned()));
        hint(Some(Blocked::Stopped));
        let blocked = hint(Some(Blocked::Stopped));
        assert!(blocked.contains(&"Resume first".to_owned()));
        assert!(!blocked.contains(&"Reply to the agent".to_owned()));
    }

    #[test]
    fn a_stopped_agent_gets_a_line_above_the_box_also_while_it_holds_text_and_resume_reaches_the_caller()
     {
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
        rows.push(Entry {
            role: Role::User,
            text: "old question".into(),
            tool: None,
            ts: None,
            images: Vec::new(),
        });
        let ctx = crate::fonts::test_context();
        let mut state = ViewState { reply: "half a thought".into(), ..ViewState::default() };
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
                facts.reply_blocked = stopped.then_some(Blocked::Stopped);
                actions = show(ui, rect, &rows, &mut state, &facts);
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
        assert!(at("Agent is not running").top() > at("old question").bottom());
        assert!(
            found.iter().any(|(text, _)| text == "half a thought"),
            "the draft stays in the box next to the reason"
        );
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
        let pasted = tidewatch_core::note_paste::format_review(
            &[&notes.threads[0]],
            "Start here",
            tidewatch_core::workflow_profile::Workflow::default(),
        );
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::User,
            text: pasted,
            tool: None,
            ts: None,
            images: Vec::new(),
        });
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
                    ask: None,
                    profile: "Builder",
                    face: None,
                    turn: Turn::Finished,
                    working: false,
                    reply_blocked: None,
                };
                let rect = ui.max_rect();
                show(ui, rect, &rows, &mut state, &facts);
            });
            shapes = format!("{:?}", out.shapes);
        }
        assert!(shapes.contains("Review"));
        assert!(shapes.contains("rename it"));
        assert!(!shapes.contains("Review from you"));
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
            ask: None,
            profile: "Builder",
            face: None,
            turn,
            working: false,
            reply_blocked: None,
        }
    }

    fn draw(rows: &Rows, state: &mut ViewState, facts: &Facts<'_>) -> Vec<Action> {
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
            images: Vec::new(),
        }
    }

    #[test]
    fn a_waiting_claude_question_shows_a_live_card_only_while_the_agent_is_mid_turn() {
        use tidewatch_core::transcript::ToolPhase;
        let mut rows = Rows::default();
        rows.push(question_entry(ToolPhase::Call));
        let mut state = ViewState::default();
        draw(&rows, &mut state, &facts(Some(Prompt::Question), Turn::Running));
        assert_eq!(state.live.as_ref().map(|live| live.seq), Some(0));
        let mut finished = ViewState::default();
        draw(&rows, &mut finished, &facts(None, Turn::Finished));
        assert!(finished.live.is_none(), "a finished agent has no menu open");
    }

    #[test]
    fn the_card_goes_away_when_the_question_is_answered_and_the_reply_box_asks_for_the_keyboard() {
        use crate::reader_rows::Asked;
        let mut rows = Rows::default();
        rows.push(question_entry(tidewatch_core::transcript::ToolPhase::Call));
        let mut state = ViewState::default();
        draw(&rows, &mut state, &facts(None, Turn::Running));
        state.focus_reply = false;
        rows.set_asked(0, Asked::Declined);
        draw(&rows, &mut state, &facts(None, Turn::Running));
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
            images: Vec::new(),
        });
        let mut state = ViewState::default();
        draw(&rows, &mut state, &facts(None, Turn::Finished));
        let labels: Vec<&str> =
            state.quick.as_ref().unwrap().1.iter().map(|q| q.label.as_str()).collect();
        assert_eq!(labels, ["Yes", "No"]);
        let mut working = ViewState::default();
        draw(&rows, &mut working, &facts(None, Turn::Running));
        assert!(working.quick.is_none(), "no chips while the agent works");
        state.quick_used = rows.newest_seq();
        draw(&rows, &mut state, &facts(None, Turn::Finished));
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
        actions: Vec<Action>,
        working: bool,
        blocked: Option<Blocked>,
        width: f32,
        copied: Vec<String>,
        time: Option<f64>,
    }

    impl Frames {
        fn new() -> Self {
            Self {
                ctx: crate::fonts::test_context(),
                state: ViewState::default(),
                shapes: Vec::new(),
                actions: Vec::new(),
                working: false,
                blocked: None,
                width: 1000.0,
                copied: Vec::new(),
                time: None,
            }
        }

        fn run(&mut self, rows: &Rows, turn: Turn, events: Vec<Event>) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, 800.0))),
                events,
                focused: true,
                time: self.time,
                ..RawInput::default()
            };
            let state = &mut self.state;
            let (working, blocked) = (self.working, self.blocked);
            let mut asked = Vec::new();
            let output = self.ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                let facts = Facts { working, reply_blocked: blocked, ..facts(None, turn) };
                asked = show(ui, rect, rows, state, &facts);
            });
            self.actions.extend(asked);
            self.copied.extend(output.platform_output.commands.iter().filter_map(|command| {
                match command {
                    egui::OutputCommand::CopyText(text) => Some(text.clone()),
                    _ => None,
                }
            }));
            self.shapes.clear();
            output.shapes.iter().for_each(|clipped| shape_texts(&clipped.shape, &mut self.shapes));
        }

        fn at(&self, text: &str) -> Option<egui::Pos2> {
            self.shapes.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| rect.center())
        }

        fn tool(&self, seq: u64, tool: Tool) -> Option<egui::Pos2> {
            self.ctx.read_response(tool_id(seq, tool)).map(|response| response.rect.center())
        }

        fn toolbar(&self, seq: u64) -> Option<Placed> {
            self.ctx.data(|data| data.get_temp::<Placed>(placed_id(seq)))
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
            images: Vec::new(),
        };
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::User,
            text: "tidy up".into(),
            tool: None,
            ts: Some(1_700_000_000_000),
            images: Vec::new(),
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
            images: Vec::new(),
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
        let say = |role, text: &str| Entry {
            role,
            text: text.into(),
            tool: None,
            ts: at,
            images: Vec::new(),
        };
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
            images: Vec::new(),
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
            images: Vec::new(),
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
            images: Vec::new(),
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
            images: Vec::new(),
        });
        assert_eq!(
            permission_tool(&rows, None),
            Some(("Bash".to_owned(), "cargo test".to_owned()))
        );
        let mut state = ViewState::default();
        let actions = draw(&rows, &mut state, &facts(Some(Prompt::Permission), Turn::Running));
        assert_eq!(actions, Vec::<Action>::new());
    }

    fn conversation(turns: usize, words: usize) -> Rows {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        for n in 0..turns {
            let ask = format!("question {n}");
            rows.push(Entry {
                role: Role::User,
                text: ask,
                tool: None,
                ts: None,
                images: Vec::new(),
            });
            let text = format!("answer {n}: {}", "word ".repeat(words + n % 7 * 30));
            rows.push(Entry {
                role: Role::Assistant,
                text,
                tool: None,
                ts: None,
                images: Vec::new(),
            });
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
                let _ = show(ui, rect, rows, state, &facts);
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
            rows.push(Entry { role, text, tool: None, ts: None, images: Vec::new() });
        }
        rows.push(Entry {
            role: Role::Assistant,
            text: "the newest message".into(),
            tool: None,
            ts: None,
            images: Vec::new(),
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
                actions = show(ui, column, rows, state, &facts(None, Turn::Finished));
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
                heights.push(composer::height(ui, text, column));
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
                show(ui, rect, &rows, &mut state, &facts);
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
                show(ui, rect, rows, &mut state, &facts(None, Turn::Running));
            });
            found.clear();
            output.shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut found));
        }
        found
    }

    fn auto_fix_record(text: &str, at_ms: i64) -> tidewatch_core::sent::Record {
        use tidewatch_core::sent::{Cap, Reason, Record};
        Record {
            at_ms,
            ..Record::new(
                Reason::AutoFix,
                tidewatch_core::sent::because::AUTO_CONFLICTS,
                text,
                Some(Cap { n: 1, max: 10 }),
            )
        }
    }

    #[test]
    fn a_message_tidewatch_typed_is_its_own_speaker_and_never_a_you_bubble() {
        use tidewatch_core::transcript::{Entry, Role};
        let prompt = "@codex commented on PR #7. Answer it.\n\n> <!-- codex-review-state:v1 -->\n> | a | b |\n> |---|---|\n> <details>";
        let mut rows = Rows::default();
        rows.tidewatch_said(vec![auto_fix_record(prompt, 0)]);
        rows.push(Entry {
            role: Role::User,
            text: prompt.into(),
            tool: None,
            ts: Some(5_000),
            images: Vec::new(),
        });
        let found = drawn_texts(&rows, 900.0);
        assert!(found.iter().any(|text| text == "Tidewatch"), "{found:?}");
        assert!(found.iter().any(|text| text == "auto"), "{found:?}");
        assert!(found.iter().any(|text| text.starts_with("@codex commented")), "{found:?}");
        assert!(
            found
                .iter()
                .any(|text| text == "Because Auto fixes conflicts on this PR \u{b7} 1 of 10"),
            "{found:?}"
        );
        assert!(found.iter().any(|text| text == "Change workflow"), "{found:?}");
        assert!(found.iter().any(|text| text == "Turn off for conflicts"), "{found:?}");
        assert!(found.iter().all(|text| !text.starts_with("You")), "{found:?}");
        assert!(found.iter().all(|text| !text.contains("codex-review-state")), "{found:?}");
    }

    #[test]
    fn a_reminder_an_older_version_sent_leaves_no_row_while_a_live_one_stays() {
        use tidewatch_core::sent::{Reason, Record};
        use tidewatch_core::transcript::{Entry, Role};
        let old = Record {
            typed: true,
            at_ms: 0,
            ..Record::new(
                Reason::LedgerGuard,
                "you answered and the progress report should show it",
                "The user answered; call update_progress so your report shows the new state.",
                None,
            )
        };
        let live = Record {
            typed: true,
            at_ms: 1_000,
            ..Record::new(
                Reason::LedgerGuard,
                tidewatch_core::sent::because::LEDGER_STOP,
                "Before you stop: send your progress report with the final state.",
                None,
            )
        };
        let mut rows = Rows::default();
        rows.tidewatch_said(vec![old.clone(), live.clone()]);
        for (record, at) in [(&old, 5_000), (&live, 6_000)] {
            rows.push(Entry {
                role: Role::User,
                text: record.text.clone(),
                tool: None,
                ts: Some(at),
                images: Vec::new(),
            });
        }
        let found = drawn_texts(&rows, 900.0);
        let said = |needle: &str| found.iter().any(|text| text.contains(needle));
        assert!(!said("The user answered"), "{found:?}");
        assert!(!said("progress report should show it"), "{found:?}");
        assert!(!said("You"), "no reminder falls back to a You bubble: {found:?}");
        assert!(
            found.iter().all(|text| text != "2"),
            "one live reminder is no run of two: {found:?}"
        );
        assert!(said("Tidewatch reminded the agent to update its report"), "{found:?}");
    }

    #[test]
    fn a_long_tidewatch_message_offers_show_all() {
        use tidewatch_core::transcript::{Entry, Role};
        let long = "Fix the failing check. ".repeat(40);
        let mut rows = Rows::default();
        rows.tidewatch_said(vec![auto_fix_record(&long, 0)]);
        rows.push(Entry {
            role: Role::User,
            text: long.clone(),
            tool: None,
            ts: Some(5_000),
            images: Vec::new(),
        });
        let found = drawn_texts(&rows, 900.0);
        assert!(found.iter().any(|text| text == "Show all"), "{found:?}");
    }

    #[test]
    fn a_message_sent_while_the_agent_is_busy_shows_queued_until_the_transcript_has_it() {
        use tidewatch_core::transcript::{Entry, Role};
        let now = now_ms();
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::User,
            text: "start".into(),
            tool: None,
            ts: Some(now),
            images: Vec::new(),
        });
        rows.queue_user("then also this", now);
        let found = drawn_texts(&rows, 900.0);
        assert!(found.iter().any(|text| text == "Queued"), "{found:?}");
        assert!(found.iter().any(|text| text == "then also this"), "{found:?}");
        rows.push(Entry {
            role: Role::User,
            text: "then also this".into(),
            tool: None,
            ts: Some(now + 1),
            images: Vec::new(),
        });
        let found = drawn_texts(&rows, 900.0);
        assert!(found.iter().all(|text| text != "Queued"), "{found:?}");
        assert_eq!(found.iter().filter(|text| *text == "then also this").count(), 1, "{found:?}");
    }
    #[test]
    fn a_queued_message_with_pictures_shows_once_with_the_reply_after_it() {
        use tidewatch_core::transcript::{Entry, Role};
        use tidewatch_core::transcript_image::Picture;
        let now = now_ms();
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::User,
            text: "start".into(),
            tool: None,
            ts: Some(now),
            images: Vec::new(),
        });
        rows.queue_user("why these picks?\n/tmp/a.png\n/tmp/b.png", now);
        assert!(drawn_texts(&rows, 900.0).iter().any(|text| text == "Queued"));
        rows.push(Entry {
            role: Role::User,
            text: "[Image #3] [Image #4]why these picks?".into(),
            tool: None,
            ts: Some(now + 1),
            images: vec![
                Picture::new(Some(3), "image/png", "AAAA"),
                Picture::new(Some(4), "image/png", "BBBB"),
            ],
        });
        rows.push(Entry {
            role: Role::Assistant,
            text: "Because of your history".into(),
            tool: None,
            ts: Some(now + 2),
            images: Vec::new(),
        });
        let found = drawn_texts(&rows, 900.0);
        assert!(found.iter().all(|text| text != "Queued"), "{found:?}");
        assert_eq!(rows.queued_len(), 0);
        let said = |wanted: &str| found.iter().position(|text| text.contains(wanted));
        assert_eq!(
            found.iter().filter(|text| text.contains("why these picks?")).count(),
            1,
            "{found:?}"
        );
        assert!(said("why these picks?") < said("Because of your history"), "{found:?}");
    }

    #[test]
    fn the_jump_pill_gets_a_strip_of_its_own_and_never_covers_a_row() {
        let body = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0));
        assert_eq!(jump_split(body, true), (body, None));
        let (rows, slot) = jump_split(body, false);
        let slot = slot.unwrap();
        assert!(body.contains_rect(rows) && body.contains_rect(slot));
        assert!(!rows.intersects(slot), "{rows:?} {slot:?}");
        assert_eq!(rows.top(), body.top());
        let short = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 90.0));
        assert_eq!(jump_split(short, false).0, short, "no room to spare, so it floats");
    }

    fn visible_texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        for clipped in &output.shapes {
            let mut found = Vec::new();
            shape_texts(&clipped.shape, &mut found);
            for (text, rect) in found {
                let seen = rect.intersect(clipped.clip_rect);
                if seen.is_positive() {
                    out.push((text, seen));
                }
            }
        }
        out
    }

    #[test]
    fn the_waiting_row_is_whole_above_the_reply_box_and_the_jump_pill_does_not_cover_it() {
        use crate::pr_watch::{Pending, Watch};
        use tidewatch_core::pr_cards::Cards;
        use tidewatch_core::pr_status::Body;
        use tidewatch_core::prefs::{GithubPrefs, PrMode};
        let comment = Body::Commented {
            trusted: true,
            author: "rev".into(),
            text: "why?".into(),
            url: String::new(),
        };
        let mut watch = Watch::new(std::path::PathBuf::from("/x"));
        watch.take_load();
        watch.loaded(Cards::default(), 0.0);
        let message = tidewatch_core::pr_prompt::message(
            &comment,
            42,
            tidewatch_core::workflow_profile::Workflow::default(),
        )
        .unwrap();
        watch.pending.push(Pending {
            id: 1,
            message,
            body: comment.clone(),
            number: 42,
            confirm: false,
        });
        let rows = conversation(20, 40);
        let ctx = crate::fonts::test_context();
        let mut state = ViewState::default();
        let mut time = 0.0;
        let mut frame = |events: Vec<Event>| {
            time += 0.016;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0))),
                events,
                focused: true,
                time: Some(time),
                ..RawInput::default()
            };
            let pr = PrData {
                watch: &watch,
                profile: "Builder",
                handling: crate::pr_watch::Handling::Answers,
                policy: GithubPrefs { mode: PrMode::Auto, ..GithubPrefs::default() },
                now_ms: 0,
            };
            let output = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                let facts = Facts { pr: Some(pr), ..facts(None, Turn::Running) };
                let _ = show(ui, rect, &rows, &mut state, &facts);
            });
            visible_texts(&output)
        };
        let row = "Will ask Builder to answer the comment when it finishes this turn";
        let mut texts = Vec::new();
        for _ in 0..12 {
            texts = frame(vec![]);
        }
        let find = |texts: &[(String, Rect)], want: &str| {
            texts.iter().find(|(text, _)| text == want).map(|(_, rect)| *rect)
        };
        let waiting = find(&texts, row).expect("the waiting row shows at the bottom");
        let reply = find(&texts, "Reply to the agent").expect("the reply box shows");
        assert!(waiting.bottom() < reply.top(), "{waiting:?} above {reply:?}");
        assert_eq!(find(&texts, "Jump to latest"), None, "following: no pill");
        frame(vec![Event::PointerMoved(pos2(300.0, 200.0))]);
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 25.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        frame(vec![wheel]);
        for _ in 0..4 {
            texts = frame(vec![]);
        }
        let still =
            find(&texts, row).expect("the pill's strip takes room from the top, not the row");
        assert_eq!(still.height(), waiting.height(), "the row is whole, not cut at the strip");
        let pill = find(&texts, "Jump to latest").expect("scrolled up: the pill shows");
        let covered = pill.expand2(vec2(12.0, 6.0));
        for (text, rect) in &texts {
            if text != "Jump to latest" {
                assert!(!rect.intersects(covered), "{text:?} {rect:?} is under the pill {pill:?}");
            }
        }
    }

    const PICTURES: &str = include_str!("../../tidewatch-core/tests/fixtures/claude_images.jsonl");

    #[test]
    fn files_put_into_the_reply_box_land_at_its_cursor_and_the_box_asks_for_the_keyboard() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(RawInput { focused: false, ..RawInput::default() }, |_| {});
        let mut state = ViewState::default();
        state.set_sentence(&ctx, "Check | and run");
        state.focus_reply = false;
        state.insert_files(&ctx, vec![PathBuf::from("/s/a b.png"), PathBuf::from("/s/c.pdf")]);
        assert_eq!(tokens::sendable(&state.reply), "Check '/s/a b.png' /s/c.pdf and run");
        assert!(state.focus_reply);
    }

    #[test]
    fn a_sent_message_draws_its_pictures_as_chips_where_their_markers_stood() {
        use crate::pictures::{self, Source};
        use tidewatch_core::transcript::{Source as Cli, parse_all};
        let ctx = crate::fonts::test_context();
        let mut rows = Rows::default();
        rows.push_all(parse_all(Cli::Claude, PICTURES));
        let mut state = ViewState::default();
        for row in rows.stamped().map(|(row, _)| row) {
            for picture in &row.pictures {
                pictures::wait_ready(&ctx, Source::Pasted(picture));
            }
        }
        let mut shapes = Vec::new();
        for _ in 0..3 {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                focused: true,
                ..RawInput::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                let rect = ui.max_rect();
                show(ui, rect, &rows, &mut state, &facts(None, Turn::Finished));
            });
            shapes.clear();
            for clipped in &out.shapes {
                flat(&clipped.shape, &mut shapes);
            }
        }
        let mut chips: Vec<Rect> = shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Rect(rect)
                    if rect.fill == brand::SUNKEN
                        && rect.rect.height() == crate::screen_kit::Size::Tag.height() =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .collect();
        chips.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
        assert_eq!(chips.len(), 3, "{chips:?}");
        let texts: Vec<(Rect, String)> = shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Text(text) => Some((
                    text.galley.rect.translate(text.pos.to_vec2()),
                    text.galley.text().to_owned(),
                )),
                _ => None,
            })
            .collect();
        let label = |at: &str| texts.iter().find(|(_, text)| text == at).map(|(rect, _)| *rect);
        for chip in &chips {
            assert!(
                chips.iter().all(|other| other == chip || !other.intersects(*chip)),
                "{chips:?}"
            );
        }
        for name in ["Image 1", "Image 2", "Image 5"] {
            assert!(label(name).is_some(), "{name} chip missing in {texts:?}");
        }
        let sentences: Vec<&(Rect, String)> =
            texts.iter().filter(|(_, text)| tokens::has_tokens(text)).collect();
        assert_eq!(sentences.len(), 2, "two messages hold chips: {texts:?}");
        for chip in &chips {
            assert!(
                sentences.iter().any(|(rect, _)| rect.contains_rect(*chip)),
                "{chip:?} is not inside its sentence {sentences:?}"
            );
        }
        let with_words =
            |words: &str| sentences.iter().filter(|(_, text)| text.contains(words)).count();
        assert_eq!(with_words("Compare"), 1, "{sentences:?}");
        assert_eq!(with_words(" with the old one"), 1, "{sentences:?}");
        assert_eq!(with_words("What is"), 1, "{sentences:?}");
        assert!(!texts.iter().any(|(_, text)| text.contains("[Image")), "{texts:?}");
        assert!(
            !texts.iter().any(|(_, text)| text.contains("use of widget ID")),
            "one picture in two messages clashes ids: {texts:?}"
        );
    }

    fn user_rows(text: &str) -> Rows {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = Rows::default();
        rows.push(Entry {
            role: Role::User,
            text: text.into(),
            tool: None,
            ts: Some(now_ms()),
            images: Vec::new(),
        });
        rows
    }

    fn key_press(key: egui::Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn settle(frames: &mut Frames, rows: &Rows) {
        for _ in 0..3 {
            frames.run(rows, Turn::Running, vec![]);
        }
    }

    fn click_text(frames: &mut Frames, rows: &Rows, text: &str) {
        let at = frames.at(text).unwrap_or_else(|| panic!("{text}: {:?}", frames.shapes));
        click_at(frames, rows, at);
    }

    fn click_tool(frames: &mut Frames, rows: &Rows, seq: u64, tool: Tool) {
        let at = frames.tool(seq, tool).unwrap_or_else(|| panic!("{tool:?} is not drawn"));
        click_at(frames, rows, at);
    }

    fn click_at(frames: &mut Frames, rows: &Rows, at: egui::Pos2) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frames.run(rows, Turn::Running, vec![Event::PointerMoved(at)]);
        frames.run(rows, Turn::Running, vec![button(true)]);
        frames.run(rows, Turn::Running, vec![button(false)]);
        settle(frames, rows);
    }

    const HELD_NOTE: &str = "Held by Tidewatch \u{b7} sent when the agent finishes its turn";

    #[test]
    fn held_messages_sit_at_the_bottom_of_the_chat_above_the_box_with_three_buttons() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        frames.working = true;
        frames.state.line.hold("also check the Android build before you push", 0).unwrap();
        settle(&mut frames, &rows);
        for want in [HELD_NOTE, "Edit", "Delete", "Send now"] {
            assert!(frames.at(want).is_some(), "{want}: {:?}", frames.shapes);
        }
        let held = frames.at("also check the Android build before you push").unwrap();
        let last_row = frames.at("All tidy.").unwrap();
        let hint = frames.at("Reply to the agent").unwrap();
        assert!(last_row.y < held.y && held.y < hint.y, "{last_row:?} {held:?} {hint:?}");
        assert!(frames.at("Up").is_none(), "one message has nowhere to move");
    }

    #[test]
    fn nothing_is_drawn_for_an_empty_line() {
        let mut frames = Frames::new();
        frames.working = true;
        settle(&mut frames, &turn_rows());
        assert!(frames.at(HELD_NOTE).is_none());
        assert!(frames.at("Send now").is_none());
    }

    #[test]
    fn delete_takes_the_message_out_and_send_now_asks_the_caller_for_that_message() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        frames.working = true;
        let id = frames.state.line.hold("first", 0).unwrap();
        settle(&mut frames, &rows);
        click_text(&mut frames, &rows, "Send now");
        assert_eq!(frames.actions, [Action::SendNow(id)]);
        click_text(&mut frames, &rows, "Delete");
        assert_eq!(frames.state.line.len(), 0);
        assert!(frames.at(HELD_NOTE).is_none(), "the card goes with its last message");
    }

    #[test]
    fn edit_opens_the_row_where_it_is_and_save_keeps_the_place_and_cancel_leaves_the_text() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        frames.working = true;
        let first = frames.state.line.hold("first", 0).unwrap();
        frames.state.line.hold("second", 0).unwrap();
        settle(&mut frames, &rows);
        let edits: Vec<_> = frames.shapes.iter().filter(|(text, _)| text == "Edit").collect();
        assert_eq!(edits.len(), 2);
        let top = edits.iter().map(|(_, rect)| rect.center()).min_by(|a, b| a.y.total_cmp(&b.y));
        let at = top.unwrap();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frames.run(&rows, Turn::Running, vec![Event::PointerMoved(at)]);
        frames.run(&rows, Turn::Running, vec![button(true)]);
        frames.run(&rows, Turn::Running, vec![button(false)]);
        settle(&mut frames, &rows);
        assert_eq!(frames.state.editing_held(), Some(first));
        assert!(frames.at("Save").is_some() && frames.at("Cancel").is_some());
        frames.state.held_edit = Some((first, "first, reworded".into()));
        settle(&mut frames, &rows);
        click_text(&mut frames, &rows, "Save");
        let texts: Vec<&str> =
            frames.state.line.items().iter().map(|held| held.text.as_str()).collect();
        assert_eq!(texts, ["first, reworded", "second"]);
        assert_eq!(frames.state.editing_held(), None);
        frames.state.held_edit = Some((first, "never mind".into()));
        settle(&mut frames, &rows);
        click_text(&mut frames, &rows, "Cancel");
        assert_eq!(frames.state.line.text_of(first), Some("first, reworded"));
        assert_eq!(frames.state.editing_held(), None);
    }

    #[test]
    fn enter_in_an_open_edit_saves_it_and_a_blank_text_deletes_the_message() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        frames.working = true;
        let id = frames.state.line.hold("first", 0).unwrap();
        frames.state.focus_reply = false;
        frames.state.begin_edit_held(id);
        settle(&mut frames, &rows);
        frames.state.held_edit = Some((id, "saved with enter".into()));
        frames.run(&rows, Turn::Running, vec![key_press(egui::Key::Enter)]);
        settle(&mut frames, &rows);
        assert_eq!(frames.state.line.text_of(id), Some("saved with enter"));
        frames.state.begin_edit_held(id);
        settle(&mut frames, &rows);
        frames.state.held_edit = Some((id, "  ".into()));
        frames.run(&rows, Turn::Running, vec![key_press(egui::Key::Enter)]);
        settle(&mut frames, &rows);
        assert_eq!(frames.state.line.len(), 0);
    }

    #[test]
    fn messages_move_up_and_down_and_the_arrows_show_only_where_there_is_a_place_to_go() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        frames.working = true;
        let ids: Vec<u64> =
            ["a", "b", "c"].iter().map(|text| frames.state.line.hold(text, 0).unwrap()).collect();
        let mut actions = Vec::new();
        frames.state.apply_held(
            held_view::Out::Shift(ids[2], crate::send_line::Way::Earlier),
            &mut actions,
        );
        let order: Vec<&str> =
            frames.state.line.items().iter().map(|held| held.text.as_str()).collect();
        assert_eq!(order, ["a", "c", "b"]);
        assert_eq!(actions, Vec::new());
        settle(&mut frames, &rows);
        assert!(frames.at("Send now").is_some());
    }

    #[test]
    fn up_in_the_empty_box_edits_the_newest_held_message_and_otherwise_recalls_history() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        frames.working = true;
        frames.state.line.hold("older", 0).unwrap();
        let newest = frames.state.line.hold("newest", 0).unwrap();
        frames.state.recall = Recall::with_entries(vec!["sent before".into()]);
        settle(&mut frames, &rows);
        frames.run(&rows, Turn::Running, vec![key_press(egui::Key::ArrowUp)]);
        assert_eq!(frames.state.editing_held(), Some(newest));
        assert_eq!(frames.state.reply, "");
        let mut plain = Frames::new();
        plain.state.recall = Recall::with_entries(vec!["sent before".into()]);
        settle(&mut plain, &rows);
        plain.run(&rows, Turn::Running, vec![key_press(egui::Key::ArrowUp)]);
        assert_eq!(plain.state.reply, "sent before");
    }

    #[test]
    fn the_stop_button_and_esc_in_an_empty_box_ask_for_a_stop_only_while_the_agent_works() {
        let rows = turn_rows();
        let mut idle = Frames::new();
        settle(&mut idle, &rows);
        assert!(idle.at("Stop").is_none());
        let mut working = Frames::new();
        working.working = true;
        settle(&mut working, &rows);
        working.run(&rows, Turn::Running, vec![key_press(egui::Key::Escape)]);
        assert_eq!(working.actions, [Action::Stop]);
        working.actions.clear();
        click_text(&mut working, &rows, "Stop");
        assert_eq!(working.actions, [Action::Stop]);
    }

    #[test]
    fn sending_follows_the_newest_message_again_even_after_scrolling_up() {
        let rows = turn_rows();
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        frames.state.scroll.release();
        assert!(!frames.state.scroll.following());
        frames.run(&rows, Turn::Finished, vec![Event::Text("hello".into())]);
        frames.run(&rows, Turn::Finished, vec![key_press(egui::Key::Enter)]);
        assert_eq!(frames.actions, [Action::Reply("hello".into())]);
        assert!(frames.state.scroll.following());
    }

    const NOT_SEEN_SEQ: u64 = u64::MAX;

    fn first_seq(rows: &Rows) -> u64 {
        rows.stamped().next().expect("a row").0.seq
    }

    fn point_at(frames: &mut Frames, rows: &Rows, at: egui::Pos2) {
        frames.run(rows, Turn::Finished, vec![Event::PointerMoved(at)]);
        settle(frames, rows);
    }

    #[test]
    fn a_message_the_agent_never_showed_reads_not_seen_and_can_be_sent_again_or_dismissed() {
        let mut rows = user_rows("start");
        rows.queue_user("lost one", now_ms() - 3 * 60_000);
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        assert!(frames.at("Not seen by the agent").is_some(), "{:?}", frames.shapes);
        assert!(frames.at("Queued").is_none());
        for tool in [Tool::SendAgain, Tool::Dismiss] {
            assert!(frames.tool(NOT_SEEN_SEQ, tool).is_some(), "{tool:?} is always shown");
        }
        click_tool(&mut frames, &rows, NOT_SEEN_SEQ, Tool::SendAgain);
        let id = rows.queued().next().unwrap().id;
        assert_eq!(
            frames.actions,
            [Action::SendAgain { text: "lost one".into(), forget: Some(id) }]
        );
        frames.actions.clear();
        click_tool(&mut frames, &rows, NOT_SEEN_SEQ, Tool::Dismiss);
        assert_eq!(frames.actions, [Action::Forget(id)]);
    }

    #[test]
    fn a_message_that_is_still_fresh_reads_queued_with_no_buttons() {
        let mut rows = user_rows("start");
        rows.queue_user("just sent", now_ms());
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        assert!(frames.at("Queued").is_some());
        assert_eq!(frames.toolbar(NOT_SEEN_SEQ).map(|placed| placed.toolbar), None);
        assert!(frames.tool(NOT_SEEN_SEQ, Tool::SendAgain).is_none());
        assert!(frames.tool(NOT_SEEN_SEQ, Tool::Dismiss).is_none());
    }

    #[test]
    fn pointing_at_your_own_bubble_offers_edit_copy_and_send_again_as_icons() {
        let rows = user_rows("run the tests again");
        let seq = first_seq(&rows);
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        for tool in [Tool::EditAndSend, Tool::Copy, Tool::SendAgain] {
            assert!(frames.tool(seq, tool).is_none(), "{tool:?} hidden until the pointer is near");
        }
        let over = frames.at("run the tests again").unwrap();
        point_at(&mut frames, &rows, over);
        let drawn: Vec<Tool> = [Tool::EditAndSend, Tool::Copy, Tool::SendAgain]
            .into_iter()
            .filter(|tool| frames.tool(seq, *tool).is_some())
            .collect();
        assert_eq!(drawn, [Tool::EditAndSend, Tool::Copy, Tool::SendAgain]);
        for label in ["Send again", "Copy", "Edit and send"] {
            assert!(frames.at(label).is_none(), "{label} is an icon, not a printed label");
        }
        click_tool(&mut frames, &rows, seq, Tool::SendAgain);
        assert_eq!(
            frames.actions,
            [Action::SendAgain { text: "run the tests again".into(), forget: None }]
        );
    }

    #[test]
    fn each_icon_names_itself_in_a_tooltip() {
        let rows = user_rows("run the tests again");
        let seq = first_seq(&rows);
        let mut frames = Frames::new();
        let mut clock = 0.0;
        frames.time = Some(clock);
        settle(&mut frames, &rows);
        let over = frames.at("run the tests again").unwrap();
        point_at(&mut frames, &rows, over);
        for (tool, name) in [
            (Tool::EditAndSend, "Edit and send"),
            (Tool::Copy, "Copy"),
            (Tool::SendAgain, "Send again"),
        ] {
            let at = frames.tool(seq, tool).unwrap();
            point_at(&mut frames, &rows, at);
            for _ in 0..12 {
                clock += 1.0;
                frames.time = Some(clock);
                frames.run(&rows, Turn::Finished, vec![]);
            }
            assert!(frames.at(name).is_some(), "{name}: {:?}", frames.shapes);
        }
    }

    #[test]
    fn the_toolbar_sits_under_the_bubble_and_stops_short_of_the_next_message() {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = user_rows("first message");
        rows.push(Entry {
            role: Role::User,
            text: "second message".into(),
            tool: None,
            ts: Some(now_ms()),
            images: Vec::new(),
        });
        let seq = first_seq(&rows);
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        let over = frames.at("first message").unwrap();
        point_at(&mut frames, &rows, over);
        let placed = frames.toolbar(seq).expect("the toolbar is drawn while the pointer is on it");
        let next = frames
            .shapes
            .iter()
            .find(|(text, _)| text == "second message")
            .map(|(_, rect)| *rect)
            .expect("the next message is drawn");
        assert_eq!(placed.toolbar.right(), placed.bubble.right());
        assert!(placed.toolbar.top() >= placed.bubble.bottom(), "{placed:?} over the bubble");
        assert!(placed.toolbar.top() - placed.bubble.bottom() <= 4.0, "{placed:?} far under it");
        assert!(placed.toolbar.bottom() <= next.top(), "{placed:?} over the next message {next:?}");
    }

    #[test]
    fn the_toolbar_never_covers_the_next_messages_you_line() {
        use tidewatch_core::transcript::{Entry, Role};
        let mut rows = user_rows("first message");
        rows.push(Entry {
            role: Role::User,
            text: "second message".into(),
            tool: None,
            ts: Some(now_ms()),
            images: Vec::new(),
        });
        let seq = first_seq(&rows);
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        let over = frames.at("first message").unwrap();
        point_at(&mut frames, &rows, over);
        let placed = frames.toolbar(seq).expect("the toolbar is drawn");
        let lines: Vec<Rect> = frames
            .shapes
            .iter()
            .filter(|(text, _)| text.starts_with("You"))
            .map(|(_, rect)| *rect)
            .collect();
        assert_eq!(lines.len(), 2, "{:?}", frames.shapes);
        let next = lines.iter().max_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
        assert!(
            placed.toolbar.bottom() <= next.top(),
            "{:?} over the next You line {next:?}",
            placed.toolbar
        );
    }

    #[test]
    fn the_toolbar_of_the_last_message_ends_above_the_reply_box() {
        let rows = user_rows("run the tests again");
        let seq = first_seq(&rows);
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        let over = frames.at("run the tests again").unwrap();
        point_at(&mut frames, &rows, over);
        let placed = frames.toolbar(seq).expect("the toolbar is drawn");
        let reply = frames.shapes.iter().find(|(text, _)| text == "Reply to the agent").unwrap().1;
        assert!(placed.toolbar.bottom() <= reply.top(), "{:?} over {reply:?}", placed.toolbar);
    }

    #[test]
    fn moving_from_the_bubble_down_to_the_toolbar_keeps_it_shown_and_leaving_hides_it() {
        let rows = user_rows("run the tests again");
        let seq = first_seq(&rows);
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        let over = frames.at("run the tests again").unwrap();
        point_at(&mut frames, &rows, over);
        let placed = frames.toolbar(seq).unwrap();
        let halfway = pos2(placed.toolbar.center().x, placed.bubble.bottom() + 1.0);
        point_at(&mut frames, &rows, halfway);
        assert!(frames.tool(seq, Tool::Copy).is_some(), "still shown in the gap");
        let on_it = frames.tool(seq, Tool::Copy).unwrap();
        point_at(&mut frames, &rows, on_it);
        assert!(frames.tool(seq, Tool::EditAndSend).is_some(), "still shown on the toolbar");
        point_at(&mut frames, &rows, pos2(20.0, 700.0));
        assert!(frames.tool(seq, Tool::Copy).is_none(), "hidden once the pointer is away");
    }

    #[test]
    fn copy_puts_the_message_text_on_the_clipboard_and_asks_the_agent_for_nothing() {
        let rows = user_rows("run the tests again");
        let seq = first_seq(&rows);
        let mut frames = Frames::new();
        settle(&mut frames, &rows);
        let over = frames.at("run the tests again").unwrap();
        point_at(&mut frames, &rows, over);
        click_tool(&mut frames, &rows, seq, Tool::Copy);
        assert_eq!(frames.copied, ["run the tests again"]);
        assert_eq!(frames.actions, Vec::<Action>::new());
    }

    #[test]
    fn edit_and_send_puts_the_message_in_the_box_and_keeps_the_draft_for_after() {
        let rows = user_rows("run the tests again");
        let seq = first_seq(&rows);
        let mut frames = Frames::new();
        frames.state.reply = "my draft".into();
        settle(&mut frames, &rows);
        let over = frames.at("run the tests again").unwrap();
        point_at(&mut frames, &rows, over);
        click_tool(&mut frames, &rows, seq, Tool::EditAndSend);
        assert_eq!(frames.state.reply, "run the tests again");
        frames.run(&rows, Turn::Finished, vec![key_press(egui::Key::Enter)]);
        assert_eq!(frames.actions, [Action::Reply("run the tests again".into())]);
        assert_eq!(frames.state.reply, "my draft");
    }

    #[test]
    fn the_reason_line_names_each_blocked_state_and_only_a_stopped_or_failed_agent_offers_resume() {
        let rows = turn_rows();
        let cases = [
            (Blocked::Stopped, "Agent is not running", true),
            (Blocked::Failed, "The agent failed", true),
            (Blocked::SettingUp, "The agent is starting up", false),
            (Blocked::Shell, "No agent is running here", false),
        ];
        for (blocked, line, resume) in cases {
            let mut frames = Frames::new();
            frames.blocked = Some(blocked);
            settle(&mut frames, &rows);
            assert!(frames.at(line).is_some(), "{line}: {:?}", frames.shapes);
            assert_eq!(frames.at("Resume").is_some(), resume, "{line}");
        }
    }

    #[test]
    fn a_narrow_column_keeps_each_held_row_inside_the_card_with_its_text_clear_of_the_buttons() {
        let rows = turn_rows();
        for width in [260.0, 340.0, 600.0] {
            let mut frames = Frames::new();
            frames.width = width;
            frames.working = true;
            frames.state.line.hold("also check the Android build before you push", 0).unwrap();
            frames.state.line.hold("then update the changelog", 0).unwrap();
            settle(&mut frames, &rows);
            let rect_of =
                |text: &str| frames.shapes.iter().find(|(drawn, _)| drawn == text).map(|(_, r)| *r);
            let note = rect_of(HELD_NOTE).expect("the card is drawn");
            for button in ["Send now", "Delete", "Edit"] {
                let rect = frames
                    .shapes
                    .iter()
                    .filter(|(drawn, _)| drawn == button)
                    .map(|(_, rect)| *rect)
                    .next()
                    .unwrap_or_else(|| panic!("{button} at {width}: {:?}", frames.shapes));
                assert!(rect.right() <= width, "{button} at {width}: {rect:?}");
                assert!(rect.top() > note.bottom(), "{button} under the note at {width}");
            }
            let texts: Vec<Rect> = frames
                .shapes
                .iter()
                .filter(|(text, _)| text.starts_with("also check") || text.starts_with("then"))
                .map(|(_, rect)| *rect)
                .collect();
            let buttons: Vec<Rect> = frames
                .shapes
                .iter()
                .filter(|(text, _)| ["Edit", "Delete", "Send now"].contains(&text.as_str()))
                .map(|(_, rect)| *rect)
                .collect();
            for rect in texts {
                for button in &buttons {
                    assert!(!rect.intersects(*button), "{rect:?} over {button:?} at {width}");
                }
            }
        }
    }
}
