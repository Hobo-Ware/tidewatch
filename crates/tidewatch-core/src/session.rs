//! The saved session in `.state/session.toml` (PLAN.md, D11): per window, the tabs open at the last
//! quit, in order, with what a restart needs to reopen each one.
//!
//! A bad file never stops the app: it loads as an empty session with a stderr note. `restore`
//! decides what to do with the tabs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agents::Level;
use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default, rename = "window")]
    pub windows: Vec<SavedWindow>,
}

/// One window: a git repository (worktree tabs) or, with `other`, a plain folder (the Other group).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindow {
    /// The repo root, or the folder in the Other group.
    pub dir: PathBuf,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub other: bool,
    /// Index into `tabs`.
    #[serde(default)]
    pub active: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub focus_mode: bool,
    #[serde(default, rename = "tab")]
    pub tabs: Vec<SavedTab>,
    #[serde(default, rename = "park", skip_serializing_if = "Vec::is_empty")]
    pub parked: Vec<SavedPark>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    Idle,
    Job,
    Waiting,
    Manual,
    Overflow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPark {
    pub tab: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<Why>,
    #[serde(default)]
    pub since_ms: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
}

/// A tab as it was open. `branch` is `Some` for a tab in an agent worktree (`dir` is that
/// worktree) and `None` for a tab in a shared folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SavedTab {
    /// The pinned shell in the window's folder.
    Main,
    Shell {
        dir: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
    },
    Agent(SavedAgentTab),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedAgentTab {
    pub dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub agent: String,
    /// The `profiles.toml` key, or the profile's display name.
    pub profile: String,
    /// The level the tab ran at, which may differ from the profile's default.
    pub level: Level,
    /// The id Tidewatch passed with `--session-id`. Codex takes none, so it is filled in later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The tab's workflow run (`stages.json`), when one is attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stages: Option<PathBuf>,
    /// The agent process ended abnormally while the app kept running (see
    /// `resume_mode::exit_is_abnormal`). The next launch types its resume command and leaves Enter.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub crashed: bool,
    /// The agent was started with remote control on, so a resume starts it the same way.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remote: bool,
}

impl SavedTab {
    /// The folder the tab ran in. `window_dir` stands in for the main tab.
    #[must_use]
    pub fn dir<'a>(&'a self, window_dir: &'a Path) -> &'a Path {
        match self {
            Self::Main => window_dir,
            Self::Shell { dir, .. } => dir,
            Self::Agent(tab) => &tab.dir,
        }
    }

    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Main => None,
            Self::Shell { branch, .. } => branch.as_deref(),
            Self::Agent(tab) => tab.branch.as_deref(),
        }
    }

    /// A short name for messages: the worktree name, else the folder name, else `main`.
    #[must_use]
    pub fn label(&self, window_dir: &Path) -> String {
        if matches!(self, Self::Main) {
            return "main".to_owned();
        }
        if let Some(branch) = self.branch() {
            return branch
                .strip_prefix(crate::worktree::BRANCH_PREFIX)
                .unwrap_or(branch)
                .to_owned();
        }
        let dir = self.dir(window_dir);
        dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into())
    }
}

impl Session {
    /// A missing file is an empty session. An unreadable or invalid one is too, plus a stderr note.
    #[must_use]
    pub fn load(paths: &TidewatchPaths) -> Self {
        Self::load_file(&paths.session_file())
    }

    fn load_file(file: &Path) -> Self {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                eprintln!("tidewatch: cannot read {} ({e}); starting empty", file.display());
                return Self::default();
            }
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: session.toml is unreadable ({e}); starting empty");
            Self::default()
        })
    }

    pub fn save(&self, paths: &TidewatchPaths) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        write_atomic(&paths.session_file(), &text)
    }

    #[must_use]
    pub fn window(&self, dir: &Path) -> Option<&SavedWindow> {
        self.windows.iter().find(|w| w.dir == dir)
    }

    /// Replaces the window of the same folder, or adds it, so a repo keeps one entry.
    pub fn replace_window(&mut self, window: SavedWindow) {
        match self.windows.iter_mut().find(|w| w.dir == window.dir) {
            Some(slot) => *slot = window,
            None => self.windows.push(window),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_tab(branch: Option<&str>) -> SavedTab {
        SavedTab::Agent(SavedAgentTab {
            dir: PathBuf::from("/root/web-ab12cd/brave-otter"),
            branch: branch.map(str::to_owned),
            agent: "claude".into(),
            profile: "builder".into(),
            level: Level::Edits,
            session_id: Some("0c2cbc96-1111-4222-8333-444455556666".into()),
            stages: Some(PathBuf::from("/root/.state/web-ab12cd/brave-otter/stages.json")),
            crashed: false,
            remote: false,
        })
    }

    fn window() -> SavedWindow {
        SavedWindow {
            dir: PathBuf::from("/work/web"),
            other: false,
            active: 2,
            focus_mode: true,
            tabs: vec![
                SavedTab::Main,
                SavedTab::Shell {
                    dir: PathBuf::from("/root/web-ab12cd/calm-comet"),
                    branch: Some("agent/calm-comet".into()),
                },
                agent_tab(Some("agent/brave-otter")),
                SavedTab::Agent(SavedAgentTab {
                    dir: PathBuf::from("/work/web"),
                    branch: None,
                    agent: "codex".into(),
                    profile: "codex-builder".into(),
                    level: Level::Plan,
                    session_id: None,
                    stages: None,
                    crashed: false,
                    remote: false,
                }),
            ],
            parked: Vec::new(),
        }
    }

    #[test]
    fn a_session_round_trips_through_the_file() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let session = Session {
            windows: vec![
                window(),
                SavedWindow {
                    dir: PathBuf::from("/scratch"),
                    other: true,
                    active: 0,
                    focus_mode: false,
                    tabs: vec![SavedTab::Main],
                    parked: Vec::new(),
                },
            ],
        };
        session.save(&paths).unwrap();
        assert_eq!(Session::load(&paths), session);
    }

    #[test]
    fn remote_control_is_remembered_and_a_file_without_it_loads_as_off() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut on = window();
        if let SavedTab::Agent(tab) = &mut on.tabs[2] {
            tab.remote = true;
        }
        let session = Session { windows: vec![on] };
        session.save(&paths).unwrap();
        let text = std::fs::read_to_string(paths.session_file()).unwrap();
        assert_eq!(text.matches("remote = true").count(), 1, "{text}");
        assert_eq!(Session::load(&paths), session);

        let old = "[[window]]\ndir = \"/a\"\n[[window.tab]]\nkind = \"agent\"\ndir = \"/a\"\n\
                   agent = \"claude\"\nprofile = \"builder\"\nlevel = \"edits\"\n";
        let loaded: Session = toml::from_str(old).unwrap();
        let SavedTab::Agent(tab) = &loaded.windows[0].tabs[0] else { panic!("an agent tab") };
        assert!(!tab.remote);
        assert!(!toml::to_string(&loaded).unwrap().contains("remote"));
    }

    #[test]
    fn parking_round_trips_and_an_old_file_loads_unparked() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let mut parked = window();
        parked.parked = vec![
            SavedPark { tab: 1, why: Some(Why::Waiting), since_ms: 42, keep: false },
            SavedPark { tab: 2, why: None, since_ms: 0, keep: true },
        ];
        let session = Session { windows: vec![parked] };
        session.save(&paths).unwrap();
        let text = std::fs::read_to_string(paths.session_file()).unwrap();
        assert!(text.contains("[[window.park]]") && text.contains("why = \"waiting\""), "{text}");
        assert_eq!(Session::load(&paths), session);

        let plain = toml::to_string(&Session { windows: vec![window()] }).unwrap();
        assert!(!plain.contains("park"), "{plain}");
        let old: Session =
            toml::from_str("[[window]]\ndir = \"/a\"\n[[window.tab]]\nkind = \"main\"\n").unwrap();
        assert_eq!(old.windows[0].parked, []);
    }

    #[test]
    fn every_reason_has_a_snake_case_word() {
        let cases = [
            (Why::Idle, "idle"),
            (Why::Job, "job"),
            (Why::Waiting, "waiting"),
            (Why::Manual, "manual"),
            (Why::Overflow, "overflow"),
        ];
        for (why, word) in cases {
            let text =
                toml::to_string(&SavedPark { tab: 0, why: Some(why), since_ms: 0, keep: false })
                    .unwrap();
            assert!(text.contains(&format!("why = \"{word}\"")), "{text}");
        }
    }

    #[test]
    fn tabs_are_written_as_tagged_tables() {
        let text = toml::to_string(&Session { windows: vec![window()] }).unwrap();
        assert!(text.contains("[[window.tab]]"), "{text}");
        assert!(text.contains("kind = \"main\""), "{text}");
        assert!(text.contains("kind = \"agent\""), "{text}");
        assert!(text.contains("level = \"edits\""), "{text}");
    }

    #[test]
    fn a_missing_file_is_an_empty_session() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(Session::load(&TidewatchPaths::new(home.path())), Session::default());
    }

    #[test]
    fn a_bad_file_is_an_empty_session_and_stays_on_disk() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        std::fs::create_dir_all(paths.state_dir()).unwrap();
        for bad in [
            "window = [[[",
            "[[window]]\ndir = 3\n",
            "[[window]]\ndir = \"/a\"\n[[window.tab]]\nkind = \"nope\"\n",
        ] {
            std::fs::write(paths.session_file(), bad).unwrap();
            assert_eq!(Session::load(&paths), Session::default(), "{bad}");
            assert_eq!(std::fs::read_to_string(paths.session_file()).unwrap(), bad);
        }
    }

    #[test]
    fn a_minimal_file_fills_the_defaults() {
        let session: Session = toml::from_str("[[window]]\ndir = \"/a\"\n").unwrap();
        let w = &session.windows[0];
        assert_eq!((w.other, w.active, w.focus_mode, w.tabs.len()), (false, 0, false, 0));
    }

    #[test]
    fn replacing_a_window_keeps_one_entry_per_folder() {
        let mut session = Session::default();
        session.replace_window(window());
        let mut changed = window();
        changed.active = 0;
        session.replace_window(changed.clone());
        session.replace_window(SavedWindow { dir: PathBuf::from("/other"), ..window() });
        assert_eq!(session.windows.len(), 2);
        assert_eq!(session.window(Path::new("/work/web")), Some(&changed));
    }

    #[test]
    fn labels_name_the_worktree_the_folder_or_main() {
        let dir = Path::new("/work/web");
        assert_eq!(SavedTab::Main.label(dir), "main");
        assert_eq!(agent_tab(Some("agent/brave-otter")).label(dir), "brave-otter");
        let shared = SavedTab::Shell { dir: PathBuf::from("/scratch/notes"), branch: None };
        assert_eq!(shared.label(dir), "notes");
        assert_eq!(SavedTab::Main.dir(dir), dir);
    }
}
