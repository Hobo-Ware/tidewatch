//! The calls Amphetamine makes on the machine, behind one seam: the admin prompt that sets lid mode
//! and starts the root helper, the one that also installs the Touch ID helper, the one that only
//! restores sleep, the Touch ID path (an installed helper run through `sudo -n` after a
//! fingerprint), `pmset -g`, and a liveness check.
//! A test or a dev run uses [`FakeSystem`], which records the calls and never touches the machine.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::amphetamine_helper::{Auth, Serve, TOUCH_ID_REASON};
use crate::amphetamine_lid::{PROMPT_CANCELLED, remove_token, write_token};

pub trait System: Send + Sync {
    /// Asks for the password once, sets lid mode and starts the helper; the helper's pid.
    fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String>;
    fn restore_lid(&self) -> Result<(), String>;
    fn sleep_disabled(&self) -> Option<bool>;
    fn alive(&self, pid: u32) -> bool;
    fn install_and_start_lid(&self, token: &Path, owner: u32, fifo: &Path) -> Result<u32, String>;
    /// Both the root-owned helper and its sudoers drop-in are in place.
    fn touch_id_installed(&self) -> bool;
    fn touch_id_current(&self) -> bool;
    fn make_fifo(&self, fifo: &Path) -> Result<(), String>;
    fn send_serve(&self, token: &Path, fifo: &Path, command: Serve) -> Result<(), String>;
    /// This Mac has a fingerprint reader with a finger enrolled.
    fn biometrics_available(&self) -> bool;
    /// Asks for the password once and installs the helper and the drop-in.
    fn install_touch_id(&self) -> Result<(), String>;
    /// Removes both files, through the helper when `sudo -n` works, else with a password.
    fn remove_touch_id(&self) -> Result<(), String>;
    /// Touch ID, or the login password when this Mac has no fingerprint.
    fn authenticate(&self, reason: &str) -> Auth;
    fn sudo_serve(&self, token: &Path, owner: u32, fifo: &Path) -> Result<u32, String>;
    /// Turns sleep back on through the installed helper with no prompt.
    fn sudo_stop(&self) -> Result<(), String>;
}

/// How a start asks for permission on this Mac.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The helper is installed and current: a fingerprint, then `sudo -n`.
    Fingerprint,
    Outdated,
    /// No helper yet but a fingerprint reader: one password prompt installs it and starts.
    SetUp,
    /// No helper and no fingerprint reader: the password prompt each time, nothing installed.
    Password,
}

impl Route {
    #[must_use]
    pub fn of(installed: bool, current: bool, biometrics: bool) -> Self {
        match (installed, current, biometrics) {
            (true, true, _) => Self::Fingerprint,
            (true, false, _) => Self::Outdated,
            (false, _, true) => Self::SetUp,
            (false, _, false) => Self::Password,
        }
    }
}

/// The route this machine takes; the fingerprint reader is only asked about while no helper exists.
#[must_use]
pub fn route_of(system: &dyn System) -> Route {
    let installed = system.touch_id_installed();
    Route::of(
        installed,
        installed && system.touch_id_current(),
        !installed && system.biometrics_available(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Started {
    pub helper: u32,
    pub serving: bool,
}

/// Starts lid mode the way this Mac allows. Only a refused fingerprint or a cancelled password
/// prompt stops the start; a missing helper, a sudo rule that no longer works, a failed setup or a
/// Mac without biometrics falls back to the plain password prompt.
pub fn begin_lid(
    system: &dyn System,
    token: &Path,
    fifo: &Path,
    owner: u32,
) -> Result<Started, String> {
    match route_of(system) {
        Route::Fingerprint => match system.authenticate(TOUCH_ID_REASON) {
            Auth::Approved => match serve_with_sudo(system, token, fifo, owner) {
                Ok(helper) => return Ok(Started { helper, serving: true }),
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
        },
        Route::SetUp => {
            let setup = system
                .make_fifo(fifo)
                .and_then(|()| system.install_and_start_lid(token, owner, fifo));
            match setup {
                Ok(helper) => return Ok(Started { helper, serving: true }),
                Err(why) if why == PROMPT_CANCELLED => return Err(why),
                Err(why) => {
                    eprintln!("tidewatch: Touch ID setup failed ({why}); asking for the password");
                }
            }
        }
        Route::Outdated | Route::Password => {}
    }
    system.start_lid(token, owner).map(|helper| Started { helper, serving: false })
}

fn serve_with_sudo(
    system: &dyn System,
    token: &Path,
    fifo: &Path,
    owner: u32,
) -> Result<u32, String> {
    system.make_fifo(fifo)?;
    system.sudo_serve(token, owner, fifo)
}

fn live_helper(system: &dyn System, serving: Option<u32>) -> Option<u32> {
    serving.filter(|&helper| system.alive(helper))
}

/// Starts lid mode, or, while a helper serves this run, sends it the deadline with no prompt. A
/// serving helper that does not answer is replaced by the prompting start.
pub fn start_lid_or_serve(
    system: &dyn System,
    serving: Option<u32>,
    token: &Path,
    fifo: &Path,
    owner: u32,
    until: Option<f64>,
) -> Result<Started, String> {
    if let Some(helper) = live_helper(system, serving) {
        match system.send_serve(token, fifo, Serve::On(until)) {
            Ok(()) => return Ok(Started { helper, serving: true }),
            Err(why) => {
                eprintln!("tidewatch: the serving helper did not answer ({why}); starting again");
            }
        }
    }
    write_token(token, until).map_err(|why| why.to_string())?;
    begin_lid(system, token, fifo, owner)
}

/// Moves the deadline of a running session. The serving helper also watches the token, so the token
/// is written when the command cannot be sent.
pub fn set_deadline(
    system: &dyn System,
    serving: Option<u32>,
    token: &Path,
    fifo: &Path,
    until: Option<f64>,
) -> Result<(), String> {
    if live_helper(system, serving).is_some()
        && system.send_serve(token, fifo, Serve::On(until)).is_ok()
    {
        return Ok(());
    }
    write_token(token, until).map_err(|why| why.to_string())
}

/// Turns sleep back on. A serving helper is told so and keeps serving for the next start of this
/// run; otherwise the token goes, which the watching helper answers by restoring sleep.
pub fn stop_lid(system: &dyn System, serving: Option<u32>, token: &Path, fifo: &Path) {
    if live_helper(system, serving).is_some() {
        match system.send_serve(token, fifo, Serve::Off) {
            Ok(()) => return,
            Err(why) => {
                eprintln!(
                    "tidewatch: the serving helper did not turn sleep off ({why}); stopping it"
                );
            }
        }
    }
    remove_token(token);
    end_lid(system);
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

#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    StartLid { token: PathBuf, owner: u32 },
    InstallAndStart { token: PathBuf, owner: u32, fifo: PathBuf },
    RestoreLid,
    SleepDisabled,
    Alive(u32),
    InstallTouchId,
    RemoveTouchId,
    Authenticate,
    SudoServe { token: PathBuf, owner: u32, fifo: PathBuf },
    SudoStop,
    MakeFifo(PathBuf),
    SendServe { fifo: PathBuf, command: Serve },
}

#[allow(clippy::struct_excessive_bools)] // independent switches of a test double, not a mode
#[derive(Debug, Default)]
struct Fake {
    calls: Vec<Call>,
    token: Option<PathBuf>,
    disabled: bool,
    helper_alive: bool,
    serving: bool,
    refuse: Option<String>,
    installed: bool,
    hold_for: Option<Duration>,
    setup_refuse: Option<String>,
    auth: Option<Auth>,
    sudo_refuse: Option<String>,
    quirks: Quirks,
}

#[allow(clippy::struct_excessive_bools)] // independent switches of a test double, not a mode
#[derive(Debug, Default)]
struct Quirks {
    sleep_stays_on: bool,
    helper_ignores_token: bool,
    biometrics: bool,
    outdated: bool,
    serve_deaf: bool,
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

    #[must_use]
    pub fn with_outdated_helper() -> Self {
        let fake = Self::new();
        {
            let mut state = fake.state();
            state.installed = true;
            state.quirks.outdated = true;
        }
        fake
    }

    #[must_use]
    pub fn serve_not_answering(self) -> Self {
        self.state().quirks.serve_deaf = true;
        self
    }

    pub fn owner_exits(&self) {
        let mut state = self.state();
        state.disabled = false;
        state.helper_alive = false;
        state.serving = false;
    }

    /// A Mac with a fingerprint reader and no helper yet.
    #[must_use]
    pub fn with_biometrics() -> Self {
        Self::new().with_reader()
    }

    /// The Mac has a fingerprint reader, whatever else is true of it.
    #[must_use]
    pub fn with_reader(self) -> Self {
        self.state().quirks.biometrics = true;
        self
    }

    /// Only the combined install and start fails, as a broken install would.
    #[must_use]
    pub fn setup_refusing(self, why: &str) -> Self {
        self.state().setup_refuse = Some(why.to_owned());
        self
    }

    /// Every start waits this long before it answers, as an open password prompt would.
    #[must_use]
    pub fn holding(self, wait: Duration) -> Self {
        self.state().hold_for = Some(wait);
        self
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

    fn wait(&self) {
        let wait = self.state().hold_for;
        if let Some(wait) = wait {
            std::thread::sleep(wait);
        }
    }

    fn begin(state: &mut Fake, token: &Path, serving: bool) -> u32 {
        state.token = Some(token.to_owned());
        state.disabled = !state.quirks.sleep_stays_on;
        state.helper_alive = true;
        state.serving = serving;
        4242
    }
}

impl System for FakeSystem {
    fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String> {
        self.state().calls.push(Call::StartLid { token: token.to_owned(), owner });
        self.wait();
        let mut state = self.state();
        if let Some(why) = state.refuse.clone() {
            return Err(why);
        }
        Ok(Self::begin(&mut state, token, false))
    }

    fn install_and_start_lid(&self, token: &Path, owner: u32, fifo: &Path) -> Result<u32, String> {
        self.state().calls.push(Call::InstallAndStart {
            token: token.to_owned(),
            owner,
            fifo: fifo.to_owned(),
        });
        self.wait();
        let mut state = self.state();
        if let Some(why) = state.refuse.clone().or_else(|| state.setup_refuse.clone()) {
            return Err(why);
        }
        state.installed = true;
        Ok(Self::begin(&mut state, token, true))
    }

    fn touch_id_current(&self) -> bool {
        let state = self.state();
        state.installed && !state.quirks.outdated
    }

    fn make_fifo(&self, fifo: &Path) -> Result<(), String> {
        self.state().calls.push(Call::MakeFifo(fifo.to_owned()));
        Ok(())
    }

    fn send_serve(&self, token: &Path, fifo: &Path, command: Serve) -> Result<(), String> {
        let mut state = self.state();
        state.calls.push(Call::SendServe { fifo: fifo.to_owned(), command });
        if !state.helper_alive || !state.serving || state.quirks.serve_deaf {
            return Err("the helper did not answer".to_owned());
        }
        state.token = Some(token.to_owned());
        state.disabled = matches!(command, Serve::On(_));
        let _ = std::fs::write(token, command.answer());
        Ok(())
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
            && !state.serving
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

    fn biometrics_available(&self) -> bool {
        self.state().quirks.biometrics
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

    fn sudo_serve(&self, token: &Path, owner: u32, fifo: &Path) -> Result<u32, String> {
        let mut state = self.state();
        state.calls.push(Call::SudoServe { token: token.to_owned(), owner, fifo: fifo.to_owned() });
        if let Some(why) = state.sudo_refuse.clone() {
            return Err(why);
        }
        Ok(Self::begin(&mut state, token, true))
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
pub use mac::{Authenticator, Biometrics, RealSystem};
#[cfg(not(target_os = "macos"))]
pub use other::RealSystem;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::Path;
    use std::process::{Command, Stdio};

    use std::io::Write;
    use std::time::Duration;

    use super::System;
    use crate::amphetamine_helper::{
        Auth, HELPER_PATH, REMOVE_PROMPT, SETUP_AND_START_PROMPT, SETUP_PROMPT, SUDO, SUDOERS_PATH,
        Serve, helper_script, install_and_serve_command, install_command, make_command_fifo,
        parse_pid, remove_command, sudo_serve_args, sudo_verb_args, valid_user,
    };
    use crate::amphetamine_lid::{
        PROMPT, PROMPT_CANCELLED, RESTORE_PROMPT, SYSTEM_PATH, admin_command, osascript_args,
        parse_sleep_disabled, restore_command,
    };

    const ANSWER_TRIES: u32 = 30;
    const ANSWER_STEP: Duration = Duration::from_millis(100);

    /// Asks the system for the fingerprint (or the password); supplied by the app, which owns the
    /// Objective-C bindings, so this crate stays free of `unsafe`.
    pub type Authenticator = fn(&str) -> Auth;
    /// Whether a fingerprint reader with an enrolled finger is there; supplied by the app too.
    pub type Biometrics = fn() -> bool;

    #[derive(Default)]
    pub struct RealSystem {
        authenticator: Option<Authenticator>,
        biometrics: Option<Biometrics>,
    }

    impl RealSystem {
        #[must_use]
        pub fn with_authenticator(authenticator: Authenticator, biometrics: Biometrics) -> Self {
            Self { authenticator: Some(authenticator), biometrics: Some(biometrics) }
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
            return Err(PROMPT_CANCELLED.to_owned());
        }
        Err(text.trim().to_owned())
    }

    impl System for RealSystem {
        fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String> {
            let pid = admin(&admin_command(token, owner, SYSTEM_PATH), PROMPT)?;
            pid.parse().map_err(|_| format!("the helper did not report its pid ({pid})"))
        }

        fn install_and_start_lid(
            &self,
            token: &Path,
            owner: u32,
            fifo: &Path,
        ) -> Result<u32, String> {
            let command = install_and_serve_command(&account()?, token, owner, fifo, SYSTEM_PATH)
                .ok_or_else(|| "cannot use this account name".to_owned())?;
            let printed = admin(&command, SETUP_AND_START_PROMPT)?;
            parse_pid(printed.rsplit(['\r', '\n']).next().unwrap_or_default())
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

        fn touch_id_current(&self) -> bool {
            std::fs::read_to_string(HELPER_PATH).is_ok_and(|text| text == helper_script())
        }

        fn make_fifo(&self, fifo: &Path) -> Result<(), String> {
            make_command_fifo(fifo).map_err(|why| why.to_string())
        }

        fn send_serve(&self, token: &Path, fifo: &Path, command: Serve) -> Result<(), String> {
            let mut pipe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(fifo)
                .map_err(|why| why.to_string())?;
            writeln!(pipe, "{}", command.line()).map_err(|why| why.to_string())?;
            drop(pipe);
            let answer = command.answer();
            for _ in 0..ANSWER_TRIES {
                if std::fs::read_to_string(token).is_ok_and(|text| text.trim() == answer) {
                    return Ok(());
                }
                std::thread::sleep(ANSWER_STEP);
            }
            Err("the helper did not answer".to_owned())
        }

        fn biometrics_available(&self) -> bool {
            self.biometrics.is_some_and(|ask| ask())
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

        fn sudo_serve(&self, token: &Path, owner: u32, fifo: &Path) -> Result<u32, String> {
            parse_pid(&sudo(&sudo_serve_args(token, owner, fifo))?)
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
    use crate::amphetamine_helper::{Auth, Serve};

    #[derive(Default)]
    pub struct RealSystem;

    impl System for RealSystem {
        fn start_lid(&self, _token: &Path, _owner: u32) -> Result<u32, String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }

        fn install_and_start_lid(
            &self,
            _token: &Path,
            _owner: u32,
            _fifo: &Path,
        ) -> Result<u32, String> {
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

        fn touch_id_current(&self) -> bool {
            false
        }

        fn make_fifo(&self, _fifo: &Path) -> Result<(), String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }

        fn send_serve(&self, _token: &Path, _fifo: &Path, _command: Serve) -> Result<(), String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }

        fn biometrics_available(&self) -> bool {
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

        fn sudo_serve(&self, _token: &Path, _owner: u32, _fifo: &Path) -> Result<u32, String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }

        fn sudo_stop(&self) -> Result<(), String> {
            Err("lid-closed mode is only on macOS".to_owned())
        }
    }
}

#[cfg(test)]
mod tests;
