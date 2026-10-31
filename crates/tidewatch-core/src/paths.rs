//! On-disk layout: `~/.tidewatch` (per-repo worktree folders, state) and `~/.config/tidewatch` (settings).
//!
//! Pure path arithmetic. Nothing here touches the disk.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const HASH_HEX_LEN: usize = 6;
const NOTES_FILE: &str = "notes.json";
const PR_CARDS_FILE: &str = "pr_cards.json";
const DUE_ASKED_FILE: &str = "due-asked.json";
const PROGRESS_FILE: &str = "progress.md";
const HOUSE_STYLE_FILE: &str = "house-style.md";
const JOBS_FILE: &str = "jobs.json";
const LINKS_FILE: &str = "links.json";
const AVATAR_FILE: &str = "avatar.json";
const PROGRESS_FEED_FILE: &str = "progress-feed.jsonl";
const SENT_FILE: &str = "tidewatch-sent.jsonl";
const PANEL_LOG_FILE: &str = "panel-log.jsonl";
const REPLY_HISTORY_FILE: &str = "reply-history.json";
const REPLY_DRAFT_FILE: &str = "reply-draft.json";

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
pub fn links_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(LINKS_FILE)
}

#[must_use]
pub fn avatar_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(AVATAR_FILE)
}

#[must_use]
pub fn progress_feed_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(PROGRESS_FEED_FILE)
}

#[must_use]
pub fn sent_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(SENT_FILE)
}

/// The messages the owner sent a tab, which sit next to its `notes.json`.
#[must_use]
pub fn reply_history_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(REPLY_HISTORY_FILE)
}

/// The half-written message of a tab's reply box, which sits next to its `notes.json`.
#[must_use]
pub fn reply_draft_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(REPLY_DRAFT_FILE)
}

/// The panel's event log, which sits next to a tab's `notes.json`.
#[must_use]
pub fn panel_log_file_beside(notes_file: &Path) -> PathBuf {
    notes_file.with_file_name(PANEL_LOG_FILE)
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
    claude_dir: PathBuf,
}

impl TidewatchPaths {
    #[must_use]
    pub fn new(home: &Path) -> Self {
        Self {
            root: home.join(".tidewatch"),
            config_dir: home.join(".config").join("tidewatch"),
            claude_dir: home.join(".claude"),
        }
    }

    /// Claude Code's config folder when it is not `~/.claude` (`CLAUDE_CONFIG_DIR`).
    #[must_use]
    pub fn with_claude_dir(mut self, dir: PathBuf) -> Self {
        self.claude_dir = dir;
        self
    }

    /// Where Claude Code keeps its projects and their memory. Tidewatch only reads the name.
    #[must_use]
    pub fn claude_dir(&self) -> &Path {
        &self.claude_dir
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

    /// The files whose seeded efforts were already moved to the per-model defaults, one per line.
    #[must_use]
    pub fn effort_migrated_file(&self) -> PathBuf {
        self.config_dir.join("effort-defaults.migrated")
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

    /// Whether settings sync is on, its repo and the last sync. Never synced: it describes this machine.
    #[must_use]
    pub fn sync_file(&self) -> PathBuf {
        self.config_dir.join("sync.toml")
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

    /// The per-repo worktree setup recipes (files to copy, folders to link, a shared build folder,
    /// a generator command).
    #[must_use]
    pub fn worktree_setup_file(&self) -> PathBuf {
        self.config_dir.join("worktree-setup.toml")
    }

    /// The per-repo services: long-running processes (a database proxy, a dev server, a tunnel)
    /// that run next to the agents.
    #[must_use]
    pub fn services_file(&self) -> PathBuf {
        self.config_dir.join("services.toml")
    }

    /// The per-repo workflow profile: how branches catch up, where fixes go, how pull requests open.
    #[must_use]
    pub fn workflow_file(&self) -> PathBuf {
        self.config_dir.join("workflow.toml")
    }

    /// The kinds of Auto message each repo has shown its owner once: `auto-seen.toml`, bookkeeping
    /// of this machine, so it sits with the state and never travels with the settings.
    #[must_use]
    pub fn auto_seen_file(&self) -> PathBuf {
        self.state_dir().join("auto-seen.toml")
    }

    /// Holds the `git-remote-tidewatch-no-push` helper that agent tabs get on their `PATH`.
    #[must_use]
    pub fn guard_bin_dir(&self) -> PathBuf {
        self.state_dir().join("guard-bin")
    }

    /// Where the guard shims remember whether a repo is public.
    #[must_use]
    pub fn guard_cache_dir(&self) -> PathBuf {
        self.state_dir().join("guard-cache")
    }

    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join(".state")
    }

    /// Where a new-tab draft stages the files its message box takes, before any tab or worktree
    /// exists: `.state/drafts/<key>`. Removed when the draft closes without Start.
    #[must_use]
    pub fn draft_dir(&self, key: u64) -> PathBuf {
        self.state_dir().join("drafts").join(format!("{key:016x}"))
    }

    #[must_use]
    pub fn me_dir(&self) -> PathBuf {
        self.state_dir().join("me")
    }

    #[must_use]
    pub fn me_avatar_file(&self) -> PathBuf {
        self.me_dir().join("avatar.jpg")
    }

    #[must_use]
    pub fn me_avatar_stamp_file(&self) -> PathBuf {
        self.me_dir().join("avatar.stamp")
    }

    /// Every worktree Tidewatch removed, one JSON line each, appended before the removal.
    #[must_use]
    pub fn removals_log(&self) -> PathBuf {
        self.state_dir().join("removals.log")
    }

    /// The id of the webhook each repo's last forwarder made, `{"owner/name": id}`: only a hook
    /// recorded here is ever deleted as stale.
    #[must_use]
    pub fn github_hooks_file(&self) -> PathBuf {
        self.state_dir().join("github-hooks.json")
    }

    /// The agent tabs Tidewatch closed, one JSON line each, the newest 100 kept.
    #[must_use]
    pub fn closed_tabs_log(&self) -> PathBuf {
        self.state_dir().join("closed-tabs.jsonl")
    }

    /// Whose Amphetamine session may have left lid-closed sleep off: written before the admin prompt.
    #[must_use]
    pub fn amphetamine_marker_file(&self) -> PathBuf {
        self.state_dir().join("amphetamine.json")
    }

    /// The deadline the root helper reads; removing it tells the helper to restore sleep.
    #[must_use]
    pub fn amphetamine_token_file(&self) -> PathBuf {
        self.state_dir().join("amphetamine.token")
    }

    #[must_use]
    pub fn amphetamine_serve_file(&self) -> PathBuf {
        self.state_dir().join("amphetamine.serve")
    }

    /// This week's tally of panel changes on time, late and missed, per source.
    #[must_use]
    pub fn panel_ontime_file(&self) -> PathBuf {
        self.state_dir().join("panel-ontime.json")
    }

    /// The scheduled items of every tab's ledger, for the Coming up list across tabs.
    #[must_use]
    pub fn schedule_file(&self) -> PathBuf {
        self.state_dir().join("schedule.toml")
    }

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

    /// The due items a tab that runs on its own was already asked to start today.
    #[must_use]
    pub fn due_asked_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(DUE_ASKED_FILE)
    }

    /// A tab's progress ledger, as the agent last sent it.
    #[must_use]
    pub fn progress_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(PROGRESS_FILE)
    }

    /// The tab's panel event log: every fact the panel shows, numbered, replayed after a restart.
    #[must_use]
    pub fn panel_log_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(PANEL_LOG_FILE)
    }

    #[must_use]
    pub fn progress_feed_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(PROGRESS_FEED_FILE)
    }

    /// What one agent is given as its house style: the house-style file and the launch text with
    /// the repo's workflow and checklist filled in.
    #[must_use]
    pub fn tab_house_style_file(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join(HOUSE_STYLE_FILE)
    }

    /// The copy of [`Self::tab_house_style_file`] in the shape Copilot reads (`*.instructions.md`).
    #[must_use]
    pub fn tab_copilot_instructions_dir(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join("copilot")
    }

    /// The plugin that carries a Copilot tab's status hooks (it holds the tab's hook token).
    #[must_use]
    pub fn tab_copilot_plugin_dir(&self, repo: &Path, tab: &str) -> PathBuf {
        self.tab_state_dir(repo, tab).join("copilot-plugin")
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
        assert!(
            paths.tab_copilot_plugin_dir(repo, "brave-otter").starts_with(notes.parent().unwrap())
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
        assert!(links_file_beside(&notes).ends_with("brave-otter/links.json"));
        assert_eq!(links_file_beside(&progress_file_beside(&notes)), links_file_beside(&notes));
        assert!(avatar_file_beside(&notes).ends_with("brave-otter/avatar.json"));
        assert_ne!(feed, paths.progress_feed_file(repo, "main"));
        assert_eq!(sent_file_beside(&notes).parent(), notes.parent());
        assert!(reply_history_file_beside(&notes).ends_with("brave-otter/reply-history.json"));
        assert!(reply_draft_file_beside(&notes).ends_with("brave-otter/reply-draft.json"));
        assert!(sent_file_beside(&notes).ends_with("brave-otter/tidewatch-sent.jsonl"));
    }

    #[test]
    fn the_panel_log_sits_with_the_notes_of_its_tab() {
        let paths = paths();
        let repo = Path::new("/work/web");
        let notes = paths.notes_file(repo, "brave-otter");
        let log = paths.panel_log_file(repo, "brave-otter");
        assert_eq!(panel_log_file_beside(&notes), log);
        assert_eq!(log.parent(), notes.parent());
        assert!(log.ends_with("brave-otter/panel-log.jsonl"));
        assert_ne!(log, paths.panel_log_file(repo, "main"));
    }

    #[test]
    fn the_panel_tally_is_one_state_file_for_the_whole_window() {
        assert_eq!(paths().panel_ontime_file(), paths().state_dir().join("panel-ontime.json"));
    }

    #[test]
    fn the_checklist_template_is_a_setting_and_the_schedule_index_is_state() {
        let paths = paths();
        assert_eq!(paths.checklist_file(), paths.config_dir().join("checklist.md"));
        assert_eq!(paths.schedule_file(), paths.state_dir().join("schedule.toml"));
    }

    #[test]
    fn the_worktree_setup_recipes_are_a_setting_not_a_file_in_any_repo() {
        let paths = paths();
        assert_eq!(paths.worktree_setup_file(), paths.config_dir().join("worktree-setup.toml"));
    }

    #[test]
    fn the_repo_services_are_a_setting_not_a_file_in_any_repo() {
        let paths = paths();
        assert_eq!(paths.services_file(), paths.config_dir().join("services.toml"));
    }

    #[test]
    fn the_seen_auto_messages_are_state_of_this_machine() {
        let paths = paths();
        assert_eq!(paths.auto_seen_file(), paths.state_dir().join("auto-seen.toml"));
    }

    #[test]
    fn the_workflow_profiles_are_a_setting_not_a_file_in_any_repo() {
        let paths = paths();
        assert_eq!(paths.workflow_file(), paths.config_dir().join("workflow.toml"));
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
    fn the_owners_face_is_state_under_a_me_folder() {
        let paths = paths();
        assert_eq!(paths.me_dir(), paths.state_dir().join("me"));
        assert_eq!(paths.me_avatar_file(), paths.state_dir().join("me/avatar.jpg"));
        assert_eq!(paths.me_avatar_stamp_file(), paths.state_dir().join("me/avatar.stamp"));
        assert_eq!(paths.me_avatar_file().parent(), paths.me_avatar_stamp_file().parent());
    }

    #[test]
    fn a_draft_stages_its_files_in_a_folder_of_its_own_under_the_state_dir() {
        let paths = paths();
        let (one, other) = (paths.draft_dir(1), paths.draft_dir(2));
        assert_eq!(one, paths.state_dir().join("drafts/0000000000000001"));
        assert_ne!(one, other);
        assert_eq!(one.parent(), other.parent());
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
