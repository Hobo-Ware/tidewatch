//! What a window is opened for: a git repository (worktree tabs) or, when the folder is not one,
//! the "Other" group (plain shell tabs in that folder).

use std::path::{Path, PathBuf};
use std::process::Command;

use tidewatch_core::limited;

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

    /// Like [`Workspace::open`], but a linked worktree opens as the repo it belongs to.
    pub(crate) fn open_in_main_repo(path: &Path) -> Self {
        Repo::discover_main(path).map_or_else(|| Self::open(path), Self::Repo)
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
        let out = limited::output(
            Command::new("git").arg("-C").arg(dir).args(["rev-parse", "--show-toplevel"]),
            limited::DEFAULT,
        )
        .ok()?;
        if !out.status.success() {
            return None;
        }
        Self::from_toplevel(&String::from_utf8_lossy(&out.stdout))
    }

    /// The repo a folder belongs to, where a linked worktree counts as its main repo.
    pub(crate) fn discover_main(path: &Path) -> Option<Self> {
        let dir = if path.is_dir() { path } else { path.parent()? };
        let out = limited::output(
            Command::new("git").arg("-C").arg(dir).args([
                "rev-parse",
                "--path-format=absolute",
                "--show-toplevel",
                "--git-common-dir",
            ]),
            limited::DEFAULT,
        )
        .ok()?;
        if !out.status.success() {
            return None;
        }
        Self::from_main_output(&String::from_utf8_lossy(&out.stdout))
    }

    /// The `origin` remote of the repo, as git spells it; none for a repo without one.
    pub(crate) fn origin(&self) -> Option<String> {
        let out = limited::output(
            Command::new("git").arg("-C").arg(&self.root).args([
                "config",
                "--get",
                "remote.origin.url",
            ]),
            limited::DEFAULT,
        )
        .ok()?;
        let url = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (out.status.success() && !url.is_empty()).then_some(url)
    }

    fn from_main_output(output: &str) -> Option<Self> {
        let mut lines = output.lines();
        let toplevel = lines.next()?;
        let common = PathBuf::from(lines.next()?);
        let root = match common.file_name() {
            Some(name) if name == ".git" => common.parent()?.to_string_lossy().into_owned(),
            _ => toplevel.to_owned(),
        };
        Self::from_toplevel(&root)
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
    fn a_linked_worktree_belongs_to_the_repo_that_holds_its_git_dir() {
        let cases = [
            ("a repo root or subfolder", "/w/web\n/w/web/.git\n", "/w/web"),
            ("a linked worktree", "/w/web-fix\n/w/web/.git\n", "/w/web"),
            ("a submodule", "/w/web/lib\n/w/web/.git/modules/lib\n", "/w/web/lib"),
            ("a repo with its git dir elsewhere", "/w/web\n/g/web.git\n", "/w/web"),
        ];
        for (name, output, want) in cases {
            let repo = Repo::from_main_output(output).unwrap();
            assert_eq!(repo.root, PathBuf::from(want), "{name}");
        }
        assert_eq!(Repo::from_main_output("\n"), None);
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
    fn a_linked_worktree_and_a_subfolder_open_as_the_main_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("web");
        let linked = dir.path().join("web-fix");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        git(&["worktree", "add", "-q", linked.to_str().unwrap(), "-b", "fix"]);
        std::fs::create_dir(linked.join("lib")).unwrap();
        let main = std::fs::canonicalize(&root).unwrap();
        for opened in [&root, &root.join("src"), &linked, &linked.join("lib")] {
            let workspace = Workspace::open_in_main_repo(opened);
            assert_eq!(std::fs::canonicalize(workspace.dir()).unwrap(), main, "{opened:?}");
            assert_eq!(workspace.chip_label(), "web");
        }
        assert_eq!(Workspace::open(&linked).chip_label(), "web-fix");
    }

    #[cfg(unix)]
    #[test]
    fn a_real_repo_is_never_other_however_the_folder_is_reached() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("web");
        let apart = dir.path().join("elsewhere.git");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let init = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q", "--separate-git-dir"])
            .arg(&apart)
            .status()
            .unwrap();
        assert!(init.success());
        let link = dir.path().join("web-link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let cases = [
            ("the root, with its git dir elsewhere and no commit", root.clone()),
            ("a subfolder", root.join("src")),
            ("a symlink to it", link.clone()),
            ("a subfolder through the symlink", link.join("src")),
        ];
        for (name, opened) in cases {
            assert!(matches!(Workspace::open(&opened), Workspace::Repo(_)), "{name}");
            assert!(matches!(Workspace::open_in_main_repo(&opened), Workspace::Repo(_)), "{name}");
        }
    }

    #[test]
    fn discovers_this_repo_from_a_subfolder() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let repo = Repo::discover(&here).expect("tests run inside a git checkout");
        assert!(here.starts_with(&repo.root));
    }
}
