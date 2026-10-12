//! On-disk layout: `~/.tidewatch` (per-repo worktree folders, state) and `~/.config/tidewatch` (settings).
//!
//! Pure path arithmetic. Nothing here touches the disk.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const HASH_HEX_LEN: usize = 6;

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

    /// Keyed by the full repo path, so two repos with the same folder name never share a directory.
    #[must_use]
    pub fn repo_dir(&self, repo: &Path) -> PathBuf {
        let name = repo.file_name().unwrap_or_default().to_string_lossy();
        let digest = Sha256::digest(repo.as_os_str().as_encoded_bytes());
        let hash = format!("{digest:x}");
        self.root.join(format!("{name}-{}", &hash[..HASH_HEX_LEN]))
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
    fn state_dir_lives_under_root() {
        let paths = paths();
        assert_eq!(paths.state_dir(), paths.root().join(".state"));
        assert!(paths.state_dir().starts_with(paths.root()));
    }
}
