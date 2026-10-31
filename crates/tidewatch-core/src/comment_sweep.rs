//! The branch sweep of the comment guard: the lines a branch added, found in `git diff -U0`, and the
//! comments among them that fail the keep-list. It also reads files git does not know yet, so a file
//! written through a shell is swept too.
//!
//! Only added lines are judged, so a comment that was already in the repo is never flagged.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::comments::{Flag, Syntax, scan};
use crate::limited;

const MAX_FILE_BYTES: u64 = 256 * 1024;

/// The lines one file gained: `(line number in the new file, text)`, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Added {
    pub path: String,
    pub lines: Vec<(usize, String)>,
}

/// One file's flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFlags {
    pub path: String,
    pub flags: Vec<Flag>,
}

fn hunk_start(header: &str) -> Option<usize> {
    let plus = header.split_whitespace().find(|part| part.starts_with('+'))?;
    plus[1..].split(',').next()?.parse().ok()
}

/// The added lines of a unified diff (`-U0` or any context), per file with real line numbers. A
/// deleted file and a binary file add nothing.
#[must_use]
pub fn added_lines(diff: &str) -> Vec<Added> {
    let mut files: Vec<Added> = Vec::new();
    let mut next = 0usize;
    let mut in_hunk = false;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
        } else if !in_hunk && let Some(path) = line.strip_prefix("+++ ") {
            if path != "/dev/null" {
                files.push(Added {
                    path: path.strip_prefix("b/").unwrap_or(path).to_owned(),
                    lines: Vec::new(),
                });
            }
        } else if line.starts_with("@@") {
            in_hunk = true;
            next = hunk_start(line).unwrap_or(0);
        } else if in_hunk && let Some(text) = line.strip_prefix('+') {
            if let Some(file) = files.last_mut() {
                file.lines.push((next, text.to_owned()));
            }
            next += 1;
        } else if in_hunk && line.starts_with(' ') {
            next += 1;
        }
    }
    files
}

/// Flags in what a file gained, run by run so line numbers stay real and a comment is never judged
/// next to code it was not written beside.
#[must_use]
pub fn flags_in(added: &Added) -> Vec<Flag> {
    let Some(syntax) = Syntax::of_path(&added.path) else { return Vec::new() };
    let mut flags = Vec::new();
    let mut run: Vec<&(usize, String)> = Vec::new();
    let flush = |run: &mut Vec<&(usize, String)>, flags: &mut Vec<Flag>| {
        if let Some(first) = run.first() {
            let text: Vec<&str> = run.iter().map(|(_, line)| line.as_str()).collect();
            flags.extend(scan(&text.join("\n"), syntax, first.0));
        }
        run.clear();
    };
    for entry in &added.lines {
        if run.last().is_some_and(|last| last.0 + 1 != entry.0) {
            flush(&mut run, &mut flags);
        }
        run.push(entry);
    }
    flush(&mut run, &mut flags);
    flags
}

/// Flags per file for a diff text.
#[must_use]
pub fn flags_of_diff(diff: &str) -> Vec<FileFlags> {
    added_lines(diff)
        .iter()
        .filter_map(|added| {
            let flags = flags_in(added);
            (!flags.is_empty()).then(|| FileFlags { path: added.path.clone(), flags })
        })
        .collect()
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = limited::output(
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stderr(Stdio::null()),
        limited::DEFAULT,
    )
    .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Where the branch forked: the remote default branch beats the upstream, because once a branch is
/// pushed `@{u}...HEAD` is empty and the sweep would go blind.
#[must_use]
pub fn base(dir: &Path) -> Option<String> {
    let head = git(dir, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).unwrap_or_default();
    [head.trim(), "origin/main", "origin/master", "main", "master"]
        .into_iter()
        .filter(|candidate| !candidate.is_empty() && *candidate != "origin/HEAD")
        .find(|candidate| git(dir, &["rev-parse", "--verify", "--quiet", candidate]).is_some())
        .map(str::to_owned)
}

/// The comments the branch in `dir` added since it forked, committed or not, and in new files git has
/// not seen. `None` when there is nothing to compare with.
#[must_use]
pub fn sweep(dir: &Path) -> Option<Vec<FileFlags>> {
    let base = base(dir)?;
    let committed = git(dir, &["diff", "-U0", &format!("{base}...HEAD"), "--"])?;
    let working = git(dir, &["diff", "-U0", "HEAD", "--"]).unwrap_or_default();
    let mut added = added_lines(&committed);
    for file in added_lines(&working) {
        match added.iter_mut().find(|known| known.path == file.path) {
            Some(known) => {
                known.lines.retain(|(line, _)| !file.lines.iter().any(|(l, _)| l == line));
                known.lines.extend(file.lines);
                known.lines.sort_by_key(|(line, _)| *line);
            }
            None => added.push(file),
        }
    }
    for path in git(dir, &["ls-files", "--others", "--exclude-standard"])?.lines() {
        let full = dir.join(path);
        let small =
            full.metadata().is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_FILE_BYTES);
        if let (true, Ok(text)) = (small, std::fs::read_to_string(&full)) {
            let lines =
                text.lines().enumerate().map(|(at, line)| (at + 1, line.to_owned())).collect();
            added.push(Added { path: path.to_owned(), lines });
        }
    }
    Some(
        added
            .iter()
            .filter_map(|file| {
                let flags = flags_in(file);
                (!flags.is_empty()).then(|| FileFlags { path: file.path.clone(), flags })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests;
