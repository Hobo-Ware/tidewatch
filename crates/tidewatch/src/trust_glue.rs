//! D14 folder trust in the app: which CLIs are installed and where each stands, the consent state,
//! and the jobs that edit the CLIs' files after a yes. The first-launch dialog and the settings
//! page read this and send `Event`s back; nothing here draws.
//!
//! `TIDEWATCH_DEV_TRUST=yes|no|undo` answers once as soon as the CLIs have been looked at, for
//! runnable checks. Without it a fresh `TIDEWATCH_HOME` shows the dialog like a real first launch.

use std::ffi::OsString;
use std::path::Path;

use tidewatch_core::agents::find_on_path;
use tidewatch_core::consent::{Consent, Effect, Event};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::trust::{TrustCli, TrustState};
use tidewatch_core::trust_apply::{ConfigFiles, Grant, Outcome};

use crate::jobs::Jobs;
use crate::trust_view::Page;

pub(crate) enum Done {
    Inspected { installed: Vec<TrustCli>, state: TrustState },
    Applied { consent: Consent, said: Vec<String>, state: TrustState, added: usize },
}

pub(crate) struct Trust {
    grant: Option<Grant>,
    root: String,
    consent: Consent,
    added: usize,
    installed: Vec<TrustCli>,
    state: Option<TrustState>,
    jobs: Jobs<Done>,
    busy: bool,
    dev: Option<Event>,
}

impl Trust {
    /// `cli_home` is the real home directory, where the CLIs keep their files (`TIDEWATCH_HOME`
    /// moves only Tidewatch's own).
    pub(crate) fn new(paths: Option<&TidewatchPaths>, cli_home: Option<&Path>) -> Self {
        let grant =
            paths.zip(cli_home).map(|(paths, home)| Grant::new(paths, ConfigFiles::from_env(home)));
        Self::with_grant(grant, dev_event(std::env::var("TIDEWATCH_DEV_TRUST").ok().as_deref()))
    }

    fn with_grant(grant: Option<Grant>, dev: Option<Event>) -> Self {
        let record = grant.as_ref().map(Grant::record).unwrap_or_default();
        let root = grant.as_ref().map_or_else(|| "~/.tidewatch".to_owned(), Grant::root_entry);
        Self {
            root,
            consent: record.consent,
            added: record.added.len(),
            grant,
            installed: Vec::new(),
            state: None,
            jobs: Jobs::new(),
            busy: false,
            dev,
        }
    }

    /// Looks at which of the agent CLIs are on `path` and where their files stand.
    pub(crate) fn refresh(
        &self,
        ctx: &egui::Context,
        bins: Vec<(TrustCli, String)>,
        path: OsString,
    ) {
        let Some(grant) = self.grant.clone() else { return };
        self.jobs.spawn(ctx, move || {
            let mut installed: Vec<TrustCli> = bins
                .into_iter()
                .filter(|(_, bin)| find_on_path(bin, Some(&path)).is_some())
                .map(|(cli, _)| cli)
                .collect();
            installed.dedup();
            let state = grant.inspect(&installed);
            Done::Inspected { installed, state }
        });
    }

    /// Applies finished jobs. The returned text, if any, is for a toast.
    pub(crate) fn drain(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut toast = None;
        for done in self.jobs.drain() {
            match done {
                Done::Inspected { installed, state } => {
                    self.installed = installed;
                    self.state = Some(state);
                    if let Some(event) = self.dev.take() {
                        self.send(event, ctx);
                    }
                }
                Done::Applied { consent, said, state, added } => {
                    self.busy = false;
                    self.consent = consent;
                    self.state = Some(state);
                    self.added = added;
                    if !said.is_empty() {
                        toast = Some(said.join("; "));
                    }
                }
            }
        }
        toast
    }

    /// The user's answer: moves the consent state and runs what it asks for off the UI thread.
    pub(crate) fn send(&mut self, event: Event, ctx: &egui::Context) {
        let Some(grant) = self.grant.clone() else { return };
        if self.busy {
            return;
        }
        let (next, effects) = self.consent.next(event);
        if effects.is_empty() {
            return;
        }
        self.consent = next;
        self.busy = true;
        let installed = self.installed.clone();
        self.jobs.spawn(ctx, move || {
            let mut outcomes = Vec::new();
            let mut said = Vec::new();
            for effect in effects {
                match effect {
                    Effect::TrustRoot => outcomes.extend(grant.trust_root(&installed)),
                    Effect::RevokeAll => outcomes.extend(grant.revoke(None)),
                    Effect::Save => {
                        if let Err(e) = grant.set_consent(next) {
                            said.push(format!("Could not save the answer: {e}"));
                        }
                    }
                }
            }
            said.extend(summarize(&outcomes, &grant.root_entry()));
            Done::Applied {
                consent: next,
                said,
                state: grant.inspect(&installed),
                added: grant.record().added.len(),
            }
        });
    }

    /// The grant to hand to a tab being opened or a worktree being removed: only after a yes.
    pub(crate) fn grant(&self) -> Option<Grant> {
        self.grant.clone().filter(|_| self.consent == Consent::Granted)
    }

    pub(crate) fn dialog_open(&self) -> bool {
        !self.busy && self.state.as_ref().is_some_and(|s| self.consent.should_ask(s))
    }

    pub(crate) fn page(&self) -> Page<'_> {
        Page {
            consent: self.consent,
            state: self.state.as_ref(),
            root: &self.root,
            added: self.added,
            busy: self.busy,
        }
    }
}

fn dev_event(value: Option<&str>) -> Option<Event> {
    match value? {
        "yes" => Some(Event::Apply),
        "no" => Some(Event::No),
        "undo" => Some(Event::Undo),
        other => {
            eprintln!("tidewatch: TIDEWATCH_DEV_TRUST={other} is not yes, no or undo");
            None
        }
    }
}

/// One line per thing worth saying: what was trusted, removed or refused.
fn summarize(outcomes: &[(TrustCli, Outcome)], root: &str) -> Vec<String> {
    outcomes
        .iter()
        .filter_map(|(cli, outcome)| match outcome {
            Outcome::Added => Some(format!("{}: trusted {root}", cli.name())),
            Outcome::Removed => Some(format!("{}: removed Tidewatch's entry", cli.name())),
            Outcome::Failed(why) => Some(format!("{}: {why}", cli.name())),
            Outcome::Unchanged | Outcome::NotApplicable => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};
    use tidewatch_core::trust::Status;

    struct Rig {
        _home: tempfile::TempDir,
        bin: tempfile::TempDir,
        paths: TidewatchPaths,
        home: std::path::PathBuf,
    }

    fn rig() -> Rig {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().canonicalize().unwrap();
        let bin = tempfile::tempdir().unwrap();
        for name in ["copilot", "claude"] {
            let file = bin.path().join(name);
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Rig { paths: TidewatchPaths::new(&path), home: path, _home: home, bin }
    }

    fn trust_for(rig: &Rig, dev: Option<Event>) -> Trust {
        let files = ConfigFiles::resolve(&rig.home, |_| None);
        Trust::with_grant(Some(Grant::new(&rig.paths, files)), dev)
    }

    fn bins() -> Vec<(TrustCli, String)> {
        vec![
            (TrustCli::Claude, "claude".into()),
            (TrustCli::Copilot, "copilot".into()),
            (TrustCli::Gemini, "gemini-not-here".into()),
        ]
    }

    fn settle(trust: &mut Trust, ctx: &egui::Context) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut toast = None;
        loop {
            toast = trust.drain(ctx).or(toast);
            if !trust.busy && trust.state.is_some() {
                return toast;
            }
            assert!(Instant::now() < deadline, "the trust jobs never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_first_launch_finds_the_installed_clis_and_asks_once() {
        let rig = rig();
        let ctx = egui::Context::default();
        let mut trust = trust_for(&rig, None);
        assert!(!trust.dialog_open(), "nothing is known yet");
        trust.refresh(&ctx, bins(), rig.bin.path().into());
        settle(&mut trust, &ctx);
        let state = trust.state.clone().unwrap();
        let statuses: Vec<_> = state.clis.iter().map(|c| (c.cli, c.status.clone())).collect();
        assert_eq!(
            statuses,
            [(TrustCli::Claude, Status::PerWorktree), (TrustCli::Copilot, Status::Needs)]
        );
        assert!(trust.dialog_open());
        assert!(trust.grant().is_none(), "no edits before a yes");
    }

    #[test]
    fn yes_trusts_the_root_saves_the_answer_and_hands_out_a_grant() {
        let rig = rig();
        let ctx = egui::Context::default();
        let mut trust = trust_for(&rig, None);
        trust.refresh(&ctx, bins(), rig.bin.path().into());
        settle(&mut trust, &ctx);
        trust.send(Event::Yes, &ctx);
        let toast = settle(&mut trust, &ctx).unwrap();
        assert!(toast.contains("GitHub Copilot: trusted "), "{toast}");
        let copilot = rig.home.join(".copilot").join("config.json");
        assert!(std::fs::read_to_string(copilot).unwrap().contains("trustedFolders"));
        assert!(!rig.home.join(".claude.json").exists(), "Claude waits for a worktree");
        assert!(trust.grant().is_some());
        assert!(!trust.dialog_open());
        assert_eq!(trust.added, 1);
        assert!(rig.paths.trust_file().exists());
        assert_eq!(
            Trust::with_grant(trust.grant(), None).consent,
            Consent::Granted,
            "it survives a restart"
        );
    }

    #[test]
    fn no_touches_no_file_and_is_not_asked_again() {
        let rig = rig();
        let ctx = egui::Context::default();
        let mut trust = trust_for(&rig, None);
        trust.refresh(&ctx, bins(), rig.bin.path().into());
        settle(&mut trust, &ctx);
        trust.send(Event::No, &ctx);
        settle(&mut trust, &ctx);
        assert!(!rig.home.join(".copilot").exists());
        assert!(trust.grant().is_none());
        assert!(!trust.dialog_open());
        let again = trust_for(&rig, None);
        assert_eq!(again.consent, Consent::Declined);
    }

    #[test]
    fn settings_can_apply_after_a_no_and_undo_after_a_yes() {
        let rig = rig();
        let ctx = egui::Context::default();
        let mut trust = trust_for(&rig, None);
        trust.refresh(&ctx, bins(), rig.bin.path().into());
        settle(&mut trust, &ctx);
        trust.send(Event::No, &ctx);
        settle(&mut trust, &ctx);
        trust.send(Event::Apply, &ctx);
        settle(&mut trust, &ctx);
        assert_eq!(trust.consent, Consent::Granted);
        let copilot = rig.home.join(".copilot").join("config.json");
        assert!(copilot.exists());
        trust.send(Event::Undo, &ctx);
        let toast = settle(&mut trust, &ctx).unwrap();
        assert!(toast.contains("removed Tidewatch's entry"), "{toast}");
        assert_eq!(trust.consent, Consent::Declined);
        assert_eq!(trust.added, 0);
        assert!(trust.grant().is_none());
    }

    #[test]
    fn a_dev_answer_is_given_once_when_the_clis_are_known() {
        let rig = rig();
        let ctx = egui::Context::default();
        let mut trust = trust_for(&rig, Some(Event::Apply));
        trust.refresh(&ctx, bins(), rig.bin.path().into());
        settle(&mut trust, &ctx);
        assert_eq!(trust.consent, Consent::Granted);
        assert!(trust.dev.is_none());
    }

    #[test]
    fn without_a_home_there_is_nothing_to_ask_or_edit() {
        let mut trust = Trust::with_grant(None, None);
        trust.send(Event::Yes, &egui::Context::default());
        assert!(!trust.dialog_open() && trust.grant().is_none());
        assert_eq!(trust.page().root, "~/.tidewatch");
    }

    #[test]
    fn dev_values_and_toast_lines() {
        assert_eq!(dev_event(Some("yes")), Some(Event::Apply));
        assert_eq!(dev_event(Some("no")), Some(Event::No));
        assert_eq!(dev_event(Some("undo")), Some(Event::Undo));
        assert_eq!(dev_event(Some("maybe")), None);
        assert_eq!(dev_event(None), None);
        let said = summarize(
            &[
                (TrustCli::Copilot, Outcome::Added),
                (TrustCli::Claude, Outcome::Unchanged),
                (TrustCli::Gemini, Outcome::NotApplicable),
                (TrustCli::Codex, Outcome::Failed("cannot read the file (x), left alone".into())),
            ],
            "/r",
        );
        assert_eq!(
            said,
            ["GitHub Copilot: trusted /r", "Codex: cannot read the file (x), left alone"]
        );
    }
}
