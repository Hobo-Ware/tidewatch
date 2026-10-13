//! D14 folder trust, the disk half: find each CLI's config file, edit it with the pure functions in
//! `trust`, and keep `trust.toml` (the consent record) in step.
//!
//! Every write goes through one path: read, compute, then a sibling temp file with the original's
//! permissions, a check that nobody else changed the file meanwhile (the CLIs rewrite their files
//! while running, `~/.claude.json` constantly), and a rename. The first change to a file also
//! leaves a copy next to it as `<name>.tidewatch-bak`. A file that does not parse is not touched.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::consent::{Added, Consent, Record};
use crate::paths::TidewatchPaths;
use crate::trust::{
    CliTrust, Coverage, Status, TrustCli, TrustError, TrustState, add_entry, assess, remove_entry,
};

pub const BACKUP_SUFFIX: &str = ".tidewatch-bak";
const TEMP_SUFFIX: &str = ".tidewatch-tmp";
const ATTEMPTS: usize = 3;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Where each CLI keeps the file Tidewatch edits, honoring the variables the CLIs honor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFiles {
    claude: PathBuf,
    copilot: PathBuf,
    codex: PathBuf,
}

impl ConfigFiles {
    #[must_use]
    pub fn resolve(home: &Path, var: impl Fn(&str) -> Option<OsString>) -> Self {
        let dir = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        Self {
            claude: dir("CLAUDE_CONFIG_DIR")
                .map_or_else(|| home.join(".claude.json"), |d| d.join(".claude.json")),
            copilot: dir("COPILOT_HOME").map_or_else(
                || home.join(".copilot").join("config.json"),
                |d| d.join("config.json"),
            ),
            codex: dir("CODEX_HOME")
                .map_or_else(|| home.join(".codex").join("config.toml"), |d| d.join("config.toml")),
        }
    }

    #[must_use]
    pub fn from_env(home: &Path) -> Self {
        Self::resolve(home, |name| std::env::var_os(name))
    }

    #[must_use]
    pub fn file(&self, cli: TrustCli) -> Option<&Path> {
        match cli {
            TrustCli::Claude => Some(&self.claude),
            TrustCli::Copilot => Some(&self.copilot),
            TrustCli::Codex => Some(&self.codex),
            TrustCli::Gemini => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Added,
    Removed,
    /// Nothing to do: already trusted, or no entry of Tidewatch's left to remove.
    Unchanged,
    /// This CLI's entry does not work that way (Gemini has no file, Claude has no root entry).
    NotApplicable,
    Failed(String),
}

/// The consent to edit, with everything an edit needs. Cheap to clone into a job thread.
#[derive(Clone, Debug)]
pub struct Grant {
    files: ConfigFiles,
    root: PathBuf,
    record_file: PathBuf,
}

impl Grant {
    #[must_use]
    pub fn new(paths: &TidewatchPaths, files: ConfigFiles) -> Self {
        Self { files, root: paths.root().to_path_buf(), record_file: paths.trust_file() }
    }

    #[must_use]
    pub fn files(&self) -> &ConfigFiles {
        &self.files
    }

    #[must_use]
    pub fn root_entry(&self) -> String {
        folder_string(&self.root)
    }

    #[must_use]
    pub fn record(&self) -> Record {
        Record::load(&self.record_file)
    }

    /// Where each of the `installed` CLIs stands. Reads the files, so call it off the UI thread.
    #[must_use]
    pub fn inspect(&self, installed: &[TrustCli]) -> TrustState {
        let root = self.root_entry();
        let clis = TrustCli::ALL
            .into_iter()
            .filter(|cli| installed.contains(cli))
            .map(|cli| {
                let file = self.files.file(cli);
                match file.map(read_optional) {
                    Some(Err(why)) => CliTrust {
                        cli,
                        file: file.map(Path::to_path_buf),
                        status: Status::Blocked(why),
                    },
                    Some(Ok(text)) => assess(cli, file, text.as_deref(), &root),
                    None => assess(cli, None, None, &root),
                }
            })
            .collect();
        TrustState { clis }
    }

    /// Writes the answer into `trust.toml`, keeping the list of entries.
    pub fn set_consent(&self, consent: Consent) -> std::io::Result<()> {
        let _one = lock();
        let mut record = Record::load(&self.record_file);
        record.consent = consent;
        record.save(&self.record_file)
    }

    /// Trusts the Tidewatch root in every installed CLI where one entry covers the worktrees.
    #[must_use]
    pub fn trust_root(&self, installed: &[TrustCli]) -> Vec<(TrustCli, Outcome)> {
        let _one = lock();
        if let Err(e) = std::fs::create_dir_all(&self.root) {
            let why = format!("cannot create {}: {e}", self.root.display());
            return installed.iter().map(|c| (*c, Outcome::Failed(why.clone()))).collect();
        }
        let root = self.root_entry();
        let mut record = Record::load(&self.record_file);
        let mut outcomes = Vec::new();
        for cli in TrustCli::ALL.into_iter().filter(|c| installed.contains(c)) {
            let outcome = if cli.coverage() == Coverage::Root {
                self.add(cli, &root, &mut record)
            } else {
                Outcome::NotApplicable
            };
            outcomes.push((cli, outcome));
        }
        outcomes
    }

    /// Trusts one new worktree for a CLI whose root entry does not cover it. Call before the agent starts.
    #[must_use]
    pub fn trust_worktree(&self, cli: TrustCli, dir: &Path) -> Outcome {
        if cli.coverage() != Coverage::PerWorktree {
            return Outcome::NotApplicable;
        }
        let _one = lock();
        let mut record = Record::load(&self.record_file);
        self.add(cli, &folder_string(dir), &mut record)
    }

    /// Removes the entries Tidewatch added: all of them, or only those for `worktree`.
    #[must_use]
    pub fn revoke(&self, worktree: Option<&Path>) -> Vec<(TrustCli, Outcome)> {
        let _one = lock();
        let mut record = Record::load(&self.record_file);
        let only = worktree.map(folder_string);
        let mut outcomes = Vec::new();
        let mut kept = Vec::new();
        for added in std::mem::take(&mut record.added) {
            if only.as_ref().is_some_and(|only| *only != added.entry) {
                kept.push(added);
                continue;
            }
            match modify(&added.file, |text| remove_entry(added.cli, text, &added.entry)) {
                Ok(true) => outcomes.push((added.cli, Outcome::Removed)),
                Ok(false) => outcomes.push((added.cli, Outcome::Unchanged)),
                Err(why) => {
                    outcomes.push((added.cli, Outcome::Failed(why)));
                    kept.push(added);
                }
            }
        }
        record.added = kept;
        if let Err(e) = record.save(&self.record_file) {
            outcomes
                .push((TrustCli::Claude, Outcome::Failed(format!("cannot save trust.toml: {e}"))));
        }
        outcomes
    }

    fn add(&self, cli: TrustCli, entry: &str, record: &mut Record) -> Outcome {
        let Some(file) = self.files.file(cli) else { return Outcome::NotApplicable };
        match modify(file, |text| add_entry(cli, text, entry)) {
            Ok(false) => Outcome::Unchanged,
            Ok(true) => {
                let added = Added { cli, file: file.to_path_buf(), entry: entry.to_owned() };
                if !record.added.contains(&added) {
                    record.added.push(added);
                }
                match record.save(&self.record_file) {
                    Ok(()) => Outcome::Added,
                    Err(e) => Outcome::Failed(format!("added, but cannot save trust.toml: {e}")),
                }
            }
            Err(why) => Outcome::Failed(why),
        }
    }
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The path as the CLIs see it: symlinks resolved, and for a folder that is already gone the
/// parent resolved plus its name.
fn folder_string(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| {
        match (path.parent().and_then(|p| std::fs::canonicalize(p).ok()), path.file_name()) {
            (Some(parent), Some(name)) => parent.join(name),
            _ => path.to_path_buf(),
        }
    });
    resolved.to_string_lossy().into_owned()
}

fn read_optional(file: &Path) -> Result<Option<String>, String> {
    match std::fs::read(file) {
        Ok(bytes) => String::from_utf8(bytes).map(Some).map_err(|_| "not UTF-8".to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read: {e}")),
    }
}

/// Runs `edit` on the file's text and writes the result. `Ok(false)`: nothing to change.
fn modify(
    file: &Path,
    edit: impl Fn(&str) -> Result<Option<String>, TrustError>,
) -> Result<bool, String> {
    for _ in 0..ATTEMPTS {
        let before = read_optional(file)?;
        let Some(after) = edit(before.as_deref().unwrap_or_default()).map_err(|e| e.to_string())?
        else {
            return Ok(false);
        };
        if write_checked(file, before.as_deref(), &after)? {
            return Ok(true);
        }
    }
    Err("the file kept changing while Tidewatch edited it, left alone".to_owned())
}

/// `Ok(false)`: the file changed since `before` was read, nothing was written.
fn write_checked(file: &Path, before: Option<&str>, after: &str) -> Result<bool, String> {
    let target = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let temp = sibling(&target, TEMP_SUFFIX);
    std::fs::write(&temp, after).map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
    let finish = || -> Result<bool, String> {
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&temp, meta.permissions()).map_err(|e| e.to_string())?;
        } else {
            private(&temp)?;
        }
        if read_optional(&target)?.as_deref() != before {
            return Ok(false);
        }
        let backup = sibling(&target, BACKUP_SUFFIX);
        if before.is_some() && !backup.exists() {
            std::fs::copy(&target, &backup)
                .map_err(|e| format!("cannot back up to {}: {e}", backup.display()))?;
        }
        std::fs::rename(&temp, &target)
            .map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
        Ok(true)
    };
    let written = finish();
    if !matches!(written, Ok(true)) {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

#[cfg(unix)]
fn private(file: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn private(_file: &Path) -> Result<(), String> {
    Ok(())
}

fn sibling(file: &Path, suffix: &str) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    file.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consent::Consent;
    use crate::trust::is_trusted;

    struct Rig {
        _home: tempfile::TempDir,
        paths: TidewatchPaths,
        grant: Grant,
        files: ConfigFiles,
    }

    fn rig() -> Rig {
        let home = tempfile::tempdir().unwrap();
        let home_path = std::fs::canonicalize(home.path()).unwrap();
        let paths = TidewatchPaths::new(&home_path);
        let files = ConfigFiles::resolve(&home_path, |_| None);
        let grant = Grant::new(&paths, files.clone());
        Rig { _home: home, paths, grant, files }
    }

    fn put(file: &Path, text: &str) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    fn read(file: &Path) -> String {
        std::fs::read_to_string(file).unwrap()
    }

    const CLAUDE: &str = "{\n  \"numStartups\": 3,\n  \"projects\": {\n    \"/old\": {\n      \"hasTrustDialogAccepted\": true\n    }\n  }\n}\n";
    const COPILOT: &str = "// managed\n{\n  \"trustedFolders\": [\n    \"/old\"\n  ]\n}\n";
    const CODEX: &str = "model = \"gpt-5\"\n";

    fn all_clis() -> Vec<TrustCli> {
        TrustCli::ALL.to_vec()
    }

    #[test]
    fn config_files_follow_the_variables_each_cli_honors() {
        let home = Path::new("/h");
        let plain = ConfigFiles::resolve(home, |_| None);
        assert_eq!(plain.file(TrustCli::Claude), Some(Path::new("/h/.claude.json")));
        assert_eq!(plain.file(TrustCli::Copilot), Some(Path::new("/h/.copilot/config.json")));
        assert_eq!(plain.file(TrustCli::Codex), Some(Path::new("/h/.codex/config.toml")));
        assert_eq!(plain.file(TrustCli::Gemini), None);
        let moved = ConfigFiles::resolve(home, |name| match name {
            "CLAUDE_CONFIG_DIR" => Some("/c".into()),
            "COPILOT_HOME" => Some("/p".into()),
            "CODEX_HOME" => Some(String::new().into()),
            _ => None,
        });
        assert_eq!(moved.file(TrustCli::Claude), Some(Path::new("/c/.claude.json")));
        assert_eq!(moved.file(TrustCli::Copilot), Some(Path::new("/p/config.json")));
        assert_eq!(moved.file(TrustCli::Codex), Some(Path::new("/h/.codex/config.toml")));
    }

    #[test]
    fn root_trust_edits_copilot_only_and_backs_up_the_original() {
        let rig = rig();
        let copilot = rig.files.file(TrustCli::Copilot).unwrap();
        put(copilot, COPILOT);
        put(rig.files.file(TrustCli::Claude).unwrap(), CLAUDE);
        let outcomes = rig.grant.trust_root(&all_clis());
        assert_eq!(
            outcomes,
            [
                (TrustCli::Claude, Outcome::NotApplicable),
                (TrustCli::Copilot, Outcome::Added),
                (TrustCli::Gemini, Outcome::NotApplicable),
                (TrustCli::Codex, Outcome::NotApplicable),
            ]
        );
        let root = rig.grant.root_entry();
        assert!(is_trusted(TrustCli::Copilot, &read(copilot), &root).unwrap());
        assert_eq!(read(&sibling(copilot, BACKUP_SUFFIX)), COPILOT);
        assert_eq!(
            read(rig.files.file(TrustCli::Claude).unwrap()),
            CLAUDE,
            "Claude's file is untouched"
        );
        assert!(rig.paths.root().is_dir());
    }

    #[test]
    fn applying_twice_changes_nothing_the_second_time_and_records_one_entry() {
        let rig = rig();
        let copilot = rig.files.file(TrustCli::Copilot).unwrap();
        put(copilot, COPILOT);
        let _ = rig.grant.trust_root(&[TrustCli::Copilot]);
        let once = read(copilot);
        let again = rig.grant.trust_root(&[TrustCli::Copilot]);
        assert_eq!(again, [(TrustCli::Copilot, Outcome::Unchanged)]);
        assert_eq!(read(copilot), once);
        assert_eq!(rig.grant.record().added.len(), 1);
    }

    #[test]
    fn the_backup_keeps_the_pristine_original_through_later_edits() {
        let rig = rig();
        let claude = rig.files.file(TrustCli::Claude).unwrap();
        put(claude, CLAUDE);
        for name in ["one", "two"] {
            let dir = rig.paths.root().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            assert_eq!(rig.grant.trust_worktree(TrustCli::Claude, &dir), Outcome::Added);
        }
        assert_eq!(read(&sibling(claude, BACKUP_SUFFIX)), CLAUDE);
    }

    #[test]
    fn per_worktree_trust_covers_claude_and_codex_and_leaves_gemini_and_copilot() {
        let rig = rig();
        put(rig.files.file(TrustCli::Claude).unwrap(), CLAUDE);
        let dir = rig.paths.root().join("repo-abc123").join("calm-otter");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(rig.grant.trust_worktree(TrustCli::Claude, &dir), Outcome::Added);
        assert_eq!(rig.grant.trust_worktree(TrustCli::Codex, &dir), Outcome::Added);
        assert_eq!(rig.grant.trust_worktree(TrustCli::Copilot, &dir), Outcome::NotApplicable);
        assert_eq!(rig.grant.trust_worktree(TrustCli::Gemini, &dir), Outcome::NotApplicable);
        let entry = folder_string(&dir);
        assert!(
            is_trusted(TrustCli::Claude, &read(rig.files.file(TrustCli::Claude).unwrap()), &entry)
                .unwrap()
        );
        assert!(
            is_trusted(TrustCli::Codex, &read(rig.files.file(TrustCli::Codex).unwrap()), &entry)
                .unwrap()
        );
        assert_eq!(rig.grant.trust_worktree(TrustCli::Claude, &dir), Outcome::Unchanged);
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone_with_no_backup_and_no_record() {
        let rig = rig();
        let claude = rig.files.file(TrustCli::Claude).unwrap();
        put(claude, "{\"projects\": ");
        let dir = rig.paths.root().join("w");
        std::fs::create_dir_all(&dir).unwrap();
        let outcome = rig.grant.trust_worktree(TrustCli::Claude, &dir);
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
        assert_eq!(read(claude), "{\"projects\": ");
        assert!(!sibling(claude, BACKUP_SUFFIX).exists());
        assert!(!sibling(claude, TEMP_SUFFIX).exists());
        assert_eq!(rig.grant.record().added, []);
    }

    #[test]
    fn undo_restores_every_file_byte_for_byte_and_empties_the_record() {
        let rig = rig();
        put(rig.files.file(TrustCli::Claude).unwrap(), CLAUDE);
        put(rig.files.file(TrustCli::Copilot).unwrap(), COPILOT);
        put(rig.files.file(TrustCli::Codex).unwrap(), CODEX);
        let dir = rig.paths.root().join("repo").join("w");
        std::fs::create_dir_all(&dir).unwrap();
        let _ = rig.grant.trust_root(&all_clis());
        let _ = rig.grant.trust_worktree(TrustCli::Claude, &dir);
        let _ = rig.grant.trust_worktree(TrustCli::Codex, &dir);
        assert_eq!(rig.grant.record().added.len(), 3);
        let outcomes = rig.grant.revoke(None);
        assert!(outcomes.iter().all(|(_, o)| *o == Outcome::Removed), "{outcomes:?}");
        assert_eq!(read(rig.files.file(TrustCli::Claude).unwrap()), CLAUDE);
        assert_eq!(read(rig.files.file(TrustCli::Copilot).unwrap()), COPILOT);
        assert_eq!(read(rig.files.file(TrustCli::Codex).unwrap()), CODEX);
        assert_eq!(rig.grant.record().added, []);
    }

    #[test]
    fn revoking_one_worktree_keeps_the_rest_even_after_its_folder_is_gone() {
        let rig = rig();
        put(rig.files.file(TrustCli::Claude).unwrap(), CLAUDE);
        let (keep, drop_me) = (rig.paths.root().join("keep"), rig.paths.root().join("drop"));
        for dir in [&keep, &drop_me] {
            std::fs::create_dir_all(dir).unwrap();
            let _ = rig.grant.trust_worktree(TrustCli::Claude, dir);
        }
        std::fs::remove_dir(&drop_me).unwrap();
        let outcomes = rig.grant.revoke(Some(&drop_me));
        assert_eq!(outcomes, [(TrustCli::Claude, Outcome::Removed)]);
        let text = read(rig.files.file(TrustCli::Claude).unwrap());
        assert!(is_trusted(TrustCli::Claude, &text, &folder_string(&keep)).unwrap());
        assert!(!is_trusted(TrustCli::Claude, &text, &folder_string(&drop_me)).unwrap());
        assert_eq!(rig.grant.record().added.len(), 1);
    }

    #[test]
    fn undo_never_removes_an_entry_the_user_made_themselves() {
        let rig = rig();
        let copilot = rig.files.file(TrustCli::Copilot).unwrap();
        let root = rig.grant.root_entry();
        put(copilot, &format!("{{\"trustedFolders\": [\"{root}\"]}}"));
        let outcomes = rig.grant.trust_root(&[TrustCli::Copilot]);
        assert_eq!(outcomes, [(TrustCli::Copilot, Outcome::Unchanged)]);
        assert_eq!(rig.grant.revoke(None), []);
        assert!(is_trusted(TrustCli::Copilot, &read(copilot), &root).unwrap());
    }

    #[test]
    fn a_missing_file_is_created_private_and_a_present_one_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let rig = rig();
        let copilot = rig.files.file(TrustCli::Copilot).unwrap();
        let _ = rig.grant.trust_root(&[TrustCli::Copilot]);
        assert_eq!(std::fs::metadata(copilot).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!sibling(copilot, BACKUP_SUFFIX).exists(), "nothing to back up");
        let claude = rig.files.file(TrustCli::Claude).unwrap();
        put(claude, CLAUDE);
        std::fs::set_permissions(claude, std::fs::Permissions::from_mode(0o640)).unwrap();
        let dir = rig.paths.root().join("w");
        std::fs::create_dir_all(&dir).unwrap();
        let _ = rig.grant.trust_worktree(TrustCli::Claude, &dir);
        assert_eq!(std::fs::metadata(claude).unwrap().permissions().mode() & 0o777, 0o640);
        assert_eq!(
            std::fs::metadata(sibling(claude, BACKUP_SUFFIX)).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn a_symlinked_config_stays_a_symlink_and_the_target_is_edited() {
        use std::os::unix::fs::symlink;
        let rig = rig();
        let claude = rig.files.file(TrustCli::Claude).unwrap();
        let real = rig.paths.root().join("dotfiles").join("claude.json");
        put(&real, CLAUDE);
        symlink(&real, claude).unwrap();
        let dir = rig.paths.root().join("w");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(rig.grant.trust_worktree(TrustCli::Claude, &dir), Outcome::Added);
        assert!(std::fs::symlink_metadata(claude).unwrap().file_type().is_symlink());
        assert!(read(&real).contains("hasTrustDialogAccepted"));
        assert!(read(&real).len() > CLAUDE.len());
    }

    #[test]
    fn a_change_made_by_the_cli_between_read_and_write_is_not_overwritten() {
        let rig = rig();
        let file = rig.files.file(TrustCli::Copilot).unwrap().to_path_buf();
        put(&file, COPILOT);
        let calls = std::cell::Cell::new(0);
        let changed = modify(&file, |text| {
            calls.set(calls.get() + 1);
            let edited = add_entry(TrustCli::Copilot, text, "/new");
            if calls.get() == 1 {
                std::fs::write(
                    &file,
                    "// managed\n{\"trustedFolders\": [\"/old\"], \"theirs\": 1}\n",
                )
                .unwrap();
            }
            edited
        });
        assert_eq!(changed, Ok(true));
        assert_eq!(calls.get(), 2, "the first attempt saw the file change and tried again");
        let text = read(&file);
        assert!(text.contains("\"theirs\": 1") && text.contains("\"/new\""), "{text}");
    }

    #[test]
    fn inspect_reports_each_installed_cli_and_reads_but_never_writes() {
        let rig = rig();
        put(rig.files.file(TrustCli::Copilot).unwrap(), "{\"trustedFolders\": [\"/\"]}");
        put(rig.files.file(TrustCli::Claude).unwrap(), "nope");
        let state = rig.grant.inspect(&[TrustCli::Claude, TrustCli::Copilot, TrustCli::Gemini]);
        let statuses: Vec<_> = state.clis.iter().map(|c| (c.cli, c.status.clone())).collect();
        assert_eq!(statuses[1], (TrustCli::Copilot, Status::Trusted));
        assert_eq!(statuses[2], (TrustCli::Gemini, Status::LaunchEnv));
        assert!(matches!(statuses[0], (TrustCli::Claude, Status::Blocked(_))));
        assert_eq!(state.clis.len(), 3, "Codex is not installed, so it is not listed");
        assert!(!rig.paths.root().exists());
        assert_eq!(read(rig.files.file(TrustCli::Claude).unwrap()), "nope");
    }

    #[test]
    fn the_answer_is_saved_without_losing_the_entries() {
        let rig = rig();
        let dir = rig.paths.root().join("w");
        std::fs::create_dir_all(&dir).unwrap();
        let _ = rig.grant.trust_worktree(TrustCli::Codex, &dir);
        rig.grant.set_consent(Consent::Granted).unwrap();
        let record = rig.grant.record();
        assert_eq!((record.consent, record.added.len()), (Consent::Granted, 1));
        let _ = rig.grant.revoke(None);
        assert_eq!(rig.grant.record().consent, Consent::Granted, "revoking keeps the answer");
    }

    #[test]
    fn the_whole_consent_round_trip_on_temp_files() {
        let rig = rig();
        let (claude, copilot, codex) = (
            rig.files.file(TrustCli::Claude).unwrap(),
            rig.files.file(TrustCli::Copilot).unwrap(),
            rig.files.file(TrustCli::Codex).unwrap(),
        );
        put(claude, CLAUDE);
        put(copilot, COPILOT);
        put(codex, CODEX);
        let originals = [CLAUDE, COPILOT, CODEX];
        let installed = all_clis();
        assert!(rig.grant.inspect(&installed).needs_consent());

        let (consent, effects) = Consent::Undecided.next(crate::consent::Event::Yes);
        assert_eq!(consent, Consent::Granted);
        assert!(effects.contains(&crate::consent::Effect::TrustRoot));
        let _ = rig.grant.trust_root(&installed);
        let worktree = rig.paths.worktree_dir(Path::new("/repo"), "calm-otter");
        std::fs::create_dir_all(&worktree).unwrap();
        for cli in [TrustCli::Claude, TrustCli::Codex] {
            assert_eq!(rig.grant.trust_worktree(cli, &worktree), Outcome::Added);
        }
        let state = rig.grant.inspect(&installed);
        assert_eq!(state.get(TrustCli::Copilot).unwrap().status, Status::Trusted);

        let (consent, effects) = consent.next(crate::consent::Event::Undo);
        assert_eq!(consent, Consent::Declined);
        assert!(effects.contains(&crate::consent::Effect::RevokeAll));
        let _ = rig.grant.revoke(None);
        for (file, original) in [claude, copilot, codex].into_iter().zip(originals) {
            assert_eq!(read(file), original, "{}", file.display());
        }
    }
}
