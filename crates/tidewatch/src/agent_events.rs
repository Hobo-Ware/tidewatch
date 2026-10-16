//! The app's end of the Tidewatch listener: starting it once, draining what agents report, and
//! turning a `report_finding` call into a note on that file and line (PLAN.md, D6).

use std::path::{Component, Path};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use kelp_core::review::{Anchor, Comment, Side};
use tidewatch_core::events::TabMessage;
use tidewatch_core::listener::{Credential, Listener};
use tidewatch_core::notes::Notes;
use tidewatch_core::paths::TidewatchPaths;

use crate::agent_run::Finding;

pub(crate) const PORT_PATIENCE: Duration = Duration::from_secs(5);

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

    /// Starts the listener on `port`, the one the agents a replaced window left running report to.
    /// The old window may still be leaving, so the port is waited for; when it stays busy the
    /// listener starts on any port, and `false` says the old agents' tokens cannot be honoured.
    pub(crate) fn start_on_port(
        paths: Option<&TidewatchPaths>,
        ctx: &egui::Context,
        port: u16,
        patience: Duration,
    ) -> (Self, bool) {
        let Some(paths) = paths else { return (Self::off(), false) };
        let wake = ctx.clone();
        let repaint = Box::new(move || wake.request_repaint());
        match Listener::start_on_port(port, paths.clone(), repaint, patience) {
            Ok((listener, inbox)) => {
                (Self { listener: Some(Arc::new(listener)), inbox: Some(inbox) }, true)
            }
            Err(e) => {
                eprintln!("tidewatch: the hook port {port} stayed busy ({e}); starting on another");
                (Self::start(Some(paths), ctx), false)
            }
        }
    }

    pub(crate) fn port(&self) -> Option<u16> {
        self.listener.as_ref().map(|listener| listener.port())
    }

    /// What the tab's agent reports with.
    pub(crate) fn credential_of(&self, tab: u64) -> Option<Credential> {
        self.listener.as_ref()?.credential_of(tab)
    }

    /// Honours a credential of the window this one replaces, for the tab its agent now sits in.
    pub(crate) fn adopt(&self, tab: u64, credential: &Credential) {
        if let Some(listener) = &self.listener {
            listener.adopt_tab(tab, credential);
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
    Anchor {
        side: Side::New,
        line_hint: line,
        text: String::new(),
        before: vec![],
        after: vec![],
        span: 0,
        block: vec![],
    }
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

    fn paths() -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        (home, paths)
    }

    #[test]
    fn a_new_window_serves_the_old_port_and_honours_the_old_agents_credential() {
        let (_home, paths) = paths();
        let ctx = egui::Context::default();
        let old = Hooks::start(Some(&paths), &ctx);
        let port = old.port().unwrap();
        let registered =
            old.listener().unwrap().register_tab(tidewatch_core::listener::TabRegistration {
                tab_id: 4,
                agent: "claude".into(),
                repo: "/work/web".into(),
                tab: "otter".into(),
                generation: 2,
            });
        let credential = old.credential_of(4).unwrap();
        assert_eq!(credential.token, registered.token);
        drop(old);

        let (new, kept) = Hooks::start_on_port(Some(&paths), &ctx, port, Duration::from_secs(5));
        assert!(kept);
        assert_eq!(new.port(), Some(port));
        new.adopt(9, &credential);
        assert_eq!(new.credential_of(9), Some(credential));
        assert_eq!(new.credential_of(4), None, "the old tab id means nothing here");
    }

    #[test]
    fn a_port_that_stays_busy_starts_elsewhere_and_says_the_tokens_cannot_be_kept() {
        let (_home, paths) = paths();
        let ctx = egui::Context::default();
        let holder = Hooks::start(Some(&paths), &ctx);
        let port = holder.port().unwrap();
        let (other, kept) = Hooks::start_on_port(Some(&paths), &ctx, port, Duration::ZERO);
        assert!(!kept);
        assert_ne!(other.port(), Some(port));
        assert!(other.port().is_some(), "agents still report to a listener of some port");
    }

    #[test]
    fn without_a_home_no_port_is_kept_and_nothing_listens() {
        let (hooks, kept) =
            Hooks::start_on_port(None, &egui::Context::default(), 5000, Duration::ZERO);
        assert!(!kept);
        assert_eq!(hooks.port(), None);
    }
}
