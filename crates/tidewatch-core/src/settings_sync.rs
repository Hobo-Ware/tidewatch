//! Settings sync: which of the config files travel to the user's own GitHub repo, this machine's
//! sync switch, and when a sync runs. The git work is `stdusk_core::sync`.

use std::ffi::OsString;
use std::path::Path;

use serde::{Deserialize, Serialize};
use stdusk_core::sync::{Files, Op, Spec};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;
use crate::tools::{GhError, Tools, run_gh};

pub const SYNCED: &[&str] = &["settings.toml", "profiles.toml", "workflows.toml"];

pub const POLL_S: f64 = 2.0;
pub const QUIET_S: f64 = 5.0;
pub const RETRY_S: f64 = 60.0;
const WAKE_SLACK_S: f64 = 0.05;

#[must_use]
pub fn spec(paths: &TidewatchPaths, tools: &Tools) -> Spec {
    let mut spec = Spec::new(paths.config_dir().to_path_buf(), Files::Only(SYNCED), "tidewatch");
    spec.env.push((OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")));
    if let Some(path) = tools.path() {
        spec.env.push((OsString::from("PATH"), path.clone()));
    }
    spec
}

/// This machine's sync choice. It lives in `sync.toml`, which is not synced, so turning sync on
/// here never turns it on elsewhere.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncPrefs {
    pub enabled: bool,
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<i64>,
}

impl SyncPrefs {
    #[must_use]
    pub fn load(paths: &TidewatchPaths) -> Self {
        let Ok(text) = std::fs::read_to_string(paths.sync_file()) else { return Self::default() };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: sync.toml is invalid ({e}); sync is off");
            Self::default()
        })
    }

    pub fn save(&self, paths: &TidewatchPaths) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        write_atomic(&paths.sync_file(), &text)
    }

    pub fn update(paths: &TidewatchPaths, change: impl FnOnce(&mut Self)) -> std::io::Result<()> {
        let mut prefs = Self::load(paths);
        change(&mut prefs);
        prefs.save(paths)
    }

    #[must_use]
    pub fn remote(&self) -> Option<String> {
        remote_url(&self.repo)
    }

    #[must_use]
    pub fn auto(&self) -> bool {
        self.enabled && self.remote().is_some()
    }
}

fn is_shorthand(repo: &str) -> bool {
    let name_ok = |part: &str| {
        !part.is_empty()
            && !part.starts_with('.')
            && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    repo.split_once('/').is_some_and(|(owner, name)| name_ok(owner) && name_ok(name))
}

/// The address git fetches from: `owner/name` is a GitHub repo over HTTPS, a URL or an absolute
/// path is taken as typed. `None` when the text is neither.
#[must_use]
pub fn remote_url(repo: &str) -> Option<String> {
    let repo = repo.trim();
    if is_shorthand(repo) {
        let name = repo.strip_suffix(".git").unwrap_or(repo);
        Some(format!("https://github.com/{name}.git"))
    } else if repo.contains("://") || repo.starts_with("git@") || repo.starts_with('/') {
        Some(repo.to_owned())
    } else {
        None
    }
}

/// `owner/name` of a GitHub repo given in any of the accepted spellings, for `gh repo create`.
#[must_use]
pub fn github_slug(repo: &str) -> Option<String> {
    let repo = repo.trim();
    let tail = repo
        .strip_prefix("https://github.com/")
        .or_else(|| repo.strip_prefix("git@github.com:"))
        .unwrap_or(repo);
    let tail = tail.strip_suffix(".git").unwrap_or(tail);
    is_shorthand(tail).then(|| tail.to_owned())
}

/// Whether git's complaint means the repo does not exist (or is not visible to this login).
#[must_use]
pub fn is_missing_repo(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("repository not found")
        || (lower.contains("repository '") && lower.contains("not found"))
}

pub fn create_private_repo(
    tools: &Tools,
    paths: &TidewatchPaths,
    slug: &str,
) -> Result<(), String> {
    let dir = paths.config_dir();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    run_gh(tools, dir, &["repo", "create", slug, "--private"])
        .map(drop)
        .map_err(|e: GhError| e.to_string())
}

/// Everything that would travel, as one string: it differs exactly when a synced file does.
#[must_use]
pub fn fingerprint(dir: &Path) -> String {
    SYNCED.iter().fold(String::new(), |mut all, name| {
        let text = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        all.push_str(name);
        all.push('\0');
        all.push_str(&text);
        all.push('\0');
        all
    })
}

/// When a sync starts: once as sync turns on or the app launches, then after the synced files
/// have sat unchanged for `QUIET_S`. A failed sync waits `RETRY_S` before the next try.
#[derive(Debug, Default)]
pub struct Autosync {
    launched: bool,
    synced: Option<String>,
    seen: Option<String>,
    seen_since: f64,
    next_poll: f64,
    retry_at: f64,
    in_flight: Option<String>,
}

impl Autosync {
    /// `now` is in seconds; `read` runs at most once per call, and only when the files are due a look.
    pub fn tick(
        &mut self,
        now: f64,
        ready: bool,
        busy: bool,
        read: impl FnOnce() -> String,
    ) -> Option<Op> {
        if !ready {
            *self = Self::default();
            return None;
        }
        if busy {
            return None;
        }
        if !self.launched {
            self.launched = true;
            let current = read();
            self.synced = Some(current.clone());
            self.in_flight = Some(current);
            self.next_poll = now + POLL_S;
            return Some(Op::Reconcile);
        }
        if now < self.next_poll {
            return None;
        }
        self.next_poll = now + POLL_S;
        let current = read();
        if self.synced.as_ref() == Some(&current) {
            self.seen = None;
            return None;
        }
        if self.seen.as_ref() != Some(&current) {
            self.seen = Some(current);
            self.seen_since = now;
            return None;
        }
        if now - self.seen_since < QUIET_S || now < self.retry_at {
            return None;
        }
        self.seen = None;
        self.in_flight = Some(current);
        Some(Op::Reconcile)
    }

    /// Seconds until a frame is needed to start the pending sync, or `None` when everything is
    /// synced and nothing has to wake the app. `current` is what the files hold now.
    #[must_use]
    pub fn wake_in(&self, now: f64, current: &str) -> Option<f64> {
        if !self.launched || self.synced.as_deref() == Some(current) {
            return None;
        }
        let due = if self.seen.as_deref() == Some(current) {
            (self.seen_since + QUIET_S).max(self.retry_at)
        } else {
            self.next_poll.max(now)
        };
        Some((due - now).max(0.0) + WAKE_SLACK_S)
    }

    /// A sync the user asked for: `fingerprint` is what was on disk when it started.
    pub fn begin_manual(&mut self, fingerprint: String) {
        self.in_flight = Some(fingerprint);
    }

    /// The running sync ended. Files that changed meanwhile, edits or pulled ones, still differ
    /// from what the sync started with, so they get one more sync.
    pub fn finished(&mut self, now: f64, ok: bool) {
        let started_with = self.in_flight.take();
        if ok {
            self.synced = started_with.or(self.synced.take());
            self.seen = None;
            self.retry_at = 0.0;
        } else {
            self.retry_at = now + RETRY_S;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use stdusk_core::sync::{Outcome, run};

    fn fp(text: &'static str) -> impl FnOnce() -> String {
        move || text.to_owned()
    }

    #[test]
    fn exactly_three_files_travel_and_they_are_the_ones_the_app_reads() {
        let paths = TidewatchPaths::new(Path::new("/h"));
        let names = [paths.prefs_file(), paths.profiles_file(), paths.workflows_file()]
            .map(|file| file.file_name().unwrap().to_string_lossy().into_owned());
        assert_eq!(names, SYNCED);
    }

    #[test]
    fn the_ignore_list_keeps_everything_but_those_three_files_home() {
        let spec = spec(&TidewatchPaths::new(Path::new("/h")), &Tools::default());
        assert_eq!(spec.files.gitignore(), "*\n!settings.toml\n!profiles.toml\n!workflows.toml\n");
        assert_eq!(spec.dir, Path::new("/h/.config/tidewatch"));
        let paths = TidewatchPaths::new(Path::new("/h"));
        assert!(!paths.trust_file().ends_with("settings.toml"));
        assert!(!spec.dir.starts_with(paths.state_dir()));
    }

    #[test]
    fn the_sync_choice_is_never_among_the_synced_files() {
        let paths = TidewatchPaths::new(Path::new("/h"));
        let name = paths.sync_file().file_name().unwrap().to_string_lossy().into_owned();
        assert!(!SYNCED.contains(&name.as_str()));
    }

    #[test]
    fn git_gets_a_path_and_no_prompts() {
        let spec = spec(&TidewatchPaths::new(Path::new("/h")), &Tools::with_path("/opt/bin"));
        let env: Vec<(String, String)> = spec
            .env
            .iter()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(
            env,
            [
                ("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned()),
                ("PATH".to_owned(), "/opt/bin".to_owned())
            ]
        );
    }

    #[test]
    fn prefs_round_trip_and_default_to_off() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        assert_eq!(SyncPrefs::load(&paths), SyncPrefs::default());
        assert!(!SyncPrefs::default().auto());
        SyncPrefs::update(&paths, |p| {
            p.enabled = true;
            p.repo = "you/tidewatch-settings".into();
            p.last_synced = Some(1_700_000_000);
        })
        .unwrap();
        let back = SyncPrefs::load(&paths);
        assert!(back.auto());
        assert_eq!(back.last_synced, Some(1_700_000_000));
        assert_eq!(back.remote().as_deref(), Some("https://github.com/you/tidewatch-settings.git"));
    }

    #[test]
    fn a_switch_without_a_usable_repo_does_not_sync() {
        let prefs = SyncPrefs { enabled: true, repo: "not a repo".into(), last_synced: None };
        assert!(!prefs.auto());
    }

    #[test]
    fn repo_text_becomes_a_git_address() {
        let cases = [
            ("you/tidewatch-settings", Some("https://github.com/you/tidewatch-settings.git")),
            (
                "  you/tidewatch-settings.git ",
                Some("https://github.com/you/tidewatch-settings.git"),
            ),
            ("git@github.com:you/x.git", Some("git@github.com:you/x.git")),
            ("https://github.com/you/x", Some("https://github.com/you/x")),
            ("/tmp/bare.git", Some("/tmp/bare.git")),
            ("", None),
            ("you", None),
            ("you/", None),
            ("a/b/c", None),
            (".hidden/x", None),
            ("two words/x", None),
        ];
        for (text, want) in cases {
            assert_eq!(remote_url(text).as_deref(), want, "{text:?}");
        }
    }

    #[test]
    fn the_slug_for_gh_comes_from_every_github_spelling() {
        let cases = [
            ("you/x", Some("you/x")),
            ("you/x.git", Some("you/x")),
            ("https://github.com/you/x.git", Some("you/x")),
            ("git@github.com:you/x", Some("you/x")),
            ("https://gitlab.com/you/x", None),
            ("/tmp/bare.git", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(github_slug(text).as_deref(), want, "{text:?}");
        }
    }

    #[test]
    fn a_missing_repo_is_told_from_other_git_failures() {
        let missing = [
            "git ls-remote: remote: Repository not found.\nfatal: repository 'https://github.com/you/x.git/' not found",
            "git ls-remote: ERROR: Repository not found.\nfatal: Could not read from remote repository.",
        ];
        for error in missing {
            assert!(is_missing_repo(error), "{error}");
        }
        for error in [
            "git ls-remote: fatal: could not read Username for 'https://github.com'",
            "git push: ! [rejected] main -> main (fetch first)",
        ] {
            assert!(!is_missing_repo(error), "{error}");
        }
    }

    #[test]
    fn the_fingerprint_changes_with_a_synced_file_only() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        let before = fingerprint(paths.config_dir());
        std::fs::write(paths.trust_file(), "x").unwrap();
        std::fs::write(paths.sync_file(), "enabled = true").unwrap();
        assert_eq!(fingerprint(paths.config_dir()), before);
        std::fs::write(paths.workflows_file(), "[x]").unwrap();
        assert_ne!(fingerprint(paths.config_dir()), before);
    }

    #[test]
    fn nothing_starts_while_sync_is_off_or_a_sync_runs() {
        let mut auto = Autosync::default();
        assert_eq!(auto.tick(0.0, false, false, fp("a")), None);
        assert_eq!(auto.tick(1.0, true, true, fp("a")), None);
    }

    #[test]
    fn a_sync_starts_once_at_launch_and_not_again_while_nothing_changes() {
        let mut auto = Autosync::default();
        assert_eq!(auto.tick(0.0, true, false, fp("a")), Some(Op::Reconcile));
        auto.finished(1.0, true);
        for second in 2..200 {
            assert_eq!(auto.tick(f64::from(second), true, false, fp("a")), None, "{second}");
        }
    }

    #[test]
    fn turning_sync_on_syncs_at_once_and_turning_it_off_forgets_the_launch() {
        let mut auto = Autosync::default();
        assert_eq!(auto.tick(0.0, false, false, fp("a")), None);
        assert_eq!(auto.tick(5.0, true, false, fp("a")), Some(Op::Reconcile));
        auto.finished(6.0, true);
        assert_eq!(auto.tick(8.0, false, false, fp("a")), None);
        assert_eq!(auto.tick(9.0, true, false, fp("a")), Some(Op::Reconcile));
    }

    #[test]
    fn a_change_is_pushed_after_it_has_been_quiet_and_not_before() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        assert_eq!(auto.tick(2.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(4.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(6.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(8.0, true, false, fp("b")), Some(Op::Reconcile));
    }

    #[test]
    fn a_change_that_keeps_changing_waits_for_the_last_edit() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        let edits = ["b", "c", "d", "e", "f"];
        for (i, text) in edits.into_iter().enumerate() {
            let now = 2.0 + 2.0 * i as f64;
            assert_eq!(auto.tick(now, true, false, move || text.to_owned()), None, "{now}");
        }
        assert_eq!(auto.tick(12.0, true, false, fp("f")), None);
        assert_eq!(auto.tick(14.0, true, false, fp("f")), None);
        assert_eq!(auto.tick(16.0, true, false, fp("f")), Some(Op::Reconcile));
    }

    #[test]
    fn an_edit_made_back_to_the_synced_text_pushes_nothing() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        assert_eq!(auto.tick(2.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(4.0, true, false, fp("a")), None);
        assert_eq!(auto.tick(20.0, true, false, fp("a")), None);
    }

    #[test]
    fn the_files_are_read_on_the_poll_not_every_frame() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        let mut reads = 0;
        for frame in 0..100 {
            let now = 1.0 + f64::from(frame) * 0.016;
            auto.tick(now, true, false, || {
                reads += 1;
                "a".to_owned()
            });
        }
        assert_eq!(reads, 1);
    }

    #[test]
    fn a_failed_sync_is_retried_after_a_minute() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        auto.tick(2.0, true, false, fp("b"));
        assert_eq!(auto.tick(8.0, true, false, fp("b")), Some(Op::Reconcile));
        auto.finished(9.0, false);
        assert_eq!(auto.tick(10.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(40.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(66.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(70.0, true, false, fp("b")), Some(Op::Reconcile));
    }

    #[test]
    fn a_failed_launch_sync_does_not_retry_until_something_changes() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(1.0, false);
        for second in 2..400 {
            assert_eq!(auto.tick(f64::from(second), true, false, fp("a")), None, "{second}");
        }
        assert_eq!(auto.tick(400.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(406.0, true, false, fp("b")), Some(Op::Reconcile));
    }

    #[test]
    fn files_that_changed_during_a_sync_get_one_more() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        assert_eq!(auto.tick(2.0, true, true, fp("b")), None);
        auto.finished(3.0, true);
        assert_eq!(auto.tick(4.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(10.0, true, false, fp("b")), Some(Op::Reconcile));
    }

    #[test]
    fn a_manual_sync_counts_as_synced_for_the_text_it_started_with() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        auto.begin_manual("b".to_owned());
        auto.finished(3.0, true);
        assert_eq!(auto.tick(4.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(30.0, true, false, fp("b")), None);
    }

    #[test]
    fn nothing_wakes_the_app_while_everything_is_synced() {
        let mut auto = Autosync::default();
        assert_eq!(auto.wake_in(0.0, "a"), None);
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        assert_eq!(auto.wake_in(1.0, "a"), None);
        assert_eq!(auto.wake_in(500.0, "a"), None);
    }

    #[test]
    fn a_pending_change_wakes_the_app_for_the_next_look_then_for_the_quiet_end() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        let first = auto.wake_in(1.0, "b").unwrap();
        assert!((first - (POLL_S - 1.0 + WAKE_SLACK_S)).abs() < 1e-9, "{first}");
        assert_eq!(auto.tick(2.0, true, false, fp("b")), None);
        let quiet = auto.wake_in(2.0, "b").unwrap();
        assert!((quiet - (QUIET_S + WAKE_SLACK_S)).abs() < 1e-9, "{quiet}");
        assert_eq!(auto.tick(2.0 + quiet, true, false, fp("b")), Some(Op::Reconcile));
    }

    #[test]
    fn a_failed_sync_wakes_the_app_for_its_retry_and_not_before() {
        let mut auto = Autosync::default();
        auto.tick(0.0, true, false, fp("a"));
        auto.finished(0.5, true);
        auto.tick(2.0, true, false, fp("b"));
        assert_eq!(auto.tick(8.0, true, false, fp("b")), Some(Op::Reconcile));
        auto.finished(9.0, false);
        assert_eq!(auto.tick(10.0, true, false, fp("b")), None);
        assert_eq!(auto.tick(16.0, true, false, fp("b")), None);
        let wake = auto.wake_in(16.0, "b").unwrap();
        assert!((wake - (9.0 + RETRY_S - 16.0 + WAKE_SLACK_S)).abs() < 1e-9, "{wake}");
    }

    #[test]
    fn a_pulled_change_is_what_the_app_reads_next() {
        let scratch = tempfile::tempdir().unwrap();
        let remote = bare_remote(&scratch);
        let mac = Machine::new();
        let other = Machine::new();
        put(mac.paths.prefs_file(), "restore_session = true\n");
        mac.sync(&remote).unwrap();
        other.sync(&remote).unwrap();
        crate::prefs::Prefs::update(&other.paths, |p| {
            p.restore_session = false;
            p.window.ui_zoom = 1.25;
            p.window.height_pct = 0.7;
            p.keep_awake.enabled = false;
        })
        .unwrap();
        let mut profiles = crate::profiles::Profiles::seeded(&["claude".to_owned()]);
        profiles.profile.remove("reviewer");
        profiles.save(&other.paths.profiles_file()).unwrap();
        crate::workflows::Workflows::defaults().save(&other.paths.workflows_file()).unwrap();
        other.sync(&remote).unwrap();
        assert!(crate::prefs::Prefs::load(&mac.paths).restore_session);
        assert_eq!(mac.sync(&remote), Ok(Outcome::Synced));
        let prefs = crate::prefs::Prefs::load(&mac.paths);
        assert!(!prefs.restore_session && !prefs.keep_awake.enabled);
        assert!((prefs.window.ui_zoom - 1.25).abs() < 1e-6);
        assert!((prefs.window.height_pct - 0.7).abs() < 1e-6);
        let loaded = crate::profiles::Profiles::load_or_seed(&mac.paths, None);
        assert_eq!(loaded, profiles);
        assert_eq!(
            crate::workflows::Workflows::load_or_seed(&mac.paths, None),
            crate::workflows::Workflows::defaults()
        );
    }

    fn git_env() -> Vec<(OsString, OsString)> {
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

    fn put(file: std::path::PathBuf, text: &str) {
        std::fs::write(file, text).unwrap();
    }

    fn get(file: std::path::PathBuf) -> String {
        std::fs::read_to_string(file).unwrap_or_default()
    }

    struct Machine {
        _home: tempfile::TempDir,
        paths: TidewatchPaths,
        spec: Spec,
    }

    impl Machine {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let paths = TidewatchPaths::new(home.path());
            let mut spec = spec(&paths, &Tools::default());
            spec.env.extend(git_env());
            std::fs::create_dir_all(paths.config_dir()).unwrap();
            Self { _home: home, paths, spec }
        }

        fn sync(&self, remote: &str) -> Result<Outcome, String> {
            run(&self.spec, Op::Reconcile, remote)
        }
    }

    fn bare_remote(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("settings.git");
        let status = Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(&path)
            .envs(git_env())
            .status()
            .unwrap();
        assert!(status.success());
        path.to_string_lossy().into_owned()
    }

    fn files_in(remote: &str) -> Vec<String> {
        let out = Command::new("git")
            .args(["-C", remote, "ls-tree", "-r", "--name-only", "main"])
            .envs(git_env())
            .output()
            .unwrap();
        let mut names: Vec<String> =
            String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect();
        names.sort();
        names
    }

    #[test]
    fn only_settings_profiles_and_workflows_reach_the_repo() {
        let scratch = tempfile::tempdir().unwrap();
        let remote = bare_remote(&scratch);
        let mac = Machine::new();
        put(mac.paths.prefs_file(), "restore_session = true\n");
        put(mac.paths.profiles_file(), "[profile.x]\n");
        put(mac.paths.workflows_file(), "[workflow.y]\n");
        put(mac.paths.trust_file(), "answered = true\n");
        put(mac.paths.sync_file(), "enabled = true\n");
        put(mac.paths.house_style_file(), "# style\n");
        put(mac.paths.checklist_file(), "- [ ] one\n");
        put(mac.paths.worktree_setup_file(), "[x]\n");
        put(mac.paths.workflows_offered_file(), "build\n");
        std::fs::create_dir_all(mac.paths.state_dir()).unwrap();
        put(mac.paths.state_dir().join("secret"), "token\n");
        assert_eq!(mac.sync(&remote), Ok(Outcome::Synced));
        assert_eq!(files_in(&remote), ["profiles.toml", "settings.toml", "workflows.toml"]);
    }

    #[test]
    fn a_second_mac_gets_the_settings_and_keeps_its_own_trust_answers() {
        let scratch = tempfile::tempdir().unwrap();
        let remote = bare_remote(&scratch);
        let mac = Machine::new();
        put(mac.paths.prefs_file(), "restore_session = false\n");
        put(mac.paths.profiles_file(), "[profile.x]\n");
        mac.sync(&remote).unwrap();
        let other = Machine::new();
        put(other.paths.trust_file(), "answered = \"this mac\"\n");
        assert_eq!(other.sync(&remote), Ok(Outcome::Synced));
        assert_eq!(get(other.paths.prefs_file()), "restore_session = false\n");
        assert_eq!(get(other.paths.profiles_file()), "[profile.x]\n");
        assert_eq!(get(other.paths.trust_file()), "answered = \"this mac\"\n");
        assert_eq!(files_in(&remote), ["profiles.toml", "settings.toml"]);
    }

    #[test]
    fn a_conflict_keeps_this_macs_settings_and_the_repo_gets_them() {
        let scratch = tempfile::tempdir().unwrap();
        let remote = bare_remote(&scratch);
        let mac = Machine::new();
        let other = Machine::new();
        put(mac.paths.prefs_file(), "ui_zoom = 1.0\n");
        mac.sync(&remote).unwrap();
        other.sync(&remote).unwrap();
        put(other.paths.prefs_file(), "ui_zoom = 0.8\n");
        other.sync(&remote).unwrap();
        put(mac.paths.prefs_file(), "ui_zoom = 1.25\n");
        assert_eq!(mac.sync(&remote), Ok(Outcome::KeptLocal));
        assert_eq!(get(mac.paths.prefs_file()), "ui_zoom = 1.25\n");
    }

    #[test]
    fn a_missing_repo_error_is_the_kind_the_offer_to_create_it_listens_for() {
        let scratch = tempfile::tempdir().unwrap();
        let mac = Machine::new();
        put(mac.paths.prefs_file(), "x = 1\n");
        let missing = scratch.path().join("nope.git").to_string_lossy().into_owned();
        let error = mac.sync(&missing).unwrap_err();
        assert!(error.starts_with("git ls-remote"), "{error}");
    }
}
