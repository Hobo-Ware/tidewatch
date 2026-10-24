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
