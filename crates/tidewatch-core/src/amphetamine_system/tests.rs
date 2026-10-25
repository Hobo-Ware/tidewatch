use std::path::Path;

use super::*;

#[test]
fn the_fake_records_every_call_and_the_helper_restores_when_the_token_goes() {
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("token");
    std::fs::write(&token, "0").unwrap();
    let fake = FakeSystem::new();
    assert_eq!(fake.start_lid(&token, 77), Ok(4242));
    assert_eq!(fake.sleep_disabled(), Some(true));
    assert!(fake.alive(4242));
    std::fs::remove_file(&token).unwrap();
    assert_eq!(fake.sleep_disabled(), Some(false));
    assert!(!fake.alive(4242));
    assert_eq!(
        fake.calls(),
        [
            Call::StartLid { token: token.clone(), owner: 77 },
            Call::SleepDisabled,
            Call::Alive(4242),
            Call::SleepDisabled,
            Call::Alive(4242),
        ]
    );
}

#[test]
fn a_refusing_fake_never_disables_sleep() {
    let fake = FakeSystem::refusing("the password prompt was cancelled");
    assert_eq!(
        fake.start_lid(Path::new("/x"), 1),
        Err("the password prompt was cancelled".to_owned())
    );
    assert_eq!(fake.sleep_disabled(), Some(false));
}

#[test]
fn a_run_that_died_leaves_sleep_off_until_restored() {
    let fake = FakeSystem::left_disabled(false);
    assert_eq!(fake.sleep_disabled(), Some(true));
    assert!(!fake.alive(1));
    assert_eq!(fake.restore_lid(), Ok(()));
    assert_eq!(fake.sleep_disabled(), Some(false));
}

fn token() -> std::path::PathBuf {
    std::path::PathBuf::from("/x/.state/amphetamine.token")
}

fn started(token: &std::path::Path, owner: u32) -> Call {
    Call::StartLid { token: token.to_owned(), owner }
}

#[test]
fn without_the_helper_the_password_prompt_runs_and_no_fingerprint_is_asked() {
    let fake = FakeSystem::new();
    assert_eq!(begin_lid(&fake, &token(), 5), Ok(4242));
    assert_eq!(fake.calls(), [started(&token(), 5)]);
}

#[test]
fn with_the_helper_a_fingerprint_then_sudo_starts_it_with_no_password() {
    let fake = FakeSystem::with_touch_id();
    assert_eq!(begin_lid(&fake, &token(), 5), Ok(4242));
    assert_eq!(fake.calls(), [Call::Authenticate, Call::SudoStart { token: token(), owner: 5 }]);
}

#[test]
fn a_refused_fingerprint_stops_the_start_and_asks_for_nothing_else() {
    let fake =
        FakeSystem::with_touch_id().answering(Auth::Denied("Touch ID was cancelled".to_owned()));
    assert_eq!(begin_lid(&fake, &token(), 5), Err("Touch ID was cancelled".to_owned()));
    assert_eq!(fake.calls(), [Call::Authenticate]);
}

#[test]
fn a_mac_without_a_fingerprint_prompt_falls_back_to_the_password() {
    let fake = FakeSystem::with_touch_id().answering(Auth::Unavailable("no passcode".to_owned()));
    assert_eq!(begin_lid(&fake, &token(), 5), Ok(4242));
    assert_eq!(fake.calls(), [Call::Authenticate, started(&token(), 5)]);
}

#[test]
fn a_sudo_rule_that_stopped_working_falls_back_to_the_password() {
    let fake = FakeSystem::with_touch_id().sudo_refusing("a password is required");
    assert_eq!(begin_lid(&fake, &token(), 5), Ok(4242));
    assert_eq!(
        fake.calls(),
        [Call::Authenticate, Call::SudoStart { token: token(), owner: 5 }, started(&token(), 5)]
    );
}

fn setup() -> Call {
    Call::InstallAndStart { token: token(), owner: 5 }
}

fn sudo() -> Call {
    Call::SudoStart { token: token(), owner: 5 }
}

#[test]
fn the_route_follows_the_helper_and_the_fingerprint_reader() {
    let cases = [
        (true, true, Route::Fingerprint),
        (true, false, Route::Fingerprint),
        (false, true, Route::SetUp),
        (false, false, Route::Password),
    ];
    for (installed, biometrics, want) in cases {
        assert_eq!(Route::of(installed, biometrics), want, "{installed} {biometrics}");
    }
}

#[test]
fn every_way_a_start_can_go_asks_for_the_right_things_in_the_right_order() {
    let cancelled = PROMPT_CANCELLED.to_owned();
    let denied = Auth::Denied("Touch ID was cancelled".to_owned());
    let unavailable = Auth::Unavailable("no passcode".to_owned());
    #[allow(clippy::type_complexity)]
    let cases: Vec<(&str, FakeSystem, Result<u32, String>, Vec<Call>)> = vec![
        (
            "no helper, no reader: the password each time, nothing installed",
            FakeSystem::new(),
            Ok(4242),
            vec![started(&token(), 5)],
        ),
        (
            "no helper, a reader: one prompt installs and starts",
            FakeSystem::with_biometrics(),
            Ok(4242),
            vec![setup()],
        ),
        (
            "that prompt cancelled: stop, no second prompt",
            FakeSystem::refusing(&cancelled).with_reader(),
            Err(cancelled.clone()),
            vec![setup()],
        ),
        (
            "that setup failed: fall back to the password",
            FakeSystem::with_biometrics().setup_refusing("visudo said no"),
            Ok(4242),
            vec![setup(), started(&token(), 5)],
        ),
        (
            "helper, fingerprint ok: sudo",
            FakeSystem::with_touch_id(),
            Ok(4242),
            vec![Call::Authenticate, sudo()],
        ),
        (
            "helper and a reader, fingerprint ok: sudo, no reinstall",
            FakeSystem::with_touch_id().with_reader(),
            Ok(4242),
            vec![Call::Authenticate, sudo()],
        ),
        (
            "helper, fingerprint refused: stop",
            FakeSystem::with_touch_id().answering(denied),
            Err("Touch ID was cancelled".to_owned()),
            vec![Call::Authenticate],
        ),
        (
            "helper, no prompt on this Mac: the password",
            FakeSystem::with_touch_id().answering(unavailable),
            Ok(4242),
            vec![Call::Authenticate, started(&token(), 5)],
        ),
        (
            "helper, sudo rule gone: the password",
            FakeSystem::with_touch_id().sudo_refusing("a password is required"),
            Ok(4242),
            vec![Call::Authenticate, sudo(), started(&token(), 5)],
        ),
    ];
    for (what, fake, result, calls) in cases {
        assert_eq!(begin_lid(&fake, &token(), 5), result, "{what}");
        assert_eq!(fake.calls(), calls, "{what}");
    }
}

#[test]
fn the_first_run_leaves_the_helper_installed_so_the_next_start_asks_for_a_fingerprint() {
    let fake = FakeSystem::with_biometrics();
    assert_eq!(begin_lid(&fake, &token(), 5), Ok(4242));
    assert!(fake.touch_id_installed());
    assert_eq!(begin_lid(&fake, &token(), 5), Ok(4242));
    assert_eq!(fake.calls(), [setup(), Call::Authenticate, sudo()]);
}

#[test]
fn a_cancelled_or_missing_setup_installs_nothing() {
    let cancelled = FakeSystem::refusing(PROMPT_CANCELLED).with_reader();
    let _ = begin_lid(&cancelled, &token(), 5);
    let plain = FakeSystem::new();
    let _ = begin_lid(&plain, &token(), 5);
    assert!(!cancelled.touch_id_installed() && !plain.touch_id_installed());
}

#[test]
fn ending_asks_the_helper_only_when_it_is_installed_and_survives_a_refusal() {
    let plain = FakeSystem::new();
    end_lid(&plain);
    assert_eq!(plain.calls(), []);
    let installed = FakeSystem::with_touch_id();
    end_lid(&installed);
    assert_eq!(installed.calls(), [Call::SudoStop]);
    let broken = FakeSystem::with_touch_id().sudo_refusing("nope");
    end_lid(&broken);
    assert_eq!(broken.calls(), [Call::SudoStop]);
}

#[test]
fn setup_and_removal_flip_the_installed_state_and_a_refused_prompt_changes_nothing() {
    let fake = FakeSystem::new();
    assert!(!fake.touch_id_installed());
    assert_eq!(fake.install_touch_id(), Ok(()));
    assert!(fake.touch_id_installed());
    assert_eq!(fake.remove_touch_id(), Ok(()));
    assert!(!fake.touch_id_installed());
    let refusing = FakeSystem::refusing("the password prompt was cancelled");
    assert!(refusing.install_touch_id().is_err());
    assert!(!refusing.touch_id_installed());
    assert_eq!(fake.calls(), [Call::InstallTouchId, Call::RemoveTouchId]);
}
