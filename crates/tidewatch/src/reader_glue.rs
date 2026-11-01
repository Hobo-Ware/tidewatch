//! The seams between the Reader and the rest of the app: starting each agent tab's feed, toggling
//! the view, drawing it in the terminal area and typing what the user chose into the pty.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use egui::{Context, Rect, RichText, Ui, UiBuilder, pos2, vec2};
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::avatar::Avatar;
use tidewatch_core::clock::now_ms;
use tidewatch_core::paths::{home_dir, sent_file_beside};
use tidewatch_core::question::{self, Reply};
use tidewatch_core::question_words;
use tidewatch_core::sent::{Reason as Said, because};
use tidewatch_core::status::{AgentStatus, Reason};
use tidewatch_core::transcript::Source;

use crate::agent_avatar::Face;
use crate::agent_ready::Ready;
use crate::ask_model::{self, Ask};
use crate::brand;
use crate::commands::Command;
use crate::dev_reader::DevReader;
use crate::due::{Due, ask_blocked};
use crate::due_view::{self, Facts as DueFacts};
use crate::flow::Flow;
use crate::flow_glue;
use crate::note_send::ENTER_DELAY_SECS;
use crate::outbox::{Offer, Screen, Send};
use crate::pane::{AgentTab, Body, FirstPrompt, Pane};
use crate::pr_card_model::CardAction;
use crate::pr_cards_view::Data as PrData;
use crate::question_auto::Poll;
use crate::question_bulk;
use crate::question_screen::{Look, ShowsScreen, menu_look};
use crate::reader::{Note, Shown, Step};
use crate::reader_keys;
use crate::reader_rows::{Asked, Kind, Origin};
use crate::reader_view::{self, Action, Blocked, Facts, Prompt, Turn};
use crate::recall::Recall;
use crate::review::ReviewFacts;
use crate::screen_kit::{Size, button_width, quiet_button};
use crate::send_line::is_command;
use crate::slash;
use crate::tabs::Tabs;
use crate::wake::request_wake;

const NOTE_H: f32 = 24.0;
const NOTE_PAD: f32 = 8.0;
const NOTE_MARGIN_X: f32 = 8.0;
const NOTE_MARGIN_Y: f32 = 4.0;
/// The pause between two key presses typed into an agent's menu.
const KEY_GAP_S: f64 = ENTER_DELAY_SECS;
/// How long a menu needs to close after Esc before text can follow.
const MENU_CLOSE_S: f64 = 0.8;
/// How long the agent gets to end its turn after Esc before a message is typed.
const INTERRUPT_GAP_S: f64 = 0.8;
const RESUME_DEBOUNCE_S: f64 = 3.0;
/// How long a closed menu must stay off the agent's screen before its card is closed.
const MENU_GONE_AFTER_S: f64 = 2.0;
const LINE_FULL_TOAST: &str =
    "Too many messages are waiting for the agent. Yours is back in the box";
const MENU_GONE: &str =
    "The agent's question is no longer on its screen. Nothing was typed. Check the terminal";

/// Starts the feed of every agent tab that has an agent running and none yet.
pub(crate) fn start(tabs: &mut Tabs<Pane<PtyTerm>>, ctx: &Context, dev: &DevReader) {
    for tab in tabs.items_mut() {
        let Some(agent) = tab.pane.agent.as_mut() else { continue };
        if !matches!(tab.pane.body, Body::Shell(_)) || !agent.reader.needs_start() {
            continue;
        }
        let Some(home) = home_dir() else { continue };
        if dev.raw {
            agent.reader.set_wants_reader(false);
        }
        let ctx = ctx.clone();
        agent.reader.start(&home, move || ctx.request_repaint());
    }
}

/// Takes what every feed read since the last frame, and types what is due into the tabs that were
/// answering a question. A pasted message gets its Enter from the flow's Enter queue.
pub(crate) fn drain(tabs: &mut Tabs<Pane<PtyTerm>>, flow: &mut Flow, ctx: &Context, now: f64) {
    let mut picked = Vec::new();
    for tab in tabs.items_mut() {
        let id = tab.id;
        if let Pane { body: Body::Shell(term), agent: Some(agent), .. } = &mut tab.pane {
            while let Some(step) = agent.reader.take_due_step(now) {
                match step {
                    Step::Keys(bytes) => term.send(&bytes),
                    Step::Paste(text) => {
                        term.paste(&text);
                        flow.queue_enter(id, now);
                        request_wake(ctx, ENTER_DELAY_SECS);
                    }
                }
            }
            if let Some(due) = agent.reader.next_step_due() {
                request_wake(ctx, (due - now).max(0.0));
            }
        }
        if let Some(agent) = tab.pane.agent.as_mut() {
            if !agent.reader.has_sent_file()
                && let Some(notes) = &agent.run.notes_file
            {
                agent.reader.attach_sent(sent_file_beside(notes));
            }
            agent.reader.poll_sent(now);
            if agent.reader.drain() {
                agent.run.note_activity(now);
            }
            close_ended_asks(agent);
            keep_reply_box(agent, ctx, now);
        }
        close_gone_menu(&mut tab.pane, now);
        if let Some(text) = pick_for_the_absent(&mut tab.pane, ctx, now) {
            picked.push((id, text));
        }
        if let Some(agent) = tab.pane.agent.as_mut() {
            if agent.first_prompt != FirstPrompt::NotNeeded && agent.reader.rows.len() > 0 {
                agent.first_prompt = FirstPrompt::NotNeeded;
            }
            if let Some(launched) = agent.launched.as_mut()
                && launched.session_id.is_none()
            {
                launched.session_id = agent.reader.codex_session_id();
            }
        }
    }
    for (tab, text) in picked {
        let send = Send::auto(&text, Said::AutoPick, because::AUTO_PICK);
        flow_glue::send_or_queue(tabs, flow, tab, send, ctx, now);
    }
}

/// Brings back a tab's sent messages and half-written reply the first time its notes folder is
/// known, and saves the reply box once it stood still.
fn keep_reply_box(agent: &mut AgentTab, ctx: &Context, now: f64) {
    if !agent.reader.store.is_attached()
        && let Some(notes) = agent.run.notes_file.clone()
    {
        let (entries, draft) = agent.reader.store.attach(&notes);
        let view = &mut agent.reader.view;
        view.recall = Recall::with_entries(entries);
        if let Some(draft) = draft.filter(|_| view.reply.is_empty()) {
            view.reply = draft.text;
            crate::tokens::append(
                &mut view.reply,
                draft.pictures.into_iter().map(crate::tokens::Target::File),
            );
        }
    }
    let view = &agent.reader.view;
    if let Some(wake) = agent.reader.store.tick(&view.reply, now) {
        request_wake(ctx, wake);
    }
}

/// Closes the card of a Claude question when its menu has been gone from the agent's screen for a
/// while: it was answered or closed in the terminal, and the card would wait for ever.
fn close_gone_menu<T: ShowsScreen>(pane: &mut Pane<T>, now: f64) {
    let (Body::Shell(term), Some(agent)) = (&pane.body, pane.agent.as_mut()) else { return };
    let waiting = agent.reader.rows.pending_question().and_then(|row| match &row.kind {
        Kind::Question(q) if matches!(q.origin, Origin::Claude) => {
            Some((row.seq, menu_look(term, &q.questions)))
        }
        _ => None,
    });
    let Some((seq, look)) = waiting else {
        agent.reader.menu_gone = None;
        return;
    };
    if look != Look::Gone {
        agent.reader.menu_gone = None;
        return;
    }
    let since = match agent.reader.menu_gone {
        Some((known, since)) if known == seq => since,
        _ => {
            agent.reader.menu_gone = Some((seq, now));
            now
        }
    };
    if now - since >= MENU_GONE_AFTER_S && agent.reader.rows.set_asked(seq, Asked::Closed) {
        agent.reader.menu_gone = None;
        agent.reader.mark_answered();
        refresh_asking(agent);
    }
}

fn waiting_question<T>(pane: &Pane<T>) -> Option<u64> {
    let agent = pane.agent.as_ref()?;
    let running = matches!(agent.run.status(), AgentStatus::Working | AgentStatus::NeedsYou(_));
    agent
        .reader
        .rows
        .pending_question()
        .filter(|row| match &row.kind {
            Kind::Question(q) => matches!(q.origin, Origin::Tidewatch(_)) || running,
            _ => false,
        })
        .map(|row| row.seq)
}

/// In a tab that runs on its own, answers the question the user left alone for the wait with the
/// agent's recommended option, else the first, and wakes the app for the moment it is due. The tab
/// stays in Needs you until its owner opens it. A held `ask_user` call is told in its result; a
/// Claude menu has no call to answer, so what the agent is to be told comes back as a message.
fn pick_for_the_absent<T: ShowsScreen>(
    pane: &mut Pane<T>,
    ctx: &Context,
    now: f64,
) -> Option<String> {
    let wait = crate::dev_reader::autopick_wait();
    let waiting = waiting_question(pane);
    let agent = pane.agent.as_mut()?;
    let autonomous = agent.autonomous();
    let poll = agent.reader.auto.poll(waiting.filter(|_| autonomous), now, wait);
    match poll {
        Poll::Idle => None,
        Poll::Counting { left } => {
            request_wake(ctx, left.clamp(0.0, 1.0));
            None
        }
        Poll::Due(seq) => {
            let asked = agent.reader.rows.question_of(seq)?.clone();
            let given = question::auto_picks(&asked.questions);
            let minutes = (wait / 60.0).round().max(1.0) as u64;
            let held = matches!(asked.origin, Origin::Tidewatch(_));
            if answer_question(pane, seq, auto_reply(held, &given, minutes), now).is_some() {
                return None;
            }
            let agent = pane.agent.as_mut()?;
            agent.reader.rows.mark_auto_picked(seq);
            agent.run.pick_made(question_words::pick_review_line(&asked.questions, &given));
            (!held).then(|| question_words::picked_for_you(&asked.questions, &given, minutes))
        }
    }
}

/// The reply Tidewatch gives for a user who stayed away. A held `ask_user` call carries the news in
/// its own result; a Claude menu takes the keys of the picks alone.
fn auto_reply(held: bool, given: &[question::Given], minutes: u64) -> Reply {
    if held {
        Reply::PickedForYou { given: given.to_vec(), minutes }
    } else {
        Reply::Answered(given.to_vec())
    }
}

/// The Reader chord on the active tab. `None` for a tab that is not an agent's.
pub(crate) fn toggle(pane: &mut Pane<PtyTerm>) -> Option<Shown> {
    pane.agent.as_mut().map(|agent| agent.reader.toggle())
}

pub(crate) fn shown(pane: &Pane<PtyTerm>) -> Option<Shown> {
    pane.agent.as_ref().map(|agent| match agent.first_prompt {
        FirstPrompt::NotNeeded => agent.reader.shown(),
        FirstPrompt::Awaiting | FirstPrompt::Sent => agent.reader.shown_before_first_message(),
    })
}

pub(crate) fn toggle_text(shown: Shown) -> String {
    let keys = Command::Reader.keys();
    match shown {
        Shown::Reader => format!("Reader view ({keys} for the raw terminal)"),
        Shown::Terminal(_) => format!("Raw terminal ({keys} for the Reader)"),
    }
}

fn waits_for_input(status: &AgentStatus) -> bool {
    matches!(
        status,
        AgentStatus::Done
            | AgentStatus::NeedsYou(Reason::Input | Reason::Attention | Reason::Quiet)
    )
}

fn prompt_of(status: &AgentStatus) -> Option<Prompt> {
    match status {
        AgentStatus::NeedsYou(Reason::Permission) => Some(Prompt::Permission),
        AgentStatus::NeedsYou(Reason::Question) => Some(Prompt::Question),
        _ => None,
    }
}

fn prompt_waiting(agent: &AgentTab) -> Option<Prompt> {
    prompt_of(agent.run.status()).filter(|_| !agent.reader.prompt_answered())
}

/// Why a ready answer cannot go to the agent now, or `None`: the reply box's own gate, which also
/// shuts while a permission or question prompt waits for its answer.
pub(crate) fn answer_blocked(agent: &AgentTab) -> Option<&'static str> {
    reply_blocked(agent)
        .map(Blocked::hint)
        .or_else(|| blocked_by_prompt(agent.run.status(), agent.reader.prompt_answered()))
}

fn blocked_by_prompt(status: &AgentStatus, answered: bool) -> Option<&'static str> {
    prompt_of(status).filter(|_| !answered).map(|_| reader_view::REPLY_BLOCKED)
}

/// Draws the Reader of the tab in `area`, and returns what the user (or a dev switch) asked for.
pub(crate) fn draw(
    ui: &mut Ui,
    area: Rect,
    tab: u64,
    pane: &mut Pane<PtyTerm>,
    dev: &mut DevReader,
    now: f64,
    review: Option<ReviewFacts<'_>>,
    (pr, due): (Option<PrData<'_>>, Option<(&Due, usize)>),
    ask: Option<&Ask>,
    avatar: Option<Arc<Avatar>>,
    agents: &[tidewatch_core::subagents::Agent],
) -> Vec<Action> {
    let Some(agent) = pane.agent.as_mut() else { return Vec::new() };
    let prompt = prompt_waiting(agent);
    let ready = (agent.first_prompt == FirstPrompt::Awaiting)
        .then_some(Ready { profile: &agent.profile_name, agent_id: &agent.run.agent_id });
    let root = pane.worktree.as_ref().map(|worktree| worktree.path.as_path());
    let turn = match agent.run.status() {
        AgentStatus::Working | AgentStatus::NeedsYou(_) => Turn::Running,
        _ => Turn::Finished,
    };
    let due = due.map(|(due, more)| DueFacts {
        due,
        more,
        profile: &agent.profile_name,
        blocked: ask_blocked(Some(agent.run.status())),
    });
    let face = Face::of(&agent.profile_name, &agent.run.agent_id, avatar);
    let agents = crate::subagents_model::list(agents, now_ms());
    agent.reader.view.pastes = agent
        .run
        .notes_file
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| dir.join(crate::pictures::PASTES_DIR));
    let facts = Facts {
        tab,
        source: agent.reader.source(),
        prompt,
        ready,
        root,
        review,
        pr,
        due,
        ask,
        profile: &agent.profile_name,
        face: Some(&face),
        turn,
        working: *agent.run.status() == AgentStatus::Working || dev.working,
        reply_blocked: dev.blocked.or_else(|| reply_blocked(agent).filter(|_| !dev.working)),
        agents: agents.as_ref(),
    };
    let reader = &mut agent.reader;
    let reads_screen = reader.source() == Some(Source::Claude)
        && prompt.is_none()
        && waits_for_input(agent.run.status())
        && reader.rows.len() > 0;
    match (&pane.body, reads_screen) {
        (Body::Shell(term), true) => {
            reader.view.ghost.refresh(term.output_gen(), || term.grid_snapshot());
        }
        _ => reader.view.ghost.clear(),
    }
    let auto_left = reader.rows.pending_question().and_then(|row| {
        reader
            .auto
            .left(row.seq, now, crate::dev_reader::autopick_wait())
            .map(|left| (row.seq, left))
    });
    reader.view.set_auto_left(auto_left);
    if dev.take_open_turns(reader.rows.len()) {
        reader.view.open_turns(&reader.rows, dev.notes_only());
    }
    if dev.take_rows() {
        use tidewatch_core::transcript::{Entry, Role};
        let now = now_ms();
        for (role, text, ago) in [
            (Role::User, "Add a greeting to the README and run the tests", 240_000),
            (
                Role::Assistant,
                "Done. The README has a greeting section and all 12 tests pass.",
                200_000,
            ),
            (Role::User, "Nice. Now check the totals in the report", 120_000),
            (Role::Assistant, "Working on it: the totals add up to 1,204.", 60_000),
        ] {
            reader.rows.push(Entry {
                role,
                text: text.to_owned(),
                tool: None,
                ts: Some(now - ago),
                images: Vec::new(),
            });
        }
    }
    if dev.take_ask() {
        reader
            .rows
            .push_ask(crate::dev_reader::stand_in_questions(), crate::dev_reader::held_reply());
    }
    let ctx = ui.ctx().clone();
    if let Some(sentence) = dev.take_sentence() {
        reader.view.set_sentence(&ctx, &sentence);
    }
    if dev.holds_box(now) {
        reader.view.request_reply_focus();
    }
    let attached = dev.take_pictures();
    if !attached.is_empty() {
        reader.view.insert_files(&ctx, attached);
    }
    if dev.take_enter(now) {
        ctx.input_mut(|input| {
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: Some(egui::Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        });
    }
    for text in dev.take_held() {
        let _ = reader.view.line.hold(&text, now_ms());
    }
    if dev.take_edit_held()
        && let Some(id) = reader.view.line.newest_id()
    {
        reader.view.begin_edit_held(id);
    }
    if let Some(text) = dev.take_not_seen() {
        reader.rows.queue_user(&text, now_ms() - 3 * 60_000);
    }
    if let Some(text) = dev.take_queued() {
        reader.rows.queue_user(&text, now_ms());
    }
    if let Some(pixels) = dev.take_scroll_up(now) {
        reader.view.scroll_up_by(pixels);
    }
    let mut actions = reader_view::show(ui, area, &reader.rows, &mut reader.view, &facts);
    actions.extend(dev.drive_card(now, reader.view.live_mut()));
    actions.extend(dev.actions(now, facts.prompt));
    actions
}

/// Takes the clicks on pull request cards out of the Reader's actions; the app handles them.
pub(crate) fn take_pr_clicks(actions: &mut Vec<Action>) -> Vec<CardAction> {
    let mut clicks = Vec::new();
    actions.retain(|action| match action {
        Action::Pr(click) => {
            clicks.push(click.clone());
            false
        }
        _ => true,
    });
    clicks
}

/// Takes the clicks on the buttons under Tidewatch's rows out of the Reader's actions.
pub(crate) fn take_tidewatch_clicks(
    actions: &mut Vec<Action>,
) -> Vec<crate::tidewatch_view::Click> {
    let mut clicks = Vec::new();
    actions.retain(|action| match action {
        Action::Tidewatch(click) => {
            clicks.push(*click);
            false
        }
        _ => true,
    });
    clicks
}

/// Takes the answers picked on the Needs you card out of the Reader's actions.
pub(crate) fn take_picks(actions: &mut Vec<Action>) -> Vec<String> {
    let mut picks = Vec::new();
    actions.retain(|action| match action {
        Action::Pick(text) => {
            picks.push(text.clone());
            false
        }
        _ => true,
    });
    picks
}

/// Takes the clicks on the rows of the running agents list out of the Reader's actions: the ids
/// of the agents whose chat to read.
pub(crate) fn take_agent_reads(actions: &mut Vec<Action>) -> Vec<String> {
    let mut reads = Vec::new();
    actions.retain(|action| match action {
        Action::SubAgent(id) => {
            reads.push(id.clone());
            false
        }
        _ => true,
    });
    reads
}

/// Takes the clicks on the Due today card out of the Reader's actions; the app handles them.
pub(crate) fn take_due_clicks(actions: &mut Vec<Action>) -> Vec<due_view::Action> {
    let mut clicks = Vec::new();
    actions.retain(|action| match action {
        Action::Due(click) => {
            clicks.push(click.clone());
            false
        }
        _ => true,
    });
    clicks
}

/// The small note over the terminal that says why it is shown instead of the Reader.
pub(crate) fn note(ui: &mut Ui, area: Rect, note: Note) {
    let text = note.text();
    let galley =
        ui.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(12.0), brand::MUTED);
    let size = galley.size() + vec2(2.0 * NOTE_PAD, 0.0);
    let rect = Rect::from_min_size(
        pos2(area.right() - size.x - NOTE_PAD, area.top() + NOTE_PAD),
        vec2(size.x, NOTE_H),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    reader_view::item_frame().inner_margin(egui::Margin::symmetric(8, 4)).show(&mut child, |ui| {
        ui.label(RichText::new(text).size(12.0).color(brand::MUTED));
    });
}

const BACK_TO_READER: &str = "Back to Reader";

/// The note over the terminal after a slash command opened a dialog there. True when Back to
/// Reader was clicked.
pub(crate) fn opened_note(ui: &mut Ui, area: Rect, command: &str) -> bool {
    let text = format!("/{command} opened in the terminal");
    let galley =
        ui.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(12.0), brand::MUTED);
    let button = button_width(ui, BACK_TO_READER, Size::Compact);
    let width = galley.size().x + ui.spacing().item_spacing.x + button + 2.0 * NOTE_MARGIN_X;
    let rect = Rect::from_min_size(
        pos2(area.right() - width - NOTE_PAD, area.top() + NOTE_PAD),
        vec2(width, Size::Compact.height() + 2.0 * NOTE_MARGIN_Y),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    let mut back = false;
    reader_view::item_frame()
        .inner_margin(egui::Margin::symmetric(NOTE_MARGIN_X as i8, NOTE_MARGIN_Y as i8))
        .show(&mut child, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).size(12.0).color(brand::MUTED));
                back = quiet_button(ui, BACK_TO_READER, true, Size::Compact).clicked();
            });
        });
    back
}

/// Does what the Reader asked for in the tab `tab`. Returns a toast when nothing could be done.
pub(crate) fn apply(
    actions: Vec<Action>,
    tabs: &mut Tabs<Pane<PtyTerm>>,
    tab: u64,
    flow: &mut Flow,
    ctx: &Context,
    now: f64,
) -> Option<String> {
    let mut toast = None;
    for action in actions {
        let action = match action {
            Action::Reply(text) => {
                toast = reply(tabs, flow, tab, &text, (ctx, now)).or(toast);
                continue;
            }
            Action::SendAgain { text, forget } => {
                if let (Some(id), Some(agent)) = (forget, tabs.agent_mut(tab)) {
                    agent.reader.rows.forget_queued(id);
                }
                toast = reply(tabs, flow, tab, &text, (ctx, now)).or(toast);
                continue;
            }
            Action::Forget(id) => {
                if let Some(agent) = tabs.agent_mut(tab) {
                    agent.reader.rows.forget_queued(id);
                }
                continue;
            }
            Action::SendNow(id) => {
                toast = send_now(tabs, flow, tab, id, ctx, now).or(toast);
                continue;
            }
            Action::Stop => {
                stop(tabs, tab);
                continue;
            }
            other => other,
        };
        let Some(found) = tabs.get_mut(tab) else { continue };
        let source = found.pane.agent.as_ref().and_then(|agent| agent.reader.source());
        match action {
            Action::Answer(answer) => {
                match (&mut found.pane.body, reader_keys::bytes(source, answer)) {
                    (Body::Shell(term), Some(keys)) => {
                        term.send(keys);
                        if let Some(agent) = found.pane.agent.as_mut() {
                            agent.reader.mark_answered();
                            agent.run.permission_answered(now);
                        }
                    }
                    _ => toast = Some("That answer is not known for this agent".to_owned()),
                }
            }
            Action::SwitchToTerminal => {
                if let Some(agent) = found.pane.agent.as_mut() {
                    agent.reader.set_wants_reader(false);
                }
            }
            Action::Resume => resume_agent(&mut found.pane, now),
            Action::Review(_)
            | Action::Pr(_)
            | Action::Tidewatch(_)
            | Action::Due(_)
            | Action::Pick(_)
            | Action::SubAgent(_)
            | Action::Reply(_)
            | Action::SendAgain { .. }
            | Action::Forget(_)
            | Action::SendNow(_)
            | Action::Stop => {}
            Action::Ask(seq, reply) => {
                if let Some(why) = answer_question(&mut found.pane, seq, reply, now) {
                    toast = Some(why.to_owned());
                }
            }
            Action::GoWithPicks => toast = go_with_picks(&mut found.pane, now),
        }
    }
    toast
}

/// Types one of the owner's messages into the agent through the one send line, and shows it as
/// queued until the transcript has it. False when the tab has no terminal to type into.
fn type_reply(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    tab: u64,
    text: &str,
    (ctx, now): (&Context, f64),
) -> bool {
    if !flow_glue::deliver(tabs, flow, tab, &Send::reply(text), ctx, now) {
        return false;
    }
    if let Some(agent) = tabs.agent_mut(tab) {
        let source = agent.reader.source();
        if !is_command(text) {
            agent.reader.queue_user(text);
        }
        show_dialog_command(agent, source, text);
    }
    true
}

/// Asks the one send line whether the owner's reply may be typed now. Claude Code takes typed text
/// while it works and shows it to the model at its next step, so a Claude reply goes mid-turn,
/// once its input box is free (a pasted message whose Enter has not gone yet counts as not free).
/// Every other CLI gets a reply only when its turn is over.
fn offer_reply(
    tabs: &Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    tab: u64,
    text: &str,
    status: &AgentStatus,
    now: f64,
) -> Offer {
    let claude = tabs
        .get(tab)
        .and_then(|found| found.pane.agent.as_ref())
        .is_some_and(|agent| agent.reader.source() == Some(Source::Claude));
    let send = if claude { Send::reply(text).mid_turn() } else { Send::reply(text) };
    let screen = match tabs.get(tab) {
        Some(_) if !claude || *status != AgentStatus::Working => Screen::Free,
        Some(_) if flow.enter_pending(tab) => Screen::Typed,
        Some(found) => flow_glue::screen_of(found, Some(status), true),
        None => Screen::Free,
    };
    flow.outbox.offer(tab, &send, Some(status), screen, now)
}

/// A message the owner sent. It goes at once when the agent can take it. Claude Code takes it
/// while it works, once its input box is free; any other CLI, or a Claude whose box holds a draft
/// or a dialog, has it held by Tidewatch in order, to be edited, deleted or sent now, and typed
/// when the agent can take it. The first message of a tab and a slash command always go at once.
fn reply(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    tab: u64,
    text: &str,
    (ctx, now): (&Context, f64),
) -> Option<String> {
    let agent = tabs.agent_mut(tab)?;
    let first = agent.first_prompt == FirstPrompt::Awaiting;
    if first {
        agent.first_prompt = FirstPrompt::Sent;
    }
    let blocked = reply_blocked(agent);
    let status = agent.run.status().clone();
    if agent.reader.view.recall.remember(text) {
        agent.reader.store.save_history(agent.reader.view.recall.entries());
    }
    agent.reader.view.jump_to_latest();
    flow.outbox.user_spoke(tab);
    let offer = if first || (is_command(text) && blocked.is_none()) {
        Offer::Go
    } else {
        offer_reply(tabs, flow, tab, text, &status, now)
    };
    if offer == Offer::Go
        && (blocked.is_none() || first)
        && type_reply(tabs, flow, tab, text, (ctx, now))
    {
        return None;
    }
    let agent = tabs.agent_mut(tab)?;
    ctx.request_repaint_after(Duration::from_secs_f64(0.5));
    if agent.reader.view.line.hold(text, now_ms()).is_err() {
        agent.reader.view.keep_in_box(text);
        return Some(LINE_FULL_TOAST.to_owned());
    }
    blocked.map(|blocked| blocked.hint().to_owned())
}

/// Types the first held message of each tab when the one send line lets it go.
pub(crate) fn release_lines(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    ctx: &Context,
    now: f64,
) -> Vec<String> {
    let mut told = Vec::new();
    let waiting: Vec<u64> = tabs
        .items()
        .iter()
        .filter(|found| found.pane.agent.as_ref().is_some_and(|a| a.reader.view.line.len() > 0))
        .map(|found| found.id)
        .collect();
    for tab in waiting {
        let Some(agent) = tabs.agent_mut(tab) else { continue };
        let status = agent.run.status().clone();
        if crate::dev_reader::pretend_working() {
            continue;
        }
        let view = &agent.reader.view;
        let Some((id, text)) =
            view.line.ready(view.editing_held()).map(|held| (held.id, held.text.clone()))
        else {
            continue;
        };
        if offer_reply(tabs, flow, tab, &text, &status, now) != Offer::Go {
            continue;
        }
        if type_reply(tabs, flow, tab, &text, (ctx, now)) {
            if let Some(agent) = tabs.agent_mut(tab) {
                agent.reader.view.line.delete(id);
            }
        } else {
            told.push("A held message could not be typed: the tab has no terminal".to_owned());
        }
    }
    if tabs.items().iter().any(|found| {
        found.pane.agent.as_ref().is_some_and(|agent| agent.reader.view.line.len() > 0)
    }) {
        ctx.request_repaint_after(Duration::from_secs_f64(0.5));
    }
    told
}

/// Whether Esc ends something: only a working agent has a turn to end, and Esc anywhere else
/// would answer a menu or do nothing.
fn interrupts(status: &AgentStatus) -> bool {
    *status == AgentStatus::Working
}

/// Ends the agent's turn, as Esc does in its own screen.
fn stop(tabs: &mut Tabs<Pane<PtyTerm>>, tab: u64) {
    let Some(found) = tabs.get_mut(tab) else { return };
    let working = found.pane.agent.as_ref().is_some_and(|a| interrupts(a.run.status()));
    if let (true, Body::Shell(term)) = (working, &mut found.pane.body) {
        term.send(reader_keys::INTERRUPT);
    }
}

/// "Send now" on a held message: the turn is stopped and the message follows once the agent has
/// taken the Esc. An agent that is not working takes it at once.
fn send_now(
    tabs: &mut Tabs<Pane<PtyTerm>>,
    flow: &mut Flow,
    tab: u64,
    id: u64,
    ctx: &Context,
    now: f64,
) -> Option<String> {
    let agent = tabs.agent_mut(tab)?;
    let held = agent.reader.view.line.delete(id)?;
    if !interrupts(agent.run.status()) {
        if type_reply(tabs, flow, tab, &held.text, (ctx, now)) {
            return None;
        }
        tabs.agent_mut(tab)?.reader.view.keep_in_box(&held.text);
        return Some("The message could not be typed: the tab has no terminal".to_owned());
    }
    if !is_command(&held.text) {
        agent.reader.queue_user(&held.text);
    }
    let steps = vec![
        (0.0, Step::Keys(reader_keys::INTERRUPT.to_vec())),
        (INTERRUPT_GAP_S, Step::Paste(held.text)),
    ];
    agent.reader.type_steps(steps, now);
    ctx.request_repaint();
    None
}

/// Why a reply typed now would not reach the agent: its pane is not at the agent's prompt, so the
/// text would run in a shell. None while the agent can take it, and before its first message.
pub(crate) fn reply_blocked(agent: &AgentTab) -> Option<Blocked> {
    if agent.first_prompt == FirstPrompt::Awaiting {
        return None;
    }
    match agent.run.status() {
        AgentStatus::Stopped => Some(Blocked::Stopped),
        AgentStatus::Failed(_) => Some(Blocked::Failed),
        AgentStatus::SettingUp => Some(Blocked::SettingUp),
        AgentStatus::Shell => Some(Blocked::Shell),
        AgentStatus::Working | AgentStatus::Done | AgentStatus::NeedsYou(_) => None,
    }
}

/// Shows the terminal when `text`, just sent to the agent, opens a dialog in Claude Code's screen.
fn show_dialog_command(agent: &mut AgentTab, source: Option<Source>, text: &str) {
    if source == Some(Source::Claude)
        && let Some(command) = slash::interactive(text)
    {
        agent.reader.open_in_terminal(command);
    }
}

/// Runs the resume command of a tab whose agent waits at its shell prompt. Ctrl+U first, so a
/// command that was typed there is not run twice.
pub(crate) fn resume_agent(pane: &mut Pane<PtyTerm>, now: f64) {
    if let (Body::Shell(term), Some(agent)) = (&mut pane.body, pane.agent.as_mut()) {
        resume_in(term, agent, now);
    }
}

pub(crate) fn resume_in(term: &mut PtyTerm, agent: &mut AgentTab, now: f64) {
    if let Some(keys) = resume_keys(agent, now) {
        term.send(&keys);
    }
}

fn resume_keys(agent: &mut AgentTab, now: f64) -> Option<Vec<u8>> {
    let line = agent.resume_line.as_deref()?;
    if agent.resume_sent_at.is_some_and(|at| now - at < RESUME_DEBOUNCE_S) {
        return None;
    }
    let keys = format!("\u{15}{line}\r").into_bytes();
    agent.resume_sent_at = Some(now);
    agent.resume_note = None;
    Some(keys)
}

/// Pasted text, `lead` seconds from now; its Enter is queued when it is pasted.
fn reply_steps(text: String, lead: f64) -> Vec<(f64, Step)> {
    vec![(lead, Step::Paste(text))]
}

/// The steps that give `given` to Claude's menu: its key presses, or when the menu cannot take them
/// by keys, Esc and the answer as a message.
fn claude_menu_steps(
    questions: &[question::Question],
    given: &[question::Given],
) -> Vec<(f64, Step)> {
    if let Some(keys) = reader_keys::claude_question_keys(questions, given) {
        return keys
            .into_iter()
            .enumerate()
            .map(|(index, keys)| (if index == 0 { 0.0 } else { KEY_GAP_S }, Step::Keys(keys)))
            .collect();
    }
    let mut steps = vec![(0.0, Step::Keys(reader_keys::CLAUDE_MENU_ESCAPE.to_vec()))];
    steps.extend(reply_steps(question::late_answer_text(questions, given), MENU_CLOSE_S));
    steps
}

fn refresh_asking(agent: &mut AgentTab) {
    let next = agent.reader.rows.pending_question().and_then(|row| match &row.kind {
        Kind::Question(q) if matches!(q.origin, Origin::Tidewatch(_)) => {
            q.questions.first().map(|question| question.text.clone())
        }
        _ => None,
    });
    agent.run.set_asking(next);
}

fn close_ended_asks(agent: &mut AgentTab) {
    if agent.run.is_asking() && agent.reader.rows.close_ended_asks() {
        refresh_asking(agent);
    }
}

/// Answers every waiting question that has a pick with it, and says which were answered and which
/// stay open.
fn go_with_picks<T: ShowsScreen>(pane: &mut Pane<T>, now: f64) -> Option<String> {
    let bulk = question_bulk::plan(&pane.agent.as_ref()?.reader.rows);
    let text = question_bulk::toast(&bulk);
    let mut refused = None;
    for answer in bulk.answers {
        refused = answer_question(pane, answer.seq, Reply::Answered(answer.given), now).or(refused);
    }
    Some(match refused {
        Some(why) => format!("{text} {why}"),
        None => text,
    })
}

/// What became of a ready answer offered to the question the agent has open.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    /// No single question waits: the caller sends it as a plain reply.
    NotOpen,
    Answered,
    Refused(&'static str),
}

/// Gives `text`, a ready answer picked outside the Reader, to the one question the agent has open
/// with its question tool: the option it names, else as typed.
pub(crate) fn answer_open<T: ShowsScreen>(pane: &mut Pane<T>, text: &str, now: f64) -> Opened {
    let Some(row) = pane.agent.as_ref().and_then(|agent| agent.reader.rows.pending_question())
    else {
        return Opened::NotOpen;
    };
    let Kind::Question(asked) = &row.kind else { return Opened::NotOpen };
    let Some(given) = ask_model::given_for(&asked.questions, text) else { return Opened::NotOpen };
    let seq = row.seq;
    match answer_question(pane, seq, Reply::Answered(given), now) {
        Some(reason) => Opened::Refused(reason),
        None => Opened::Answered,
    }
}

/// Gives the user's answer to the questions in row `seq`. A held `ask_user` call takes it through
/// its channel, or as a typed message when the call already ended. Claude's menu takes key presses,
/// one a gap apart; a skip closes the menu with Esc and then says so. Nothing is typed into a Claude
/// menu the screen shows is gone (an unreadable screen counts as open); the reason comes back.
fn answer_question<T: ShowsScreen>(
    pane: &mut Pane<T>,
    seq: u64,
    reply: Reply,
    now: f64,
) -> Option<&'static str> {
    let agent = pane.agent.as_ref()?;
    let asked = agent.reader.rows.question_of(seq).cloned()?;
    let gone =
        matches!(&pane.body, Body::Shell(term) if menu_look(term, &asked.questions) == Look::Gone);
    if matches!(asked.origin, Origin::Claude) && gone {
        return Some(MENU_GONE);
    }
    let agent = pane.agent.as_mut()?;
    let (steps, state) = match (&asked.origin, reply) {
        (Origin::Tidewatch(held), Reply::Answered(given)) => {
            let pairs = question::pairs(&asked.questions, &given);
            let steps = if held.send(Reply::Answered(given.clone())) {
                Vec::new()
            } else {
                reply_steps(question::late_answer_text(&asked.questions, &given), 0.0)
            };
            (steps, Asked::Answered(pairs))
        }
        (Origin::Tidewatch(held), Reply::PickedForYou { given, minutes }) => {
            let pairs = question::pairs(&asked.questions, &given);
            let steps = if held.send(Reply::PickedForYou { given: given.clone(), minutes }) {
                Vec::new()
            } else {
                reply_steps(question_words::picked_for_you(&asked.questions, &given, minutes), 0.0)
            };
            (steps, Asked::Answered(pairs))
        }
        (Origin::Tidewatch(held), Reply::Skipped) => {
            let steps = if held.send(Reply::Skipped) {
                Vec::new()
            } else {
                reply_steps(question_words::SKIP_REPLY.to_owned(), 0.0)
            };
            (steps, Asked::Declined)
        }
        (Origin::Claude, Reply::Answered(given) | Reply::PickedForYou { given, .. }) => {
            let pairs = question::pairs(&asked.questions, &given);
            let steps = claude_menu_steps(&asked.questions, &given);
            (steps, Asked::Sent(pairs))
        }
        (Origin::Claude, Reply::Skipped) => {
            let mut steps = vec![(0.0, Step::Keys(reader_keys::CLAUDE_MENU_ESCAPE.to_vec()))];
            steps.extend(reply_steps(question_words::SKIP_REPLY.to_owned(), MENU_CLOSE_S));
            (steps, Asked::Declined)
        }
    };
    agent.reader.rows.set_asked(seq, state);
    agent.reader.type_steps(steps, now);
    refresh_asking(agent);
    if matches!(asked.origin, Origin::Claude) {
        agent.reader.mark_answered();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use tidewatch_core::question::{AskReply, Given, QuestionOption, Select};

    use crate::agent_run::AgentRun;

    #[derive(Default)]
    struct Fake(Vec<String>);

    impl ShowsScreen for Fake {
        fn rows(&self) -> Vec<String> {
            self.0.clone()
        }
    }

    fn menu_rows() -> Fake {
        Fake(vec!["Where?".into(), "\u{276f} 1. Stage".into(), "  2. Run".into()])
    }

    #[test]
    fn the_suggestion_is_read_only_while_the_agent_waits_for_the_next_message() {
        let cases = [
            (AgentStatus::Done, true),
            (AgentStatus::NeedsYou(Reason::Input), true),
            (AgentStatus::NeedsYou(Reason::Quiet), true),
            (AgentStatus::NeedsYou(Reason::Attention), true),
            (AgentStatus::Working, false),
            (AgentStatus::NeedsYou(Reason::Permission), false),
            (AgentStatus::NeedsYou(Reason::Question), false),
            (AgentStatus::Stopped, false),
            (AgentStatus::Failed("x".into()), false),
            (AgentStatus::Shell, false),
            (AgentStatus::SettingUp, false),
        ];
        for (status, want) in cases {
            assert_eq!(waits_for_input(&status), want, "{status:?}");
        }
    }
    #[test]
    fn a_ready_answer_is_shut_while_a_prompt_waits_unanswered_and_otherwise_open() {
        let shut = Some(reader_view::REPLY_BLOCKED);
        let cases = [
            (AgentStatus::NeedsYou(Reason::Permission), false, shut),
            (AgentStatus::NeedsYou(Reason::Question), false, shut),
            (AgentStatus::NeedsYou(Reason::Permission), true, None),
            (AgentStatus::NeedsYou(Reason::Input), false, None),
            (AgentStatus::NeedsYou(Reason::Attention), false, None),
            (AgentStatus::Done, false, None),
            (AgentStatus::Working, false, None),
        ];
        for (status, answered, want) in cases {
            assert_eq!(
                blocked_by_prompt(&status, answered),
                want,
                "{status:?} answered {answered}"
            );
        }
    }

    use crate::pane::AgentTab;
    use crate::reader_rows::QuestionRow;

    fn reading(status: AgentStatus, first: FirstPrompt) -> Option<Blocked> {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        agent.first_prompt = first;
        agent.run.hold(Some(status), None);
        agent.run.refresh(1.0);
        reply_blocked(&agent)
    }

    #[test]
    fn a_second_click_on_resume_right_after_the_first_sends_nothing() {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        agent.resume_line = Some("claude --resume abc".into());
        assert_eq!(resume_keys(&mut agent, 10.0), Some(b"\x15claude --resume abc\r".to_vec()));
        assert_eq!(resume_keys(&mut agent, 10.2), None);
        assert_eq!(resume_keys(&mut agent, 13.5), Some(b"\x15claude --resume abc\r".to_vec()));
    }

    #[test]
    fn a_reply_only_reaches_an_agent_that_is_at_its_prompt_and_never_a_shell() {
        let open = FirstPrompt::NotNeeded;
        let cases = [
            (AgentStatus::Working, None),
            (AgentStatus::Done, None),
            (AgentStatus::NeedsYou(Reason::Input), None),
            (AgentStatus::NeedsYou(Reason::Attention), None),
            (AgentStatus::NeedsYou(Reason::Quiet), None),
            (AgentStatus::NeedsYou(Reason::Permission), None),
            (AgentStatus::Stopped, Some(Blocked::Stopped)),
            (AgentStatus::Failed("x".into()), Some(Blocked::Failed)),
            (AgentStatus::SettingUp, Some(Blocked::SettingUp)),
            (AgentStatus::Shell, Some(Blocked::Shell)),
        ];
        for (status, want) in cases {
            assert_eq!(reading(status.clone(), open), want, "{status:?}");
        }
        assert_eq!(reading(AgentStatus::SettingUp, FirstPrompt::Awaiting), None);
    }

    fn questions() -> Vec<question::Question> {
        let option = |label: &str| QuestionOption::new(label);
        vec![
            question::Question {
                text: "Where?".into(),
                header: None,
                decision: None,
                select: Select::Single,
                options: vec![option("Stage"), option("Run")],
            },
            question::Question {
                text: "Which?".into(),
                header: None,
                decision: None,
                select: Select::Multi,
                options: vec![option("Unit"), option("Lints")],
            },
        ]
    }

    fn answered() -> Vec<Given> {
        vec![Given { picked: vec![1], other: None }, Given { picked: vec![0, 1], other: None }]
    }

    fn pane(origin: Origin) -> (Pane<Fake>, u64) {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        let row = QuestionRow { questions: questions(), asked: Asked::Pending, origin };
        agent.reader.rows.push_all(Vec::new());
        let seq = agent.reader.rows.push_question_row(row);
        let pane = Pane {
            body: crate::pane::Body::Shell(menu_rows()),
            worktree: None,
            agent: Some(agent),
        };
        (pane, seq)
    }

    fn sent(source: Option<Source>, text: &str) -> Option<String> {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        show_dialog_command(&mut agent, source, text);
        agent.reader.opened().map(str::to_owned)
    }

    #[test]
    fn a_sent_interactive_command_opens_the_terminal_and_an_output_command_does_not() {
        let claude = Some(Source::Claude);
        let cases = [
            (claude, "/login", Some("login")),
            (claude, "/usage", Some("usage")),
            (claude, "/theme", Some("theme")),
            (claude, "/model opus", None),
            (claude, "/context", None),
            (claude, "/compact", None),
            (claude, "run the tests", None),
            (Some(Source::Codex), "/login", None),
            (Some(Source::Gemini), "/model", None),
            (None, "/login", None),
        ];
        for (source, text, want) in cases {
            assert_eq!(sent(source, text).as_deref(), want, "{source:?} {text:?}");
        }
    }

    fn typed(pane: &mut Pane<Fake>) -> Vec<(f64, Step)> {
        let reader = &mut pane.agent.as_mut().unwrap().reader;
        let mut out = Vec::new();
        while let Some(due) = reader.next_step_due() {
            out.extend(reader.take_due_step(due).map(|step| (due - 100.0, step)));
        }
        out
    }

    fn keys(bytes: &[&[u8]]) -> Vec<Step> {
        bytes.iter().map(|b| Step::Keys(b.to_vec())).collect()
    }

    fn steps_only(typed: Vec<(f64, Step)>) -> Vec<Step> {
        typed.into_iter().map(|(_, step)| step).collect()
    }

    fn asked_of(pane: &Pane<Fake>, seq: u64) -> Asked {
        pane.agent.as_ref().unwrap().reader.rows.question_of(seq).unwrap().asked.clone()
    }

    #[test]
    fn claude_gets_each_key_press_a_gap_apart_and_the_row_waits_for_its_result() {
        let (mut pane, seq) = pane(Origin::Claude);
        answer_question(&mut pane, seq, Reply::Answered(answered()), 100.0);
        let typed = typed(&mut pane);
        assert_eq!(steps_only(typed.clone()), keys(&[b"2", b"1", b"2", b"\x1b[C", b"1"]));
        let times: Vec<f64> = typed.iter().map(|(at, _)| (at * 10.0).round() / 10.0).collect();
        assert_eq!(times, [0.0, 0.3, 0.6, 0.9, 1.2]);
        assert!(matches!(asked_of(&pane, seq), Asked::Sent(pairs) if pairs[0].1 == "Run"));
    }

    #[test]
    fn a_claude_answer_the_menu_cannot_take_closes_the_menu_and_is_typed_as_a_message() {
        let (mut pane, seq) = pane(Origin::Claude);
        let nothing_ticked = vec![answered().remove(0), Given::default()];
        answer_question(&mut pane, seq, Reply::Answered(nothing_ticked), 100.0);
        let steps = steps_only(typed(&mut pane));
        assert_eq!(steps[0], Step::Keys(b"\x1b".to_vec()));
        assert!(
            matches!(&steps[1], Step::Paste(text) if text.starts_with("My answers to your earlier questions"))
        );
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn skipping_a_claude_menu_escapes_then_says_so() {
        let (mut pane, seq) = pane(Origin::Claude);
        answer_question(&mut pane, seq, Reply::Skipped, 100.0);
        let typed = typed(&mut pane);
        assert_eq!(typed[0], (0.0, Step::Keys(b"\x1b".to_vec())));
        assert_eq!(typed[1].1, Step::Paste(question_words::SKIP_REPLY.into()));
        assert!(typed[1].0 >= MENU_CLOSE_S - 0.001);
        assert_eq!(typed.len(), 2);
        assert_eq!(asked_of(&pane, seq), Asked::Declined);
    }

    #[test]
    fn a_held_ask_user_call_gets_the_answer_through_its_channel_and_nothing_is_typed() {
        let (sender, receiver) = channel();
        let (mut pane, seq) = pane(Origin::Tidewatch(AskReply::new(sender)));
        answer_question(&mut pane, seq, Reply::Answered(answered()), 100.0);
        assert_eq!(receiver.recv().unwrap(), Reply::Answered(answered()));
        assert_eq!(typed(&mut pane), Vec::new());
        assert!(matches!(asked_of(&pane, seq), Asked::Answered(_)));
    }

    #[test]
    fn a_held_call_hears_it_was_picked_for_the_user_and_a_call_that_ended_gets_it_typed() {
        let given = vec![Given { picked: vec![0], other: None }, Given::default()];
        let (sender, receiver) = channel();
        let (mut held, seq) = pane(Origin::Tidewatch(AskReply::new(sender)));
        let reply = Reply::PickedForYou { given: given.clone(), minutes: 5 };
        answer_question(&mut held, seq, reply.clone(), 100.0);
        assert_eq!(receiver.recv().unwrap(), reply);
        assert_eq!(typed(&mut held), Vec::new());

        let (sender, receiver) = channel();
        drop(receiver);
        let (mut ended, seq) = pane(Origin::Tidewatch(AskReply::new(sender)));
        answer_question(&mut ended, seq, reply, 100.0);
        let steps = steps_only(typed(&mut ended));
        let Step::Paste(text) = &steps[0] else { panic!("{steps:?}") };
        assert!(text.starts_with("Tidewatch picked "), "{text}");
        assert!(text.contains(" for you after 5 minutes. The user may change them."), "{text}");
        assert!(text.contains("Q: Where?\nA: Stage"), "{text}");
        assert_eq!(steps.len(), 1);
    }

    #[test]
    fn only_a_held_call_is_answered_with_the_news_a_menu_takes_the_picks_alone() {
        let given = vec![Given { picked: vec![1], other: None }];
        assert_eq!(
            auto_reply(true, &given, 5),
            Reply::PickedForYou { given: given.clone(), minutes: 5 }
        );
        assert_eq!(auto_reply(false, &given, 5), Reply::Answered(given));
    }

    #[test]
    fn a_late_answer_to_an_ask_user_call_that_ended_is_typed_as_a_message() {
        let (sender, receiver) = channel();
        drop(receiver);
        let (mut pane, seq) = pane(Origin::Tidewatch(AskReply::new(sender)));
        answer_question(&mut pane, seq, Reply::Answered(answered()), 100.0);
        let steps = steps_only(typed(&mut pane));
        let Step::Paste(text) = &steps[0] else { panic!("{steps:?}") };
        assert!(text.contains("Q: Where?\nA: Run"), "{text}");
        assert_eq!(steps.len(), 1);
        assert!(matches!(asked_of(&pane, seq), Asked::Answered(_)));
    }

    #[test]
    fn skipping_a_held_call_tells_it_and_a_dead_one_gets_a_typed_skip() {
        let (sender, receiver) = channel();
        let (mut live, seq) = pane(Origin::Tidewatch(AskReply::new(sender)));
        answer_question(&mut live, seq, Reply::Skipped, 100.0);
        assert_eq!(receiver.recv().unwrap(), Reply::Skipped);
        assert_eq!(typed(&mut live), Vec::new());
        let (sender, receiver) = channel();
        drop(receiver);
        let (mut dead, seq) = pane(Origin::Tidewatch(AskReply::new(sender)));
        answer_question(&mut dead, seq, Reply::Skipped, 100.0);
        assert_eq!(steps_only(typed(&mut dead)), [Step::Paste(question_words::SKIP_REPLY.into())]);
        assert_eq!(asked_of(&dead, seq), Asked::Declined);
    }

    #[test]
    fn a_call_that_ended_unanswered_leaves_no_question_and_no_asking() {
        let (sender, _keep) = channel();
        let held = AskReply::new(sender);
        let (mut pane, seq) = pane(Origin::Tidewatch(held.clone()));
        let agent = pane.agent.as_mut().unwrap();
        agent.run.set_asking(Some("Which?".into()));
        close_ended_asks(agent);
        assert_eq!(asked_of(&pane, seq), Asked::Pending, "the call still waits");
        let agent = pane.agent.as_mut().unwrap();
        assert!(agent.run.is_asking());
        held.end_mark().mark();
        close_ended_asks(agent);
        assert!(!agent.run.is_asking());
        assert_eq!(asked_of(&pane, seq), Asked::Ended);
    }

    fn ask(pane: &mut Pane<Fake>, labels: &[&str]) -> std::sync::mpsc::Receiver<Reply> {
        let (sender, receiver) = channel();
        let question = question::Question {
            text: format!("Pick one of {labels:?}?"),
            header: Some(labels[0].to_owned()),
            decision: None,
            select: Select::Single,
            options: labels.iter().map(|label| QuestionOption::new(*label)).collect(),
        };
        pane.agent.as_mut().unwrap().reader.rows.push_ask(vec![question], AskReply::new(sender));
        receiver
    }

    #[test]
    fn go_with_picks_answers_the_rows_that_have_one_and_names_the_others() {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        agent.reader.rows.push_all(Vec::new());
        let mut pane = Pane {
            body: crate::pane::Body::Shell(Fake::default()),
            worktree: None,
            agent: Some(agent),
        };
        let first = ask(&mut pane, &["Stage (Recommended)", "Run"]);
        let open = ask(&mut pane, &["Fast", "Slow"]);
        let last = ask(&mut pane, &["Unit", "Lints (Recommended)"]);
        let toast = go_with_picks(&mut pane, 100.0).unwrap();
        assert_eq!(
            first.recv().unwrap(),
            Reply::Answered(vec![Given { picked: vec![0], other: None }])
        );
        assert_eq!(
            last.recv().unwrap(),
            Reply::Answered(vec![Given { picked: vec![1], other: None }])
        );
        assert!(open.try_recv().is_err());
        assert_eq!(toast, "Went with the picks for 2: Stage, Lints. Still open: Fast.");
        let rows = &pane.agent.as_ref().unwrap().reader.rows;
        assert_eq!(rows.pending_questions().len(), 1);
    }

    #[test]
    fn go_with_picks_when_no_question_has_one_answers_nothing_and_names_them() {
        let (mut pane, seq) = pane(Origin::Claude);
        let toast = go_with_picks(&mut pane, 100.0);
        assert_eq!(
            toast.as_deref(),
            Some("No waiting question has a pick. Still open: Where?, Which?.")
        );
        assert_eq!(asked_of(&pane, seq), Asked::Pending);
        assert_eq!(typed(&mut pane), Vec::new());
    }

    #[test]
    fn answering_a_row_that_is_gone_types_nothing() {
        let (mut pane, seq) = pane(Origin::Claude);
        answer_question(&mut pane, seq + 50, Reply::Skipped, 100.0);
        assert_eq!(typed(&mut pane), Vec::new());
    }

    fn on_its_own(pane: &mut Pane<Fake>, level: tidewatch_core::agents::Level) {
        pane.agent.as_mut().unwrap().launched = Some(crate::pane::Launched {
            profile_id: "builder".into(),
            level,
            session_id: None,
            remote: false,
            dir: None,
        });
    }

    fn ask_with_recommended(pane: &mut Pane<Fake>) -> (u64, std::sync::mpsc::Receiver<Reply>) {
        let (sender, receiver) = channel();
        let row = QuestionRow {
            questions: vec![question::Question {
                text: "Which?".into(),
                header: None,
                decision: None,
                select: Select::Single,
                options: vec![
                    QuestionOption::new("Stage"),
                    QuestionOption::new("Run (Recommended)"),
                ],
            }],
            asked: Asked::Pending,
            origin: Origin::Tidewatch(AskReply::new(sender)),
        };
        let seq = pane.agent.as_mut().unwrap().reader.rows.push_question_row(row);
        (seq, receiver)
    }

    fn pane_without_questions() -> Pane<Fake> {
        let agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        Pane { body: crate::pane::Body::Shell(Fake::default()), worktree: None, agent: Some(agent) }
    }

    #[test]
    fn a_yolo_tab_answers_a_question_nobody_touched_with_the_recommended_option() {
        let ctx = Context::default();
        let mut pane = pane_without_questions();
        on_its_own(&mut pane, tidewatch_core::agents::Level::Yolo);
        let (seq, receiver) = ask_with_recommended(&mut pane);
        pick_for_the_absent(&mut pane, &ctx, 1_000.0);
        pick_for_the_absent(&mut pane, &ctx, 1_000.0 + crate::question_auto::WAIT_SECS - 1.0);
        assert!(receiver.try_recv().is_err());
        assert!(matches!(asked_of(&pane, seq), Asked::Pending));
        assert_eq!(pane.agent.as_ref().unwrap().run.pick_to_review(), None);
        let note = pick_for_the_absent(&mut pane, &ctx, 1_000.0 + crate::question_auto::WAIT_SECS);
        let picked = Given { picked: vec![1], other: None };
        assert_eq!(
            receiver.recv().unwrap(),
            Reply::PickedForYou { given: vec![picked], minutes: 5 }
        );
        assert_eq!(note, None, "the held call's result tells the agent, no message follows");
        assert!(matches!(asked_of(&pane, seq), Asked::Answered(_)));
        assert!(pane.agent.as_ref().unwrap().reader.rows.auto_picked(seq));
        let line = pane.agent.as_ref().unwrap().run.pick_to_review().map(str::to_owned);
        assert!(line.is_some_and(|line| line.ends_with(" for you")));
    }

    #[test]
    fn a_ready_answer_picked_elsewhere_goes_to_the_one_open_question_as_its_option_or_as_typed() {
        for (text, want) in [
            ("Run", Given { picked: vec![1], other: None }),
            ("Elsewhere", Given { picked: Vec::new(), other: Some("Elsewhere".into()) }),
        ] {
            let mut pane = pane_without_questions();
            let (seq, receiver) = ask_with_recommended(&mut pane);
            assert_eq!(answer_open(&mut pane, text, 5.0), Opened::Answered);
            assert_eq!(receiver.recv().unwrap(), Reply::Answered(vec![want]));
            assert!(matches!(asked_of(&pane, seq), Asked::Answered(_)));
        }
    }

    #[test]
    fn a_ready_answer_is_left_to_the_plain_reply_when_no_single_question_is_open() {
        let mut none = pane_without_questions();
        assert_eq!(answer_open(&mut none, "Run", 5.0), Opened::NotOpen);
        let (mut several, seq) = pane(Origin::Tidewatch(AskReply::new(channel().0)));
        assert_eq!(
            answer_open(&mut several, "Run", 5.0),
            Opened::NotOpen,
            "two questions in one call"
        );
        assert!(matches!(asked_of(&several, seq), Asked::Pending));
    }

    #[test]
    fn a_plain_tab_keeps_its_question_for_as_long_as_it_takes() {
        let ctx = Context::default();
        let mut pane = pane_without_questions();
        on_its_own(&mut pane, tidewatch_core::agents::Level::Edits);
        let (seq, receiver) = ask_with_recommended(&mut pane);
        for at in [0.0, 400.0, 4_000.0, 400_000.0] {
            pick_for_the_absent(&mut pane, &ctx, at);
        }
        assert!(receiver.try_recv().is_err());
        assert!(matches!(asked_of(&pane, seq), Asked::Pending));
        assert!(!pane.agent.as_ref().unwrap().reader.rows.auto_picked(seq));
        assert_eq!(pane.agent.as_ref().unwrap().run.pick_to_review(), None);
    }

    #[test]
    fn an_answer_given_before_the_wait_ends_is_never_overridden() {
        let ctx = Context::default();
        let mut pane = pane_without_questions();
        on_its_own(&mut pane, tidewatch_core::agents::Level::Yolo);
        let (seq, receiver) = ask_with_recommended(&mut pane);
        pick_for_the_absent(&mut pane, &ctx, 0.0);
        let mine = Given { picked: vec![0], other: None };
        answer_question(&mut pane, seq, Reply::Answered(vec![mine.clone()]), 10.0);
        pick_for_the_absent(&mut pane, &ctx, crate::question_auto::WAIT_SECS + 50.0);
        assert_eq!(receiver.recv().unwrap(), Reply::Answered(vec![mine]));
        assert!(receiver.try_recv().is_err());
        assert!(!pane.agent.as_ref().unwrap().reader.rows.auto_picked(seq));
        assert_eq!(pane.agent.as_ref().unwrap().run.pick_to_review(), None);
    }

    fn with_screen(pane: &mut Pane<Fake>, rows: &[&str]) {
        pane.body =
            crate::pane::Body::Shell(Fake(rows.iter().map(|row| (*row).to_owned()).collect()));
    }

    #[test]
    fn nothing_is_typed_into_a_claude_menu_the_screen_shows_is_gone() {
        let gone: [(&str, &[&str]); 2] = [
            ("the answer echo", &["User answered Claude's questions:", "Where? -> Run"]),
            ("the input box", &["> ", "Where? is what I asked"]),
        ];
        let replies = [
            ("answers the menu takes by keys", Reply::Answered(answered())),
            (
                "answers it cannot take by keys",
                Reply::Answered(vec![answered().remove(0), Given::default()]),
            ),
            ("a skip", Reply::Skipped),
        ];
        for (screen, rows) in gone {
            for (what, reply) in &replies {
                let (mut pane, seq) = pane(Origin::Claude);
                with_screen(&mut pane, rows);
                let refused = answer_question(&mut pane, seq, reply.clone(), 100.0);
                assert_eq!(refused, Some(MENU_GONE), "{what} on {screen}");
                assert_eq!(typed(&mut pane), Vec::new(), "{what} on {screen}");
                assert_eq!(asked_of(&pane, seq), Asked::Pending, "{what} on {screen}");
            }
        }
    }

    #[test]
    fn an_empty_or_blank_screen_is_no_proof_and_the_answer_is_typed() {
        for rows in [&[][..], &["", "  "][..]] {
            let (mut pane, seq) = pane(Origin::Claude);
            with_screen(&mut pane, rows);
            assert_eq!(answer_question(&mut pane, seq, Reply::Answered(answered()), 100.0), None);
            assert_eq!(typed(&mut pane).len(), 5, "{rows:?}");
        }
    }

    #[test]
    fn a_claude_menu_that_is_on_the_terminal_is_answered_and_nothing_is_refused() {
        let (mut pane, seq) = pane(Origin::Claude);
        assert_eq!(answer_question(&mut pane, seq, Reply::Answered(answered()), 100.0), None);
        assert_eq!(typed(&mut pane).len(), 5);
    }

    #[test]
    fn a_held_ask_user_call_needs_no_menu_on_the_terminal() {
        let (sender, receiver) = channel();
        let (mut pane, seq) = pane(Origin::Tidewatch(AskReply::new(sender)));
        with_screen(&mut pane, &[]);
        assert_eq!(answer_question(&mut pane, seq, Reply::Answered(answered()), 100.0), None);
        assert_eq!(receiver.recv().unwrap(), Reply::Answered(answered()));
    }

    #[test]
    fn go_with_picks_says_when_a_claude_menu_was_gone_and_types_nothing_into_it() {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        agent.reader.rows.push_all(Vec::new());
        let row = QuestionRow {
            questions: vec![question::Question {
                text: "Where?".into(),
                header: None,
                decision: None,
                select: Select::Single,
                options: vec![
                    QuestionOption::new("Stage (Recommended)"),
                    QuestionOption::new("Run"),
                ],
            }],
            asked: Asked::Pending,
            origin: Origin::Claude,
        };
        let seq = agent.reader.rows.push_question_row(row);
        let mut pane = Pane {
            body: crate::pane::Body::Shell(Fake(vec!["> ".into()])),
            worktree: None,
            agent: Some(agent),
        };
        let toast = go_with_picks(&mut pane, 100.0).unwrap();
        assert_eq!(toast, format!("Went with the picks for 1: Stage. {MENU_GONE}"));
        assert_eq!(typed(&mut pane), Vec::new());
        assert_eq!(asked_of(&pane, seq), Asked::Pending);
    }

    #[test]
    fn a_yolo_tab_does_not_count_a_pick_it_could_not_type_into_a_gone_menu() {
        let ctx = Context::default();
        let (mut pane, seq) = pane(Origin::Claude);
        with_screen(&mut pane, &["> "]);
        on_its_own(&mut pane, tidewatch_core::agents::Level::Yolo);
        let agent = pane.agent.as_mut().unwrap();
        agent.run.hold(Some(AgentStatus::Working), None);
        agent.run.refresh(1.0);
        pick_for_the_absent(&mut pane, &ctx, 0.0);
        pick_for_the_absent(&mut pane, &ctx, crate::question_auto::WAIT_SECS);
        assert_eq!(typed(&mut pane), Vec::new());
        assert_eq!(asked_of(&pane, seq), Asked::Pending);
        assert!(!pane.agent.as_ref().unwrap().reader.rows.auto_picked(seq));
    }

    fn idle_pty() -> PtyTerm {
        pty_running("sleep 20")
    }

    fn pty_running(script: &str) -> PtyTerm {
        use std::collections::BTreeMap;
        use stdusk_core::repaint::Repaint;
        use stdusk_core::shell::ShellProfile;
        use stdusk_core::terminal::SpawnOpts;
        let opts = SpawnOpts {
            detect_progress: false,
            shell_integration: false,
            autosuggestions: false,
            scrollback_lines: 100,
            word_separators: " ".into(),
            bold_bright: false,
            agent_tracking: None,
            cwd: None,
            profile: Some(ShellProfile {
                shell: Some("/bin/sh".into()),
                args: vec!["-c".into(), script.into()],
                cwd: None,
                env: BTreeMap::new(),
            }),
        };
        PtyTerm::spawn(75, 25, Repaint::none(), &opts)
    }

    #[test]
    fn a_message_typed_for_an_answer_gets_its_enter_from_the_enter_queue_after_the_quiet_wait() {
        let mut tabs = Tabs::new(Pane::creating());
        let agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        let pane = Pane { body: Body::Shell(idle_pty()), worktree: None, agent: Some(agent) };
        let id = tabs.push("agent".into(), pane);
        let mut flow = Flow::new(tidewatch_core::tools::Tools::default());
        let ctx = Context::default();
        let (sender, receiver) = channel();
        drop(receiver);
        let row = QuestionRow {
            questions: questions(),
            asked: Asked::Pending,
            origin: Origin::Tidewatch(AskReply::new(sender)),
        };
        let found = tabs.get_mut(id).unwrap();
        let seq = found.pane.agent.as_mut().unwrap().reader.rows.push_question_row(row);
        answer_question(&mut found.pane, seq, Reply::Answered(answered()), 10.0);
        drain(&mut tabs, &mut flow, &ctx, 10.0);
        let busy = |_: u64| Some(1);
        assert_eq!(flow.due_enters(10.0, busy), Vec::<u64>::new());
        assert_eq!(flow.due_enters(10.0 + ENTER_DELAY_SECS, busy), Vec::<u64>::new());
        let settled = 10.0 + ENTER_DELAY_SECS + crate::note_send::ENTER_QUIET_SECS + 0.01;
        assert_eq!(flow.due_enters(settled, busy), [id]);
    }

    fn agent_in(status: AgentStatus) -> (Tabs<Pane<PtyTerm>>, u64, Flow) {
        let mut tabs = Tabs::new(Pane::creating());
        let mut run = AgentRun::setting_up("claude".into(), 0.0);
        run.set_status(status);
        let agent = AgentTab::new("Builder".into(), run);
        let pane = Pane { body: Body::Shell(idle_pty()), worktree: None, agent: Some(agent) };
        let id = tabs.push("agent".into(), pane);
        (tabs, id, Flow::new(tidewatch_core::tools::Tools::default()))
    }

    fn claude_box(draft: &str, want: Screen) -> PtyTerm {
        let rule = "\u{2500}".repeat(40);
        let term = pty_running(&format!(
            "printf '%s\\n' 'Working...' '{rule}' '\u{276f} {draft}' '{rule}'; sleep 20"
        ));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while crate::prompt_ghost::screen_of(&term.grid_snapshot()) != want {
            assert!(std::time::Instant::now() < deadline, "the box never reached {want:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
        term
    }

    fn show_screen(tabs: &mut Tabs<Pane<PtyTerm>>, id: u64, term: PtyTerm) {
        tabs.get_mut(id).unwrap().pane.body = Body::Shell(term);
    }

    fn set_status(tabs: &mut Tabs<Pane<PtyTerm>>, id: u64, status: AgentStatus) {
        tabs.agent_mut(id).unwrap().run.set_status(status);
    }

    fn held(tabs: &mut Tabs<Pane<PtyTerm>>, id: u64) -> Vec<String> {
        let agent = tabs.agent_mut(id).unwrap();
        agent.reader.view.line.items().iter().map(|held| held.text.clone()).collect()
    }

    fn queued(tabs: &mut Tabs<Pane<PtyTerm>>, id: u64) -> usize {
        tabs.agent_mut(id).unwrap().reader.rows.queued_len()
    }

    fn say(
        tabs: &mut Tabs<Pane<PtyTerm>>,
        flow: &mut Flow,
        id: u64,
        text: &str,
        now: f64,
    ) -> Option<String> {
        apply(vec![Action::Reply(text.into())], tabs, id, flow, &Context::default(), now)
    }

    fn release(tabs: &mut Tabs<Pane<PtyTerm>>, flow: &mut Flow, now: f64) -> Vec<String> {
        release_lines(tabs, flow, &Context::default(), now)
    }

    #[test]
    fn a_reply_to_a_working_agent_whose_box_is_not_free_is_held_and_typed_when_the_turn_ends() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        assert_eq!(say(&mut tabs, &mut flow, id, "also check the Android build", 1.0), None);
        assert_eq!(held(&mut tabs, id), ["also check the Android build"]);
        assert_eq!(queued(&mut tabs, id), 0, "the agent has not seen it, so it is not queued");
        assert_eq!(release(&mut tabs, &mut flow, 2.0), Vec::<String>::new());
        assert_eq!(held(&mut tabs, id), ["also check the Android build"]);
        set_status(&mut tabs, id, AgentStatus::Done);
        release(&mut tabs, &mut flow, 3.0);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 1);
        let busy = |_: u64| Some(1);
        assert_eq!(flow.due_enters(3.0, busy), Vec::<u64>::new());
        assert_eq!(flow.due_enters(3.0 + ENTER_DELAY_SECS, busy), Vec::<u64>::new());
        let settled = 3.0 + ENTER_DELAY_SECS + crate::note_send::ENTER_QUIET_SECS + 0.01;
        assert_eq!(flow.due_enters(settled, busy), [id]);
    }

    #[test]
    fn a_reply_to_a_working_claude_is_typed_at_once_while_its_input_box_is_free() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        show_screen(&mut tabs, id, claude_box("", Screen::Free));
        assert_eq!(say(&mut tabs, &mut flow, id, "also check the Android build", 1.0), None);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 1, "it shows as queued until the transcript has it");
        assert!(flow.enter_pending(id), "the paste gets its Enter");
        let busy = |_: u64| Some(1);
        let settled = 1.0 + ENTER_DELAY_SECS + crate::note_send::ENTER_QUIET_SECS + 0.01;
        assert_eq!(flow.due_enters(settled, busy).len(), 0);
        assert_eq!(flow.due_enters(settled + 3.0, busy), [id]);
    }

    #[test]
    fn a_reply_to_a_working_claude_waits_while_its_box_holds_a_draft_or_a_dialog() {
        for screen in [Screen::Typed, Screen::NoBox] {
            let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
            let term = match screen {
                Screen::NoBox => idle_pty(),
                _ => claude_box("half a thought", screen),
            };
            show_screen(&mut tabs, id, term);
            say(&mut tabs, &mut flow, id, "also check the Android build", 1.0);
            assert_eq!(held(&mut tabs, id), ["also check the Android build"], "{screen:?}");
            assert_eq!(queued(&mut tabs, id), 0, "{screen:?}");
            assert!(!flow.enter_pending(id), "{screen:?}");
        }
    }

    #[test]
    fn a_held_reply_on_a_working_claude_is_typed_once_the_box_is_free_without_waiting_for_done() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        show_screen(&mut tabs, id, claude_box("half a thought", Screen::Typed));
        say(&mut tabs, &mut flow, id, "also check the Android build", 1.0);
        assert_eq!(release(&mut tabs, &mut flow, 2.0), Vec::<String>::new());
        assert_eq!(held(&mut tabs, id), ["also check the Android build"]);
        show_screen(&mut tabs, id, claude_box("", Screen::Free));
        release(&mut tabs, &mut flow, 3.0);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 1);
        assert_eq!(*tabs.agent_mut(id).unwrap().run.status(), AgentStatus::Working);
        assert!(flow.enter_pending(id));
    }

    #[test]
    fn a_second_mid_turn_reply_waits_until_the_first_has_its_enter() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        show_screen(&mut tabs, id, claude_box("", Screen::Free));
        say(&mut tabs, &mut flow, id, "one", 1.0);
        say(&mut tabs, &mut flow, id, "two", 1.1);
        assert_eq!(held(&mut tabs, id), ["two"]);
        assert_eq!(queued(&mut tabs, id), 1);
        flow.due_enters(10.0, |_| None);
        assert!(!flow.enter_pending(id));
        release(&mut tabs, &mut flow, 10.1);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 2);
    }

    #[test]
    fn a_reply_typed_mid_turn_leaves_the_queue_when_the_transcript_shows_it() {
        use tidewatch_core::transcript::{Entry, Role};
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        show_screen(&mut tabs, id, claude_box("", Screen::Free));
        say(&mut tabs, &mut flow, id, "also check the Android build", 1.0);
        assert_eq!(queued(&mut tabs, id), 1);
        tabs.agent_mut(id).unwrap().reader.rows.push(Entry {
            role: Role::User,
            text: "also check the Android build".into(),
            tool: None,
            ts: Some(now_ms()),
            images: Vec::new(),
        });
        assert_eq!(queued(&mut tabs, id), 0);
    }

    #[test]
    fn a_reply_to_a_working_codex_is_held_for_the_turn_even_with_a_free_box() {
        let mut tabs = Tabs::new(Pane::creating());
        let mut run = AgentRun::setting_up("codex".into(), 0.0);
        run.set_status(AgentStatus::Working);
        let pane = Pane {
            body: Body::Shell(claude_box("", Screen::Free)),
            worktree: None,
            agent: Some(AgentTab::new("Builder".into(), run)),
        };
        let id = tabs.push("agent".into(), pane);
        let mut flow = Flow::new(tidewatch_core::tools::Tools::default());
        say(&mut tabs, &mut flow, id, "also check the Android build", 1.0);
        assert_eq!(held(&mut tabs, id), ["also check the Android build"]);
        release(&mut tabs, &mut flow, 2.0);
        assert_eq!(held(&mut tabs, id), ["also check the Android build"]);
        set_status(&mut tabs, id, AgentStatus::Done);
        release(&mut tabs, &mut flow, 3.0);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
    }

    #[test]
    fn a_reply_to_an_agent_that_is_done_is_typed_at_once() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Done);
        assert_eq!(say(&mut tabs, &mut flow, id, "next, the docs", 1.0), None);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 1);
    }

    #[test]
    fn two_replies_in_a_row_never_merge_the_second_waits_for_the_agent_to_leave_done() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Done);
        say(&mut tabs, &mut flow, id, "one", 1.0);
        say(&mut tabs, &mut flow, id, "two", 1.5);
        assert_eq!(held(&mut tabs, id), ["two"]);
        assert_eq!(release(&mut tabs, &mut flow, 2.0), Vec::<String>::new());
        assert_eq!(held(&mut tabs, id), ["two"]);
        set_status(&mut tabs, id, AgentStatus::Working);
        release(&mut tabs, &mut flow, 3.0);
        set_status(&mut tabs, id, AgentStatus::Done);
        release(&mut tabs, &mut flow, 4.0);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 2);
    }

    #[test]
    fn the_line_keeps_its_order_and_types_one_message_per_idle_moment() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        for text in ["a", "b", "c"] {
            say(&mut tabs, &mut flow, id, text, 1.0);
        }
        set_status(&mut tabs, id, AgentStatus::Done);
        release(&mut tabs, &mut flow, 2.0);
        release(&mut tabs, &mut flow, 2.1);
        assert_eq!(held(&mut tabs, id), ["b", "c"]);
        assert_eq!(queued(&mut tabs, id), 1);
    }

    #[test]
    fn a_slash_command_runs_at_once_even_while_the_agent_works() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        say(&mut tabs, &mut flow, id, "/model opus", 1.0);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 0, "a command has no transcript line to wait for");
    }

    #[test]
    fn the_first_message_of_a_tab_goes_whatever_the_agent_reports() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::SettingUp);
        tabs.agent_mut(id).unwrap().first_prompt = FirstPrompt::Awaiting;
        assert_eq!(say(&mut tabs, &mut flow, id, "build the thing", 1.0), None);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(tabs.agent_mut(id).unwrap().first_prompt, FirstPrompt::Sent);
    }

    #[test]
    fn a_reply_to_an_agent_that_is_not_running_is_kept_and_the_toast_says_why() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Stopped);
        let toast = say(&mut tabs, &mut flow, id, "are you there", 1.0);
        assert_eq!(toast.as_deref(), Some("Resume first"));
        assert_eq!(held(&mut tabs, id), ["are you there"]);
        release(&mut tabs, &mut flow, 2.0);
        assert_eq!(held(&mut tabs, id), ["are you there"], "nothing is typed into a shell");
        set_status(&mut tabs, id, AgentStatus::Done);
        release(&mut tabs, &mut flow, 3.0);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
    }

    #[test]
    fn a_full_line_hands_the_message_back_to_the_box_and_says_so() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        for n in 0..crate::send_line::MAX_HELD {
            say(&mut tabs, &mut flow, id, &format!("m{n}"), 1.0);
        }
        let toast = say(&mut tabs, &mut flow, id, "one too many", 2.0);
        assert_eq!(toast.as_deref(), Some(LINE_FULL_TOAST));
        let agent = tabs.agent_mut(id).unwrap();
        assert_eq!(agent.reader.view.line.len(), crate::send_line::MAX_HELD);
        assert_eq!(agent.reader.view.reply, "one too many");
    }

    #[test]
    fn a_message_being_edited_holds_the_line_until_it_is_saved_or_left() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        say(&mut tabs, &mut flow, id, "first", 1.0);
        say(&mut tabs, &mut flow, id, "second", 1.0);
        let first = tabs.agent_mut(id).unwrap().reader.view.line.items()[0].id;
        tabs.agent_mut(id).unwrap().reader.view.begin_edit_held(first);
        set_status(&mut tabs, id, AgentStatus::Done);
        release(&mut tabs, &mut flow, 2.0);
        assert_eq!(held(&mut tabs, id), ["first", "second"]);
        tabs.agent_mut(id).unwrap().reader.view.line.set_text(first, "first, edited");
        tabs.agent_mut(id)
            .unwrap()
            .reader
            .view
            .apply_held(crate::held_view::Out::Cancel, &mut Vec::new());
        release(&mut tabs, &mut flow, 3.0);
        assert_eq!(held(&mut tabs, id), ["second"]);
    }

    #[test]
    fn send_now_stops_a_working_agent_and_types_the_message_after_the_esc() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        say(&mut tabs, &mut flow, id, "drop everything and do this", 1.0);
        let held_id = tabs.agent_mut(id).unwrap().reader.view.line.items()[0].id;
        let ctx = Context::default();
        assert_eq!(
            apply(vec![Action::SendNow(held_id)], &mut tabs, id, &mut flow, &ctx, 10.0),
            None
        );
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 1);
        let agent = tabs.agent_mut(id).unwrap();
        assert_eq!(
            agent.reader.take_due_step(10.0),
            Some(Step::Keys(reader_keys::INTERRUPT.to_vec()))
        );
        assert_eq!(agent.reader.take_due_step(10.0 + INTERRUPT_GAP_S - 0.1), None);
        assert_eq!(
            agent.reader.take_due_step(10.0 + INTERRUPT_GAP_S),
            Some(Step::Paste("drop everything and do this".into()))
        );
    }

    #[test]
    fn send_now_to_an_agent_that_is_not_working_types_at_once_without_an_esc() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        say(&mut tabs, &mut flow, id, "go", 1.0);
        set_status(&mut tabs, id, AgentStatus::Done);
        let held_id = tabs.agent_mut(id).unwrap().reader.view.line.items()[0].id;
        let ctx = Context::default();
        apply(vec![Action::SendNow(held_id)], &mut tabs, id, &mut flow, &ctx, 10.0);
        assert_eq!(held(&mut tabs, id), Vec::<String>::new());
        assert_eq!(queued(&mut tabs, id), 1);
        assert_eq!(tabs.agent_mut(id).unwrap().reader.take_due_step(99.0), None);
    }

    #[test]
    fn stop_means_esc_only_for_a_working_agent() {
        let cases = [
            (AgentStatus::Working, true),
            (AgentStatus::Done, false),
            (AgentStatus::NeedsYou(Reason::Permission), false),
            (AgentStatus::Stopped, false),
            (AgentStatus::Shell, false),
        ];
        for (status, want) in cases {
            assert_eq!(interrupts(&status), want, "{status:?}");
        }
    }

    #[test]
    fn send_again_replaces_the_queued_copy_and_goes_through_the_line() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Working);
        tabs.agent_mut(id).unwrap().reader.rows.queue_user("lost one", 0);
        let queued_id = tabs.agent_mut(id).unwrap().reader.rows.queued().next().unwrap().id;
        let again = Action::SendAgain { text: "lost one".into(), forget: Some(queued_id) };
        apply(vec![again], &mut tabs, id, &mut flow, &Context::default(), 5.0);
        assert_eq!(queued(&mut tabs, id), 0);
        assert_eq!(held(&mut tabs, id), ["lost one"], "it waits for the turn like any reply");
    }

    #[test]
    fn dismiss_forgets_a_queued_message_without_sending_it() {
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Done);
        tabs.agent_mut(id).unwrap().reader.rows.queue_user("lost one", 0);
        let queued_id = tabs.agent_mut(id).unwrap().reader.rows.queued().next().unwrap().id;
        apply(vec![Action::Forget(queued_id)], &mut tabs, id, &mut flow, &Context::default(), 5.0);
        assert_eq!((queued(&mut tabs, id), held(&mut tabs, id)), (0, Vec::<String>::new()));
    }

    #[test]
    fn a_sent_message_is_remembered_for_up_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("tab").join("notes.json");
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Done);
        tabs.agent_mut(id).unwrap().run.notes_file = Some(notes.clone());
        let ctx = Context::default();
        drain(&mut tabs, &mut flow, &ctx, 1.0);
        say(&mut tabs, &mut flow, id, "run the tests", 2.0);
        assert_eq!(tabs.agent_mut(id).unwrap().reader.view.recall.entries(), ["run the tests"]);
        let (mut again, id2, mut flow2) = agent_in(AgentStatus::Done);
        again.agent_mut(id2).unwrap().run.notes_file = Some(notes);
        drain(&mut again, &mut flow2, &ctx, 1.0);
        let recall = &again.agent_mut(id2).unwrap().reader.view.recall;
        assert_eq!(recall.entries(), ["run the tests"]);
    }

    #[test]
    fn a_half_written_reply_is_saved_once_it_stood_still_and_comes_back_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("tab").join("notes.json");
        let ctx = Context::default();
        let (mut tabs, id, mut flow) = agent_in(AgentStatus::Done);
        tabs.agent_mut(id).unwrap().run.notes_file = Some(notes.clone());
        drain(&mut tabs, &mut flow, &ctx, 1.0);
        tabs.agent_mut(id).unwrap().reader.view.reply = "check the totals in".into();
        drain(&mut tabs, &mut flow, &ctx, 2.0);
        drain(&mut tabs, &mut flow, &ctx, 2.0 + crate::reply_store::SETTLE_SECS + 0.01);
        let (mut again, id2, mut flow2) = agent_in(AgentStatus::Done);
        again.agent_mut(id2).unwrap().run.notes_file = Some(notes);
        drain(&mut again, &mut flow2, &ctx, 1.0);
        assert_eq!(again.agent_mut(id2).unwrap().reader.view.reply, "check the totals in");
    }

    fn claude_menu_pane(screen: &[&str]) -> (Pane<Fake>, u64) {
        let (mut pane, seq) = pane(Origin::Claude);
        with_screen(&mut pane, screen);
        (pane, seq)
    }

    #[test]
    fn the_card_of_a_claude_menu_closes_after_the_menu_stayed_off_the_screen() {
        let (mut pane, seq) = claude_menu_pane(&["> ", "Where? is what I asked"]);
        close_gone_menu(&mut pane, 10.0);
        assert_eq!(asked_of(&pane, seq), Asked::Pending, "a screen can lag for a moment");
        close_gone_menu(&mut pane, 10.0 + MENU_GONE_AFTER_S - 0.1);
        assert_eq!(asked_of(&pane, seq), Asked::Pending);
        close_gone_menu(&mut pane, 10.0 + MENU_GONE_AFTER_S);
        assert_eq!(asked_of(&pane, seq), Asked::Closed);
    }

    #[test]
    fn the_card_stays_while_the_menu_is_on_screen_or_the_screen_cannot_be_read() {
        for screen in [vec!["Where?", "\u{276f} 1. Stage", "  2. Run"], vec![]] {
            let (mut pane, seq) = claude_menu_pane(&screen);
            close_gone_menu(&mut pane, 10.0);
            close_gone_menu(&mut pane, 100.0);
            assert_eq!(asked_of(&pane, seq), Asked::Pending, "{screen:?}");
        }
    }

    #[test]
    fn a_menu_that_comes_back_restarts_the_wait() {
        let (mut pane, seq) = claude_menu_pane(&["> ", "nothing"]);
        close_gone_menu(&mut pane, 10.0);
        with_screen(&mut pane, &["Where?", "\u{276f} 1. Stage", "  2. Run"]);
        close_gone_menu(&mut pane, 11.0);
        with_screen(&mut pane, &["> ", "nothing"]);
        close_gone_menu(&mut pane, 12.5);
        assert_eq!(asked_of(&pane, seq), Asked::Pending);
        close_gone_menu(&mut pane, 12.5 + MENU_GONE_AFTER_S);
        assert_eq!(asked_of(&pane, seq), Asked::Closed);
    }
}
