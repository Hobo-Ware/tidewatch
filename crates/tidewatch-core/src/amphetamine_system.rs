//! The calls Amphetamine makes on the machine, behind one seam: the admin prompt that sets lid mode
//! and starts the root helper, the one that only restores sleep, `pmset -g`, and a liveness check.
//! A test or a dev run uses [`FakeSystem`], which records the calls and never touches the machine.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub trait System: Send + Sync {
    /// Asks for the password once, sets lid mode and starts the helper; the helper's pid.
    fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String>;
    fn restore_lid(&self) -> Result<(), String>;
    fn sleep_disabled(&self) -> Option<bool>;
    fn alive(&self, pid: u32) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    StartLid { token: PathBuf, owner: u32 },
    RestoreLid,
    SleepDisabled,
    Alive(u32),
}

#[derive(Debug, Default)]
struct Fake {
    calls: Vec<Call>,
    token: Option<PathBuf>,
    disabled: bool,
    helper_alive: bool,
    refuse: Option<String>,
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
}

impl System for FakeSystem {
    fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String> {
        let mut state = self.state();
        state.calls.push(Call::StartLid { token: token.to_owned(), owner });
        if let Some(why) = state.refuse.clone() {
            return Err(why);
        }
        state.token = Some(token.to_owned());
        state.disabled = true;
        state.helper_alive = true;
        Ok(4242)
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
        if state.token.as_ref().is_some_and(|token| !token.exists()) {
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
}

#[cfg(target_os = "macos")]
pub use mac::RealSystem;
#[cfg(not(target_os = "macos"))]
pub use other::RealSystem;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::Path;
    use std::process::{Command, Stdio};

    use super::System;
    use crate::amphetamine_lid::{
        PROMPT, RESTORE_PROMPT, SYSTEM_PATH, admin_command, osascript_args, parse_sleep_disabled,
        restore_command,
    };

    #[derive(Default)]
    pub struct RealSystem;

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
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use std::path::Path;

    use super::System;

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
    }
}

#[cfg(test)]
mod tests;
