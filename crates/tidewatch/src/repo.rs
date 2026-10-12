//! What a window is opened for: a git repository (worktree tabs) or, when the folder is not one,
//! the "Other" group (plain shell tabs in that folder).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Repo {
    pub(crate) root: PathBuf,
    pub(crate) label: String,
}

pub(crate) enum Workspace {
    Repo(Repo),
    Other { dir: PathBuf, label: String },
}

impl Workspace {
    pub(crate) fn open(path: &Path) -> Self {
        Repo::discover(path).map_or_else(
            || {
                let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
                Self::Other { dir: dir.to_path_buf(), label: folder_label(dir) }
            },
            Self::Repo,
        )
    }

    pub(crate) fn dir(&self) -> &Path {
        match self {
            Self::Repo(repo) => &repo.root,
            Self::Other { dir, .. } => dir,
        }
    }

    pub(crate) fn chip_label(&self) -> String {
        match self {
            Self::Repo(repo) => repo.label.clone(),
            Self::Other { label, .. } => format!("Other: {label}"),
        }
    }
}

fn folder_label(dir: &Path) -> String {
    dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned())
}

impl Repo {
    pub(crate) fn discover(path: &Path) -> Option<Self> {
        let dir = if path.is_dir() { path } else { path.parent()? };
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Self::from_toplevel(&String::from_utf8_lossy(&out.stdout))
    }

    fn from_toplevel(output: &str) -> Option<Self> {
        let root = PathBuf::from(output.trim());
        let label = root.file_name()?.to_string_lossy().into_owned();
        Some(Self { root, label })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_the_root_folder_name() {
        let repo = Repo::from_toplevel("/Users/me/Git/kelp\n").unwrap();
        assert_eq!(repo.label, "kelp");
        assert_eq!(repo.root, PathBuf::from("/Users/me/Git/kelp"));
    }

    #[test]
    fn empty_output_is_not_a_repo() {
        assert_eq!(Repo::from_toplevel("\n"), None);
    }

    #[test]
    fn a_folder_outside_any_repo_opens_in_the_other_group() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(dir.path());
        assert!(matches!(workspace, Workspace::Other { .. }));
        assert_eq!(workspace.dir(), dir.path());
        let name = dir.path().file_name().unwrap().to_string_lossy();
        assert_eq!(workspace.chip_label(), format!("Other: {name}"));
    }

    #[test]
    fn a_file_outside_any_repo_uses_its_parent_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(Workspace::open(&file).dir(), dir.path());
    }

    #[test]
    fn a_git_folder_opens_as_a_repo_with_its_name_as_the_chip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("web");
        std::fs::create_dir(&root).unwrap();
        let init = Command::new("git").arg("-C").arg(&root).args(["init", "-q"]).status().unwrap();
        assert!(init.success());
        let workspace = Workspace::open(&root);
        assert!(matches!(workspace, Workspace::Repo(_)));
        assert_eq!(workspace.chip_label(), "web");
    }

    #[test]
    fn discovers_this_repo_from_a_subfolder() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let repo = Repo::discover(&here).expect("tests run inside a git checkout");
        assert!(here.starts_with(&repo.root));
    }
}
