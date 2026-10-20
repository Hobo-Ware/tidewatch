//! On-disk layout: `~/.tidewatch` (per-repo worktree folders, state) and `~/.config/tidewatch` (settings).
//!
//! Pure path arithmetic. Nothing here touches the disk.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const HASH_HEX_LEN: usize = 6;
const NOTES_FILE: &str = "notes.json";
const STAGES_FILE: &str = "stages.json";
const PR_CARDS_FILE: &str = "pr_cards.json";
const PROGRESS_FILE: &str = "progress.md";
const HOUSE_STYLE_FILE: &str = "house-style.md";
const JOBS_FILE: &str = "jobs.json";
const AVATAR_FILE: &str = "avatar.json";
const PROGRESS_FEED_FILE: &str = "progress-feed.jsonl";

/// The ledger file that sits next to a tab's `notes.json`.
#[must_use]
pub fn progress_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(PROGRESS_FILE)
}

#[must_use]
pub fn jobs_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(JOBS_FILE)
}

#[must_use]
pub fn avatar_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(AVATAR_FILE)
}

#[must_use]
pub fn progress_feed_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(PROGRESS_FEED_FILE)
}

fn repo_key(repo: &Path) -> String {
    let name = repo.file_name().unwrap_or_default().to_string_lossy();
    let digest = Sha256::digest(repo.as_os_str().as_encoded_bytes());
    let hash = format!("{digest:x}");
    format!("{name}-{}", &hash[..HASH_HEX_LEN])
}

#[derive(Clone, Debug)]
pub struct TidewatchPaths {
    root: PathBuf,
    config_dir: PathBuf,
}

impl TidewatchPaths {
    #[must_use]
    pub fn new(home: &Path) -> Self {
        Self { root: home.join(".tidewatch"), config_dir: home.join(".config").join("tidewatch") }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Settings live apart from the worktrees so no CLI walking up from a worktree finds them.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    #[must_use]
    pub fn profiles_file(&self) -> PathBuf {
        self.config_dir.join("profiles.toml")
    }

    #[must_use]
    pub fn workflows_file(&self) -> PathBuf {
        self.config_dir.join("workflows.toml")
    }

    /// The built-in workflow ids already offered to this `workflows.toml`, one per line.
    #[must_use]
    pub fn workflows_offered_file(&self) -> PathBuf {
        self.config_dir.join("workflows.offered")
    }

    /// The answer to the D14 folder-trust question and the entries Tidewatch added to the CLIs' files.
    #[must_use]
    pub fn trust_file(&self) -> PathBuf {
        self.config_dir.join("trust.toml")
    }

    /// The few app-wide switches, such as restoring the last session at launch.
    #[must_use]
    pub fn prefs_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    #[must_use]
    pub fn house_style_file(&self) -> PathBuf {
        self.config_dir.join(HOUSE_STYLE_FILE)
    }

    /// The checklist template every agent's progress ledger starts from.
    #[must_use]
    pub fn checklist_file(&self) -> PathBuf {
        self.config_dir.join("checklist.md")
    }

    /// Holds the `git-remote-tidewatch-no-push` helper that agent tabs get on their `PATH`.
    #[must_use]
    pub fn guard_bin_dir(&self) -> PathBuf {
        self.state_dir().join("guard-bin")
    }

    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join(".state")
    }

    /// The scheduled items of every tab's ledger, for the Coming up list across tabs.
    #[must_use]
    pub fn schedule_file(&self) -> PathBuf {
        self.state_dir().join("schedule.toml")
    }

    /// The Meeseeks that poofed, for the Overview's Poofed today list.
    #[must_use]
    pub fn meeseeks_file(&self) -> PathBuf {
        self.state_dir().join("meeseeks.toml")
    }

    /// The tabs open at the last quit, restored at the next launch.
    #[must_use]
    pub fn session_file(&self) -> PathBuf {
        self.state_dir().join("session.toml")
    }

    /// Exists while a run is live and until its clean quit: what a crashed run leaves behind.
    #[must_use]
    pub fn run_marker_file(&self) -> PathBuf {
        self.state_dir().join("running")
    }

    /// The folder of the window used last, so a launch with no path can reopen it.
    #[must_use]
    pub fn last_workspace_file(&self) -> PathBuf {
        self.state_dir().join("last-workspace.toml")
    }

    /// The Unix socket of a running window lives here (`instance`). Under the state dir, so a
    /// `TIDEWATCH_HOME` run never sees the real one.
    #[must_use]
    pub fn instance_dir(&self) -> PathBuf {
        self.state_dir().join("instances")
    }

    /// The socket of the one window an app run serves. A Unix socket path must stay under about
    /// 100 bytes, so the name is short.
    #[must_use]
    pub fn app_socket(&self) -> PathBuf {
        self.instance_dir().join("tidewatch.sock")
    }

    /// Where a window hands its terminals to the copy that replaces it. Beside the instance folder,
    /// not in it, so a toggle sent to every socket there never reaches it. The process id keeps two
    /// overlapping restarts apart.
    #[must_use]
    pub fn handoff_socket(&self, pid: u32) -> PathBuf {
        self.state_dir().join(format!("handoff-{pid}.sock"))
    }

    /// Keyed by the full repo path, so two repos with the same folder name never share a directory.
    #[must_use]
    pub fn repo_dir(&self, repo: &Path) -> PathBuf {
        self.root.join(repo_key(repo))
    }

    /// Where a tab keeps its state: `.state/<repo-dir>/<tab>`. Survives the tab, so closing one
    /// keeps its notes as history.
    #[must_use]
    pub fn tab_state_dir(&self, repo: &Path, tab: &str) -> PathBuf {
        self.state_dir().join(repo_key(repo)).join(tab)
    }

    #[must_use]
    pub fn notes_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(NOTES_FILE)
    }

    /// A tab's pull request cards: the GitHub events shown in its Reader, and what was already seen.
    #[must_use]
    pub fn pr_cards_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(PR_CARDS_FILE)
    }

    /// A tab's progress ledger, as the agent last sent it.
    #[must_use]
    pub fn progress_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(PROGRESS_FILE)
    }

    #[must_use]
    pub fn progress_feed_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(PROGRESS_FEED_FILE)
    }

    /// What one agent is given as its house style: the house-style file and the ledger format with
    /// the repo's checklist filled in.
    #[must_use]
    pub fn tab_house_style_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(HOUSE_STYLE_FILE)
    }

    /// The copy of [`Self::tab_house_style_file`] in the shape Copilot reads (`*.instructions.md`).
    #[must_use]
    pub fn tab_copilot_instructions_dir(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join("copilot")
    }

    /// A tab's workflow run: stage states, summaries and findings, enough to resume after a restart.
    #[must_use]
    pub fn stages_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(STAGES_FILE)
    }

    #[must_use]
    pub fn worktree_dir(&self, repo: &Path, tab: &str) -> PathBuf {
        self.repo_dir(repo).join(tab)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> TidewatchPaths {
        TidewatchPaths::new(Path::new("/Users/dev"))
    }

    #[test]
    fn same_folder_name_at_different_paths_gets_different_dirs() {
        let paths = paths();
        assert_ne!(
            paths.repo_dir(Path::new("/work/web")),
            paths.repo_dir(Path::new("/personal/web")),
        );
    }

    #[test]
    fn the_trust_record_sits_with_the_settings_not_in_the_worktree_root() {
        let paths = paths();
        assert_eq!(paths.trust_file(), Path::new("/Users/dev/.config/tidewatch/trust.toml"));
        assert!(!paths.trust_file().starts_with(paths.root()));
    }

    #[test]
    fn the_handoff_socket_is_per_process_and_outside_the_instance_folder() {
        let paths = paths();
        let socket = paths.handoff_socket(4242);
        assert_eq!(socket, Path::new("/Users/dev/.tidewatch/.state/handoff-4242.sock"));
        assert!(!socket.starts_with(paths.instance_dir()));
        assert_ne!(socket, paths.handoff_socket(4243));
        assert!(socket.as_os_str().len() < 104);
    }

    #[test]
    fn same_path_gives_same_dir() {
        let paths = paths();
        let repo = Path::new("/work/web");
        assert_eq!(paths.repo_dir(repo), paths.repo_dir(repo));
    }

    #[test]
    fn worktree_lives_under_repo_dir() {
        let paths = paths();
        let repo = Path::new("/work/web");
        let repo_dir = paths.repo_dir(repo);
        for tab in ["main", "agent-login-fix"] {
            let worktree = paths.worktree_dir(repo, tab);
            assert_eq!(worktree, repo_dir.join(tab), "tab {tab}");
            assert!(worktree.starts_with(&repo_dir), "tab {tab}");
        }
    }

    #[test]
    fn settings_live_under_dot_config_not_under_the_worktree_root() {
        let paths = paths();
        let config = Path::new("/Users/dev/.config/tidewatch");
        assert_eq!(paths.config_dir(), config);
        assert_eq!(paths.profiles_file(), config.join("profiles.toml"));
        assert_eq!(paths.house_style_file(), config.join("house-style.md"));
        assert!(!paths.config_dir().starts_with(paths.root()));
    }

    #[test]
    fn guard_helper_dir_is_state() {
        let paths = paths();
        assert_eq!(paths.guard_bin_dir(), paths.state_dir().join("guard-bin"));
    }

    #[test]
    fn notes_live_in_the_tab_state_dir_named_like_the_repo_dir() {
        let paths = paths();
        let repo = Path::new("/work/web");
        let repo_dir_name = paths.repo_dir(repo).file_name().unwrap().to_owned();
        let notes = paths.notes_file(repo, "brave-otter");
        assert_eq!(
            notes,
            paths.state_dir().join(repo_dir_name).join("brave-otter").join("notes.json")
        );
        assert_ne!(notes, paths.notes_file(repo, "main"));
        assert_ne!(notes, paths.notes_file(Path::new("/other/web"), "brave-otter"));
    }

    #[test]
    fn workflows_sit_next_to_profiles_and_stages_next_to_notes() {
        let paths = paths();
        let repo = Path::new("/work/web");
        assert_eq!(paths.workflows_file(), paths.config_dir().join("workflows.toml"));
        assert_eq!(
            paths.stages_file(repo, "brave-otter").parent(),
            paths.notes_file(repo, "brave-otter").parent()
        );
        assert!(paths.stages_file(repo, "brave-otter").ends_with("brave-otter/stages.json"));
        assert_ne!(paths.stages_file(repo, "main"), paths.stages_file(repo, "brave-otter"));
    }

    #[test]
    fn pull_request_cards_sit_next_to_the_notes_of_their_tab() {
        let paths = paths();
        let repo = Path::new("/work/web");
        assert_eq!(
            paths.pr_cards_file(repo, "brave-otter").parent(),
            paths.notes_file(repo, "brave-otter").parent()
        );
        assert!(paths.pr_cards_file(repo, "brave-otter").ends_with("brave-otter/pr_cards.json"));
        assert_ne!(paths.pr_cards_file(repo, "main"), paths.pr_cards_file(repo, "brave-otter"));
    }

    #[test]
    fn the_ledger_and_the_agent_house_style_sit_with_the_notes_of_their_tab() {
        let paths = paths();
        let repo = Path::new("/work/web");
        let notes = paths.notes_file(repo, "brave-otter");
        assert_eq!(paths.progress_file(repo, "brave-otter").parent(), notes.parent());
        assert!(paths.progress_file(repo, "brave-otter").ends_with("brave-otter/progress.md"));
        assert_eq!(paths.tab_house_style_file(repo, "brave-otter").parent(), notes.parent());
        assert!(
            paths
                .tab_copilot_instructions_dir(repo, "brave-otter")
                .starts_with(notes.parent().unwrap())
        );
        assert_ne!(paths.progress_file(repo, "main"), paths.progress_file(repo, "brave-otter"));
        assert_eq!(progress_file_beside(&notes), paths.progress_file(repo, "brave-otter"));
    }

    #[test]
    fn jobs_avatar_and_the_progress_feed_sit_with_the_notes_of_their_tab() {
        let paths = paths();
        let repo = Path::new("/work/web");
        let notes = paths.notes_file(repo, "brave-otter");
        let feed = paths.progress_feed_file(repo, "brave-otter");
        assert_eq!(feed.parent(), notes.parent());
        assert!(feed.ends_with("brave-otter/progress-feed.jsonl"));
        assert_eq!(progress_feed_file_beside(&notes), feed);
        assert!(jobs_file_beside(&notes).ends_with("brave-otter/jobs.json"));
        assert!(avatar_file_beside(&notes).ends_with("brave-otter/avatar.json"));
        assert_ne!(feed, paths.progress_feed_file(repo, "main"));
    }

    #[test]
    fn the_checklist_template_is_a_setting_and_the_schedule_index_is_state() {
        let paths = paths();
        assert_eq!(paths.checklist_file(), paths.config_dir().join("checklist.md"));
        assert_eq!(paths.schedule_file(), paths.state_dir().join("schedule.toml"));
    }

    #[test]
    fn session_file_sits_in_the_state_dir_next_to_the_worktree_registry() {
        let paths = paths();
        assert_eq!(paths.session_file(), paths.state_dir().join("session.toml"));
    }

    #[test]
    fn the_run_marker_sits_in_the_state_dir_next_to_the_session() {
        let paths = paths();
        assert_eq!(paths.run_marker_file(), paths.state_dir().join("running"));
    }

    #[test]
    fn the_last_workspace_sits_in_the_state_dir_next_to_the_session() {
        let paths = paths();
        assert_eq!(paths.last_workspace_file(), paths.state_dir().join("last-workspace.toml"));
    }

    #[test]
    fn state_dir_lives_under_root() {
        let paths = paths();
        assert_eq!(paths.state_dir(), paths.root().join(".state"));
        assert!(paths.state_dir().starts_with(paths.root()));
    }

    #[test]
    fn the_window_has_one_short_socket_under_the_state_dir() {
        let paths = paths();
        let socket = paths.app_socket();
        assert_eq!(socket.parent(), Some(paths.instance_dir().as_path()));
        assert!(paths.instance_dir().starts_with(paths.state_dir()));
        assert!(socket.extension().is_some_and(|ext| ext == "sock"));
    }

    #[test]
    fn another_home_has_another_socket() {
        let other = TidewatchPaths::new(Path::new("/tmp/throwaway"));
        assert_ne!(paths().app_socket(), other.app_socket());
        assert!(!other.instance_dir().starts_with(paths().state_dir()));
    }
}
