//! Giving a Claude session to another folder: `claude --resume <id>` only finds sessions in the
//! project folder of the directory it starts in, so the conversation is copied (never moved) from
//! its project folder to the new folder's, with no overwrite and each file compared with its source.
//! The old folder and its files are never touched.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use tidewatch_core::claude_memory::project_dir_name;
use tidewatch_core::transcript_locate::claude_project_dir_name;

fn projects(claude_dir: &Path) -> PathBuf {
    claude_dir.join("projects")
}

fn forms(cwd: &Path) -> Vec<PathBuf> {
    let mut forms = vec![cwd.to_path_buf()];
    if let Ok(real) = fs::canonicalize(cwd)
        && real != cwd
    {
        forms.push(real);
    }
    forms
}

/// The session file of `id`: in the project folder of `cwd` when it is there, else in any project
/// folder.
pub(crate) fn find_session(claude_dir: &Path, cwd: &Path, id: &str) -> Option<PathBuf> {
    let file = format!("{id}.jsonl");
    let root = projects(claude_dir);
    let direct = forms(cwd)
        .into_iter()
        .map(|cwd| root.join(claude_project_dir_name(&cwd)).join(&file))
        .find(|path| path.is_file());
    direct.or_else(|| {
        let mut folders: Vec<PathBuf> =
            fs::read_dir(&root).ok()?.filter_map(Result::ok).map(|entry| entry.path()).collect();
        folders.sort();
        folders.into_iter().map(|dir| dir.join(&file)).find(|path| path.is_file())
    })
}

fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    let (mut a, mut b) = (File::open(a)?, File::open(b)?);
    let (mut left, mut right) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let (n, m) = (fill(&mut a, &mut left)?, fill(&mut b, &mut right)?);
        if n != m || left[..n] != right[..m] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
}

fn fill(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match file.read(&mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

fn copy_file(from: &Path, to: &Path) -> io::Result<()> {
    match OpenOptions::new().write(true).create_new(true).open(to) {
        Ok(mut target) => {
            io::copy(&mut File::open(from)?, &mut target)?;
            drop(target);
            if same_bytes(from, to)? {
                return Ok(());
            }
            let _ = fs::remove_file(to);
            Err(io::Error::other(format!("{} differs from its source", to.display())))
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && same_bytes(from, to)? => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            Err(io::Error::other(format!("{} already holds a different file", to.display())))
        }
        Err(e) => Err(e),
    }
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copies the session file `source` (and the folder of the same name beside it) into the project
/// folder Claude Code uses for `to_cwd`. An error names what went wrong; what was copied before it
/// stays, and nothing at the source changes.
pub(crate) fn copy_session(claude_dir: &Path, source: &Path, to_cwd: &Path) -> Result<(), String> {
    let name = |path: &Path| path.file_name().map(std::ffi::OsStr::to_owned);
    let (Some(file_name), Some(stem)) = (name(source), source.file_stem().map(ToOwned::to_owned))
    else {
        return Err("the session file has no name".to_owned());
    };
    let real = fs::canonicalize(to_cwd).unwrap_or_else(|_| to_cwd.to_path_buf());
    let folder = project_dir_name(&real).ok_or_else(|| {
        "the folder's name is too long for Claude Code's project folder".to_owned()
    })?;
    let target = projects(claude_dir).join(folder);
    fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    copy_file(source, &target.join(&file_name)).map_err(|e| e.to_string())?;
    let sidecar = source.with_file_name(&stem);
    if sidecar.is_dir() {
        copy_tree(&sidecar, &target.join(&stem)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
