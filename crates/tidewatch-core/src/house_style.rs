//! The house-style file (D13): one Tidewatch-owned markdown file added to every agent session.
//!
//! `ensure` creates it with the default text and never touches it again, so edits survive. Each agent
//! gets that text with the progress ledger format (`compose`) appended. Copilot only reads
//! `*.instructions.md` files with `applyTo` front matter, so a derived copy is rendered for it.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;
use crate::progress_prompt;

pub const DEFAULT_HOUSE_STYLE: &str = "\
# Tidewatch house style

You are running inside Tidewatch, which shows your replies as markdown. Follow these rules in every
answer. The repo's own instructions (CLAUDE.md, AGENTS.md) still apply and win on conflicts.

## Answers

- Write markdown. Start with a one or two line summary, then details under short headings.
- Use lists for several items and fenced code blocks, with a language, for code, commands and output.
- Refer to code as `path:line` (for example `src/main.rs:42`) so Tidewatch can link it.
- Keep it short. No filler openers and no closing recap.

## End of every turn

Finish with exactly this block:

**Done**
- what changed or what you found

**Needs you**
- the decision or input you need, or \"nothing\"

**Next**
- the single next step

## Tidewatch tools

When the `tidewatch` MCP tools are available:

- Call `stage_done` with a short summary when the task or stage you were given is finished.
- Call `reply_to_note` to answer a review note (`list_notes` shows them) instead of answering it only in chat.

## Progress ledger

Keep a progress ledger for the task with the `update_progress` tool of the `tidewatch` MCP server: write it before any other work, update it at every milestone, and call `get_progress` first after a context reset.

## Questions

When you need the user to choose between options, ask a multiple-choice question instead of describing the options in prose. Use your own question tool if you have one (for example `AskUserQuestion`), otherwise call the `ask_user` tool of the `tidewatch` MCP server. Put the option you recommend first and end its label with (Recommended).
";

/// Writes the default text over `house-style.md`, edits included. Returns its path.
pub fn reset(paths: &TidewatchPaths) -> std::io::Result<PathBuf> {
    let file = paths.house_style_file();
    std::fs::create_dir_all(paths.config_dir())?;
    write_atomic(&file, DEFAULT_HOUSE_STYLE)?;
    Ok(file)
}

const COPILOT_FILE_NAME: &str = "tidewatch.instructions.md";

/// Creates `house-style.md` with the default text when it is missing. An existing file, edited or
/// empty, is never overwritten. Returns its path.
pub fn ensure(paths: &TidewatchPaths) -> std::io::Result<PathBuf> {
    let file = paths.house_style_file();
    std::fs::create_dir_all(paths.config_dir())?;
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&file) {
        Ok(mut created) => created.write_all(DEFAULT_HOUSE_STYLE.as_bytes())?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    Ok(file)
}

/// The house style as a Copilot instructions file: `applyTo: "**"` front matter, then the text.
#[must_use]
pub fn copilot_instructions_md(house_style: &str) -> String {
    format!("---\napplyTo: \"**\"\n---\n\n{house_style}")
}

/// The house style an agent is given: `house_style` as the user keeps it, then the ledger format
/// with `checklist` filled in. The user's file is never changed.
#[must_use]
pub fn compose(house_style: &str, checklist: &str) -> String {
    format!("{}\n\n{}", house_style.trim_end(), progress_prompt::section(checklist))
}

/// Rewrites the derived Copilot copy of `house_style` in `dir` and returns `dir`, the value for
/// `COPILOT_CUSTOM_INSTRUCTIONS_DIRS`. Safe to call on every launch.
pub fn sync_copilot_instructions(dir: &Path, house_style: &str) -> std::io::Result<PathBuf> {
    write_atomic(&dir.join(COPILOT_FILE_NAME), &copilot_instructions_md(house_style))?;
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        (home, paths)
    }

    #[test]
    fn default_text_asks_for_the_d13_contract() {
        for needle in [
            "markdown",
            "path:line",
            "**Done**",
            "**Needs you**",
            "**Next**",
            "stage_done",
            "reply_to_note",
            "ask_user",
            "AskUserQuestion",
            "update_progress",
            "get_progress",
        ] {
            assert!(DEFAULT_HOUSE_STYLE.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn ensure_creates_the_file_with_the_default_text() {
        let (_home, paths) = paths();
        let file = ensure(&paths).unwrap();
        assert_eq!(file, paths.house_style_file());
        assert_eq!(std::fs::read_to_string(file).unwrap(), DEFAULT_HOUSE_STYLE);
    }

    #[test]
    fn ensure_keeps_an_edited_file() {
        let (_home, paths) = paths();
        ensure(&paths).unwrap();
        std::fs::write(paths.house_style_file(), "my own style\n").unwrap();
        ensure(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), "my own style\n");
    }

    #[test]
    fn reset_puts_the_default_text_back_over_edits_and_creates_a_missing_file() {
        let (_home, paths) = paths();
        assert_eq!(reset(&paths).unwrap(), paths.house_style_file());
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), DEFAULT_HOUSE_STYLE);
        std::fs::write(paths.house_style_file(), "my own style\n").unwrap();
        reset(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), DEFAULT_HOUSE_STYLE);
    }

    #[test]
    fn ensure_keeps_an_emptied_file() {
        let (_home, paths) = paths();
        ensure(&paths).unwrap();
        std::fs::write(paths.house_style_file(), "").unwrap();
        ensure(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), "");
    }

    #[test]
    fn ensure_recreates_a_deleted_file() {
        let (_home, paths) = paths();
        ensure(&paths).unwrap();
        std::fs::remove_file(paths.house_style_file()).unwrap();
        ensure(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), DEFAULT_HOUSE_STYLE);
    }

    #[test]
    fn copilot_copy_has_apply_to_front_matter_and_holds_the_text_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let written = sync_copilot_instructions(dir.path(), "be brief\n").unwrap();
        assert_eq!(written, dir.path());
        let text = std::fs::read_to_string(dir.path().join("tidewatch.instructions.md")).unwrap();
        assert_eq!(text, "---\napplyTo: \"**\"\n---\n\nbe brief\n");
    }

    #[test]
    fn compose_appends_the_ledger_format_after_whatever_the_user_wrote() {
        let checklist = "### 1. Mine\n- [ ] only this\n";
        for mine in ["be brief\n", "be brief", "be brief\n\n\n", ""] {
            let text = compose(mine, checklist);
            assert!(text.starts_with(mine.trim_end()), "{mine:?}");
            assert!(text.contains("## Progress ledger format\n"), "{mine:?}");
            assert!(text.contains("## Checklist\n### 1. Mine\n- [ ] only this\n\n## Decisions"));
            assert!(text.ends_with('\n'));
        }
        assert!(compose(DEFAULT_HOUSE_STYLE, checklist).contains("`get_progress` first"));
    }
}
