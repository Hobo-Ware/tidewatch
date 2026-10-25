//! The seams between the Reader and the rest of the app: starting each agent tab's feed, toggling
//! the view, drawing it in the terminal area and typing what the user chose into the pty.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui::{Context, Rect, RichText, Ui, UiBuilder, pos2, vec2};
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::avatar::Avatar;
use tidewatch_core::question::{self, Reply};
use tidewatch_core::status::{AgentStatus, Reason};

use crate::agent_avatar::Face;
use crate::agent_ready::Ready;
use crate::brand;
use crate::commands::Command;
use crate::dev_reader::DevReader;
use crate::due::{Due, ask_blocked};
use crate::due_view::{self, Facts as DueFacts};
use crate::flow::Flow;
use crate::note_send::ENTER_DELAY_SECS;
use crate::pane::{AgentTab, Body, FirstPrompt, Pane};
use crate::pr_card_model::CardAction;
use crate::pr_cards_view::Data as PrData;
use crate::question_bulk;
use crate::reader::{Note, Shown, Step};
use crate::reader_keys;
use crate::reader_rows::{Asked, Kind, Origin};
use crate::reader_view::{self, Action, Facts, Prompt, Turn};
use crate::review::ReviewFacts;
use crate::tabs::Tabs;

const NOTE_H: f32 = 24.0;
const NOTE_PAD: f32 = 8.0;
const WAKE_SLACK_S: f64 = 0.02;
/// The pause between two key presses typed into an agent's menu.
const KEY_GAP_S: f64 = ENTER_DELAY_SECS;
/// How long a menu needs to close after Esc before text can follow.
const MENU_CLOSE_S: f64 = 0.8;

/// Starts the feed of every agent tab that has an agent running and none yet.
pub(crate) fn start(tabs: &mut Tabs<Pane<PtyTerm>>, ctx: &Context, dev: &DevReader) {
    for tab in tabs.items_mut() {
        let Some(agent) = tab.pane.agent.as_mut() else { continue };
        if !matches!(tab.pane.body, Body::Shell(_)) || !agent.reader.needs_start() {
            continue;
        }
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { continue };
        if dev.raw {
            agent.reader.set_wants_reader(false);
        }
        let ctx = ctx.clone();
        agent.reader.start(&home, move || ctx.request_repaint());
    }
}

/// Takes what every feed read since the last frame, and types what is due into the tabs that were
/// answering a question.
pub(crate) fn drain(tabs: &mut Tabs<Pane<PtyTerm>>, ctx: &Context, now: f64) {
    for tab in tabs.items_mut() {
        if let Pane { body: Body::Shell(term), agent: Some(agent), .. } = &mut tab.pane {
            while let Some(step) = agent.reader.take_due_step(now) {
                match step {
                    Step::Keys(bytes) => term.send(&bytes),
                    Step::Paste(text) => term.paste(&text),
                }
            }
            if let Some(due) = agent.reader.next_step_due() {
                ctx.request_repaint_after(Duration::from_secs_f64(
                    (due - now).max(0.0) + WAKE_SLACK_S,
                ));
            }
        }
        if let Some(agent) = tab.pane.agent.as_mut() {
            agent.reader.drain();
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

fn prompt_of(status: &AgentStatus) -> Option<Prompt> {
    match status {
        AgentStatus::NeedsYou(Reason::Permission) => Some(Prompt::Permission),
        AgentStatus::NeedsYou(Reason::Question) => Some(Prompt::Question),
        _ => None,
    }
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
    avatar: Option<Arc<Avatar>>,
) -> Vec<Action> {
    let Some(agent) = pane.agent.as_mut() else { return Vec::new() };
    let prompt = prompt_of(agent.run.status()).filter(|_| !agent.reader.prompt_answered());
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
    let facts = Facts {
        tab,
        source: agent.reader.source(),
        prompt,
        ready,
        root,
        review,
        pr,
        due,
        profile: &agent.profile_name,
        face: Some(&face),
        turn,
        stopped: *agent.run.status() == AgentStatus::Stopped,
    };
    let reader = &mut agent.reader;
    if dev.take_open_turns(reader.rows.len()) {
        reader.view.open_turns(&reader.rows, dev.notes_only());
    }
    let mut actions = reader_view::show(ui, area, &reader.rows, &mut reader.view, facts);
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
        let Some(found) = tabs.get_mut(tab) else { continue };
        let source = found.pane.agent.as_ref().and_then(|agent| agent.reader.source());
        match action {
            Action::Reply(text) => {
                if let Some(agent) = found.pane.agent.as_mut()
                    && agent.first_prompt == FirstPrompt::Awaiting
                {
                    agent.first_prompt = FirstPrompt::Sent;
                }
                if let Body::Shell(term) = &mut found.pane.body {
                    term.paste(&text);
                    flow.queue_enter(tab, now);
                    ctx.request_repaint_after(Duration::from_secs_f64(
                        ENTER_DELAY_SECS + WAKE_SLACK_S,
                    ));
                }
            }
            Action::Answer(answer) => {
                match (&mut found.pane.body, reader_keys::bytes(source, answer)) {
                    (Body::Shell(term), Some(keys)) => {
                        term.send(keys);
                        if let Some(agent) = found.pane.agent.as_mut() {
                            agent.reader.mark_answered();
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
            Action::Resume => resume_agent(&mut found.pane),
            Action::Review(_) | Action::Pr(_) | Action::Due(_) => {}
            Action::Ask(seq, reply) => answer_question(&mut found.pane, seq, reply, now),
            Action::GoWithPicks => toast = go_with_picks(&mut found.pane, now),
        }
    }
    toast
}

/// Runs the resume command of a tab whose agent waits at its shell prompt. Ctrl+U first, so a
/// command that was typed there is not run twice.
pub(crate) fn resume_agent(pane: &mut Pane<PtyTerm>) {
    if let (Body::Shell(term), Some(agent)) = (&mut pane.body, pane.agent.as_mut()) {
        resume_in(term, agent);
    }
}

pub(crate) fn resume_in(term: &mut PtyTerm, agent: &mut AgentTab) {
    let Some(line) = agent.resume_line.as_deref() else { return };
    term.send(format!("\u{15}{line}\r").as_bytes());
    agent.resume_note = None;
}

/// Pasted text and the Enter that sends it, `lead` seconds from now.
fn reply_steps(text: String, lead: f64) -> Vec<(f64, Step)> {
    vec![(lead, Step::Paste(text)), (ENTER_DELAY_SECS, Step::Keys(b"\r".to_vec()))]
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

/// Answers every waiting question that has a pick with it, and says which were answered and which
/// stay open.
fn go_with_picks<T>(pane: &mut Pane<T>, now: f64) -> Option<String> {
    let bulk = question_bulk::plan(&pane.agent.as_ref()?.reader.rows);
    let text = question_bulk::toast(&bulk);
    for answer in bulk.answers {
        answer_question(pane, answer.seq, Reply::Answered(answer.given), now);
    }
    Some(text)
}

/// Gives the user's answer to the questions in row `seq`. A held `ask_user` call takes it through
/// its channel, or as a typed message when the call already ended. Claude's menu takes key presses,
/// one a gap apart; a skip closes the menu with Esc and then says so.
fn answer_question<T>(pane: &mut Pane<T>, seq: u64, reply: Reply, now: f64) {
    let Some(agent) = pane.agent.as_mut() else { return };
    let Some(asked) = agent.reader.rows.question_of(seq).cloned() else { return };
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
        (Origin::Tidewatch(held), Reply::Skipped) => {
            let steps = if held.send(Reply::Skipped) {
                Vec::new()
            } else {
                reply_steps(question::SKIP_REPLY.to_owned(), 0.0)
            };
            (steps, Asked::Declined)
        }
        (Origin::Claude, Reply::Answered(given)) => {
            let pairs = question::pairs(&asked.questions, &given);
            let steps = claude_menu_steps(&asked.questions, &given);
            (steps, Asked::Sent(pairs))
        }
        (Origin::Claude, Reply::Skipped) => {
            let mut steps = vec![(0.0, Step::Keys(reader_keys::CLAUDE_MENU_ESCAPE.to_vec()))];
            steps.extend(reply_steps(question::SKIP_REPLY.to_owned(), MENU_CLOSE_S));
            (steps, Asked::Declined)
        }
    };
    agent.reader.rows.set_asked(seq, state);
    agent.reader.type_steps(steps, now);
    let next = agent.reader.rows.pending_question().and_then(|row| match &row.kind {
        Kind::Question(q) if matches!(q.origin, Origin::Tidewatch(_)) => {
            q.questions.first().map(|question| question.text.clone())
        }
        _ => None,
    });
    agent.run.set_asking(next);
    if matches!(asked.origin, Origin::Claude) {
        agent.reader.mark_answered();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use tidewatch_core::question::{AskReply, Given, QuestionOption, Select};

    use crate::agent_run::AgentRun;
    use crate::pane::AgentTab;
    use crate::reader_rows::QuestionRow;

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

    fn pane(origin: Origin) -> (Pane<()>, u64) {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        let row = QuestionRow { questions: questions(), asked: Asked::Pending, origin };
        agent.reader.rows.push_all(Vec::new());
        let seq = agent.reader.rows.push_question_row(row);
        let pane = Pane { body: crate::pane::Body::Shell(()), worktree: None, agent: Some(agent) };
        (pane, seq)
    }

    fn typed(pane: &mut Pane<()>) -> Vec<(f64, Step)> {
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

    fn asked_of(pane: &Pane<()>, seq: u64) -> Asked {
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
        assert_eq!(steps[2], Step::Keys(b"\r".to_vec()));
    }

    #[test]
    fn skipping_a_claude_menu_escapes_then_says_so() {
        let (mut pane, seq) = pane(Origin::Claude);
        answer_question(&mut pane, seq, Reply::Skipped, 100.0);
        let typed = typed(&mut pane);
        assert_eq!(typed[0], (0.0, Step::Keys(b"\x1b".to_vec())));
        assert_eq!(typed[1].1, Step::Paste(question::SKIP_REPLY.into()));
        assert!(typed[1].0 >= MENU_CLOSE_S - 0.001);
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
    fn a_late_answer_to_an_ask_user_call_that_ended_is_typed_as_a_message() {
        let (sender, receiver) = channel();
        drop(receiver);
        let (mut pane, seq) = pane(Origin::Tidewatch(AskReply::new(sender)));
        answer_question(&mut pane, seq, Reply::Answered(answered()), 100.0);
        let steps = steps_only(typed(&mut pane));
        let Step::Paste(text) = &steps[0] else { panic!("{steps:?}") };
        assert!(text.contains("Q: Where?\nA: Run"), "{text}");
        assert_eq!(steps[1], Step::Keys(b"\r".to_vec()));
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
        assert_eq!(steps_only(typed(&mut dead))[0], Step::Paste(question::SKIP_REPLY.into()));
        assert_eq!(asked_of(&dead, seq), Asked::Declined);
    }

    fn ask(pane: &mut Pane<()>, labels: &[&str]) -> std::sync::mpsc::Receiver<Reply> {
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
        let mut pane =
            Pane { body: crate::pane::Body::Shell(()), worktree: None, agent: Some(agent) };
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
}
