//! Which repo a session that runs in a plain folder belongs to, for Change this tab: the repo its
//! tool calls touched most (the file paths of Edit, Write and Read, and the folders of Bash `cd` and
//! `git -C`, newest weighted first), else the repo with the newest commit among the folders one
//! level down. The transcript parsing and the weighing are pure; the folder reads are not.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::repo::Repo;
use crate::retarget_copy::find_session;

const DECAY: f64 = 0.97;
const MAX_TOUCHES: usize = 400;
const MAX_TAIL_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FOLDERS: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    Wrote,
    Ran,
    Read,
}

impl Act {
    fn weight(self) -> f64 {
        match self {
            Self::Wrote => 3.0,
            Self::Ran => 1.5,
            Self::Read => 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Touch {
    pub(crate) path: PathBuf,
    pub(crate) act: Act,
}

fn absolute(path: &str, base: &Path) -> Option<PathBuf> {
    if path.is_empty() || path == "-" || path.starts_with('~') || path.starts_with('$') {
        return None;
    }
    let path = Path::new(path);
    Some(if path.is_absolute() { path.to_path_buf() } else { base.join(path) })
}

fn shell_paths(command: &str) -> Vec<String> {
    let spaced = command.replace("&&", " ; ").replace("||", " ; ").replace(['\n', '|', ';'], " ; ");
    let words = shell_words::split(&spaced)
        .unwrap_or_else(|_| spaced.split_whitespace().map(str::to_owned).collect());
    words
        .iter()
        .enumerate()
        .filter(|(_, word)| matches!(word.as_str(), "cd" | "-C"))
        .filter_map(|(at, _)| words.get(at + 1))
        .filter(|next| !next.starts_with('-') && next.as_str() != ";")
        .cloned()
        .collect()
}

/// The paths the tool calls of a Claude transcript touched, oldest first. `cwd` is where a
/// relative path is read from when a record names no folder of its own.
pub(crate) fn touches(text: &str, cwd: &Path) -> Vec<Touch> {
    let mut found = Vec::new();
    for line in text.lines().filter(|line| line.contains("\"tool_use\"")) {
        let Ok(record) = serde_json::from_str::<Value>(line) else { continue };
        if record["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        let base = record["cwd"].as_str().map_or_else(|| cwd.to_path_buf(), PathBuf::from);
        let Some(blocks) = record["message"]["content"].as_array() else { continue };
        for block in blocks.iter().filter(|block| block["type"] == "tool_use") {
            let input = &block["input"];
            let named = |key: &str| input[key].as_str();
            let (act, paths): (Act, Vec<String>) = match block["name"].as_str() {
                Some("Edit" | "Write" | "MultiEdit") => {
                    (Act::Wrote, named("file_path").map(str::to_owned).into_iter().collect())
                }
                Some("NotebookEdit") => {
                    (Act::Wrote, named("notebook_path").map(str::to_owned).into_iter().collect())
                }
                Some("Read") => {
                    (Act::Read, named("file_path").map(str::to_owned).into_iter().collect())
                }
                Some("Glob" | "Grep") => {
                    (Act::Read, named("path").map(str::to_owned).into_iter().collect())
                }
                Some("Bash") => (Act::Ran, named("command").map(shell_paths).unwrap_or_default()),
                _ => continue,
            };
            found.extend(
                paths
                    .iter()
                    .filter_map(|path| absolute(path, &base))
                    .map(|path| Touch { path, act }),
            );
        }
    }
    let skip = found.len().saturating_sub(MAX_TOUCHES);
    found.drain(..skip);
    found
}

/// The repo with the most weight: each touch counts for its kind, less the further back it is. A
/// tie goes to the repo touched most recently.
pub(crate) fn most_worked(
    touched: &[Touch],
    repo_of: &mut dyn FnMut(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let mut scores: Vec<(PathBuf, f64)> = Vec::new();
    for (age, touch) in touched.iter().rev().enumerate() {
        let Some(repo) = repo_of(&touch.path) else { continue };
        let weight = DECAY.powi(i32::try_from(age).unwrap_or(i32::MAX)) * touch.act.weight();
        match scores.iter_mut().find(|(known, _)| *known == repo) {
            Some((_, score)) => *score += weight,
            None => scores.push((repo, weight)),
        }
    }
    scores
        .into_iter()
        .reduce(|best, next| if next.1 > best.1 { next } else { best })
        .map(|(repo, _)| repo)
}

/// The repo with the newest time; equal times go to the first path in order.
pub(crate) fn newest(repos: &[(PathBuf, u64)]) -> Option<PathBuf> {
    let mut sorted: Vec<&(PathBuf, u64)> = repos.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
        .into_iter()
        .reduce(|best, next| if next.1 > best.1 { next } else { best })
        .map(|(path, _)| path.clone())
}

fn repo_of_path(path: &Path) -> Option<PathBuf> {
    let mut dir = if path.is_dir() { Some(path) } else { path.parent() };
    while let Some(candidate) = dir {
        if candidate.is_dir() {
            return Repo::discover_main(candidate).map(|repo| repo.root);
        }
        dir = candidate.parent();
    }
    None
}

fn read_tail(file: &Path) -> Option<String> {
    let mut file = File::open(file).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return Some(text);
    }
    text.split_once('\n').map(|(_, rest)| rest.to_owned())
}

fn commit_time(repo: &Path) -> Option<u64> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--format=%ct"])
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn mtime(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified.duration_since(std::time::UNIX_EPOCH).ok().map(|time| time.as_secs())
}

/// The repos one level below `dir` with the time of their newest commit, else of their `.git`.
pub(crate) fn repos_below(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(".git").exists())
        .take(MAX_FOLDERS)
        .filter_map(|path| {
            let root = Repo::discover_main(&path)?.root;
            let time = commit_time(&root).or_else(|| mtime(&path.join(".git"))).unwrap_or(0);
            Some((root, time))
        })
        .collect()
}

/// The repo a session in the plain folder `cwd` worked in: from its transcript when it has one,
/// else the newest repo below the folder.
pub(crate) fn worked_in(
    claude_dir: &Path,
    cwd: &Path,
    session_id: Option<&str>,
) -> Option<PathBuf> {
    let from_transcript = session_id
        .and_then(|id| find_session(claude_dir, cwd, id))
        .and_then(|file| read_tail(&file))
        .and_then(|text| {
            let mut known: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
            let mut repo_of = |path: &Path| {
                let key =
                    if path.is_dir() { path } else { path.parent().unwrap_or(path) }.to_path_buf();
                known.entry(key).or_insert_with(|| repo_of_path(path)).clone()
            };
            most_worked(&touches(&text, cwd), &mut repo_of)
        });
    from_transcript.or_else(|| newest(&repos_below(cwd)))
}

#[cfg(test)]
mod tests;
