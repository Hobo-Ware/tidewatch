//! The house-style file (D13): one Tidewatch-owned markdown file added to every agent session.
//!
//! `ensure` creates it with the default text and never touches it again, so edits survive. Copilot
//! only reads `*.instructions.md` files with `applyTo` front matter, so a derived copy is rendered for it.

use std::io::Write;
use std::path::PathBuf;

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

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

/// Rewrites the derived Copilot copy from the current `house-style.md` and returns its directory,
/// the value for `COPILOT_CUSTOM_INSTRUCTIONS_DIRS`. Safe to call on every launch.
pub fn sync_copilot_instructions(paths: &TidewatchPaths) -> std::io::Result<PathBuf> {
    let source = ensure(paths)?;
    let body = std::fs::read_to_string(source)?;
    let dir = paths.copilot_instructions_dir();
    write_atomic(&dir.join(COPILOT_FILE_NAME), &copilot_instructions_md(&body))?;
    Ok(dir)
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
    fn copilot_copy_has_apply_to_front_matter_and_follows_edits() {
        let (_home, paths) = paths();
        ensure(&paths).unwrap();
        std::fs::write(paths.house_style_file(), "be brief\n").unwrap();
        let dir = sync_copilot_instructions(&paths).unwrap();
        assert_eq!(dir, paths.copilot_instructions_dir());
        let text = std::fs::read_to_string(dir.join("tidewatch.instructions.md")).unwrap();
        assert_eq!(text, "---\napplyTo: \"**\"\n---\n\nbe brief\n");
    }
}
