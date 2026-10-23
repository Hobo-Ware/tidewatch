//! The `git` and `gh` shims an agent tab gets first on its `PATH`, and the environment that wakes
//! them up.
//!
//! A shim passes straight to the real tool unless the call can matter (`git commit`, `gh pr`,
//! `gh issue`, `gh run`) and the environment names a guard (`TIDEWATCH_GUARD_CONFIG` and a binary to
//! ask, `TIDEWATCH_GUARD_BIN`). Then it asks `tidewatch --guard <tool> <args>`, which applies the
//! rules and runs the real tool. If that binary is gone the shim runs the real tool, so a stale tab
//! never loses `git`. The real tool is found by skipping the shim's own directory on `PATH`.
//!
//! Nothing is written to the repo and no git hook is installed: the repo's own hooks keep working.
//! A commit whose message comes from an editor is not seen; agents give `-m` or `-F`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::guard_run::{BIN_ENV, CACHE_ENV, CONFIG_ENV, DIR_ENV, REPO_ENV};

/// What a tab's launch needs to wake the shims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardCtx {
    /// The Tidewatch binary that answers `--guard`.
    pub bin: PathBuf,
    /// `settings.toml`, read on every call so a switch applies to a running tab.
    pub config: PathBuf,
    /// The repo the tab belongs to, for its own rule answers.
    pub repo: PathBuf,
    pub cache_dir: PathBuf,
}

impl GuardCtx {
    /// The variables for the agent's environment. `shim_dir` is the folder holding the shims.
    #[must_use]
    pub fn env(&self, shim_dir: &Path) -> Vec<(String, String)> {
        let show = |path: &Path| path.display().to_string();
        vec![
            (BIN_ENV.to_owned(), show(&self.bin)),
            (CONFIG_ENV.to_owned(), show(&self.config)),
            (REPO_ENV.to_owned(), show(&self.repo)),
            (CACHE_ENV.to_owned(), show(&self.cache_dir)),
            (DIR_ENV.to_owned(), show(shim_dir)),
        ]
    }
}

/// The shim for `tool`. `watch` is the `case` pattern of the words that make a call worth asking about.
fn script(tool: &str, watch: &str) -> String {
    SHIM_TEMPLATE.replace("@TOOL@", tool).replace("@WATCH@", watch)
}

const SHIM_TEMPLATE: &str = r#"#!/bin/sh
self=$0
real=
old_ifs=$IFS
IFS=:
set -f
for dir in $PATH; do
  [ -n "$dir" ] || continue
  cand="$dir/@TOOL@"
  if [ -f "$cand" ] && [ -x "$cand" ] && ! [ "$cand" -ef "$self" ]; then real=$cand; break; fi
done
IFS=$old_ifs
set +f
if [ -z "$real" ]; then echo "tidewatch guard: no real @TOOL@ on PATH" >&2; exit 127; fi
watch=
for arg in "$@"; do
  case "$arg" in @WATCH@) watch=1; break ;; esac
done
if [ -n "$watch" ] && [ -n "${TIDEWATCH_GUARD_CONFIG:-}" ] && [ -x "${TIDEWATCH_GUARD_BIN:-}" ]; then
  exec "$TIDEWATCH_GUARD_BIN" --guard @TOOL@ "$@"
fi
exec "$real" "$@"
"#;

/// File name and content of every shim.
#[must_use]
pub fn shims() -> [(&'static str, String); 2] {
    [("git", script("git", "commit")), ("gh", script("gh", "pr|issue|run"))]
}

/// Writes the shims into `dir` as executable files. Rewrites a file only when its content differs.
pub fn ensure_shims(dir: &Path) -> std::io::Result<()> {
    for (name, content) in shims() {
        let file = dir.join(name);
        let current = std::fs::read_to_string(&file).ok();
        let executable = file.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
        if current.as_deref() != Some(content.as_str()) || !executable {
            crate::fsutil::write_atomic(&file, &content)?;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
