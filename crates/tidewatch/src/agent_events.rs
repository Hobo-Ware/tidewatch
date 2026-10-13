//! The app's end of the Tidewatch listener: starting it once, draining what agents report, and
//! turning a `report_finding` call into a note on that file and line (PLAN.md, D6).

use std::path::{Component, Path};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use kelp_core::review::{Anchor, Comment, Side};
use tidewatch_core::events::TabMessage;
use tidewatch_core::listener::Listener;
use tidewatch_core::notes::Notes;
use tidewatch_core::paths::TidewatchPaths;

use crate::agent_run::Finding;

/// The listener and the channel it reports on. Both are absent when the listener could not start
/// (or there is no home folder): status then comes from the terminal and process layers alone.
pub(crate) struct Hooks {
    listener: Option<Arc<Listener>>,
    inbox: Option<Receiver<TabMessage>>,
}

impl Hooks {
    pub(crate) fn off() -> Self {
        Self { listener: None, inbox: None }
    }

    /// Starts the listener, waking the UI on every message. A bind failure is logged once.
    pub(crate) fn start(paths: Option<&TidewatchPaths>, ctx: &egui::Context) -> Self {
        let Some(paths) = paths else { return Self::off() };
        let wake = ctx.clone();
        match Listener::start(paths.clone(), Box::new(move || wake.request_repaint())) {
            Ok((listener, inbox)) => {
                Self { listener: Some(Arc::new(listener)), inbox: Some(inbox) }
            }
            Err(e) => {
                eprintln!(
                    "tidewatch: the hook listener could not start ({e}); agent status will use \
                     terminal and process signals only"
                );
                Self::off()
            }
        }
    }

    pub(crate) fn listener(&self) -> Option<Arc<Listener>> {
        self.listener.clone()
    }

    /// Everything agents reported since the last call, oldest first.
    pub(crate) fn drain(&self) -> Vec<TabMessage> {
        self.inbox.as_ref().map(|inbox| inbox.try_iter().collect()).unwrap_or_default()
    }

    /// Revokes the tab's token when the tab closes.
    pub(crate) fn close_tab(&self, tab: u64) {
        if let Some(listener) = &self.listener {
            listener.unregister_tab(tab);
        }
    }
}

/// True for a path that stays inside the worktree: relative, with no `..`.
pub(crate) fn stays_inside(file: &str) -> bool {
    let path = Path::new(file);
    path.is_relative()
        && path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// The anchor for a finding. With the file's text the thread follows its line when the file moves
/// on; without it the line number alone places it.
pub(crate) fn finding_anchor(file_text: Option<&str>, line: u32) -> Anchor {
    let index = usize::try_from(line).unwrap_or(0).saturating_sub(1);
    if let Some(text) = file_text {
        let lines: Vec<&str> = text.lines().collect();
        if index < lines.len() {
            return Anchor::at(&lines, index, Side::New);
        }
    }
    Anchor { side: Side::New, line_hint: line, text: String::new(), before: vec![], after: vec![] }
}

pub(crate) fn finding_body(finding: &Finding) -> String {
    if finding.blocking { format!("Blocking: {}", finding.text) } else { finding.text.clone() }
}

/// Adds the finding as a new thread on the working tree, signed by the agent.
pub(crate) fn add_finding(
    notes: &mut Notes,
    finding: &Finding,
    author: &str,
    file_text: Option<&str>,
    now: i64,
) -> u64 {
    let comment = Comment { author: author.to_owned(), time: now, body: finding_body(finding) };
    notes.add(&finding.file, None, finding_anchor(file_text, finding.line), comment)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(blocking: bool) -> Finding {
        Finding { file: "src/a.rs".into(), line: 2, blocking, text: "unwrap can panic".into() }
    }

    #[test]
    fn a_finding_becomes_a_thread_on_its_file_and_line_signed_by_the_agent() {
        let mut notes = Notes::default();
        let text = "fn a() {\n    x.unwrap();\n}\n";
        let id = add_finding(&mut notes, &finding(false), "claude", Some(text), 7);
        let thread = &notes.threads[0];
        assert_eq!(
            (thread.id, thread.path.as_str(), thread.commit.as_deref()),
            (id, "src/a.rs", None)
        );
        assert_eq!(thread.anchor.line_hint, 2);
        assert_eq!(thread.anchor.text, "    x.unwrap();");
        assert_eq!(thread.comments[0].author, "claude");
        assert_eq!(thread.comments[0].body, "unwrap can panic");
        assert_eq!(thread.comments[0].time, 7);
        assert!(!thread.resolved);
    }

    #[test]
    fn a_blocking_finding_says_so() {
        let mut notes = Notes::default();
        add_finding(&mut notes, &finding(true), "claude", None, 1);
        assert_eq!(notes.threads[0].comments[0].body, "Blocking: unwrap can panic");
    }

    #[test]
    fn a_missing_file_or_line_still_places_the_note_by_number() {
        let bare = finding_anchor(None, 9);
        assert_eq!((bare.line_hint, bare.text.as_str()), (9, ""));
        let past_the_end = finding_anchor(Some("one line\n"), 50);
        assert_eq!((past_the_end.line_hint, past_the_end.text.as_str()), (50, ""));
        let zero = finding_anchor(Some("one line\n"), 0);
        assert_eq!(zero.text, "one line");
    }

    #[test]
    fn two_findings_are_two_threads() {
        let mut notes = Notes::default();
        let a = add_finding(&mut notes, &finding(false), "claude", None, 1);
        let b = add_finding(&mut notes, &finding(true), "claude", None, 2);
        assert_ne!(a, b);
        assert_eq!(notes.threads.len(), 2);
    }

    #[test]
    fn only_paths_inside_the_worktree_are_read() {
        let cases = [
            ("src/a.rs", true),
            ("./src/a.rs", true),
            ("../secret", false),
            ("src/../../secret", false),
            ("/etc/passwd", false),
        ];
        for (file, want) in cases {
            assert_eq!(stays_inside(file), want, "{file}");
        }
    }

    #[test]
    fn without_a_listener_nothing_arrives_and_closing_is_harmless() {
        let hooks = Hooks::off();
        assert!(hooks.listener().is_none());
        assert_eq!(hooks.drain(), Vec::new());
        hooks.close_tab(3);
    }
}
