//! Where each CLI keeps the session file of a tab (PLAN.md, D12), found from what Tidewatch knows:
//! the worktree folder and the session id it passed with `--session-id`.
//!
//! Checked against claude 2.1.296, gemini 0.46.0, copilot 1.0.91 and codex 0.162.1 on 2026-10-13.
//! Tidewatch cannot pass Codex a session id, so its rollout is found by folder and start time (a
//! hook's `transcript_path` wins when there is one; an id is matched against the file name). A
//! Codex TUI writes its rollout at start, before any prompt, so a stale one in the same folder is
//! told apart by its start time. The lookups read directory listings and first lines: call them off
//! the UI thread. A file may not exist yet when the agent has just started; ask again later.

use std::fs;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::rfc3339;
use crate::transcript::Source;

const CODEX_DAYS_SCANNED: usize = 3;
const CODEX_FILES_READ_PER_DAY: usize = 200;

/// What is known about a tab, and where the CLIs keep their files.
#[derive(Clone, Debug)]
pub struct Locator {
    pub home: PathBuf,
    /// The agent's working directory: the worktree folder.
    pub cwd: PathBuf,
    /// The id given to `--session-id`. Claude, Gemini and Copilot need it; Codex does not take one.
    pub session_id: Option<String>,
    /// A path the CLI reported itself (a hook payload's `transcript_path`). Used when it exists.
    pub hint: Option<PathBuf>,
    /// `COPILOT_HOME`, when set.
    pub copilot_home: Option<PathBuf>,
    /// `CODEX_HOME`, when set.
    pub codex_home: Option<PathBuf>,
    /// Codex only: ignore rollouts that started before this (unix milliseconds).
    pub started_after_ms: Option<i64>,
}

impl Locator {
    #[must_use]
    pub fn new(home: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            cwd: cwd.into(),
            session_id: None,
            hint: None,
            copilot_home: None,
            codex_home: None,
            started_after_ms: None,
        }
    }

    /// The session file of `source`, if it exists yet.
    #[must_use]
    pub fn locate(&self, source: Source) -> Option<PathBuf> {
        if let Some(hint) = self.hint.as_ref().filter(|p| p.is_file()) {
            return Some(hint.clone());
        }
        match source {
            Source::Claude => self.claude(),
            Source::Gemini => self.gemini(),
            Source::Copilot => self.copilot(),
            Source::Codex => self.codex(),
        }
    }

    fn session(&self) -> Option<&str> {
        self.session_id.as_deref().filter(|id| safe_id(id))
    }

    fn cwd_forms(&self) -> Vec<PathBuf> {
        let mut forms = vec![self.cwd.clone()];
        if let Ok(real) = fs::canonicalize(&self.cwd)
            && real != self.cwd
        {
            forms.push(real);
        }
        forms
    }

    fn claude(&self) -> Option<PathBuf> {
        let id = self.session()?;
        let projects = self.home.join(".claude").join("projects");
        let file = format!("{id}.jsonl");
        let direct = self
            .cwd_forms()
            .into_iter()
            .map(|cwd| projects.join(claude_project_dir_name(&cwd)).join(&file))
            .find(|path| path.is_file());
        direct.or_else(|| {
            sorted_children(&projects)
                .into_iter()
                .map(|dir| dir.join(&file))
                .find(|path| path.is_file())
        })
    }

    fn gemini(&self) -> Option<PathBuf> {
        let id = self.session()?;
        let tmp = self.home.join(".gemini").join("tmp");
        let slugs = self
            .cwd_forms()
            .into_iter()
            .filter_map(|cwd| cwd.file_name().map(|name| tmp.join(name).join("chats")));
        let others = sorted_children(&tmp).into_iter().map(|dir| dir.join("chats"));
        slugs.chain(others).find_map(|chats| gemini_session_in(&chats, id))
    }

    fn copilot(&self) -> Option<PathBuf> {
        let id = self.session()?;
        let base = self.copilot_home.clone().unwrap_or_else(|| self.home.join(".copilot"));
        let file = base.join("session-state").join(id).join("events.jsonl");
        file.is_file().then_some(file)
    }

    fn codex(&self) -> Option<PathBuf> {
        let base = self.codex_home.clone().unwrap_or_else(|| self.home.join(".codex"));
        let sessions = base.join("sessions");
        let cwds = self.cwd_forms();
        let id = self.session();
        for day in newest_day_dirs(&sessions).into_iter().take(CODEX_DAYS_SCANNED) {
            let mut files: Vec<PathBuf> =
                sorted_children(&day).into_iter().filter(|p| rollout_name(p).is_some()).collect();
            files.reverse();
            for file in files.into_iter().take(CODEX_FILES_READ_PER_DAY) {
                let by_id = id.is_some_and(|id| {
                    rollout_name(&file).is_some_and(|n| n.ends_with(&format!("-{id}.jsonl")))
                });
                if by_id || (id.is_none() && self.rollout_is_ours(&file, &cwds)) {
                    return Some(file);
                }
            }
        }
        None
    }

    fn rollout_is_ours(&self, file: &Path, cwds: &[PathBuf]) -> bool {
        let Some(meta) = first_line_json(file) else {
            return false;
        };
        let payload = &meta["payload"];
        let in_cwd =
            payload["cwd"].as_str().is_some_and(|c| cwds.iter().any(|w| w == Path::new(c)));
        let started = meta["timestamp"].as_str().and_then(rfc3339::unix_millis);
        let late_enough = match (self.started_after_ms, started) {
            (Some(after), Some(at)) => at >= after,
            _ => true,
        };
        meta["type"].as_str() == Some("session_meta")
            && payload["thread_source"].as_str() != Some("subagent")
            && in_cwd
            && late_enough
    }
}

/// The folder name Claude Code uses under `~/.claude/projects` for a working directory: every
/// character that is not an ASCII letter or digit becomes `-` (one per UTF-16 unit, as Claude's own
/// regex does). Checked on real folders: `/` and `.` and `_` and a space all became `-`, so
/// `.../tidewatch/.claude/worktrees/x` is `...-tidewatch--claude-worktrees-x`. Claude shortens a name
/// over 200 characters with a hash; [`Locator`] then finds the file by scanning for the session id.
#[must_use]
pub fn claude_project_dir_name(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .flat_map(|c| {
            let units = if c.is_ascii_alphanumeric() { 0 } else { c.len_utf16() };
            if units == 0 { c.to_string() } else { "-".repeat(units) }.chars().collect::<Vec<_>>()
        })
        .collect()
}

fn safe_id(id: &str) -> bool {
    id.len() >= 8 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let mut children: Vec<PathBuf> =
        fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    children.sort();
    children
}

fn gemini_session_in(chats: &Path, id: &str) -> Option<PathBuf> {
    let short = id.get(..8)?;
    let suffix = format!("-{short}.jsonl");
    sorted_children(chats).into_iter().find(|path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("session-") && n.ends_with(&suffix))
            && first_line_json(path).is_some_and(|h| h["sessionId"].as_str() == Some(id))
    })
}

fn first_line_json(file: &Path) -> Option<Value> {
    let mut line = String::new();
    BufReader::new(fs::File::open(file).ok()?).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

fn rollout_name(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    let is_jsonl = path.extension().is_some_and(|ext| ext == "jsonl");
    (name.starts_with("rollout-") && is_jsonl).then_some(name)
}

fn numbered_children(dir: &Path, digits: usize) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = sorted_children(dir)
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.len() == digits && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    found.reverse();
    found
}

fn newest_day_dirs(sessions: &Path) -> Vec<PathBuf> {
    numbered_children(sessions, 4)
        .into_iter()
        .flat_map(|year| numbered_children(&year, 2))
        .flat_map(|month| numbered_children(&month, 2))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    const SID: &str = "25920b07-a014-43ec-bdb5-16ffbb1fe88b";

    #[test]
    fn claude_folder_names_match_real_folders() {
        let cases = [
            (
                "/Users/me/Git/Hoboware/tidewatch/.claude/worktrees/phase-4-worktrees",
                "-Users-me-Git-Hoboware-tidewatch--claude-worktrees-phase-4-worktrees",
            ),
            ("/work/tw_demo.repo x", "-work-tw-demo-repo-x"),
            (
                "/Users/me/.tidewatch/web-ab12cd/brave-otter",
                "-Users-me--tidewatch-web-ab12cd-brave-otter",
            ),
            ("/tmp/café", "-tmp-caf-"),
            ("/tmp/a\u{1F600}b", "-tmp-a--b"),
        ];
        for (cwd, want) in cases {
            assert_eq!(claude_project_dir_name(Path::new(cwd)), want, "{cwd}");
        }
    }

    #[test]
    fn claude_file_is_found_by_folder_and_session_id() {
        let home = tempfile::tempdir().unwrap();
        let cwd = Path::new("/Users/me/.tidewatch/web-ab12cd/brave-otter");
        let file = home
            .path()
            .join(".claude/projects/-Users-me--tidewatch-web-ab12cd-brave-otter")
            .join(format!("{SID}.jsonl"));
        let mut locator = Locator::new(home.path(), cwd);
        locator.session_id = Some(SID.into());
        assert_eq!(locator.locate(Source::Claude), None);
        touch(&file, "");
        assert_eq!(locator.locate(Source::Claude), Some(file));
    }

    #[test]
    fn claude_file_is_found_by_scanning_when_the_folder_name_differs() {
        let home = tempfile::tempdir().unwrap();
        let file =
            home.path().join(".claude/projects/some-hashed-long-name").join(format!("{SID}.jsonl"));
        touch(&file, "");
        touch(&home.path().join(".claude/projects/other/unrelated.jsonl"), "");
        let mut locator = Locator::new(home.path(), "/very/long/path");
        locator.session_id = Some(SID.into());
        assert_eq!(locator.locate(Source::Claude), Some(file));
    }

    #[test]
    fn a_session_id_that_could_climb_out_is_ignored() {
        let home = tempfile::tempdir().unwrap();
        touch(&home.path().join("secret.jsonl"), "");
        let mut locator = Locator::new(home.path(), "/w");
        for bad in ["../../secret", "a/b", "short", ""] {
            locator.session_id = Some(bad.into());
            for source in [Source::Claude, Source::Gemini, Source::Copilot] {
                assert_eq!(locator.locate(source), None, "{bad:?} {source:?}");
            }
        }
    }

    #[test]
    fn a_hint_wins_when_it_exists() {
        let home = tempfile::tempdir().unwrap();
        let hint = home.path().join("elsewhere.jsonl");
        let mut locator = Locator::new(home.path(), "/w");
        locator.hint = Some(hint.clone());
        assert_eq!(locator.locate(Source::Gemini), None);
        touch(&hint, "");
        assert_eq!(locator.locate(Source::Gemini), Some(hint));
    }

    #[test]
    fn gemini_file_is_found_by_the_repo_folder_name_and_checked_against_its_header() {
        let home = tempfile::tempdir().unwrap();
        let chats = home.path().join(".gemini/tmp/brave-otter/chats");
        let id = "dcb4eb89-2468-423d-b8fa-0a11f6bc6e65";
        let impostor = chats.join("session-2026-10-12T23-03-dcb4eb89.jsonl");
        touch(&impostor, "{\"sessionId\":\"dcb4eb89-0000-0000-0000-000000000000\"}\n");
        let real = chats.join("session-2026-10-12T23-04-dcb4eb89.jsonl");
        touch(&real, &format!("{{\"sessionId\":\"{id}\"}}\n"));
        let mut locator = Locator::new(home.path(), "/Users/me/.tidewatch/web-ab12cd/brave-otter");
        locator.session_id = Some(id.into());
        assert_eq!(locator.locate(Source::Gemini), Some(real));
    }

    #[test]
    fn gemini_file_is_found_under_another_slug() {
        let home = tempfile::tempdir().unwrap();
        let id = "dcb4eb89-2468-423d-b8fa-0a11f6bc6e65";
        let file = home
            .path()
            .join(".gemini/tmp/brave-otter-1/chats/session-2026-10-12T23-04-dcb4eb89.jsonl");
        touch(&file, &format!("{{\"sessionId\":\"{id}\"}}\n"));
        let mut locator = Locator::new(home.path(), "/x/brave-otter");
        locator.session_id = Some(id.into());
        assert_eq!(locator.locate(Source::Gemini), Some(file));
    }

    #[test]
    fn copilot_file_follows_copilot_home() {
        let home = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let id = "daacbb63-b6cb-422a-a4bd-c504b9f08e11";
        let mut locator = Locator::new(home.path(), "/w");
        locator.session_id = Some(id.into());
        let default_file = home.path().join(".copilot/session-state").join(id).join("events.jsonl");
        touch(&default_file, "");
        assert_eq!(locator.locate(Source::Copilot), Some(default_file));
        locator.copilot_home = Some(custom.path().to_path_buf());
        assert_eq!(locator.locate(Source::Copilot), None);
        let custom_file = custom.path().join("session-state").join(id).join("events.jsonl");
        touch(&custom_file, "");
        assert_eq!(locator.locate(Source::Copilot), Some(custom_file));
    }

    fn rollout(
        home: &Path,
        day: &str,
        time: &str,
        id: &str,
        cwd: &str,
        stamp: &str,
        source: &str,
    ) -> PathBuf {
        let file =
            home.join(".codex/sessions").join(day).join(format!("rollout-{time}-{id}.jsonl"));
        let meta = format!(
            "{{\"timestamp\":\"{stamp}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\",\"thread_source\":\"{source}\"}}}}\n"
        );
        touch(&file, &meta);
        file
    }

    #[test]
    fn codex_rollout_is_found_by_folder_newest_first_and_skips_subagents() {
        let home = tempfile::tempdir().unwrap();
        let old = rollout(
            home.path(),
            "2026/10/12",
            "2026-10-12T08-00-00",
            "01a0f1e4-0000-7000-8000-000000000001",
            "/w/a",
            "2026-10-12T08:00:00Z",
            "user",
        );
        let other = rollout(
            home.path(),
            "2026/10/13",
            "2026-10-13T09-00-00",
            "01a0f1e4-0000-7000-8000-000000000002",
            "/w/b",
            "2026-10-13T09:00:00Z",
            "user",
        );
        let sub = rollout(
            home.path(),
            "2026/10/13",
            "2026-10-13T09-30-00",
            "01a0f1e4-0000-7000-8000-000000000003",
            "/w/a",
            "2026-10-13T09:30:00Z",
            "subagent",
        );
        let mut locator = Locator::new(home.path(), "/w/a");
        assert_eq!(locator.locate(Source::Codex), Some(old.clone()));
        let mine = rollout(
            home.path(),
            "2026/10/13",
            "2026-10-13T09-10-00",
            "01a0f1e4-0000-7000-8000-000000000004",
            "/w/a",
            "2026-10-13T09:10:00Z",
            "user",
        );
        assert_eq!(locator.locate(Source::Codex), Some(mine.clone()));
        locator.started_after_ms = rfc3339::unix_millis("2026-10-13T09:20:00Z");
        assert_eq!(locator.locate(Source::Codex), None);
        let _ = (other, sub);
    }

    #[test]
    fn a_real_codex_rollout_is_found_by_folder_and_start_time_under_codex_home() {
        let home = tempfile::tempdir().unwrap();
        let codex_home = tempfile::tempdir().unwrap();
        let name = "rollout-2026-10-13T19-57-41-01a13669-bce5-7242-a392-fd967b70ea72.jsonl";
        let file = codex_home.path().join("sessions/2026/10/13").join(name);
        touch(&file, include_str!("../tests/fixtures/codex_live.jsonl"));
        let mut locator = Locator::new(home.path(), "/work/demo");
        assert_eq!(locator.locate(Source::Codex), None);
        locator.codex_home = Some(codex_home.path().to_path_buf());
        assert_eq!(locator.locate(Source::Codex), Some(file.clone()));
        locator.started_after_ms = rfc3339::unix_millis("2026-10-13T17:57:36Z");
        assert_eq!(locator.locate(Source::Codex), Some(file));
        locator.started_after_ms = rfc3339::unix_millis("2026-10-13T17:57:42Z");
        assert_eq!(locator.locate(Source::Codex), None);
        let other = Locator {
            codex_home: locator.codex_home.clone(),
            ..Locator::new(home.path(), "/work/other")
        };
        assert_eq!(other.locate(Source::Codex), None);
    }

    #[test]
    fn codex_rollout_is_found_by_session_id_when_there_is_one() {
        let home = tempfile::tempdir().unwrap();
        let id = "01a0f1e4-0000-7000-8000-000000000009";
        let file = rollout(
            home.path(),
            "2026/10/13",
            "2026-10-13T09-00-00",
            id,
            "/anywhere",
            "2026-10-13T09:00:00Z",
            "user",
        );
        let mut locator = Locator::new(home.path(), "/w/a");
        locator.session_id = Some(id.into());
        assert_eq!(locator.locate(Source::Codex), Some(file));
    }
}
