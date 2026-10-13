//! On-disk layout: `~/.tidewatch` (per-repo worktree folders, state) and `~/.config/tidewatch` (settings).
//!
//! Pure path arithmetic. Nothing here touches the disk.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const HASH_HEX_LEN: usize = 6;
const SOCKET_HEX_LEN: usize = 12;
const NOTES_FILE: &str = "notes.json";
const STAGES_FILE: &str = "stages.json";

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
        self.config_dir.join("house-style.md")
    }

    /// Derived copy of the house style in the shape Copilot reads (`*.instructions.md`).
    #[must_use]
    pub fn copilot_instructions_dir(&self) -> PathBuf {
        self.config_dir.join("copilot")
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

    /// The tabs open at the last quit, restored at the next launch.
    #[must_use]
    pub fn session_file(&self) -> PathBuf {
        self.state_dir().join("session.toml")
    }

    /// One Unix socket per running window lives here (`instance`). Under the state dir, so a
    /// `TIDEWATCH_HOME` run never sees the real one.
    #[must_use]
    pub fn instance_dir(&self) -> PathBuf {
        self.state_dir().join("instances")
    }

    /// The socket of the window for `dir` (the repo root, or the folder in the Other group). Named
    /// by a short hash only: a Unix socket path must stay under about 100 bytes.
    #[must_use]
    pub fn instance_socket(&self, dir: &Path) -> PathBuf {
        let digest = Sha256::digest(dir.as_os_str().as_encoded_bytes());
        let hash = format!("{digest:x}");
        self.instance_dir().join(format!("{}.sock", &hash[..SOCKET_HEX_LEN]))
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
        assert_eq!(paths.copilot_instructions_dir(), config.join("copilot"));
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
    fn session_file_sits_in_the_state_dir_next_to_the_worktree_registry() {
        let paths = paths();
        assert_eq!(paths.session_file(), paths.state_dir().join("session.toml"));
    }

    #[test]
    fn state_dir_lives_under_root() {
        let paths = paths();
        assert_eq!(paths.state_dir(), paths.root().join(".state"));
        assert!(paths.state_dir().starts_with(paths.root()));
    }

    #[test]
    fn each_folder_has_its_own_short_socket_under_the_state_dir() {
        let paths = paths();
        let web = paths.instance_socket(Path::new("/work/web"));
        assert_eq!(web.parent(), Some(paths.instance_dir().as_path()));
        assert!(paths.instance_dir().starts_with(paths.state_dir()));
        assert_eq!(web, paths.instance_socket(Path::new("/work/web")));
        assert_ne!(web, paths.instance_socket(Path::new("/personal/web")));
        assert!(web.extension().is_some_and(|ext| ext == "sock"));
        let long = paths.instance_socket(Path::new(&"/very/long/folder/name/".repeat(20)));
        assert_eq!(long.file_name().unwrap().len(), web.file_name().unwrap().len());
    }

    #[test]
    fn another_home_has_other_sockets() {
        let other = TidewatchPaths::new(Path::new("/tmp/throwaway"));
        let dir = Path::new("/work/web");
        assert_ne!(paths().instance_socket(dir), other.instance_socket(dir));
        assert!(!other.instance_dir().starts_with(paths().state_dir()));
    }
}
