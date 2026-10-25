//! The Touch ID path of Amphetamine as strings: the root-owned helper that is installed once, the
//! sudoers drop-in that lets one user run only that helper without a password, the one admin
//! command that installs or removes both, and the `sudo -n` arguments that use it. Nothing here
//! touches the machine.

use std::path::Path;

use crate::amphetamine_lid::{admin_command, shell_quote};

pub const HELPER_PATH: &str = "/Library/PrivilegedHelperTools/tidewatch-amphetamine";
pub const SUDOERS_PATH: &str = "/etc/sudoers.d/tidewatch-amphetamine";
pub const SUDO: &str = "/usr/bin/sudo";

pub const SETUP_PROMPT: &str =
    "Tidewatch wants to install a small helper so Touch ID can keep this Mac awake.";
pub const SETUP_AND_START_PROMPT: &str = "Tidewatch wants to keep this Mac awake with the lid closed. It also installs a small helper so Touch ID can do this from now on.";
pub const REMOVE_PROMPT: &str = "Tidewatch wants to remove its Touch ID helper.";
pub const TOUCH_ID_REASON: &str = "Tidewatch wants to keep this Mac awake with the lid closed.";

const TOKEN_TAIL: &str = ".state/amphetamine.token";

/// What the Touch ID prompt came back with. Only `Unavailable` falls back to the password prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    Approved,
    Denied(String),
    Unavailable(String),
}

#[must_use]
pub fn helper_script() -> String {
    format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH
SELF={self_path}
SUDOERS={sudoers_path}

valid_token() {{
  case $1 in
    /*) ;;
    *) return 1;;
  esac
  case $1 in
    *[!A-Za-z0-9._/@+-]*|*..*) return 1;;
  esac
  case $1 in
    */{tail}) return 0;;
  esac
  return 1
}}

valid_pid() {{
  case $1 in
    ''|*[!0-9]*) return 1;;
  esac
  return 0
}}

watch() {{
  token=$1
  owner=$2
  trap 'pmset -a disablesleep 0; exit 0' TERM INT
  while :; do
    deadline=$(cat "$token" 2>/dev/null) || break
    case $deadline in ''|*[!0-9]*) break;; esac
    kill -0 "$owner" 2>/dev/null || break
    if [ "$deadline" -gt 0 ] && [ "$(date +%s)" -ge "$deadline" ]; then break; fi
    sleep 2
  done
  pmset -a disablesleep 0
}}

main() {{
  case ${{1:-}} in
    start)
      valid_token "${{2:-}}" && valid_pid "${{3:-}}" || exit 2
      pmset -a disablesleep 1 || exit 1
      ( trap '' HUP; watch "$2" "$3" ) >/dev/null 2>&1 </dev/null &
      echo $!
      ;;
    stop) pmset -a disablesleep 0;;
    uninstall) rm -f "$SUDOERS" "$SELF";;
    *) exit 2;;
  esac
}}

main "$@"
"#,
        self_path = shell_quote(HELPER_PATH),
        sudoers_path = shell_quote(SUDOERS_PATH),
        tail = TOKEN_TAIL,
    )
}

/// A short account name, as macOS makes them: nothing a sudoers line could read as syntax, and not
/// the shape of a sudoers alias (`ALL` would name everyone).
#[must_use]
pub fn valid_user(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let alias_shaped =
        name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    first_ok
        && !alias_shaped
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[must_use]
pub fn sudoers_text(user: &str) -> Option<String> {
    valid_user(user).then(|| {
        format!(
            "# Installed by Tidewatch: lets {user} run only the Amphetamine helper without a password.\n\
             {user} ALL=(root) NOPASSWD: {HELPER_PATH}\n"
        )
    })
}

/// The one admin command that installs both files. The drop-in is checked with `visudo -cf` before
/// it moves, and lands under a name sudo ignores until the final rename, so a broken sudoers file
/// is never in place. The helper's folder must already belong to root, or a user could swap the file.
#[must_use]
pub fn install_command(user: &str, search_path: &str) -> Option<String> {
    let sudoers = sudoers_text(user)?;
    let dir = Path::new(HELPER_PATH).parent()?.to_string_lossy().into_owned();
    Some(format!(
        "PATH={path}; export PATH; set -e; \
         work=$(mktemp -d /tmp/tidewatch-setup.XXXXXX); trap 'rm -rf \"$work\"' EXIT; \
         printf %s {helper} > \"$work/helper\"; \
         printf %s {sudoers} > \"$work/sudoers\"; \
         visudo -cf \"$work/sudoers\" >/dev/null; \
         mkdir -p {dir}; \
         [ \"$(stat -f %u {dir})\" = 0 ] || exit 1; \
         install -o root -g wheel -m 0755 \"$work/helper\" {helper_path}.new; \
         mv -f {helper_path}.new {helper_path}; \
         install -o root -g wheel -m 0440 \"$work/sudoers\" {sudoers_path}.new; \
         mv -f {sudoers_path}.new {sudoers_path}",
        path = shell_quote(search_path),
        helper = shell_quote(&helper_script()),
        sudoers = shell_quote(&sudoers),
        dir = shell_quote(&dir),
        helper_path = shell_quote(HELPER_PATH),
        sudoers_path = shell_quote(SUDOERS_PATH),
    ))
}

/// First use on a Mac with a fingerprint reader: the install above, then the password-path start,
/// in one admin script so one prompt does both. `set -e` stops the start if the install fails, and
/// the pid the start prints is the last line.
#[must_use]
pub fn install_and_start_command(
    user: &str,
    token: &Path,
    owner: u32,
    search_path: &str,
) -> Option<String> {
    Some(format!(
        "{}; {}",
        install_command(user, search_path)?,
        admin_command(token, owner, search_path)
    ))
}

#[must_use]
pub fn remove_command(search_path: &str) -> String {
    format!(
        "PATH={}; export PATH; rm -f {} {}",
        shell_quote(search_path),
        shell_quote(SUDOERS_PATH),
        shell_quote(HELPER_PATH),
    )
}

#[must_use]
pub fn sudo_start_args(token: &Path, owner: u32) -> Vec<String> {
    vec![
        "-n".to_owned(),
        HELPER_PATH.to_owned(),
        "start".to_owned(),
        token.to_string_lossy().into_owned(),
        owner.to_string(),
    ]
}

#[must_use]
pub fn sudo_verb_args(verb: &str) -> Vec<String> {
    vec!["-n".to_owned(), HELPER_PATH.to_owned(), verb.to_owned()]
}

pub fn parse_pid(output: &str) -> Result<u32, String> {
    output
        .trim()
        .parse()
        .map_err(|_| format!("the helper did not report its pid ({})", output.trim()))
}

#[cfg(test)]
mod tests;
