//! D14 folder trust: the pure half. Given a CLI's config file as text, say whether a folder is
//! trusted and return the text with it trusted (or no longer). Nothing here touches the disk.
//!
//! How each CLI stores trust, as checked on 2026-10-13 (details in `PLAN.md`, Phase 0 findings):
//!
//! - Claude Code 2.1.296: `projects["<path>"].hasTrustDialogAccepted` in `~/.claude.json`. A parent
//!   entry does not cover a git worktree (the lookup stops at the git root), so each worktree gets
//!   its own entry.
//! - Copilot 1.0.96: `trustedFolders` array in `~/.copilot/config.json` (a file with `//` comments).
//!   An entry covers every folder under it, worktrees included, so the Tidewatch root is enough.
//! - Gemini 0.46.0: `~/.gemini/trustedFolders.json` would work too, but Tidewatch already sets
//!   `GEMINI_CLI_TRUST_WORKSPACE=true` for its tabs, so no file is edited.
//! - Codex 0.162.1: `[projects."<path>"] trust_level = "trusted"` in `~/.codex/config.toml`. A
//!   parent folder does not cover a worktree. Codex resolves a worktree to its main repository, so an
//!   entry for that repository would, but it is the user's own folder, so each worktree gets its own
//!   entry (it wins over the repository's, and an explicit `untrusted` on the worktree is honored).
//!
//! Every edit is a splice into the original text, so keys, order, indentation and comments the edit
//! does not touch stay as they were. A file that does not parse is refused, never rewritten.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agents::Wiring;
use crate::jsonc_edit::{self as json, Object, Span, Value};

const CLAUDE_KEY: &str = "hasTrustDialogAccepted";
const COPILOT_KEY: &str = "trustedFolders";
const CODEX_VALUE: &str = "trusted";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustCli {
    Claude,
    Copilot,
    Gemini,
    Codex,
}

/// What one entry covers, which decides what Tidewatch writes and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// One entry for the Tidewatch root covers every worktree under it.
    Root,
    /// The root does not cover a worktree: each new worktree gets its own entry before it starts.
    PerWorktree,
    /// Tidewatch sets an environment variable for the tab. No file is edited.
    LaunchEnv,
}

impl TrustCli {
    pub const ALL: [Self; 4] = [Self::Claude, Self::Copilot, Self::Gemini, Self::Codex];

    #[must_use]
    pub fn from_wiring(wiring: Wiring) -> Option<Self> {
        match wiring {
            Wiring::Claude => Some(Self::Claude),
            Wiring::Copilot => Some(Self::Copilot),
            Wiring::Gemini => Some(Self::Gemini),
            Wiring::Codex => Some(Self::Codex),
            Wiring::Custom => None,
        }
    }

    #[must_use]
    pub fn wiring(self) -> Wiring {
        match self {
            Self::Claude => Wiring::Claude,
            Self::Copilot => Wiring::Copilot,
            Self::Gemini => Wiring::Gemini,
            Self::Codex => Wiring::Codex,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Copilot => "GitHub Copilot",
            Self::Gemini => "Gemini CLI",
            Self::Codex => "Codex",
        }
    }

    #[must_use]
    pub fn coverage(self) -> Coverage {
        match self {
            Self::Copilot => Coverage::Root,
            Self::Claude | Self::Codex => Coverage::PerWorktree,
            Self::Gemini => Coverage::LaunchEnv,
        }
    }

    /// The line this CLI's file gets for `folder`, as shown in the consent dialog.
    #[must_use]
    pub fn added_line(self, folder: &str) -> String {
        let quoted = json::escape(folder);
        match self {
            Self::Claude => format!("\"projects\": {{ {quoted}: {{ \"{CLAUDE_KEY}\": true }} }}"),
            Self::Copilot => format!("\"{COPILOT_KEY}\": [ ..., {quoted} ]"),
            Self::Codex => format!("[projects.{quoted}]  trust_level = \"{CODEX_VALUE}\""),
            Self::Gemini => "GEMINI_CLI_TRUST_WORKSPACE=true (environment, no file)".to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustError {
    /// The file is not valid JSON (with comments) or TOML. Never edited.
    Unparsable(String),
    /// It parses, but not into the shape this CLI writes.
    Shape(String),
    /// The user set the folder to untrusted. Tidewatch does not overrule that.
    Declined,
    /// This CLI keeps no file Tidewatch edits.
    NoFile,
}

impl fmt::Display for TrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unparsable(why) => write!(f, "cannot read the file ({why}), left alone"),
            Self::Shape(why) => write!(f, "unexpected content ({why}), left alone"),
            Self::Declined => f.write_str("the folder is marked untrusted there, left alone"),
            Self::NoFile => f.write_str("no file to edit"),
        }
    }
}

type Result<T> = std::result::Result<T, TrustError>;

fn parse_json(text: &str) -> Result<Option<Object>> {
    json::document(text).map_err(|e| TrustError::Unparsable(e.to_string()))
}

fn parse_toml(text: &str) -> Result<toml::Table> {
    text.parse::<toml::Table>().map_err(|e| TrustError::Unparsable(e.message().to_owned()))
}

fn shape<T>(what: &str) -> Result<T> {
    Err(TrustError::Shape(what.to_owned()))
}

fn object_member(text: &str, parent: &Object, key: &str) -> Result<Option<(usize, Object)>> {
    let Some(index) = parent.members.iter().position(|m| m.key == key) else { return Ok(None) };
    match json::object_at(text, parent.members[index].value) {
        Some(object) => Ok(Some((index, object))),
        None => shape(&format!("{key} is not an object")),
    }
}

fn flag_is_true(text: &str, span: Span) -> Result<bool> {
    match &text[span.start..span.end] {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => shape(&format!("{CLAUDE_KEY} is not true or false")),
    }
}

fn same_folder(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

fn covers(entry: &str, folder: &str) -> bool {
    let entry = entry.trim_end_matches('/');
    let folder = folder.trim_end_matches('/');
    folder == entry || folder.strip_prefix(entry).is_some_and(|rest| rest.starts_with('/'))
}

fn trust_array(text: &str, doc: &Object) -> Result<Option<json::Array>> {
    let Some(member) = doc.get(COPILOT_KEY) else { return Ok(None) };
    match json::array_at(text, member.value) {
        Some(array) => Ok(Some(array)),
        None => shape(&format!("{COPILOT_KEY} is not an array")),
    }
}

/// Whether `folder` is trusted by this file. An empty text is a file with no entries.
pub fn is_trusted(cli: TrustCli, text: &str, folder: &str) -> Result<bool> {
    match cli {
        TrustCli::Claude => claude_trusted(text, folder),
        TrustCli::Copilot => {
            let Some(doc) = parse_json(text)? else { return Ok(false) };
            let Some(array) = trust_array(text, &doc)? else { return Ok(false) };
            Ok(array
                .elements
                .iter()
                .filter_map(|span| json::string_at(text, *span))
                .any(|entry| covers(&entry, folder)))
        }
        TrustCli::Codex => {
            Ok(codex_level(&parse_toml(text)?, folder).as_deref() == Some(CODEX_VALUE))
        }
        TrustCli::Gemini => Err(TrustError::NoFile),
    }
}

fn claude_trusted(text: &str, folder: &str) -> Result<bool> {
    let Some(doc) = parse_json(text)? else { return Ok(false) };
    let Some((_, projects)) = object_member(text, &doc, "projects")? else { return Ok(false) };
    let Some((_, entry)) = object_member(text, &projects, folder)? else { return Ok(false) };
    match entry.get(CLAUDE_KEY) {
        Some(flag) => flag_is_true(text, flag.value),
        None => Ok(false),
    }
}

fn codex_level(table: &toml::Table, folder: &str) -> Option<String> {
    table
        .get("projects")?
        .as_table()?
        .get(folder)?
        .as_table()?
        .get("trust_level")?
        .as_str()
        .map(str::to_owned)
}

/// The text with `folder` trusted, or `None` when it already is. Refuses an unparsable file.
pub fn add_entry(cli: TrustCli, text: &str, folder: &str) -> Result<Option<String>> {
    if is_trusted(cli, text, folder)? {
        return Ok(None);
    }
    let edited = match cli {
        TrustCli::Claude => claude_add(text, folder)?,
        TrustCli::Copilot => copilot_add(text, folder)?,
        TrustCli::Codex => codex_add(text, folder)?,
        TrustCli::Gemini => return Err(TrustError::NoFile),
    };
    match is_trusted(cli, &edited, folder) {
        Ok(true) => Ok(Some(edited)),
        _ => shape("the edit did not take"),
    }
}

fn entry_value() -> Value {
    Value::Object(vec![(CLAUDE_KEY.to_owned(), Value::Bool(true))])
}

fn claude_add(text: &str, folder: &str) -> Result<String> {
    let Some(doc) = parse_json(text)? else {
        let projects = Value::Object(vec![(folder.to_owned(), entry_value())]);
        return Ok(json::fresh_document("projects", &projects));
    };
    let Some((_, projects)) = object_member(text, &doc, "projects")? else {
        let projects = Value::Object(vec![(folder.to_owned(), entry_value())]);
        return Ok(json::insert_member(text, &doc, "projects", &projects));
    };
    let Some((_, entry)) = object_member(text, &projects, folder)? else {
        return Ok(json::insert_member(text, &projects, folder, &entry_value()));
    };
    match entry.get(CLAUDE_KEY) {
        Some(flag) => {
            flag_is_true(text, flag.value)?;
            Ok(json::replace(text, flag.value, "true"))
        }
        None => Ok(json::insert_member(text, &entry, CLAUDE_KEY, &Value::Bool(true))),
    }
}

fn copilot_add(text: &str, folder: &str) -> Result<String> {
    let Some(doc) = parse_json(text)? else {
        return Ok(json::fresh_document(
            COPILOT_KEY,
            &Value::Array(vec![Value::Str(folder.to_owned())]),
        ));
    };
    match trust_array(text, &doc)? {
        Some(array) => Ok(json::push_element(text, &array, &Value::Str(folder.to_owned()))),
        None => Ok(json::insert_member(
            text,
            &doc,
            COPILOT_KEY,
            &Value::Array(vec![Value::Str(folder.to_owned())]),
        )),
    }
}

fn codex_header(folder: &str) -> String {
    format!("[projects.{}]", json::escape(folder))
}

fn codex_block(folder: &str) -> String {
    format!("{}\ntrust_level = \"{CODEX_VALUE}\"\n", codex_header(folder))
}

fn codex_add(text: &str, folder: &str) -> Result<String> {
    let table = parse_toml(text)?;
    match codex_level(&table, folder).as_deref() {
        Some("untrusted") => return Err(TrustError::Declined),
        Some(_) => return shape("the folder has a trust_level other than trusted"),
        None => {}
    }
    if table.get("projects").and_then(|p| p.as_table()).is_some_and(|p| p.contains_key(folder)) {
        return shape("the folder has an entry without a trust_level");
    }
    let mut out = text.to_owned();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(&codex_block(folder));
    Ok(out)
}

/// The text without the entry Tidewatch added for `folder`, or `None` when there is nothing of
/// Tidewatch's to remove. Claude's entry keeps whatever else Claude wrote into it.
pub fn remove_entry(cli: TrustCli, text: &str, folder: &str) -> Result<Option<String>> {
    let edited = match cli {
        TrustCli::Claude => claude_remove(text, folder)?,
        TrustCli::Copilot => copilot_remove(text, folder)?,
        TrustCli::Codex => codex_remove(text, folder)?,
        TrustCli::Gemini => return Err(TrustError::NoFile),
    };
    if let Some(edited) = &edited {
        match cli {
            TrustCli::Codex => parse_toml(edited).map(drop)?,
            _ => parse_json(edited).map(drop)?,
        }
    }
    Ok(edited)
}

fn claude_remove(text: &str, folder: &str) -> Result<Option<String>> {
    let Some(doc) = parse_json(text)? else { return Ok(None) };
    let Some((projects_at, projects)) = object_member(text, &doc, "projects")? else {
        return Ok(None);
    };
    let Some((index, entry)) = object_member(text, &projects, folder)? else { return Ok(None) };
    let Some(flag) = entry.members.iter().position(|m| m.key == CLAUDE_KEY) else {
        return Ok(None);
    };
    if !flag_is_true(text, entry.members[flag].value)? {
        return Ok(None);
    }
    if entry.members.len() > 1 {
        Ok(Some(json::remove_member(text, &entry, flag)))
    } else if projects.members.len() > 1 {
        Ok(Some(json::remove_member(text, &projects, index)))
    } else {
        Ok(Some(json::remove_member(text, &doc, projects_at)))
    }
}

fn copilot_remove(text: &str, folder: &str) -> Result<Option<String>> {
    let Some(doc) = parse_json(text)? else { return Ok(None) };
    let Some(array) = trust_array(text, &doc)? else { return Ok(None) };
    let found = array.elements.iter().position(|span| {
        json::string_at(text, *span).is_some_and(|entry| same_folder(&entry, folder))
    });
    let Some(index) = found else { return Ok(None) };
    if array.elements.len() > 1 {
        return Ok(Some(json::remove_element(text, &array, index)));
    }
    let list = doc.members.iter().position(|m| m.key == COPILOT_KEY).unwrap_or_default();
    Ok(Some(json::remove_member(text, &doc, list)))
}

fn codex_remove(text: &str, folder: &str) -> Result<Option<String>> {
    parse_toml(text)?;
    let header = codex_header(folder);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(at) = lines.iter().position(|l| l.trim() == header) else { return Ok(None) };
    let end = lines[at + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |n| at + 1 + n);
    let ours = lines[at + 1..end]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .all(|l| l.replace(' ', "") == format!("trust_level=\"{CODEX_VALUE}\""));
    if !ours {
        return Ok(None);
    }
    let start = if at > 0 && lines[at - 1].trim().is_empty() { at - 1 } else { at };
    let kept: String = lines[..start].concat() + &lines[end..].concat();
    Ok(Some(kept))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The Tidewatch root is already trusted there (by Tidewatch's entry or the user's own).
    Trusted,
    /// Consent would add the root.
    Needs,
    /// Parent trust does not cover worktrees: each new worktree gets an entry when it starts.
    PerWorktree,
    /// Handled by an environment variable on the tab. Nothing to edit.
    LaunchEnv,
    /// The file cannot be edited safely. The reason is for the user.
    Blocked(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliTrust {
    pub cli: TrustCli,
    pub file: Option<PathBuf>,
    pub status: Status,
}

/// Where each installed CLI stands, for the consent dialog and the settings page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustState {
    pub clis: Vec<CliTrust>,
}

impl TrustState {
    /// The CLIs consent would let Tidewatch edit a file for, now or at each new worktree.
    #[must_use]
    pub fn changes(&self) -> Vec<&CliTrust> {
        self.clis
            .iter()
            .filter(|c| matches!(c.status, Status::Needs | Status::PerWorktree))
            .collect()
    }

    /// Worth asking about: there is something Tidewatch would edit.
    #[must_use]
    pub fn needs_consent(&self) -> bool {
        !self.changes().is_empty()
    }

    #[must_use]
    pub fn get(&self, cli: TrustCli) -> Option<&CliTrust> {
        self.clis.iter().find(|c| c.cli == cli)
    }
}

/// One CLI's standing from its file's text (`None`: no file yet) and the Tidewatch root.
#[must_use]
pub fn assess(cli: TrustCli, file: Option<&Path>, text: Option<&str>, root: &str) -> CliTrust {
    let file = file.map(Path::to_path_buf);
    let status = match cli.coverage() {
        Coverage::LaunchEnv => Status::LaunchEnv,
        coverage => match is_trusted(cli, text.unwrap_or_default(), root) {
            Err(e) => Status::Blocked(e.to_string()),
            Ok(true) => Status::Trusted,
            Ok(false) if coverage == Coverage::Root => Status::Needs,
            Ok(false) => Status::PerWorktree,
        },
    };
    CliTrust { cli, file, status }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/home/u/.tidewatch";
    const WORKTREE: &str = "/home/u/.tidewatch/repo-abc123/calm-otter";

    fn add(cli: TrustCli, text: &str, folder: &str) -> String {
        add_entry(cli, text, folder).unwrap().expect("an edit")
    }

    #[test]
    fn claude_empty_file_gets_a_projects_entry() {
        let out = add(TrustCli::Claude, "", WORKTREE);
        assert_eq!(
            out,
            "{\n  \"projects\": {\n    \"/home/u/.tidewatch/repo-abc123/calm-otter\": {\n      \"hasTrustDialogAccepted\": true\n    }\n  }\n}\n"
        );
        assert!(is_trusted(TrustCli::Claude, &out, WORKTREE).unwrap());
    }

    #[test]
    fn claude_keeps_unrelated_keys_and_formatting_byte_for_byte() {
        let before = "{\n  \"numStartups\": 12,\n  \"cached\": {\"a\": [1, 2.50, 1e3]},\n  \"projects\": {\n    \"/other\": {\n      \"hasTrustDialogAccepted\": true,\n      \"lastCost\": 5.453764799999999\n    }\n  },\n  \"tail\": \"\u{e9}\"\n}\n";
        let out = add(TrustCli::Claude, before, WORKTREE);
        assert!(out.starts_with(
            "{\n  \"numStartups\": 12,\n  \"cached\": {\"a\": [1, 2.50, 1e3]},\n  \"projects\": {\n    \"/other\": {\n      \"hasTrustDialogAccepted\": true,\n      \"lastCost\": 5.453764799999999\n    },\n    \"/home/u/.tidewatch/repo-abc123/calm-otter\": {\n      \"hasTrustDialogAccepted\": true\n    }\n  },\n  \"tail\": \"\u{e9}\"\n}\n"
        ));
        assert_eq!(out.len(), out.trim_end().len() + 1);
        assert!(is_trusted(TrustCli::Claude, &out, "/other").unwrap());
    }

    #[test]
    fn claude_adds_projects_when_the_file_has_none() {
        let out = add(TrustCli::Claude, "{\n  \"theme\": \"dark\"\n}\n", WORKTREE);
        assert!(out.starts_with("{\n  \"theme\": \"dark\",\n  \"projects\": {"));
        assert!(is_trusted(TrustCli::Claude, &out, WORKTREE).unwrap());
    }

    #[test]
    fn claude_flips_a_false_flag_and_fills_a_missing_one() {
        let out = add(
            TrustCli::Claude,
            &format!("{{\"projects\": {{\"{WORKTREE}\": {{\"hasTrustDialogAccepted\": false}}}}}}"),
            WORKTREE,
        );
        assert_eq!(
            out,
            format!("{{\"projects\": {{\"{WORKTREE}\": {{\"hasTrustDialogAccepted\": true}}}}}}")
        );
        let out = add(
            TrustCli::Claude,
            &format!("{{\"projects\": {{\"{WORKTREE}\": {{\"lastCost\": 1}}}}}}"),
            WORKTREE,
        );
        assert!(out.contains("\"lastCost\": 1, \"hasTrustDialogAccepted\": true"));
    }

    #[test]
    fn claude_an_ancestor_entry_is_not_trust_for_a_worktree() {
        let text =
            format!("{{\"projects\": {{\"{ROOT}\": {{\"hasTrustDialogAccepted\": true}}}}}}");
        assert!(!is_trusted(TrustCli::Claude, &text, WORKTREE).unwrap());
    }

    #[test]
    fn already_trusted_is_unchanged_for_every_format() {
        let claude =
            format!("{{\"projects\": {{\"{WORKTREE}\": {{\"hasTrustDialogAccepted\": true}}}}}}");
        let copilot = format!("// c\n{{\"trustedFolders\": [\"{ROOT}\"]}}");
        let codex = format!("[projects.\"{WORKTREE}\"]\ntrust_level = \"trusted\"\n");
        for (cli, text, folder) in [
            (TrustCli::Claude, claude.as_str(), WORKTREE),
            (TrustCli::Copilot, copilot.as_str(), ROOT),
            (TrustCli::Copilot, copilot.as_str(), WORKTREE),
            (TrustCli::Codex, codex.as_str(), WORKTREE),
        ] {
            assert_eq!(add_entry(cli, text, folder), Ok(None), "{cli:?} {folder}");
        }
    }

    #[test]
    fn copilot_keeps_its_comment_header_and_other_keys() {
        let before = "// User settings belong in settings.json.\n// This file is managed automatically.\n{\n  \"firstLaunchAt\": \"2026-07-03T02:28:14.334Z\",\n  \"trustedFolders\": [\n    \"/a\",\n    \"/b\"\n  ],\n  \"recentModelIds\": [\n    \"m\"\n  ]\n}\n";
        let out = add(TrustCli::Copilot, before, ROOT);
        assert_eq!(
            out,
            before.replace("    \"/b\"\n", "    \"/b\",\n    \"/home/u/.tidewatch\"\n")
        );
    }

    #[test]
    fn copilot_creates_the_list_when_missing_or_the_file_is_empty() {
        let out = add(TrustCli::Copilot, "{\n  \"a\": 1\n}\n", ROOT);
        assert_eq!(
            out,
            "{\n  \"a\": 1,\n  \"trustedFolders\": [\n    \"/home/u/.tidewatch\"\n  ]\n}\n"
        );
        let out = add(TrustCli::Copilot, "", ROOT);
        assert_eq!(out, "{\n  \"trustedFolders\": [\n    \"/home/u/.tidewatch\"\n  ]\n}\n");
    }

    #[test]
    fn copilot_a_trusted_ancestor_already_covers_the_root() {
        let text = "{\"trustedFolders\": [\"/home/u/\"]}";
        assert_eq!(add_entry(TrustCli::Copilot, text, ROOT), Ok(None));
        assert!(
            !is_trusted(TrustCli::Copilot, "{\"trustedFolders\": [\"/home/user\"]}", ROOT).unwrap()
        );
    }

    #[test]
    fn codex_appends_a_table_and_keeps_the_rest() {
        let before = "model = \"gpt-5\"\n# keep me\n[projects.\"/x\"]\ntrust_level = \"trusted\"\n";
        let out = add(TrustCli::Codex, before, WORKTREE);
        assert_eq!(
            out,
            format!("{before}\n[projects.\"{WORKTREE}\"]\ntrust_level = \"trusted\"\n")
        );
        assert_eq!(
            add(TrustCli::Codex, "", WORKTREE),
            format!("[projects.\"{WORKTREE}\"]\ntrust_level = \"trusted\"\n")
        );
        let no_newline = add(TrustCli::Codex, "a = 1", WORKTREE);
        assert!(no_newline.starts_with("a = 1\n\n[projects."));
    }

    #[test]
    fn codex_respects_an_explicit_untrusted() {
        let text = format!("[projects.\"{WORKTREE}\"]\ntrust_level = \"untrusted\"\n");
        assert_eq!(add_entry(TrustCli::Codex, &text, WORKTREE), Err(TrustError::Declined));
    }

    #[test]
    fn codex_inline_projects_table_is_refused_not_corrupted() {
        let text = "projects = { \"/x\" = { trust_level = \"trusted\" } }\n";
        assert!(matches!(add_entry(TrustCli::Codex, text, WORKTREE), Err(TrustError::Shape(_))));
    }

    #[test]
    fn unparsable_files_are_refused_in_every_format() {
        for (cli, text) in [
            (TrustCli::Claude, "{\"projects\": "),
            (TrustCli::Claude, "[]"),
            (TrustCli::Copilot, "{\"trustedFolders\": [\"a\",]}"),
            (TrustCli::Codex, "projects = [[["),
        ] {
            assert!(
                matches!(add_entry(cli, text, WORKTREE), Err(TrustError::Unparsable(_))),
                "{cli:?} {text}"
            );
            assert!(matches!(remove_entry(cli, text, WORKTREE), Err(TrustError::Unparsable(_))));
        }
    }

    #[test]
    fn a_surprising_shape_is_refused() {
        for (cli, text) in [
            (TrustCli::Claude, "{\"projects\": []}"),
            (
                TrustCli::Claude,
                "{\"projects\": {\"/home/u/.tidewatch/repo-abc123/calm-otter\": null}}",
            ),
            (TrustCli::Copilot, "{\"trustedFolders\": \"/a\"}"),
        ] {
            assert!(
                matches!(add_entry(cli, text, WORKTREE), Err(TrustError::Shape(_))),
                "{cli:?} {text}"
            );
        }
    }

    #[test]
    fn gemini_has_no_file() {
        assert_eq!(add_entry(TrustCli::Gemini, "", ROOT), Err(TrustError::NoFile));
        assert_eq!(remove_entry(TrustCli::Gemini, "", ROOT), Err(TrustError::NoFile));
    }

    #[test]
    fn undo_removes_only_our_claude_entry_and_leaves_the_rest_exact() {
        let original = "{\n  \"a\": 1,\n  \"projects\": {\n    \"/other\": {\n      \"hasTrustDialogAccepted\": true\n    }\n  },\n  \"z\": 2\n}\n";
        let added = add(TrustCli::Claude, original, WORKTREE);
        let back = remove_entry(TrustCli::Claude, &added, WORKTREE).unwrap().unwrap();
        assert_eq!(back, original);
        assert_eq!(remove_entry(TrustCli::Claude, &back, WORKTREE), Ok(None));
    }

    #[test]
    fn undo_on_a_claude_entry_with_session_data_drops_only_the_flag() {
        let text = format!(
            "{{\n  \"projects\": {{\n    \"{WORKTREE}\": {{\n      \"hasTrustDialogAccepted\": true,\n      \"lastCost\": 1.5\n    }}\n  }}\n}}\n"
        );
        let out = remove_entry(TrustCli::Claude, &text, WORKTREE).unwrap().unwrap();
        assert_eq!(
            out,
            format!(
                "{{\n  \"projects\": {{\n    \"{WORKTREE}\": {{\n      \"lastCost\": 1.5\n    }}\n  }}\n}}\n"
            )
        );
        assert!(!is_trusted(TrustCli::Claude, &out, WORKTREE).unwrap());
    }

    #[test]
    fn undo_removes_only_our_copilot_entry_and_never_a_users_ancestor() {
        let original = "// c\n{\n  \"trustedFolders\": [\n    \"/a\",\n    \"/b\"\n  ]\n}\n";
        let added = add(TrustCli::Copilot, original, ROOT);
        assert_eq!(remove_entry(TrustCli::Copilot, &added, ROOT).unwrap().unwrap(), original);
        assert_eq!(remove_entry(TrustCli::Copilot, original, ROOT), Ok(None));
        assert_eq!(
            remove_entry(TrustCli::Copilot, "{\"trustedFolders\": [\"/home/u\"]}", ROOT),
            Ok(None)
        );
    }

    #[test]
    fn undo_also_drops_the_key_it_had_to_create() {
        let claude = "{\n  \"theme\": \"dark\"\n}\n";
        let added = add(TrustCli::Claude, claude, WORKTREE);
        assert_eq!(remove_entry(TrustCli::Claude, &added, WORKTREE).unwrap().unwrap(), claude);
        let copilot = "// c\n{\n  \"firstLaunchAt\": \"x\"\n}\n";
        let added = add(TrustCli::Copilot, copilot, ROOT);
        assert_eq!(remove_entry(TrustCli::Copilot, &added, ROOT).unwrap().unwrap(), copilot);
    }

    #[test]
    fn undo_removes_only_our_codex_table() {
        let original = "model = \"gpt-5\"\n\n[projects.\"/x\"]\ntrust_level = \"trusted\"\n";
        let added = add(TrustCli::Codex, original, WORKTREE);
        assert_eq!(remove_entry(TrustCli::Codex, &added, WORKTREE).unwrap().unwrap(), original);
        assert_eq!(remove_entry(TrustCli::Codex, original, WORKTREE), Ok(None));
        let user_table =
            format!("[projects.\"{WORKTREE}\"]\ntrust_level = \"trusted\"\nnote = 1\n");
        assert_eq!(remove_entry(TrustCli::Codex, &user_table, WORKTREE), Ok(None));
    }

    #[test]
    fn paths_with_quotes_and_backslashes_round_trip() {
        let odd = "/home/u/.tidewatch/we\"ird\\dir";
        for cli in [TrustCli::Claude, TrustCli::Codex] {
            let added = add(cli, "", odd);
            assert!(is_trusted(cli, &added, odd).unwrap(), "{cli:?}");
            assert_eq!(
                remove_entry(cli, &added, odd).unwrap().unwrap().trim(),
                if cli == TrustCli::Claude { "{}" } else { "" }
            );
        }
    }

    fn state_of(cli: TrustCli, text: Option<&str>) -> Status {
        assess(cli, Some(Path::new("/f")), text, ROOT).status
    }

    #[test]
    fn state_says_who_is_trusted_who_needs_it_and_who_cannot_use_the_root() {
        assert_eq!(state_of(TrustCli::Copilot, None), Status::Needs);
        assert_eq!(
            state_of(TrustCli::Copilot, Some("{\"trustedFolders\": [\"/home/u\"]}")),
            Status::Trusted
        );
        assert_eq!(state_of(TrustCli::Claude, Some("{}")), Status::PerWorktree);
        assert_eq!(state_of(TrustCli::Codex, None), Status::PerWorktree);
        assert_eq!(state_of(TrustCli::Gemini, None), Status::LaunchEnv);
        assert!(matches!(state_of(TrustCli::Copilot, Some("{")), Status::Blocked(_)));
        assert!(matches!(state_of(TrustCli::Claude, Some("nope")), Status::Blocked(_)));
    }

    #[test]
    fn consent_is_only_worth_asking_when_a_file_would_change() {
        let state = TrustState {
            clis: vec![
                assess(TrustCli::Gemini, None, None, ROOT),
                assess(TrustCli::Copilot, None, Some("{\"trustedFolders\": [\"/home/u\"]}"), ROOT),
            ],
        };
        assert!(!state.needs_consent());
        let state = TrustState { clis: vec![assess(TrustCli::Claude, None, None, ROOT)] };
        assert!(state.needs_consent());
        assert_eq!(state.changes().len(), 1);
    }

    #[test]
    fn the_dialog_line_names_the_key_each_cli_gets() {
        assert_eq!(
            TrustCli::Copilot.added_line(ROOT),
            "\"trustedFolders\": [ ..., \"/home/u/.tidewatch\" ]"
        );
        assert!(TrustCli::Claude.added_line(WORKTREE).contains("hasTrustDialogAccepted\": true"));
        assert!(TrustCli::Codex.added_line(WORKTREE).contains("trust_level = \"trusted\""));
    }

    #[test]
    fn only_the_four_known_wirings_map_to_a_cli() {
        for cli in TrustCli::ALL {
            assert_eq!(TrustCli::from_wiring(cli.wiring()), Some(cli));
        }
        assert_eq!(TrustCli::from_wiring(Wiring::Custom), None);
    }
}
