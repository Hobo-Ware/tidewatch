//! The operating system's side of keeping awake: one hold that stops the computer from
//! idle-sleeping while the screen may still turn off and lock. macOS takes an IOKit power
//! assertion (`PreventUserIdleSystemSleep`, what `caffeinate -i` takes, but named "Tidewatch" in
//! `pmset -g assertions`); Linux runs `systemd-inhibit` around a `cat` that ends when its stdin
//! closes. Either way the OS drops the hold when this process dies, so a crash cannot leave it on.

pub(crate) const REASON: &str = "Tidewatch: agents are working";

pub(crate) trait Hold {
    /// Takes the hold. Does nothing when already held. The error is the OS text.
    fn take(&mut self, reason: &str) -> Result<(), String>;
    fn release(&mut self);
    /// Whether the OS ended a held hold on its own (the Linux helper process exited).
    fn lost(&mut self) -> bool;
}

#[cfg(not(target_os = "macos"))]
pub(crate) use linux::RealHold;
#[cfg(target_os = "macos")]
pub(crate) use mac::RealHold;

#[cfg(target_os = "macos")]
mod mac {
    use objc2_core_foundation::CFString;
    use objc2_io_kit::{
        IOPMAssertionCreateWithName, IOPMAssertionRelease, kIOPMAssertionLevelOn, kIOReturnSuccess,
    };

    use super::Hold;

    const ASSERTION_TYPE: &str = "PreventUserIdleSystemSleep";

    #[derive(Default)]
    pub(crate) struct RealHold {
        assertion: Option<u32>,
    }

    impl Hold for RealHold {
        /// The binding is `unsafe fn` only because its out pointer must be valid: it points at a
        /// local, and both strings are live CFStrings the call copies.
        #[allow(unsafe_code)]
        fn take(&mut self, reason: &str) -> Result<(), String> {
            if self.assertion.is_some() {
                return Ok(());
            }
            let kind = CFString::from_str(ASSERTION_TYPE);
            let name = CFString::from_str(reason);
            let mut id = 0;
            let status = unsafe {
                IOPMAssertionCreateWithName(
                    Some(&kind),
                    kIOPMAssertionLevelOn,
                    Some(&name),
                    &raw mut id,
                )
            };
            if status != kIOReturnSuccess {
                return Err(format!("IOPMAssertionCreateWithName returned {status}"));
            }
            self.assertion = Some(id);
            Ok(())
        }

        fn release(&mut self) {
            if let Some(id) = self.assertion.take() {
                IOPMAssertionRelease(id);
            }
        }

        fn lost(&mut self) -> bool {
            false
        }
    }

    impl Drop for RealHold {
        fn drop(&mut self) {
            self.release();
        }
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn inhibit_args(reason: &str) -> Vec<String> {
    ["--what=idle:sleep", "--who=Tidewatch", "--mode=block", "--why", reason, "cat"]
        .map(str::to_owned)
        .to_vec()
}

#[cfg(not(target_os = "macos"))]
mod linux {
    use std::process::{Child, Command, Stdio};

    use super::{Hold, inhibit_args};

    #[derive(Default)]
    pub(crate) struct RealHold {
        helper: Option<Child>,
    }

    impl Hold for RealHold {
        /// `systemd-inhibit` holds the inhibitor while `cat` runs; `cat` runs while this process
        /// keeps the write end of its stdin open, which the OS closes if we die.
        fn take(&mut self, reason: &str) -> Result<(), String> {
            if self.helper.is_some() {
                return Ok(());
            }
            let child = Command::new("systemd-inhibit")
                .args(inhibit_args(reason))
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("systemd-inhibit: {e}"))?;
            self.helper = Some(child);
            Ok(())
        }

        fn release(&mut self) {
            if let Some(mut child) = self.helper.take() {
                drop(child.stdin.take());
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        fn lost(&mut self) -> bool {
            let ended =
                self.helper.as_mut().is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
            if ended {
                self.helper = None;
            }
            ended
        }
    }

    impl Drop for RealHold {
        fn drop(&mut self) {
            self.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linux_command_blocks_idle_and_sleep_with_the_reason_shown() {
        assert_eq!(
            inhibit_args(REASON),
            [
                "--what=idle:sleep",
                "--who=Tidewatch",
                "--mode=block",
                "--why",
                "Tidewatch: agents are working",
                "cat"
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_real_assertion_is_taken_once_and_released() {
        let mut hold = RealHold::default();
        assert_eq!(hold.take("Tidewatch test: hold"), Ok(()));
        assert_eq!(hold.take("Tidewatch test: hold"), Ok(()));
        assert!(!hold.lost());
        hold.release();
        hold.release();
    }
}
