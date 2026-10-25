use std::path::{Path, PathBuf};

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

fn token() -> PathBuf {
    PathBuf::from("/x/.state/amphetamine.token")
}

fn fifo() -> PathBuf {
    PathBuf::from("/x/.state/amphetamine.serve")
}

fn watching() -> Started {
    Started { helper: 4242, serving: false }
}

fn serving() -> Started {
    Started { helper: 4242, serving: true }
}

fn started(token: &Path, owner: u32) -> Call {
    Call::StartLid { token: token.to_owned(), owner }
}

fn setup() -> Call {
    Call::InstallAndStart { token: token(), owner: 5, fifo: fifo() }
}

fn sudo() -> Call {
    Call::SudoServe { token: token(), owner: 5, fifo: fifo() }
}

fn scratch() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("amphetamine.token");
    let fifo = dir.path().join("amphetamine.serve");
    std::fs::write(&token, "0").unwrap();
    (dir, token, fifo)
}

#[test]
fn without_the_helper_the_password_prompt_runs_and_no_fingerprint_is_asked() {
    let fake = FakeSystem::new();
    assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), Ok(watching()));
    assert_eq!(fake.calls(), [started(&token(), 5)]);
}

#[test]
fn with_the_helper_a_fingerprint_then_the_helper_serves_with_no_password() {
    let fake = FakeSystem::with_touch_id();
    assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), Ok(serving()));
    assert_eq!(fake.calls(), [Call::Authenticate, Call::MakeFifo(fifo()), sudo()]);
}

#[test]
fn a_refused_fingerprint_stops_the_start_and_asks_for_nothing_else() {
    let fake =
        FakeSystem::with_touch_id().answering(Auth::Denied("Touch ID was cancelled".to_owned()));
    assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), Err("Touch ID was cancelled".to_owned()));
    assert_eq!(fake.calls(), [Call::Authenticate]);
}

#[test]
fn a_mac_without_a_fingerprint_prompt_falls_back_to_the_password() {
    let fake = FakeSystem::with_touch_id().answering(Auth::Unavailable("no passcode".to_owned()));
    assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), Ok(watching()));
    assert_eq!(fake.calls(), [Call::Authenticate, started(&token(), 5)]);
}

#[test]
fn a_sudo_rule_that_stopped_working_falls_back_to_the_password() {
    let fake = FakeSystem::with_touch_id().sudo_refusing("a password is required");
    assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), Ok(watching()));
    assert_eq!(
        fake.calls(),
        [Call::Authenticate, Call::MakeFifo(fifo()), sudo(), started(&token(), 5)]
    );
}

#[test]
fn the_route_follows_the_helper_its_version_and_the_fingerprint_reader() {
    let cases = [
        ((true, true, true), Route::Fingerprint),
        ((true, true, false), Route::Fingerprint),
        ((true, false, true), Route::Outdated),
        ((true, false, false), Route::Outdated),
        ((false, false, true), Route::SetUp),
        ((false, false, false), Route::Password),
    ];
    for ((installed, current, biometrics), want) in cases {
        assert_eq!(
            Route::of(installed, current, biometrics),
            want,
            "{installed} {current} {biometrics}"
        );
    }
}

#[test]
fn an_outdated_helper_is_routed_as_outdated_and_starts_with_the_password() {
    let fake = FakeSystem::with_outdated_helper();
    assert!(fake.touch_id_installed());
    assert!(!fake.touch_id_current());
    assert_eq!(route_of(&fake), Route::Outdated);
    assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), Ok(watching()));
    assert_eq!(fake.calls(), [started(&token(), 5)]);
}

#[test]
fn every_way_a_start_can_go_asks_for_the_right_things_in_the_right_order() {
    let cancelled = PROMPT_CANCELLED.to_owned();
    let denied = Auth::Denied("Touch ID was cancelled".to_owned());
    let unavailable = Auth::Unavailable("no passcode".to_owned());
    #[allow(clippy::type_complexity)]
    let cases: Vec<(&str, FakeSystem, Result<Started, String>, Vec<Call>)> = vec![
        (
            "no helper, no reader: the password each time, nothing installed",
            FakeSystem::new(),
            Ok(watching()),
            vec![started(&token(), 5)],
        ),
        (
            "no helper, a reader: one prompt installs and starts the helper serving",
            FakeSystem::with_biometrics(),
            Ok(serving()),
            vec![Call::MakeFifo(fifo()), setup()],
        ),
        (
            "that prompt cancelled: stop, no second prompt",
            FakeSystem::refusing(&cancelled).with_reader(),
            Err(cancelled.clone()),
            vec![Call::MakeFifo(fifo()), setup()],
        ),
        (
            "that setup failed: fall back to the password",
            FakeSystem::with_biometrics().setup_refusing("visudo said no"),
            Ok(watching()),
            vec![Call::MakeFifo(fifo()), setup(), started(&token(), 5)],
        ),
        (
            "helper, fingerprint ok: sudo serves",
            FakeSystem::with_touch_id(),
            Ok(serving()),
            vec![Call::Authenticate, Call::MakeFifo(fifo()), sudo()],
        ),
        (
            "helper and a reader, fingerprint ok: sudo, no reinstall",
            FakeSystem::with_touch_id().with_reader(),
            Ok(serving()),
            vec![Call::Authenticate, Call::MakeFifo(fifo()), sudo()],
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
            Ok(watching()),
            vec![Call::Authenticate, started(&token(), 5)],
        ),
        (
            "helper, sudo rule gone: the password",
            FakeSystem::with_touch_id().sudo_refusing("a password is required"),
            Ok(watching()),
            vec![Call::Authenticate, Call::MakeFifo(fifo()), sudo(), started(&token(), 5)],
        ),
        (
            "helper older than this build: the password, and setup is asked again",
            FakeSystem::with_outdated_helper(),
            Ok(watching()),
            vec![started(&token(), 5)],
        ),
    ];
    for (what, fake, result, calls) in cases {
        assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), result, "{what}");
        assert_eq!(fake.calls(), calls, "{what}");
    }
}

#[test]
fn the_first_run_leaves_the_helper_installed_so_the_next_start_asks_for_a_fingerprint() {
    let fake = FakeSystem::with_biometrics();
    assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), Ok(serving()));
    assert!(fake.touch_id_installed());
    assert_eq!(begin_lid(&fake, &token(), &fifo(), 5), Ok(serving()));
    assert_eq!(
        fake.calls(),
        [Call::MakeFifo(fifo()), setup(), Call::Authenticate, Call::MakeFifo(fifo()), sudo()]
    );
}

#[test]
fn a_cancelled_or_missing_setup_installs_nothing() {
    let cancelled = FakeSystem::refusing(PROMPT_CANCELLED).with_reader();
    let _ = begin_lid(&cancelled, &token(), &fifo(), 5);
    let plain = FakeSystem::new();
    let _ = begin_lid(&plain, &token(), &fifo(), 5);
    assert!(!cancelled.touch_id_installed() && !plain.touch_id_installed());
}

#[test]
fn a_second_activation_in_the_same_run_sends_a_command_and_does_not_prompt() {
    let (_dir, token, fifo) = scratch();
    let fake = FakeSystem::with_touch_id();
    let first = begin_lid(&fake, &token, &fifo, 5);
    assert_eq!(first, Ok(serving()));
    let until = Some(1_800_000_000.0);
    assert_eq!(start_lid_or_serve(&fake, Some(4242), &token, &fifo, 5, until), Ok(serving()));
    assert_eq!(
        fake.calls(),
        [
            Call::Authenticate,
            Call::MakeFifo(fifo.clone()),
            Call::SudoServe { token: token.clone(), owner: 5, fifo: fifo.clone() },
            Call::Alive(4242),
            Call::SendServe { fifo: fifo.clone(), command: Serve::On(until) },
        ]
    );
}

#[test]
fn a_helper_gone_before_the_next_activation_falls_back_to_the_prompt() {
    let (_dir, token, fifo) = scratch();
    let fake = FakeSystem::with_touch_id();
    assert_eq!(begin_lid(&fake, &token, &fifo, 5), Ok(serving()));
    fake.owner_exits();
    assert_eq!(start_lid_or_serve(&fake, Some(4242), &token, &fifo, 5, None), Ok(serving()));
    let calls = fake.calls();
    assert_eq!(
        calls[3..],
        [
            Call::Alive(4242),
            Call::Authenticate,
            Call::MakeFifo(fifo.clone()),
            Call::SudoServe { token: token.clone(), owner: 5, fifo: fifo.clone() },
        ]
    );
    assert!(!calls.iter().any(|call| matches!(call, Call::SendServe { .. })));
}

#[test]
fn a_serving_helper_that_does_not_answer_is_replaced_by_the_prompting_start() {
    let (_dir, token, fifo) = scratch();
    let fake = FakeSystem::with_touch_id().serve_not_answering();
    assert_eq!(begin_lid(&fake, &token, &fifo, 5), Ok(serving()));
    let until = Some(1_800_000_000.0);
    assert_eq!(start_lid_or_serve(&fake, Some(4242), &token, &fifo, 5, until), Ok(serving()));
    let calls = fake.calls();
    assert_eq!(
        calls[3..],
        [
            Call::Alive(4242),
            Call::SendServe { fifo: fifo.clone(), command: Serve::On(until) },
            Call::Authenticate,
            Call::MakeFifo(fifo.clone()),
            Call::SudoServe { token: token.clone(), owner: 5, fifo: fifo.clone() },
        ]
    );
    assert_eq!(std::fs::read_to_string(&token).unwrap(), "1800000000");
}

#[test]
fn the_helper_restores_sleep_and_exits_when_its_parent_pid_is_gone() {
    let (_dir, token, fifo) = scratch();
    let fake = FakeSystem::with_touch_id();
    assert_eq!(begin_lid(&fake, &token, &fifo, 5), Ok(serving()));
    assert_eq!(fake.sleep_disabled(), Some(true));
    fake.owner_exits();
    assert_eq!(fake.sleep_disabled(), Some(false));
    assert!(!fake.alive(4242));
    assert_eq!(
        fake.send_serve(&token, &fifo, Serve::On(None)),
        Err("the helper did not answer".to_owned())
    );
}

#[test]
fn a_deadline_moves_through_the_serving_helper_or_the_token_without_one() {
    let (_dir, token, fifo) = scratch();
    let fake = FakeSystem::with_touch_id();
    assert_eq!(begin_lid(&fake, &token, &fifo, 5), Ok(serving()));
    assert_eq!(set_deadline(&fake, Some(4242), &token, &fifo, Some(1_900_000_000.0)), Ok(()));
    assert_eq!(
        fake.calls().last(),
        Some(&Call::SendServe { fifo: fifo.clone(), command: Serve::On(Some(1_900_000_000.0)) })
    );
    let plain = FakeSystem::new();
    assert_eq!(set_deadline(&plain, None, &token, &fifo, None), Ok(()));
    assert_eq!(std::fs::read_to_string(&token).unwrap(), "0");
    assert_eq!(plain.calls(), []);
}

#[test]
fn turning_off_tells_the_serving_helper_and_keeps_it_for_the_next_start() {
    let (_dir, token, fifo) = scratch();
    let fake = FakeSystem::with_touch_id();
    assert_eq!(begin_lid(&fake, &token, &fifo, 5), Ok(serving()));
    stop_lid(&fake, Some(4242), &token, &fifo);
    assert_eq!(
        fake.calls().last(),
        Some(&Call::SendServe { fifo: fifo.clone(), command: Serve::Off })
    );
    assert!(fake.alive(4242));
    assert!(token.exists());
}

#[test]
fn turning_off_without_an_answer_removes_the_token_and_stops_the_helper() {
    let (_dir, token, fifo) = scratch();
    let fake = FakeSystem::with_touch_id().serve_not_answering();
    assert_eq!(begin_lid(&fake, &token, &fifo, 5), Ok(serving()));
    stop_lid(&fake, Some(4242), &token, &fifo);
    assert!(!token.exists());
    assert_eq!(fake.calls().last(), Some(&Call::SudoStop));
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
