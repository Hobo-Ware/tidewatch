//! Which tab group a taken-over session lands in (pure). A folder is known by the repo that holds
//! its git dir (`main`, so a linked worktree is its main repo) and by its `origin` remote. Folders
//! of one repo share a group by `main` alone; separate clones of one remote are gathered under the
//! open repo of that remote, else under the one of them that is not a `_TREE` folder, else the first.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ident {
    pub(crate) main: PathBuf,
    pub(crate) origin: Option<String>,
}

const TREE_MARK: &str = "_TREE";

/// A remote spelled the way every clone of it agrees on: `git@github.com:Acme/Web.git` and
/// `https://github.com/acme/web` are both `github.com/acme/web`.
pub(crate) fn normalize_origin(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest.to_owned(),
        None => match url.split_once(':') {
            Some((host, path)) if !host.contains('/') => format!("{host}/{path}"),
            _ => url.to_owned(),
        },
    };
    let (authority, path) = rest.split_once('/').unwrap_or((&rest, ""));
    let host = authority.rsplit('@').next().unwrap_or(authority);
    format!("{host}/{path}").to_lowercase()
}

/// `web_TREE` and `web_TREE_2`: the folders a repo's extra checkouts are named by.
fn is_tree_folder(dir: &Path) -> bool {
    let name = dir.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    name.rsplit_once(TREE_MARK).is_some_and(|(_, after)| {
        after.is_empty()
            || after.strip_prefix('_').is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The repo root each mover's tab group is for, `None` for a mover that is in no repo. `open` is
/// the repos the window holds, `movers` the folders of the sessions moved together.
pub(crate) fn group_roots(open: &[Ident], movers: &[Option<Ident>]) -> Vec<Option<PathBuf>> {
    movers.iter().map(|mover| mover.as_ref().map(|ident| root_for(ident, open, movers))).collect()
}

fn root_for(ident: &Ident, open: &[Ident], movers: &[Option<Ident>]) -> PathBuf {
    let Some(origin) = &ident.origin else { return ident.main.clone() };
    if open.iter().any(|held| held.main == ident.main) {
        return ident.main.clone();
    }
    if let Some(held) = open.iter().find(|held| held.origin.as_ref() == Some(origin)) {
        return held.main.clone();
    }
    let kin: Vec<&Ident> =
        movers.iter().flatten().filter(|other| other.origin.as_ref() == Some(origin)).collect();
    kin.iter()
        .find(|other| !is_tree_folder(&other.main))
        .or(kin.first())
        .map_or_else(|| ident.main.clone(), |other| other.main.clone())
}

#[cfg(test)]
mod tests;
