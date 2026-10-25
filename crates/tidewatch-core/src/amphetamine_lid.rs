//! Lid-closed mode of Amphetamine: the root helper script, the one admin command that starts it,
//! the marker and token files, and the launch recovery.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;

pub const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
pub const POLL_SECS: u32 = 2;

pub const PROMPT: &str = "Tidewatch wants to keep this Mac awake with the lid closed.";
pub const RESTORE_PROMPT: &str = "Tidewatch wants to turn sleep back on.";
pub const PROMPT_CANCELLED: &str = "the password prompt was cancelled";

const HELPER: &str = r#"token=$1
owner=$2
trap 'pmset -a disablesleep 0; exit 0' TERM HUP INT
while :; do
  deadline=$(cat "$token" 2>/dev/null) || break
  case $deadline in ''|*[!0-9]*) break;; esac
  kill -0 "$owner" 2>/dev/null || break
  if [ "$deadline" -gt 0 ] && [ "$(date +%s)" -ge "$deadline" ]; then break; fi
  sleep 2
done
pmset -a disablesleep 0
"#;

#[must_use]
pub fn helper_script() -> &'static str {
    HELPER
}

#[must_use]
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[must_use]
pub fn admin_command(token: &Path, owner: u32, search_path: &str) -> String {
    format!(
        "PATH={path}; export PATH; pmset -a disablesleep 1 || exit 1; \
         nohup /bin/sh -c {script} amphetamine {token} {owner} >/dev/null 2>&1 </dev/null & echo $!",
        path = shell_quote(search_path),
        script = shell_quote(helper_script()),
        token = shell_quote(&token.to_string_lossy()),
    )
}

#[must_use]
pub fn restore_command(search_path: &str) -> String {
    format!("PATH={}; export PATH; pmset -a disablesleep 0", shell_quote(search_path))
}

fn applescript_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[must_use]
pub fn osascript_args(command: &str, prompt: &str) -> Vec<String> {
    let script = format!(
        "do shell script {} with prompt {} with administrator privileges",
        applescript_string(command),
        applescript_string(prompt),
    );
    vec!["-e".to_owned(), script]
}

#[must_use]
pub fn token_text(until: Option<f64>) -> String {
    until.map_or_else(|| "0".to_owned(), |until| format!("{}", until.max(1.0).round() as u64))
}

pub fn write_token(token: &Path, until: Option<f64>) -> std::io::Result<()> {
    write_atomic(token, &token_text(until))
}

pub fn remove_token(token: &Path) {
    if let Err(e) = std::fs::remove_file(token)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("tidewatch: cannot remove {} ({e})", token.display());
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Marker {
    pub pid: u32,
    pub helper: Option<u32>,
}

#[must_use]
pub fn read_marker(file: &Path) -> Option<Marker> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

pub fn write_marker(file: &Path, marker: &Marker) {
    let Ok(text) = serde_json::to_string(marker) else { return };
    if let Err(e) = write_atomic(file, &text) {
        eprintln!("tidewatch: cannot write {} ({e})", file.display());
    }
}

pub fn remove_marker(file: &Path) {
    remove_token(file);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    Nothing,
    Stale,
    HelperRestores,
    NeedsPassword,
}

#[must_use]
pub fn recovery(marker: Option<&Marker>, disabled: Option<bool>, helper_alive: bool) -> Recovery {
    match (marker, disabled) {
        (None, _) | (Some(_), None) => Recovery::Nothing,
        (Some(_), Some(false)) => Recovery::Stale,
        (Some(_), Some(true)) if helper_alive => Recovery::HelperRestores,
        (Some(_), Some(true)) => Recovery::NeedsPassword,
    }
}

#[must_use]
pub fn parse_sleep_disabled(pmset_g: &str) -> Option<bool> {
    let line = pmset_g.lines().find(|line| line.trim_start().starts_with("SleepDisabled"))?;
    match line.split_whitespace().nth(1)? {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
