//! The calls Amphetamine makes on the machine, behind one seam: the admin prompt that sets lid mode
//! and starts the root helper, the one that only restores sleep, the Touch ID path (an installed
//! helper run through `sudo -n` after a fingerprint), `pmset -g`, and a liveness check.
//! A test or a dev run uses [`FakeSystem`], which records the calls and never touches the machine.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::amphetamine_helper::{Auth, TOUCH_ID_REASON};

pub trait System: Send + Sync {
    /// Asks for the password once, sets lid mode and starts the helper; the helper's pid.
    fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String>;
    fn restore_lid(&self) -> Result<(), String>;
    fn sleep_disabled(&self) -> Option<bool>;
    fn alive(&self, pid: u32) -> bool;
    /// Both the root-owned helper and its sudoers drop-in are in place.
    fn touch_id_installed(&self) -> bool;
    /// Asks for the password once and installs the helper and the drop-in.
    fn install_touch_id(&self) -> Result<(), String>;
    /// Removes both files, through the helper when `sudo -n` works, else with a password.
    fn remove_touch_id(&self) -> Result<(), String>;
    /// Touch ID, or the login password when this Mac has no fingerprint.
    fn authenticate(&self, reason: &str) -> Auth;
    /// Starts lid mode through the installed helper with no prompt; the helper's pid.
    fn sudo_start(&self, token: &Path, owner: u32) -> Result<u32, String>;
    /// Turns sleep back on through the installed helper with no prompt.
    fn sudo_stop(&self) -> Result<(), String>;
}

/// Starts lid mode the way this Mac allows: a fingerprint and the installed helper, else the
/// password prompt. Only a refused fingerprint stops the start; a missing helper, a sudo rule that
/// no longer works or a Mac without biometrics falls back.
pub fn begin_lid(system: &dyn System, token: &Path, owner: u32) -> Result<u32, String> {
    if system.touch_id_installed() {
        match system.authenticate(TOUCH_ID_REASON) {
            Auth::Approved => match system.sudo_start(token, owner) {
                Ok(pid) => return Ok(pid),
                Err(why) => {
                    eprintln!(
                        "tidewatch: the Touch ID helper failed ({why}); asking for the password"
                    );
                }
            },
            Auth::Denied(why) => return Err(why),
            Auth::Unavailable(why) => {
                eprintln!("tidewatch: Touch ID is unavailable ({why}); asking for the password");
            }
        }
    }
    system.start_lid(token, owner)
}

/// Turns sleep back on at once when the helper is installed. The root loop of a session started
/// with a password restores it by itself once the token goes, so a failure here is not an error.
pub fn end_lid(system: &dyn System) {
    if system.touch_id_installed()
        && let Err(why) = system.sudo_stop()
    {
        eprintln!("tidewatch: the helper could not turn sleep back on at once ({why})");
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    StartLid { token: PathBuf, owner: u32 },
    RestoreLid,
    SleepDisabled,
    Alive(u32),
    InstallTouchId,
    RemoveTouchId,
    Authenticate,
    SudoStart { token: PathBuf, owner: u32 },
    SudoStop,
}

#[derive(Debug, Default)]
struct Fake {
    calls: Vec<Call>,
    token: Option<PathBuf>,
    disabled: bool,
    helper_alive: bool,
    refuse: Option<String>,
    installed: bool,
    auth: Option<Auth>,
    sudo_refuse: Option<String>,
    quirks: Quirks,
}

#[derive(Debug, Default)]
struct Quirks {
    sleep_stays_on: bool,
    helper_ignores_token: bool,
}

#[derive(Debug, Default)]
pub struct FakeSystem {
    state: Mutex<Fake>,
}

impl FakeSystem {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn refusing(why: &str) -> Self {
        let fake = Self::new();
        fake.state().refuse = Some(why.to_owned());
        fake
    }

    /// A Mac with the Touch ID helper installed, whose fingerprint is accepted.
    #[must_use]
    pub fn with_touch_id() -> Self {
        let fake = Self::new();
        fake.state().installed = true;
        fake
    }

    /// The fingerprint prompt answers `auth` instead of approving.
    #[must_use]
    pub fn answering(self, auth: Auth) -> Self {
        self.state().auth = Some(auth);
        self
    }

    /// `sudo -n` fails, as it does once the drop-in is gone.
    #[must_use]
    pub fn sudo_refusing(self, why: &str) -> Self {
        self.state().sudo_refuse = Some(why.to_owned());
        self
    }

    /// A start that reports success but leaves sleep on, as a `pmset` that did nothing would.
    #[must_use]
    pub fn sleep_stays_on(self) -> Self {
        self.state().quirks.sleep_stays_on = true;
        self
    }

    /// A helper that never lets go when the token goes, as a dead one would not.
    #[must_use]
    pub fn helper_ignoring_the_token(self) -> Self {
        self.state().quirks.helper_ignores_token = true;
        self
    }

    /// A machine a dead run left with sleep off.
    #[must_use]
    pub fn left_disabled(helper_alive: bool) -> Self {
        let fake = Self::new();
        {
            let mut state = fake.state();
            state.disabled = true;
            state.helper_alive = helper_alive;
        }
        fake
    }

    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.state().calls.clone()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, Fake> {
        self.state.lock().expect("a poisoned fake means a test already failed")
    }

    fn begin(state: &mut Fake, token: &Path) -> u32 {
        state.token = Some(token.to_owned());
        state.disabled = !state.quirks.sleep_stays_on;
        state.helper_alive = true;
        4242
    }
}

impl System for FakeSystem {
    fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String> {
        let mut state = self.state();
        state.calls.push(Call::StartLid { token: token.to_owned(), owner });
        if let Some(why) = state.refuse.clone() {
            return Err(why);
        }
        Ok(Self::begin(&mut state, token))
    }

    fn restore_lid(&self) -> Result<(), String> {
        let mut state = self.state();
        state.calls.push(Call::RestoreLid);
        state.disabled = false;
        Ok(())
    }

    /// The fake helper restores sleep as soon as the token file is gone, like the real one.
    fn sleep_disabled(&self) -> Option<bool> {
        let mut state = self.state();
        state.calls.push(Call::SleepDisabled);
        if !state.quirks.helper_ignores_token
            && state.token.as_ref().is_some_and(|token| !token.exists())
        {
            state.token = None;
            state.disabled = false;
            state.helper_alive = false;
        }
        Some(state.disabled)
    }

    fn alive(&self, pid: u32) -> bool {
        let mut state = self.state();
        state.calls.push(Call::Alive(pid));
        state.helper_alive
    }

    fn touch_id_installed(&self) -> bool {
        self.state().installed
    }

    fn install_touch_id(&self) -> Result<(), String> {
        let mut state = self.state();
        state.calls.push(Call::InstallTouchId);
        if let Some(why) = state.refuse.clone() {
            return Err(why);
        }
        state.installed = true;
        Ok(())
    }

    fn remove_touch_id(&self) -> Result<(), String> {
        let mut state = self.state();
        state.calls.push(Call::RemoveTouchId);
        if let Some(why) = state.refuse.clone() {
            return Err(why);
        }
        state.installed = false;
        Ok(())
    }

    fn authenticate(&self, _reason: &str) -> Auth {
        let mut state = self.state();
        state.calls.push(Call::Authenticate);
        state.auth.clone().unwrap_or(Auth::Approved)
    }

    fn sudo_start(&self, token: &Path, owner: u32) -> Result<u32, String> {
        let mut state = self.state();
        state.calls.push(Call::SudoStart { token: token.to_owned(), owner });
        if let Some(why) = state.sudo_refuse.clone() {
            return Err(why);
        }
        Ok(Self::begin(&mut state, token))
    }

    fn sudo_stop(&self) -> Result<(), String> {
        let mut state = self.state();
        state.calls.push(Call::SudoStop);
        if let Some(why) = state.sudo_refuse.clone() {
            return Err(why);
        }
        state.disabled = false;
        state.helper_alive = false;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub use mac::{Authenticator, RealSystem};
#[cfg(not(target_os = "macos"))]
pub use other::RealSystem;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::Path;
    use std::process::{Command, Stdio};

    use super::System;
    use crate::amphetamine_helper::{
        Auth, HELPER_PATH, REMOVE_PROMPT, SETUP_PROMPT, SUDO, SUDOERS_PATH, install_command,
        parse_pid, remove_command, sudo_start_args, sudo_verb_args, valid_user,
    };
    use crate::amphetamine_lid::{
        PROMPT, RESTORE_PROMPT, SYSTEM_PATH, admin_command, osascript_args, parse_sleep_disabled,
        restore_command,
    };

    /// Asks the system for the fingerprint (or the password); supplied by the app, which owns the
    /// Objective-C bindings, so this crate stays free of `unsafe`.
    pub type Authenticator = fn(&str) -> Auth;

    #[derive(Default)]
    pub struct RealSystem {
        authenticator: Option<Authenticator>,
    }

    impl RealSystem {
        #[must_use]
        pub fn with_authenticator(authenticator: Authenticator) -> Self {
            Self { authenticator: Some(authenticator) }
        }
    }

    fn sudo(args: &[String]) -> Result<String, String> {
        let output = Command::new(SUDO)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("sudo: {e}"))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }

    fn account() -> Result<String, String> {
        let output = Command::new("/usr/bin/id")
            .arg("-un")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("id: {e}"))?;
        let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if valid_user(&name) {
            Ok(name)
        } else {
            Err(format!("cannot use the account name {name:?}"))
        }
    }

    fn admin(command: &str, prompt: &str) -> Result<String, String> {
        let output = Command::new("/usr/bin/osascript")
            .args(osascript_args(command, prompt))
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("osascript: {e}"))?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        let text = String::from_utf8_lossy(&output.stderr);
        if text.contains("-128") {
            return Err("the password prompt was cancelled".to_owned());
        }
        Err(text.trim().to_owned())
    }

    impl System for RealSystem {
        fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String> {
            let pid = admin(&admin_command(token, owner, SYSTEM_PATH), PROMPT)?;
            pid.parse().map_err(|_| format!("the helper did not report its pid ({pid})"))
        }

        fn restore_lid(&self) -> Result<(), String> {
            if self.touch_id_installed() && self.sudo_stop().is_ok() {
                return Ok(());
            }
            admin(&restore_command(SYSTEM_PATH), RESTORE_PROMPT).map(|_| ())
        }

        fn sleep_disabled(&self) -> Option<bool> {
            let output =
                Command::new("/usr/bin/pmset").arg("-g").stdin(Stdio::null()).output().ok()?;
            parse_sleep_disabled(&String::from_utf8_lossy(&output.stdout))
        }

        fn alive(&self, pid: u32) -> bool {
            Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "pid="])
                .stdin(Stdio::null())
                .output()
                .is_ok_and(|out| !out.stdout.is_empty())
        }

        fn touch_id_installed(&self) -> bool {
            Path::new(HELPER_PATH).exists() && Path::new(SUDOERS_PATH).exists()
        }

        fn install_touch_id(&self) -> Result<(), String> {
            let command = install_command(&account()?, SYSTEM_PATH)
                .ok_or_else(|| "cannot use this account name".to_owned())?;
            admin(&command, SETUP_PROMPT).map(|_| ())
        }

        fn remove_touch_id(&self) -> Result<(), String> {
            if sudo(&sudo_verb_args("uninstall")).is_ok() {
                return Ok(());
            }
            admin(&remove_command(SYSTEM_PATH), REMOVE_PROMPT).map(|_| ())
        }

        fn authenticate(&self, reason: &str) -> Auth {
            self.authenticator.map_or_else(
                || Auth::Unavailable("no fingerprint prompt in this build".to_owned()),
                |ask| ask(reason),
            )
        }

        fn sudo_start(&self, token: &Path, owner: u32) -> Result<u32, String> {
            parse_pid(&sudo(&sudo_start_args(token, owner))?)
        }

        fn sudo_stop(&self) -> Result<(), String> {
            sudo(&sudo_verb_args("stop")).map(|_| ())
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use std::path::Path;

    use super::System;
    use crate::amphetamine_helper::Auth;

    #[derive(Default)]
    pub struct RealSystem;

    impl System for RealSystem {
        fn start_lid(&self, _token: &Path, _owner: u32) -> Result<u32, String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }

        fn restore_lid(&self) -> Result<(), String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }

        fn sleep_disabled(&self) -> Option<bool> {
            None
        }

        fn alive(&self, _pid: u32) -> bool {
            false
        }

        fn touch_id_installed(&self) -> bool {
            false
        }

        fn install_touch_id(&self) -> Result<(), String> {
            Err("Touch ID for Amphetamine is only on macOS".to_owned())
        }

        fn remove_touch_id(&self) -> Result<(), String> {
            Err("Touch ID for Amphetamine is only on macOS".to_owned())
        }

        fn authenticate(&self, _reason: &str) -> Auth {
            Auth::Unavailable("only on macOS".to_owned())
        }

        fn sudo_start(&self, _token: &Path, _owner: u32) -> Result<u32, String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }

        fn sudo_stop(&self) -> Result<(), String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }
    }
}

#[cfg(test)]
mod tests;
