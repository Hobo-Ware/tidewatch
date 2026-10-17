//! The checklist template every agent's progress ledger starts from: `checklist.md` in the settings
//! folder (installed like `house-style.md`), or the file a repo's `.tidewatch.toml` names.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;
use crate::test_command::checklist_path;

/// Largest template text that goes in front of an agent, in bytes.
const MAX_TEMPLATE_BYTES: usize = 16 * 1024;

pub const DEFAULT_CHECKLIST: &str = "\
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

fn usable(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    if text.len() > MAX_TEMPLATE_BYTES {
        eprintln!("tidewatch: {} is over {MAX_TEMPLATE_BYTES} bytes; not using it", file.display());
        return None;
    }
    (!text.trim().is_empty()).then(|| format!("{}\n", text.trim_end()))
}

/// The template for `repo`: the file its `.tidewatch.toml` names, else `checklist.md`, else the
/// default text. A file that is missing, empty or too large falls through to the next.
#[must_use]
pub fn for_repo(paths: &TidewatchPaths, repo: &Path) -> String {
    checklist_path(repo)
        .and_then(|file| usable(&file))
        .or_else(|| usable(&paths.checklist_file()))
        .unwrap_or_else(|| DEFAULT_CHECKLIST.to_owned())
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
    fn the_default_is_ten_phases_of_open_items_and_the_closing_rule() {
        let phases = checklist_phases(DEFAULT_CHECKLIST);
        let titles: Vec<_> = phases.iter().map(|p| (p.number, p.title.as_str())).collect();
        assert_eq!(
            titles,
            [
                (Some(1), "Understand and decide"),
                (Some(2), "Build"),
                (Some(3), "Gate"),
                (Some(4), "Validate against production"),
                (Some(5), "Preview"),
                (Some(6), "PR"),
                (Some(7), "Review"),
                (Some(8), "Ship"),
                (Some(9), "Verify live"),
                (Some(10), "Close out"),
            ]
        );
        let counts: Vec<_> = phases.iter().map(|p| p.items.len()).collect();
        assert_eq!(counts, [4, 4, 3, 2, 2, 2, 4, 2, 3, 3]);
        assert!(phases.iter().flat_map(|p| &p.items).all(|item| item.mark == Mark::Todo));
        assert_eq!(
            phases[9].text,
            ["Phases that do not apply are marked skipped with a reason, never dropped."]
        );
        assert!(phases[..9].iter().all(|p| p.text.is_empty()));
    }

    #[test]
    fn the_default_ships_publicly_so_it_is_plain_ascii_without_dashes() {
        assert!(DEFAULT_CHECKLIST.is_ascii());
        assert!(!DEFAULT_CHECKLIST.contains(" -- "));
    }

    #[test]
    fn ensure_creates_the_file_and_keeps_an_edited_or_emptied_one() {
        let (_home, paths) = paths();
        let file = ensure(&paths).unwrap();
        assert_eq!(file, paths.checklist_file());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), DEFAULT_CHECKLIST);
        for edited in ["### 1. Mine\n- [ ] x\n", ""] {
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
    fn a_repo_without_an_override_gets_the_settings_file_or_else_the_default() {
        let (_home, paths) = paths();
        let repo = repo_with(&[]);
        assert_eq!(for_repo(&paths, repo.path()), DEFAULT_CHECKLIST);
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.checklist_file(), "### 1. Mine\n- [ ] a\n\n\n").unwrap();
        assert_eq!(for_repo(&paths, repo.path()), "### 1. Mine\n- [ ] a\n");
        std::fs::write(paths.checklist_file(), "  \n").unwrap();
        assert_eq!(for_repo(&paths, repo.path()), DEFAULT_CHECKLIST);
    }

    #[test]
    fn the_repos_own_file_wins_and_a_bad_one_falls_back() {
        let (_home, paths) = paths();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.checklist_file(), "### 1. Settings\n- [ ] s\n").unwrap();
        let toml = "checklist = \"docs/steps.md\"\n";
        let own =
            repo_with(&[(".tidewatch.toml", toml), ("docs/steps.md", "### 1. Own\n- [ ] o\n")]);
        assert_eq!(for_repo(&paths, own.path()), "### 1. Own\n- [ ] o\n");
        let missing = repo_with(&[(".tidewatch.toml", toml)]);
        assert_eq!(for_repo(&paths, missing.path()), "### 1. Settings\n- [ ] s\n");
        let empty = repo_with(&[(".tidewatch.toml", toml), ("docs/steps.md", "\n")]);
        assert_eq!(for_repo(&paths, empty.path()), "### 1. Settings\n- [ ] s\n");
        let huge = "x".repeat(MAX_TEMPLATE_BYTES + 1);
        let big = repo_with(&[(".tidewatch.toml", toml), ("docs/steps.md", &huge)]);
        assert_eq!(for_repo(&paths, big.path()), "### 1. Settings\n- [ ] s\n");
        let outside = repo_with(&[(".tidewatch.toml", "checklist = \"../steps.md\"\n")]);
        assert_eq!(for_repo(&paths, outside.path()), "### 1. Settings\n- [ ] s\n");
    }
}
