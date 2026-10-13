//! The seams between the Reader and the rest of the app: starting each agent tab's feed, toggling
//! the view, drawing it in the terminal area and typing what the user chose into the pty.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Context, Rect, RichText, Ui, UiBuilder, pos2, vec2};
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::colors;
use tidewatch_core::status::{AgentStatus, Reason};

use crate::commands::Command;
use crate::dev_reader::DevReader;
use crate::flow::Flow;
use crate::note_send::ENTER_DELAY_SECS;
use crate::pane::{Body, Pane};
use crate::reader::{Note, Shown};
use crate::reader_keys;
use crate::reader_view::{self, Action, Facts, Prompt};
use crate::tabs::Tabs;

const NOTE_H: f32 = 24.0;
const NOTE_PAD: f32 = 8.0;
const WAKE_SLACK_S: f64 = 0.02;

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

/// Takes what every feed read since the last frame.
pub(crate) fn drain(tabs: &mut Tabs<Pane<PtyTerm>>) {
    for tab in tabs.items_mut() {
        if let Some(agent) = tab.pane.agent.as_mut() {
            agent.reader.drain();
        }
    }
}

/// The Reader chord on the active tab. `None` for a tab that is not an agent's.
pub(crate) fn toggle(pane: &mut Pane<PtyTerm>) -> Option<Shown> {
    pane.agent.as_mut().map(|agent| agent.reader.toggle())
}

pub(crate) fn shown(pane: &Pane<PtyTerm>) -> Option<Shown> {
    pane.agent.as_ref().map(|agent| agent.reader.shown())
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
    pane: &mut Pane<PtyTerm>,
    dev: &mut DevReader,
    now: f64,
) -> Vec<Action> {
    let Some(agent) = pane.agent.as_mut() else { return Vec::new() };
    let prompt = prompt_of(agent.run.status()).filter(|_| !agent.reader.prompt_answered());
    let facts = Facts { source: agent.reader.source(), prompt };
    let reader = &mut agent.reader;
    let mut actions = reader_view::show(ui, area, &reader.rows, &mut reader.view, facts);
    actions.extend(dev.actions(now, facts.prompt));
    actions
}

/// The small note over the terminal that says why it is shown instead of the Reader.
pub(crate) fn note(ui: &mut Ui, area: Rect, note: Note) {
    let text = note.text();
    let galley =
        ui.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(12.0), colors::dim());
    let size = galley.size() + vec2(2.0 * NOTE_PAD, 0.0);
    let rect = Rect::from_min_size(
        pos2(area.right() - size.x - NOTE_PAD, area.top() + NOTE_PAD),
        vec2(size.x, NOTE_H),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    egui::Frame::new()
        .fill(colors::elevated())
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(&mut child, |ui| {
            ui.label(RichText::new(text).size(12.0).color(colors::dim()));
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
        }
    }
    toast
}
