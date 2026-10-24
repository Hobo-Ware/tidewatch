//! The system fingerprint prompt for Amphetamine (LocalAuthentication, "device owner" policy: Touch
//! ID, with the login password as its own fallback). It blocks until the person answers, so it runs
//! on a `Jobs` thread. Never run from a test.

#[cfg(any(target_os = "macos", test))]
use tidewatch_core::amphetamine_helper::Auth;

#[cfg(any(target_os = "macos", test))]
const AUTHENTICATION_FAILED: isize = -1;
#[cfg(any(target_os = "macos", test))]
const USER_CANCEL: isize = -2;
#[cfg(any(target_os = "macos", test))]
const USER_FALLBACK: isize = -3;
#[cfg(any(target_os = "macos", test))]
const SYSTEM_CANCEL: isize = -4;
#[cfg(any(target_os = "macos", test))]
const APP_CANCEL: isize = -9;

/// Which answers end the start (the person said no) and which mean this Mac cannot ask at all.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn auth_from_error(code: isize, text: &str) -> Auth {
    match code {
        USER_CANCEL | USER_FALLBACK | SYSTEM_CANCEL | APP_CANCEL => {
            Auth::Denied("Touch ID was cancelled".to_owned())
        }
        AUTHENTICATION_FAILED => Auth::Denied("Touch ID did not recognise you".to_owned()),
        _ => Auth::Unavailable(format!("{text} ({code})")),
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub(crate) fn authenticate(reason: &str) -> Auth {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    let policy = LAPolicy::DeviceOwnerAuthentication;
    let context = unsafe { LAContext::new() };
    if let Err(error) = unsafe { context.canEvaluatePolicy_error(policy) } {
        return auth_from_error(error.code(), &error.localizedDescription().to_string());
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let reply = RcBlock::new(move |ok: Bool, error: *mut NSError| {
        let answer = if ok.as_bool() {
            Auth::Approved
        } else {
            let error = unsafe { error.as_ref() };
            error.map_or_else(
                || Auth::Unavailable("no answer".to_owned()),
                |e| auth_from_error(e.code(), &e.localizedDescription().to_string()),
            )
        };
        let _ = sender.send(answer);
    });
    unsafe {
        context.evaluatePolicy_localizedReason_reply(policy, &NSString::from_str(reason), &reply);
    }
    receiver.recv().unwrap_or_else(|_| Auth::Unavailable("no answer".to_owned()))
}

#[cfg(test)]
mod tests;
