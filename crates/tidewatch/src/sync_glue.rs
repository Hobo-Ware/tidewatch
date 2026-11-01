//! `Host`: settings sync for the running app. It owns this machine's sync choice, starts a sync
//! on a worker thread when `Autosync` says so or the user asks, and turns each result into the
//! note the Sync card shows. A screenshot run is never active: it reads the choice and does nothing.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use stdusk_core::sync::{Op, Outcome};
use stdusk_egui::sync::SyncSlot;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::settings_sync::{
    Autosync, POLL_S, SyncPrefs, create_private_repo, fingerprint, github_slug, is_missing_repo,
    spec,
};
use tidewatch_core::tools::Tools;

use crate::jobs::Jobs;
use crate::sync_view::{Change, Note, View};

struct Watch {
    on: Arc<AtomicBool>,
    latest: Arc<Mutex<String>>,
    thread: Option<std::thread::Thread>,
}

impl Watch {
    fn idle(dir: &std::path::Path) -> Self {
        Self {
            on: Arc::new(AtomicBool::new(false)),
            latest: Arc::new(Mutex::new(fingerprint(dir))),
            thread: None,
        }
    }

    fn start(dir: PathBuf, ctx: egui::Context) -> Self {
        let watch = Self::idle(&dir);
        let (on, latest) = (watch.on.clone(), watch.latest.clone());
        let handle = std::thread::spawn(move || {
            loop {
                if !on.load(Ordering::Relaxed) {
                    std::thread::park();
                    continue;
                }
                std::thread::sleep(Duration::from_secs_f64(POLL_S));
                let now = fingerprint(&dir);
                let changed = {
                    let mut latest = latest.lock().unwrap();
                    let changed = *latest != now;
                    *latest = now;
                    changed
                };
                if changed {
                    ctx.request_repaint_after(Duration::from_millis(50));
                }
            }
        });
        Self { thread: Some(handle.thread().clone()), ..watch }
    }

    fn watch(&self, wanted: bool) {
        if self.on.swap(wanted, Ordering::Relaxed) != wanted
            && wanted
            && let Some(thread) = &self.thread
        {
            thread.unpark();
        }
    }

    fn snapshot(&self) -> String {
        self.latest.lock().unwrap().clone()
    }
}

pub(crate) struct Host {
    active: bool,
    watch: Watch,
    writer: Option<Sender<SyncPrefs>>,
    paths: Option<TidewatchPaths>,
    tools: Tools,
    slot: SyncSlot,
    created: Jobs<Result<(), String>>,
    prefs: SyncPrefs,
    auto: Autosync,
    busy: bool,
    started_with: String,
    reload: bool,
    note: Note,
}

pub(crate) fn wall_secs() -> i64 {
    i64::try_from(tidewatch_core::clock::now_secs()).unwrap_or(0)
}

fn first_line(error: &str) -> String {
    let line =
        error.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("unknown error");
    line.chars().take(140).collect()
}

fn note_for(result: &Result<Outcome, String>, applied: bool, repo: &str) -> Note {
    match result {
        Ok(Outcome::KeptLocal) => Note::KeptLocal,
        Ok(Outcome::Synced) if applied => Note::Updated { repo: repo.to_owned() },
        Ok(Outcome::Synced) => Note::None,
        Err(error) if is_missing_repo(error) => Note::MissingRepo { slug: github_slug(repo) },
        Err(error) => Note::Failed(first_line(error)),
    }
}

fn start_writer(paths: TidewatchPaths) -> Sender<SyncPrefs> {
    let (sender, receiver) = channel::<SyncPrefs>();
    std::thread::spawn(move || {
        while let Ok(mut prefs) = receiver.recv() {
            while let Ok(newer) = receiver.try_recv() {
                prefs = newer;
            }
            if let Err(e) = prefs.save(&paths) {
                eprintln!("tidewatch: cannot save sync.toml: {e}");
            }
        }
    });
    sender
}

impl Host {
    pub(crate) fn new(
        paths: Option<&TidewatchPaths>,
        tools: Tools,
        active: bool,
        ctx: &egui::Context,
    ) -> Self {
        let active = active && paths.is_some();
        let watch = match paths {
            Some(paths) if active => Watch::start(paths.config_dir().to_path_buf(), ctx.clone()),
            Some(paths) => Watch::idle(paths.config_dir()),
            None => Watch::idle(std::path::Path::new("")),
        };
        Self {
            active,
            watch,
            writer: paths.filter(|_| active).cloned().map(start_writer),
            prefs: paths.map(SyncPrefs::load).unwrap_or_default(),
            paths: paths.cloned(),
            tools,
            slot: SyncSlot::default(),
            created: Jobs::new(),
            auto: Autosync::default(),
            busy: false,
            started_with: String::new(),
            reload: false,
            note: Note::None,
        }
    }

    pub(crate) fn repo(&self) -> &str {
        &self.prefs.repo
    }

    pub(crate) fn view(&self, now: i64) -> View {
        View {
            enabled: self.prefs.enabled,
            has_repo: self.prefs.remote().is_some(),
            busy: self.busy,
            last_synced: self.prefs.last_synced,
            now,
            note: self.note.clone(),
        }
    }

    /// True once after a sync replaced synced files on disk: the app reads them again.
    pub(crate) fn drive(&mut self, ctx: &egui::Context, clock: f64) -> bool {
        if !self.active {
            return false;
        }
        let Some(paths) = self.paths.clone() else { return false };
        let finished = self.slot.lock().unwrap().take();
        if let Some((_, result)) = finished {
            self.finish(&result, clock, &paths);
        }
        for created in self.created.drain() {
            self.created_repo(created, ctx, clock, &paths);
        }
        let ready = self.prefs.auto();
        self.watch.watch(ready);
        let watch = &self.watch;
        if let Some(op) = self.auto.tick(clock, ready, self.busy, || watch.snapshot()) {
            self.start(op, ctx, &paths);
        }
        if ready && !self.busy {
            let wake = self.auto.wake_in(clock, &self.watch.latest.lock().unwrap());
            if let Some(seconds) = wake {
                ctx.request_repaint_after(Duration::from_secs_f64(seconds));
            }
        }
        std::mem::take(&mut self.reload)
    }

    pub(crate) fn apply(&mut self, change: Change, ctx: &egui::Context) {
        let Some(paths) = self.paths.clone().filter(|_| self.active) else { return };
        match change {
            Change::Enabled(on) => {
                self.prefs.enabled = on;
                self.note = Note::None;
                self.save();
            }
            Change::Repo(text) => {
                if text == self.prefs.repo {
                    return;
                }
                self.prefs.repo = text;
                self.note = Note::None;
                self.auto = Autosync::default();
                self.save();
            }
            Change::SyncNow => self.sync_now(ctx, &paths),
            Change::CreateRepo => self.create(ctx, &paths),
        }
    }

    fn sync_now(&mut self, ctx: &egui::Context, paths: &TidewatchPaths) {
        if self.busy || self.prefs.remote().is_none() {
            return;
        }
        self.auto.begin_manual(fingerprint(paths.config_dir()));
        self.start(Op::Reconcile, ctx, paths);
    }

    fn create(&mut self, ctx: &egui::Context, paths: &TidewatchPaths) {
        let Some(slug) = github_slug(&self.prefs.repo).filter(|_| !self.busy) else { return };
        self.busy = true;
        let (tools, paths) = (self.tools.clone(), paths.clone());
        self.created.spawn(ctx, move || create_private_repo(&tools, &paths, &slug));
    }

    fn created_repo(
        &mut self,
        result: Result<(), String>,
        ctx: &egui::Context,
        clock: f64,
        paths: &TidewatchPaths,
    ) {
        self.busy = false;
        match result {
            Ok(()) => {
                self.note = Note::None;
                self.sync_now(ctx, paths);
            }
            Err(error) => {
                self.auto.finished(clock, false);
                self.note = Note::Failed(first_line(&error));
            }
        }
    }

    fn start(&mut self, op: Op, ctx: &egui::Context, paths: &TidewatchPaths) {
        let Some(remote) = self.prefs.remote() else { return };
        self.busy = true;
        self.started_with = fingerprint(paths.config_dir());
        let spec = spec(paths, &self.tools);
        stdusk_egui::sync::spawn(Some(spec), op, remote, &self.slot, ctx.clone());
    }

    fn finish(&mut self, result: &Result<Outcome, String>, clock: f64, paths: &TidewatchPaths) {
        self.busy = false;
        self.auto.finished(clock, result.is_ok());
        let applied = result.is_ok() && fingerprint(paths.config_dir()) != self.started_with;
        self.reload = applied;
        self.note = note_for(result, applied, &self.prefs.repo);
        if result.is_ok() {
            self.prefs.last_synced = Some(wall_secs());
            self.save();
        }
    }

    fn save(&self) {
        if let Some(writer) = &self.writer {
            let _ = writer.send(self.prefs.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Instant;

    fn wait_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn git_env() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
            ("GIT_COMMITTER_NAME", "Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }

    fn bare_remote(dir: &Path) -> String {
        let path = dir.join("settings.git");
        let status = std::process::Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(&path)
            .envs(git_env())
            .status()
            .unwrap();
        assert!(status.success());
        path.to_string_lossy().into_owned()
    }

    fn sync_to_end(host: &mut Host, ctx: &egui::Context, clock: f64) -> bool {
        host.apply(Change::SyncNow, ctx);
        assert!(host.busy);
        let mut reloads = 0;
        wait_until(|| {
            reloads += usize::from(host.drive(ctx, clock));
            !host.busy
        });
        reloads += usize::from(host.drive(ctx, clock));
        reloads > 0
    }

    #[test]
    fn a_pull_that_replaced_synced_files_asks_the_app_to_read_them_again_once() {
        let scratch = tempfile::tempdir().unwrap();
        let remote = bare_remote(scratch.path());
        let other_home = tempfile::tempdir().unwrap();
        let other_paths = TidewatchPaths::new(other_home.path());
        std::fs::create_dir_all(other_paths.config_dir()).unwrap();
        std::fs::write(other_paths.prefs_file(), "restore_session = false\n").unwrap();
        let mut other = spec(&other_paths, &Tools::default());
        other.env.extend(git_env());
        stdusk_core::sync::run(&other, Op::Reconcile, &remote).unwrap();

        let home = tempfile::tempdir().unwrap();
        let mut host = host_in(home.path(), &remote, false, true);
        let ctx = egui::Context::default();
        assert!(sync_to_end(&mut host, &ctx, 1.0), "the pulled settings.toml is new here");
        assert_eq!(
            std::fs::read_to_string(TidewatchPaths::new(home.path()).prefs_file()).unwrap(),
            "restore_session = false\n"
        );
        assert_eq!(host.note, Note::Updated { repo: remote.clone() });
        assert!(host.view(0).last_synced.is_some());
        assert!(!sync_to_end(&mut host, &ctx, 2.0), "nothing new, nothing to read again");
        assert_eq!(host.note, Note::None);
    }

    #[test]
    fn the_watcher_asks_for_a_frame_only_when_a_synced_file_changed() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        let ctx = egui::Context::default();
        let requests = Arc::new(Mutex::new(0_usize));
        let counted = requests.clone();
        ctx.set_request_repaint_callback(move |_| *counted.lock().unwrap() += 1);
        let watch = Watch::start(paths.config_dir().to_path_buf(), ctx);
        let before = watch.snapshot();
        watch.watch(true);
        std::thread::sleep(Duration::from_secs_f64(POLL_S * 2.0 + 0.5));
        std::fs::write(paths.trust_file(), "answered = true\n").unwrap();
        std::fs::write(paths.sync_file(), "enabled = true\n").unwrap();
        std::thread::sleep(Duration::from_secs_f64(POLL_S + 0.5));
        assert_eq!(*requests.lock().unwrap(), 0, "unsynced files and quiet time ask for nothing");
        std::fs::write(paths.prefs_file(), "restore_session = false\n").unwrap();
        wait_until(|| *requests.lock().unwrap() > 0);
        assert_ne!(watch.snapshot(), before);
    }

    #[test]
    fn a_switched_off_watcher_never_reads_the_files() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        let watch = Watch::start(paths.config_dir().to_path_buf(), egui::Context::default());
        let before = watch.snapshot();
        std::fs::write(paths.prefs_file(), "restore_session = false\n").unwrap();
        std::thread::sleep(Duration::from_secs_f64(POLL_S + 0.5));
        assert_eq!(watch.snapshot(), before);
        watch.watch(true);
        wait_until(|| watch.snapshot() != before);
    }

    #[test]
    fn what_a_result_says_to_the_user() {
        let cases = [
            (Ok(Outcome::Synced), false, Note::None),
            (Ok(Outcome::Synced), true, Note::Updated { repo: "you/x".to_owned() }),
            (Ok(Outcome::KeptLocal), true, Note::KeptLocal),
            (Ok(Outcome::KeptLocal), false, Note::KeptLocal),
            (
                Err("git ls-remote: ERROR: Repository not found.".to_owned()),
                false,
                Note::MissingRepo { slug: Some("you/x".to_owned()) },
            ),
            (
                Err("git push: rejected\nmore".to_owned()),
                false,
                Note::Failed("git push: rejected".to_owned()),
            ),
        ];
        for (result, applied, want) in cases {
            assert_eq!(note_for(&result, applied, "you/x"), want, "{result:?}");
        }
    }

    #[test]
    fn a_long_git_complaint_is_cut_to_its_first_line() {
        assert_eq!(first_line("\n\n  one\ntwo"), "one");
        assert_eq!(first_line(&"x".repeat(500)).len(), 140);
        assert_eq!(first_line(""), "unknown error");
    }

    fn host_in(home: &Path, repo: &str, enabled: bool, active: bool) -> Host {
        let paths = TidewatchPaths::new(home);
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        SyncPrefs { enabled, repo: repo.to_owned(), last_synced: None }.save(&paths).unwrap();
        Host::new(Some(&paths), Tools::default(), active, &egui::Context::default())
    }

    #[test]
    fn an_inactive_host_never_starts_a_sync_whatever_the_choice_says() {
        let home = tempfile::tempdir().unwrap();
        let mut host = host_in(home.path(), "/nonexistent/never.git", true, false);
        let ctx = egui::Context::default();
        for second in 0..40 {
            host.drive(&ctx, f64::from(second) * 3.0);
        }
        host.apply(Change::SyncNow, &ctx);
        assert!(!host.busy);
        assert_eq!(host.view(0).last_synced, None);
    }

    #[test]
    fn turning_the_switch_and_the_repo_are_saved_apart_from_the_synced_files() {
        let home = tempfile::tempdir().unwrap();
        let mut host = host_in(home.path(), "", false, true);
        let ctx = egui::Context::default();
        host.apply(Change::Repo("you/tidewatch-settings".into()), &ctx);
        host.apply(Change::Enabled(true), &ctx);
        let paths = TidewatchPaths::new(home.path());
        wait_until(|| {
            let saved = SyncPrefs::load(&paths);
            saved.enabled && saved.repo == "you/tidewatch-settings"
        });
        assert!(host.view(0).has_repo);
        assert!(!paths.prefs_file().exists());
    }

    #[test]
    fn sync_now_against_a_missing_repo_ends_with_the_offer_to_create_it() {
        let home = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let missing = scratch.path().join("absent.git").to_string_lossy().into_owned();
        let mut host = host_in(home.path(), &missing, false, true);
        let ctx = egui::Context::default();
        host.apply(Change::SyncNow, &ctx);
        assert!(host.busy);
        wait_until(|| {
            host.drive(&ctx, 1.0);
            !host.busy
        });
        assert!(matches!(host.note, Note::Failed(_)), "{:?}", host.note);
        assert_eq!(host.view(0).last_synced, None);
    }
}
