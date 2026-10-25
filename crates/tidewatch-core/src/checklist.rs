//! The must-do items every agent is asked to include as steps in its progress ledger: `checklist.md`
//! in the settings folder (installed like `house-style.md`), or the file a repo's `.tidewatch.toml`
//! names. Optional: with neither, an agent plans its own steps under the four stages.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;
use crate::test_command::checklist_path;

/// Largest template text that goes in front of an agent, in bytes.
const MAX_TEMPLATE_BYTES: usize = 16 * 1024;

/// What a new `checklist.md` holds: a note and no items, so the default asks for nothing extra.
pub const DEFAULT_CHECKLIST: &str = "\
<!-- Must-do steps for every task, one per line, for example:
- [ ] Run the full test suite before the PR
Tidewatch asks each agent to add them as steps where they fit. -->
";

/// The text older installs were given as `checklist.md`: the ten phases. A file that still holds
/// exactly this is not a list the user wrote, so it counts as no file.
const PREVIOUS_DEFAULT: &str = "\
### 1. Understand and decide
- [ ] Restate the goal in one line and name any open decision (ask before building when it is the owner's call)
- [ ] Read the code that owns the behavior, plus the repo rules for that area
- [ ] Measure first on real data, read-only: counts, timings, who is affected
- [ ] For a UI change: a mockup at desktop and phone width with real copy, and a yes before building
### 2. Build
- [ ] A fresh worktree from the main branch
- [ ] The smallest change that does the job; reuse existing helpers and components
- [ ] Pure logic in its own small function with a unit test
- [ ] Remove anything the change left unused
### 3. Gate
- [ ] Format, type check and lint on the touched files
- [ ] The full test suites for every area touched
- [ ] Gate output written to a log and the exit code read
### 4. Validate against production
- [ ] Run the real query or call the code builds, read-only, and compare with a hand count
- [ ] Time it on the biggest real case, before and after, at least twice
### 5. Preview
- [ ] Run the built screens against a stub for rare states and get a yes
- [ ] Undo temporary preview edits before pushing
### 6. PR
- [ ] Stage explicit paths, a conventional commit message
- [ ] A body with what and why, numbers, verified and not verified
### 7. Review
- [ ] Wait for CI and bot reviewers
- [ ] Verify every comment against the code before acting
- [ ] Fix with fixup commits folded into the original
- [ ] Reply on every thread and resolve the adopted ones
### 8. Ship
- [ ] Merge only when CI is green, no threads are open and the head is the pushed commit
- [ ] Watch the deploy of the merge commit
### 9. Verify live
- [ ] Errors before and after on the touched routes
- [ ] Timings over equal windows, before and after
- [ ] Read back what the change should have written
### 10. Close out
- [ ] Remove the worktree and branch, stop any servers you started
- [ ] Update the ledger: what shipped, what was verified, what is next
- [ ] Report: done, verified, not verified, next steps, needs you
Phases that do not apply are marked skipped with a reason, never dropped.
";

/// Writes the default text over `checklist.md`, edits included. Returns its path.
pub fn reset(paths: &TidewatchPaths) -> std::io::Result<PathBuf> {
    let file = paths.checklist_file();
    write_atomic(&file, DEFAULT_CHECKLIST)?;
    Ok(file)
}

/// Creates `checklist.md` with the default text when it is missing. An existing file, edited or
/// empty, is never overwritten. Returns its path.
pub fn ensure(paths: &TidewatchPaths) -> std::io::Result<PathBuf> {
    let file = paths.checklist_file();
    std::fs::create_dir_all(paths.config_dir())?;
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&file) {
        Ok(mut created) => created.write_all(DEFAULT_CHECKLIST.as_bytes())?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    Ok(file)
}

fn without_notes(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find("<!--") {
        out.push_str(&rest[..open]);
        match rest[open..].find("-->") {
            Some(close) => rest = &rest[open + close + "-->".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The must-do items in `file`, notes in `<!-- -->` left out. `None` when the file is missing,
/// too large, holds nothing but notes and blanks, or still holds the previous default.
#[must_use]
pub fn must_dos(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    if text.len() > MAX_TEMPLATE_BYTES {
        eprintln!("tidewatch: {} is over {MAX_TEMPLATE_BYTES} bytes; not using it", file.display());
        return None;
    }
    let items = without_notes(&text);
    let items = items.trim();
    (!items.is_empty() && items != PREVIOUS_DEFAULT.trim()).then(|| format!("{items}\n"))
}

/// The must-do items for `repo`: the file its `.tidewatch.toml` names, else `checklist.md`. A file
/// that has none falls through to the next; `None` when neither has any.
#[must_use]
pub fn for_repo(paths: &TidewatchPaths, repo: &Path) -> Option<String> {
    checklist_path(repo)
        .and_then(|file| must_dos(&file))
        .or_else(|| must_dos(&paths.checklist_file()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{Mark, checklist_phases};

    fn paths() -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        (home, paths)
    }

    fn repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        for (name, text) in files {
            let file = repo.path().join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        repo
    }

    #[test]
    fn the_default_asks_for_nothing_and_ships_as_plain_ascii_without_dashes() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.checklist_file(), DEFAULT_CHECKLIST).unwrap();
        assert_eq!(must_dos(&paths.checklist_file()), None);
        assert_eq!(for_repo(&paths, repo_with(&[]).path()), None);
        assert!(DEFAULT_CHECKLIST.is_ascii());
        assert!(!DEFAULT_CHECKLIST.contains(" -- "));
    }

    #[test]
    fn the_previous_default_is_the_ten_phases_and_counts_as_no_file() {
        let phases = checklist_phases(PREVIOUS_DEFAULT);
        assert_eq!(phases.len(), 10);
        assert_eq!(phases[3].title, "Validate against production");
        assert!(phases.iter().flat_map(|p| &p.items).all(|item| item.mark == Mark::Todo));
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.checklist_file(), PREVIOUS_DEFAULT).unwrap();
        assert_eq!(for_repo(&paths, repo_with(&[]).path()), None);
        let edited = PREVIOUS_DEFAULT.replace("Preview", "Look");
        std::fs::write(paths.checklist_file(), &edited).unwrap();
        assert_eq!(
            for_repo(&paths, repo_with(&[]).path()),
            Some(edited.trim_end().to_owned() + "\n")
        );
    }

    #[test]
    fn notes_are_left_out_of_the_items() {
        let cases = [
            ("<!-- note -->\n- [ ] a\n", Some("- [ ] a\n")),
            ("- [ ] a\n<!-- one\ntwo -->\n- [ ] b\n", Some("- [ ] a\n\n- [ ] b\n")),
            ("- [ ] a\n<!-- never closed\n- [ ] b\n", Some("- [ ] a\n")),
            ("<!-- only a note -->\n\n", None),
            ("  \n", None),
            ("", None),
        ];
        for (text, want) in cases {
            let (_home, paths) = paths();
            std::fs::create_dir_all(paths.config_dir()).unwrap();
            std::fs::write(paths.checklist_file(), text).unwrap();
            assert_eq!(must_dos(&paths.checklist_file()).as_deref(), want, "{text:?}");
        }
    }

    #[test]
    fn ensure_creates_the_file_and_keeps_an_edited_or_emptied_one() {
        let (_home, paths) = paths();
        let file = ensure(&paths).unwrap();
        assert_eq!(file, paths.checklist_file());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), DEFAULT_CHECKLIST);
        for edited in ["### 1. Mine\n- [ ] x\n", PREVIOUS_DEFAULT, ""] {
            std::fs::write(&file, edited).unwrap();
            ensure(&paths).unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);
        }
    }

    #[test]
    fn reset_puts_the_default_back_over_edits_and_creates_a_missing_file() {
        let (_home, paths) = paths();
        assert_eq!(reset(&paths).unwrap(), paths.checklist_file());
        std::fs::write(paths.checklist_file(), "mine").unwrap();
        reset(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.checklist_file()).unwrap(), DEFAULT_CHECKLIST);
    }

    #[test]
    fn a_repo_without_an_override_gets_the_settings_file_or_else_nothing() {
        let (_home, paths) = paths();
        let repo = repo_with(&[]);
        assert_eq!(for_repo(&paths, repo.path()), None);
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.checklist_file(), "- [ ] a\n\n\n").unwrap();
        assert_eq!(for_repo(&paths, repo.path()).as_deref(), Some("- [ ] a\n"));
        std::fs::write(paths.checklist_file(), "  \n").unwrap();
        assert_eq!(for_repo(&paths, repo.path()), None);
    }

    #[test]
    fn the_repos_own_file_wins_and_a_bad_one_falls_back() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.checklist_file(), "- [ ] s\n").unwrap();
        let toml = "checklist = \"docs/steps.md\"\n";
        let own = repo_with(&[(".tidewatch.toml", toml), ("docs/steps.md", "- [ ] o\n")]);
        assert_eq!(for_repo(&paths, own.path()).as_deref(), Some("- [ ] o\n"));
        let missing = repo_with(&[(".tidewatch.toml", toml)]);
        assert_eq!(for_repo(&paths, missing.path()).as_deref(), Some("- [ ] s\n"));
        let empty = repo_with(&[(".tidewatch.toml", toml), ("docs/steps.md", "\n")]);
        assert_eq!(for_repo(&paths, empty.path()).as_deref(), Some("- [ ] s\n"));
        let huge = "x".repeat(MAX_TEMPLATE_BYTES + 1);
        let big = repo_with(&[(".tidewatch.toml", toml), ("docs/steps.md", &huge)]);
        assert_eq!(for_repo(&paths, big.path()).as_deref(), Some("- [ ] s\n"));
        let outside = repo_with(&[(".tidewatch.toml", "checklist = \"../steps.md\"\n")]);
        assert_eq!(for_repo(&paths, outside.path()).as_deref(), Some("- [ ] s\n"));
    }
}
