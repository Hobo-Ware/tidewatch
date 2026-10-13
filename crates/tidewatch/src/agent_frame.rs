//! What happens to the agent tabs each frame, apart from drawing: messages from the listener go to
//! the tab they belong to, terminal signals feed each tab's status, a changed status may notify, and
//! the result says when the UI next needs a frame. Generic over the pty so it runs on plain data in
//! tests.

use std::path::PathBuf;

use tidewatch_core::events::{TabEvent, TabMessage};

use crate::agent_run::{Change, Finding, TermSnapshot};
use crate::notify::Notifier;
use crate::pane::{Body, Pane};
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    ReloadNotes { tab: u64 },
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
        TabEvent::Hook(_) | TabEvent::StageDone { .. } => None,
    };
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
            agent.run.feed(snapshot(term), now);
        }
        if let Some(change) = agent.run.refresh(now) {
            notifier.offer(tab.id, &tab.title, &change, agent.run.detail(), away, now);
            changes.push((tab.id, change));
        }
        if status_view::is_animated(status_view::tone(agent.run.status())) {
            wake.animate = true;
        }
        if let Some(at) = agent.run.next_change_at(now) {
            wake.next_change = Some(wake.next_change.map_or(at, |soonest| soonest.min(at)));
        }
    }
    Polled { wake, changes }
}

/// Tones of every agent tab, in tab order (for the Overview button's badge and tests).
pub(crate) fn tones<T>(tabs: &Tabs<Pane<T>>) -> Vec<Tone> {
    tabs.items()
        .iter()
        .filter_map(|tab| tab.pane.agent.as_ref())
        .map(|agent| status_view::tone(agent.run.status()))
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
            agent: Some(AgentTab { profile_name: "Builder".into(), run }),
        };
        let mut tabs = Tabs::new(Pane::shell(0));
        let id = tabs.push("brave-otter".into(), pane);
        (tabs, id)
    }

    fn hook(tab: u64, kind: HookKind, message: Option<&str>) -> TabMessage {
        TabMessage {
            tab_id: tab,
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
    fn a_finding_asks_for_a_note_in_the_tabs_notes_file() {
        let (mut tabs, id) = tabs_with_agent();
        let message = TabMessage {
            tab_id: id,
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
    fn a_note_reply_asks_for_the_notes_to_be_read_again_and_stage_done_is_only_kept() {
        let (mut tabs, id) = tabs_with_agent();
        let reply =
            TabMessage { tab_id: id, event: TabEvent::NoteReply { note_id: 1, text: "ok".into() } };
        assert_eq!(route(&mut tabs, reply, 1.0), Some(Follow::ReloadNotes { tab: id }));
        let done =
            TabMessage { tab_id: id, event: TabEvent::StageDone { summary: "tests added".into() } };
        assert_eq!(route(&mut tabs, done, 2.0), None);
        let agent = tabs.get_mut(id).unwrap().pane.agent.take().unwrap();
        assert_eq!(agent.run.report.stage_done.as_deref(), Some("tests added"));
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
                    repo: home.path().join("web"),
                    tab: "brave-otter".into(),
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
                    repo: home.path().join("web"),
                    tab: "brave-otter".into(),
                })
                .token;
            assert!(post(&hooks, "/hook/claude", "wrong", CLAUDE_PERMISSION).contains("401"));
            hooks.close_tab(id);
            assert!(post(&hooks, "/hook/claude", &token, CLAUDE_PERMISSION).contains("401"));
            std::thread::sleep(Duration::from_millis(50));
            assert_eq!(hooks.drain(), Vec::new());
        }
    }
}
