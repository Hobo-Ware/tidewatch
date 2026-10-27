//! What happens to the agent tabs each frame, apart from drawing: messages from the listener go to
//! the tab they belong to, terminal signals feed each tab's status, a changed status may notify, and
//! the result says when the UI next needs a frame. Generic over the pty so it runs on plain data in
//! tests.

use std::path::PathBuf;

use tidewatch_core::avatar::Avatar;
use tidewatch_core::background::Signal;
use tidewatch_core::events::{TabEvent, TabMessage};
use tidewatch_core::job::Report;
use tidewatch_core::link_board::Link;
use tidewatch_core::progress::Ledger;
use tidewatch_core::services_mcp::ServiceCall;
use tidewatch_core::status::HookKind;

use crate::agent_run::{Change, Finding, TermSnapshot};
use crate::notify::Notifier;
use crate::pane::{Body, Pane};
use crate::remote_session::Step;
use crate::status_view::{self, Tone};
use crate::tabs::Tabs;

/// When the UI next needs a frame because of agent status.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Wake {
    /// A tab is working: its bar moves.
    pub(crate) animate: bool,
    /// The earliest moment a status changes with no new input, on the egui clock.
    pub(crate) next_change: Option<f64>,
}

/// What one pass over the tabs found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Polled {
    pub(crate) wake: Wake,
    /// Every status that changed, by tab id.
    pub(crate) changes: Vec<(u64, Change)>,
}

/// What a message asks the app to do beyond updating the tab's run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Follow {
    /// Add the finding to the tab's notes file.
    Finding {
        tab: u64,
        notes_file: PathBuf,
        dir: Option<PathBuf>,
        author: String,
        finding: Finding,
    },
    /// The listener wrote the tab's notes file: read it again.
    ReloadNotes {
        tab: u64,
    },
    /// The agent sent its progress ledger; it is already saved in the tab's state folder.
    Progress {
        tab: u64,
        ledger: Box<Ledger>,
    },
    /// The agent reported links; they are already saved in the tab's state folder.
    Links {
        tab: u64,
        links: Vec<Link>,
    },
    /// The agent reported a job; the tab's board takes it.
    Job {
        tab: u64,
        report: Box<Report>,
    },
    /// The agent drew an avatar; it is already saved in the tab's state folder.
    Avatar {
        tab: u64,
        avatar: Box<Avatar>,
    },
    /// A background call of the agent started or ended.
    Background {
        tab: u64,
        signal: Signal,
    },
    MeeseeksDone {
        tab: u64,
        summary: String,
    },
    /// The agent called a repo service tool; its call waits for the app's answer.
    Services {
        tab: u64,
        call: ServiceCall,
    },
}

/// Hands one listener message to its tab. A message for a closed tab is dropped.
pub(crate) fn route<T>(tabs: &mut Tabs<Pane<T>>, message: TabMessage, now: f64) -> Option<Follow> {
    let tab = tabs.get_mut(message.tab_id)?;
    let agent = tab.pane.agent.as_mut()?;
    let follow = match &message.event {
        TabEvent::Finding { file, line, blocking, text } => {
            agent.run.notes_file.clone().map(|notes_file| Follow::Finding {
                tab: message.tab_id,
                notes_file,
                dir: tab.pane.worktree.as_ref().map(|worktree| worktree.path.clone()),
                author: agent.run.agent_id.clone(),
                finding: Finding {
                    file: file.clone(),
                    line: *line,
                    blocking: *blocking,
                    text: text.clone(),
                },
            })
        }
        TabEvent::NoteReply { .. } => Some(Follow::ReloadNotes { tab: message.tab_id }),
        TabEvent::Progress { ledger } => {
            Some(Follow::Progress { tab: message.tab_id, ledger: ledger.clone() })
        }
        TabEvent::Links { links } => {
            Some(Follow::Links { tab: message.tab_id, links: links.clone() })
        }
        TabEvent::Ask { questions, reply } => {
            agent.run.set_asking(questions.first().map(|question| question.text.clone()));
            agent.reader.ask_arrived(questions.clone(), reply.clone());
            None
        }
        TabEvent::Job { report } => {
            Some(Follow::Job { tab: message.tab_id, report: Box::new(report.clone()) })
        }
        TabEvent::Background(signal) => {
            Some(Follow::Background { tab: message.tab_id, signal: signal.clone() })
        }
        TabEvent::Avatar { avatar } => {
            Some(Follow::Avatar { tab: message.tab_id, avatar: Box::new(avatar.clone()) })
        }
        TabEvent::MeeseeksDone { summary } => agent
            .meeseeks
            .is_some()
            .then(|| Follow::MeeseeksDone { tab: message.tab_id, summary: summary.clone() }),
        TabEvent::MeeseeksName { name } => {
            if let Some(meeseeks) = agent.meeseeks.as_mut() {
                meeseeks.name = Some(name.clone());
            }
            None
        }
        TabEvent::Services(call) => {
            Some(Follow::Services { tab: message.tab_id, call: call.clone() })
        }
        TabEvent::Hook(_) => None,
    };
    if let TabEvent::Hook(hook) = &message.event {
        agent.reader.hook(hook.transcript_path.clone());
        if matches!(hook.kind, HookKind::TurnDone)
            && let Some(meeseeks) = agent.meeseeks.as_mut()
        {
            meeseeks.turn();
        }
    }
    agent.run.on_event(message.event, now);
    follow
}

/// Feeds every agent tab's terminal signals into its status, notifies for changes while the window
/// is `away`, and returns what changed and when the UI next needs a frame.
pub(crate) fn poll<T>(
    tabs: &mut Tabs<Pane<T>>,
    snapshot: impl Fn(&T) -> TermSnapshot,
    notifier: &mut Notifier,
    away: bool,
    now: f64,
) -> Polled {
    let mut wake = Wake::default();
    let mut changes = Vec::new();
    for tab in tabs.items_mut() {
        let Some(agent) = tab.pane.agent.as_mut() else { continue };
        if let Body::Shell(term) = &tab.pane.body {
            let seen = snapshot(term);
            if seen.activity {
                agent.reader.poke(now);
            }
            agent.run.feed(seen, now);
        }
        if let Some(change) = agent.run.refresh(now) {
            let label = crate::tab_title::of_agent(agent, &tab.title).1;
            notifier.offer(tab.id, &label, &change, agent.run.detail(), away, now);
            changes.push((tab.id, change));
        }
        if status_view::is_animated(agent.run.shown_tone()) {
            wake.animate = true;
        }
        if let Some(at) = agent.run.next_change_at(now) {
            wake.next_change = Some(wake.next_change.map_or(at, |soonest| soonest.min(at)));
        }
    }
    Polled { wake, changes }
}

/// The terminal rows a look for the remote link reads, and how wide the terminal is.
pub(crate) struct Rows {
    pub(crate) lines: Vec<String>,
    pub(crate) cols: usize,
}

/// Looks at the terminal of every agent tab that runs with remote control and has shown no link yet.
/// `output` is a cheap counter that moves with the terminal's output; `rows` reads the buffer and
/// is only called when there is new output and the last look was not just now. Returns the time
/// the next look is due when output came in too soon after one.
pub(crate) fn watch_links<T>(
    tabs: &mut Tabs<Pane<T>>,
    output: impl Fn(&T) -> u64,
    rows: impl Fn(&T) -> Rows,
    now: f64,
) -> Option<f64> {
    let mut later: Option<f64> = None;
    for tab in tabs.items_mut() {
        let Pane { body: Body::Shell(term), agent: Some(agent), .. } = &mut tab.pane else {
            continue;
        };
        let Some(session) = agent.remote.as_mut() else { continue };
        let seen = output(term);
        match session.step(seen, now) {
            Step::Rest => {}
            Step::Scan => {
                let Rows { lines, cols } = rows(term);
                session.scanned(seen, now, &lines, cols, &agent.run.agent_id);
            }
            Step::Later(at) => later = Some(later.map_or(at, |soonest| soonest.min(at))),
        }
    }
    later
}

/// Tones of every agent tab, in tab order (for the Overview button's badge and tests).
pub(crate) fn tones<T>(tabs: &Tabs<Pane<T>>) -> Vec<Tone> {
    tabs.items()
        .iter()
        .filter_map(|tab| tab.pane.agent.as_ref())
        .map(|agent| agent.run.shown_tone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use stdusk_core::progress::Progress;
    use tidewatch_core::status::{HookEvent, HookKind};

    use crate::agent_run::AgentRun;
    use crate::pane::AgentTab;

    fn tabs_with_agent() -> (Tabs<Pane<u32>>, u64) {
        let mut run = AgentRun::setting_up("claude".into(), 0.0);
        run.started(Some(PathBuf::from("/state/notes.json")), 0.0);
        let pane = Pane {
            body: Body::Shell(1),
            worktree: None,
            agent: Some(AgentTab::new("Builder".into(), run)),
        };
        let mut tabs = Tabs::new(Pane::shell(0));
        let id = tabs.push("brave-otter".into(), pane);
        (tabs, id)
    }

    fn hook(tab: u64, kind: HookKind, message: Option<&str>) -> TabMessage {
        TabMessage {
            tab_id: tab,
            generation: 0,
            event: TabEvent::Hook(HookEvent {
                kind,
                session_id: None,
                transcript_path: None,
                message: message.map(str::to_owned),
            }),
        }
    }

    fn recording_notifier() -> (Notifier, Arc<Mutex<Vec<String>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&sent);
        (Notifier::new(move |n| sink.lock().unwrap().push(n.body)), sent)
    }

    #[test]
    fn a_hook_for_a_tab_updates_that_tabs_status_and_one_for_a_closed_tab_is_dropped() {
        let quiet = |_: &u32| TermSnapshot::default();
        let (mut tabs, id) = tabs_with_agent();
        let (mut notifier, _) = recording_notifier();
        poll(&mut tabs, quiet, &mut notifier, false, 1.0);
        assert_eq!(route(&mut tabs, hook(id, HookKind::TurnStarted, None), 2.0), None);
        let prompt = hook(id, HookKind::PermissionNeeded, Some("Bash: touch hello.txt"));
        assert_eq!(route(&mut tabs, prompt, 3.0), None);
        poll(&mut tabs, quiet, &mut notifier, false, 3.0);
        assert_eq!(tones(&tabs), [Tone::Needs]);
        assert_eq!(route(&mut tabs, hook(99, HookKind::TurnDone, None), 4.0), None);
        assert_eq!(route(&mut tabs, hook(0, HookKind::TurnDone, None), 4.0), None);
    }

    #[test]
    fn an_ask_user_call_waits_in_the_reader_and_a_working_agent_shows_as_needing_you() {
        let quiet = |_: &u32| TermSnapshot::default();
        let (mut tabs, id) = tabs_with_agent();
        let (mut notifier, _) = recording_notifier();
        route(&mut tabs, hook(id, HookKind::TurnStarted, None), 1.0);
        poll(&mut tabs, quiet, &mut notifier, false, 1.0);
        assert_eq!(tones(&tabs), [Tone::Working]);
        let (sender, _receiver) = std::sync::mpsc::channel();
        let question = tidewatch_core::question::Question {
            text: "Where?".into(),
            header: None,
            decision: None,
            select: tidewatch_core::question::Select::Single,
            options: vec![],
        };
        let message = TabMessage {
            tab_id: id,
            generation: 0,
            event: TabEvent::Ask {
                questions: vec![question],
                reply: tidewatch_core::question::AskReply::new(sender),
            },
        };
        assert_eq!(route(&mut tabs, message, 2.0), None);
        poll(&mut tabs, quiet, &mut notifier, false, 2.0);
        assert_eq!(tones(&tabs), [Tone::Needs]);
        let agent = tabs.get_mut(id).unwrap().pane.agent.as_ref().unwrap();
        assert!(agent.reader.rows.pending_question().is_some());
        assert_eq!(agent.run.detail(), Some("Where?"));
    }

    #[test]
    fn a_finding_asks_for_a_note_in_the_tabs_notes_file() {
        let (mut tabs, id) = tabs_with_agent();
        let message = TabMessage {
            tab_id: id,
            generation: 0,
            event: TabEvent::Finding {
                file: "src/a.rs".into(),
                line: 4,
                blocking: true,
                text: "unwrap".into(),
            },
        };
        let follow = route(&mut tabs, message, 1.0).unwrap();
        assert_eq!(
            follow,
            Follow::Finding {
                tab: id,
                notes_file: PathBuf::from("/state/notes.json"),
                dir: None,
                author: "claude".into(),
                finding: Finding {
                    file: "src/a.rs".into(),
                    line: 4,
                    blocking: true,
                    text: "unwrap".into()
                },
            }
        );
        let agent = tabs.get_mut(id).unwrap().pane.agent.take().unwrap();
        assert_eq!(agent.run.report.finding.unwrap().file, "src/a.rs");
    }

    #[test]
    fn a_ledger_the_agent_sends_asks_for_it_to_be_kept_for_that_tab() {
        let (mut tabs, id) = tabs_with_agent();
        let ledger = Ledger::parse("# Goal\nShip it\n");
        let message = TabMessage {
            tab_id: id,
            generation: 0,
            event: TabEvent::Progress { ledger: Box::new(ledger.clone()) },
        };
        assert_eq!(
            route(&mut tabs, message, 1.0),
            Some(Follow::Progress { tab: id, ledger: Box::new(ledger) })
        );
        let gone = TabMessage {
            tab_id: 99,
            generation: 0,
            event: TabEvent::Progress { ledger: Box::default() },
        };
        assert_eq!(route(&mut tabs, gone, 1.0), None);
    }

    #[test]
    fn a_note_reply_asks_for_the_notes_to_be_read_again() {
        let (mut tabs, id) = tabs_with_agent();
        let reply = TabMessage {
            tab_id: id,
            generation: 0,
            event: TabEvent::NoteReply { note_id: 1, text: "ok".into() },
        };
        assert_eq!(route(&mut tabs, reply, 1.0), Some(Follow::ReloadNotes { tab: id }));
    }

    #[test]
    fn a_change_notifies_only_while_the_window_is_away() {
        let quiet = |_: &u32| TermSnapshot::default();
        let (mut tabs, id) = tabs_with_agent();
        let (mut notifier, sent) = recording_notifier();
        poll(&mut tabs, quiet, &mut notifier, true, 1.0);
        route(&mut tabs, hook(id, HookKind::PermissionNeeded, Some("Bash: touch hello.txt")), 2.0);
        poll(&mut tabs, quiet, &mut notifier, false, 2.0);
        assert_eq!(*sent.lock().unwrap(), Vec::<String>::new());

        let (mut tabs, id) = tabs_with_agent();
        poll(&mut tabs, quiet, &mut notifier, true, 1.0);
        route(&mut tabs, hook(id, HookKind::PermissionNeeded, Some("Bash: touch hello.txt")), 2.0);
        poll(&mut tabs, quiet, &mut notifier, true, 2.0);
        assert_eq!(
            *sent.lock().unwrap(),
            ["brave-otter needs you (wants permission)\nBash: touch hello.txt"]
        );
    }

    #[test]
    fn terminal_signals_reach_the_status() {
        let (mut tabs, _) = tabs_with_agent();
        let (mut notifier, _) = recording_notifier();
        let working = |_: &u32| TermSnapshot {
            progress: Progress::Indeterminate,
            activity: true,
            ..Default::default()
        };
        let wake = poll(&mut tabs, working, &mut notifier, false, 1.0).wake;
        assert_eq!(tones(&tabs), [Tone::Working]);
        assert!(wake.animate);
    }

    #[test]
    fn a_waiting_or_finished_tab_asks_for_no_animation_and_a_quiet_one_names_its_deadline() {
        let quiet = |_: &u32| TermSnapshot::default();
        let (mut tabs, id) = tabs_with_agent();
        let (mut notifier, _) = recording_notifier();
        let wake = poll(&mut tabs, quiet, &mut notifier, false, 1.0).wake;
        assert!(wake.animate);
        assert_eq!(wake.next_change, Some(tidewatch_core::status::STATUS_QUIET_SECS));

        route(&mut tabs, hook(id, HookKind::TurnDone, Some("done")), 2.0);
        let wake = poll(&mut tabs, quiet, &mut notifier, false, 2.0).wake;
        assert_eq!(wake, Wake { animate: false, next_change: None });
    }

    #[test]
    fn shell_tabs_and_tabs_without_a_pty_are_left_alone() {
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        tabs.push("new worktree".into(), Pane::creating());
        let (mut notifier, _) = recording_notifier();
        let wake = poll(&mut tabs, |_| unreachable!(), &mut notifier, true, 1.0).wake;
        assert_eq!(wake, Wake::default());
        assert_eq!(tones(&tabs), Vec::<Tone>::new());
    }

    mod live {
        use super::*;
        use std::io::{Read, Write};
        use std::net::TcpStream;
        use std::time::{Duration, Instant};
        use tidewatch_core::listener::TabRegistration;
        use tidewatch_core::paths::TidewatchPaths;

        use crate::agent_events::Hooks;

        fn post(hooks: &Hooks, path: &str, token: &str, body: &str) -> String {
            let base = hooks.listener().unwrap().base_url();
            let mut stream = TcpStream::connect(base.trim_start_matches("http://")).unwrap();
            let request = format!(
                "POST {path} HTTP/1.1\r\nHost: tidewatch\r\nAuthorization: Bearer {token}\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(request.as_bytes()).unwrap();
            let mut answer = String::new();
            let _ = stream.read_to_string(&mut answer);
            answer
        }

        fn wait_for_message(hooks: &Hooks) -> TabMessage {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(message) = hooks.drain().into_iter().next() {
                    return message;
                }
                assert!(Instant::now() < deadline, "no message from the listener");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        const CLAUDE_PERMISSION: &str = r#"{"session_id":"df94b279","hook_event_name":"Notification",
            "message":"Claude needs your permission to use Bash","notification_type":"permission_prompt"}"#;

        #[test]
        fn a_claude_permission_notice_posted_to_the_listener_turns_the_tab_amber() {
            let home = tempfile::tempdir().unwrap();
            let paths = TidewatchPaths::new(home.path());
            let hooks = Hooks::start(Some(&paths), &egui::Context::default());
            let (mut tabs, id) = tabs_with_agent();
            let token = hooks
                .listener()
                .expect("the listener binds on loopback")
                .register_tab(TabRegistration {
                    tab_id: id,
                    agent: "claude".into(),
                    role: tidewatch_core::mcp::TabRole::Agent,
                    repo: home.path().join("web"),
                    tab: "brave-otter".into(),
                    generation: 0,
                })
                .token;
            let (mut notifier, _) = recording_notifier();
            poll(&mut tabs, |_: &u32| TermSnapshot::default(), &mut notifier, false, 1.0);
            assert_eq!(tones(&tabs), [Tone::Working]);

            let answer = post(&hooks, "/hook/claude", &token, CLAUDE_PERMISSION);
            assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
            let message = wait_for_message(&hooks);
            assert_eq!(message.tab_id, id);
            route(&mut tabs, message, 2.0);
            poll(&mut tabs, |_: &u32| TermSnapshot::default(), &mut notifier, false, 2.0);
            assert_eq!(tones(&tabs), [Tone::Needs]);
        }

        #[test]
        fn a_closed_tabs_token_is_refused_and_a_wrong_one_never_reaches_a_tab() {
            let home = tempfile::tempdir().unwrap();
            let paths = TidewatchPaths::new(home.path());
            let hooks = Hooks::start(Some(&paths), &egui::Context::default());
            let (_, id) = tabs_with_agent();
            let token = hooks
                .listener()
                .unwrap()
                .register_tab(TabRegistration {
                    tab_id: id,
                    agent: "claude".into(),
                    role: tidewatch_core::mcp::TabRole::Agent,
                    repo: home.path().join("web"),
                    tab: "brave-otter".into(),
                    generation: 0,
                })
                .token;
            assert!(post(&hooks, "/hook/claude", "wrong", CLAUDE_PERMISSION).contains("401"));
            hooks.close_tab(id);
            assert!(post(&hooks, "/hook/claude", &token, CLAUDE_PERMISSION).contains("401"));
            std::thread::sleep(Duration::from_millis(50));
            assert_eq!(hooks.drain(), Vec::new());
        }
    }

    #[test]
    fn links_are_looked_for_only_on_tabs_with_remote_control_and_only_on_new_output() {
        use crate::remote_session::RemoteSession;
        use std::cell::Cell;
        let link = "https://claude.ai/code/session_01AbCd";
        let (mut tabs, id) = tabs_with_agent();
        let plain = tabs.push("plain".into(), {
            let mut pane = Pane::shell(2);
            pane.agent = Some(crate::pane::AgentTab::new(
                "Reviewer".into(),
                AgentRun::setting_up("claude".into(), 0.0),
            ));
            pane
        });
        tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().remote =
            Some(RemoteSession::new("web \u{b7} otter".into()));
        let reads = Cell::new(0);
        let rows = |_: &u32| {
            reads.set(reads.get() + 1);
            Rows { lines: vec![format!("Remote Control active {link}")], cols: 80 }
        };
        let link_of = |tabs: &Tabs<Pane<u32>>, id: u64| {
            tabs.items()
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.pane.agent.as_ref()?.remote.as_ref()?.link().map(str::to_owned))
        };
        assert_eq!(watch_links(&mut tabs, |_| 0, rows, 1.0), None, "no output yet");
        assert_eq!(reads.get(), 0);
        assert_eq!(watch_links(&mut tabs, |_| 7, rows, 1.0), None);
        assert_eq!(reads.get(), 1, "only the tab with remote control is read");
        assert_eq!(link_of(&tabs, id).as_deref(), Some(link));
        assert_eq!(link_of(&tabs, plain), None);
        assert_eq!(watch_links(&mut tabs, |_| 9, rows, 5.0), None);
        assert_eq!(reads.get(), 1, "a tab with its link is not read again");
    }

    #[test]
    fn output_that_comes_too_soon_after_a_look_asks_for_a_wake_up() {
        use crate::remote_session::RemoteSession;
        let (mut tabs, id) = tabs_with_agent();
        tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().remote =
            Some(RemoteSession::new("web".into()));
        let rows = |_: &u32| Rows { lines: vec!["starting".into()], cols: 80 };
        assert_eq!(watch_links(&mut tabs, |_| 1, rows, 1.0), None);
        let later = watch_links(&mut tabs, |_| 2, rows, 1.5);
        assert_eq!(later, Some(3.0));
        assert_eq!(watch_links(&mut tabs, |_| 2, rows, 3.0), None);
    }

    fn meeseeks_tab() -> (Tabs<Pane<u32>>, u64) {
        let (mut tabs, id) = tabs_with_agent();
        tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().meeseeks =
            Some(tidewatch_core::meeseeks::Meeseeks::summon(
                "x",
                tidewatch_core::meeseeks::Criteria::default(),
                0,
            ));
        (tabs, id)
    }

    fn turns(tabs: &Tabs<Pane<u32>>, id: u64) -> u32 {
        tabs.items()
            .iter()
            .find(|tab| tab.id == id)
            .and_then(|tab| tab.pane.agent.as_ref()?.meeseeks.as_ref())
            .map_or(0, |meeseeks| meeseeks.turns)
    }

    #[test]
    fn every_stop_hook_is_one_turn_of_a_meeseeks_and_nothing_else_is() {
        let (mut tabs, id) = meeseeks_tab();
        for kind in [HookKind::TurnStarted, HookKind::PermissionNeeded] {
            route(&mut tabs, hook(id, kind, None), 1.0);
        }
        assert_eq!(turns(&tabs, id), 0);
        for _ in 0..3 {
            route(&mut tabs, hook(id, HookKind::TurnStarted, None), 2.0);
            route(&mut tabs, hook(id, HookKind::TurnDone, Some("done")), 3.0);
        }
        assert_eq!(turns(&tabs, id), 3);
        let (mut plain, plain_id) = tabs_with_agent();
        route(&mut plain, hook(plain_id, HookKind::TurnDone, None), 3.0);
        assert_eq!(turns(&plain, plain_id), 0);
    }

    fn said(tab: u64) -> TabMessage {
        TabMessage {
            tab_id: tab,
            generation: 0,
            event: TabEvent::MeeseeksDone { summary: "renamed it".into() },
        }
    }

    #[test]
    fn a_chosen_name_is_kept_on_a_meeseeks_and_ignored_by_any_other_tab() {
        let named = |tab| TabMessage {
            tab_id: tab,
            generation: 0,
            event: TabEvent::MeeseeksName { name: "fix-https-validation".into() },
        };
        let (mut tabs, id) = meeseeks_tab();
        assert_eq!(route(&mut tabs, named(id), 1.0), None);
        let chosen = |tabs: &Tabs<Pane<u32>>, id| {
            tabs.items()
                .iter()
                .find(|tab| tab.id == id)
                .and_then(|tab| tab.pane.agent.as_ref()?.meeseeks.as_ref()?.name.clone())
        };
        assert_eq!(chosen(&tabs, id).as_deref(), Some("fix-https-validation"));
        let (mut plain, plain_id) = tabs_with_agent();
        assert_eq!(route(&mut plain, named(plain_id), 1.0), None);
        assert_eq!(chosen(&plain, plain_id), None);
    }

    #[test]
    fn meeseeks_done_reaches_the_app_only_from_a_meeseeks_tab() {
        let (mut tabs, id) = meeseeks_tab();
        assert_eq!(
            route(&mut tabs, said(id), 1.0),
            Some(Follow::MeeseeksDone { tab: id, summary: "renamed it".into() })
        );
        let (mut plain, plain_id) = tabs_with_agent();
        assert_eq!(route(&mut plain, said(plain_id), 1.0), None, "an ordinary agent is ignored");
        assert_eq!(route(&mut tabs, said(99), 1.0), None, "and so is a closed tab");
    }
}
