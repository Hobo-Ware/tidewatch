//! The git repository a window is opened for: root path and the folder name used as its label.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Repo {
    pub(crate) root: PathBuf,
    pub(crate) label: String,
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
    fn discovers_this_repo_from_a_subfolder() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let repo = Repo::discover(&here).expect("tests run inside a git checkout");
        assert!(here.starts_with(&repo.root));
    }
}
