use super::*;

fn found() -> Detected {
    Detected {
        cli: Cli::Claude,
        session_id: Some("7d198e16-6157-439f-a389-34a4951f5dd8".into()),
        level: Some(Level::Edits),
        working: false,
        cwd: PathBuf::from("/work/web"),
    }
}

#[test]
fn a_scan_starts_only_while_the_window_shows_and_a_shell_tab_exists() {
    let cases = [(true, 1, true), (false, 1, false), (true, 0, false)];
    for (visible, shells, want) in cases {
        assert_eq!(
            ShellAgents::default().begin_scan(0.0, visible, shells),
            want,
            "{visible} {shells}"
        );
    }
}

#[test]
fn scans_come_one_at_a_time_and_a_few_seconds_apart() {
    let mut agents = ShellAgents::default();
    assert!(agents.begin_scan(10.0, true, 2));
    assert!(!agents.begin_scan(10.1, true, 2), "one at a time");
    agents.finish(Vec::new());
    assert!(!agents.begin_scan(10.0 + EVERY_S - 0.5, true, 2), "too soon");
    assert_eq!(agents.next_in(11.0, true, 2), Some(EVERY_S - 1.0));
    assert!(agents.begin_scan(10.0 + EVERY_S, true, 2));
}

#[test]
fn a_failed_scan_frees_the_next_one() {
    let mut agents = ShellAgents::default();
    assert!(agents.begin_scan(0.0, true, 1));
    agents.failed();
    assert!(agents.begin_scan(EVERY_S, true, 1));
}

#[test]
fn what_a_scan_found_replaces_what_the_last_one_found() {
    let mut agents = ShellAgents::default();
    agents.finish(vec![(3, found()), (4, found())]);
    assert!(agents.found(3).is_some() && agents.found(4).is_some());
    agents.finish(vec![(4, found())]);
    assert!(agents.found(3).is_none() && agents.found(4).is_some());
}

#[test]
fn a_session_the_tab_ended_is_remembered_until_the_tab_forgets_it() {
    let mut agents = ShellAgents::default();
    agents.finish(vec![(3, found())]);
    let kept = Remembered { cli: Cli::Claude, session_id: "id".into() };
    agents.remember(3, kept.clone());
    assert_eq!((agents.found(3), agents.remembered(3)), (None, Some(&kept)));
    agents.forget(3);
    assert_eq!(agents.remembered(3), None);
}

#[test]
fn the_stand_in_reports_a_found_session_on_the_first_shell_tab_only() {
    let cases = [
        (Some("1"), Fake::Idle, false),
        (Some("working"), Fake::Working, true),
        (Some("0"), Fake::Off, false),
        (None, Fake::Off, false),
    ];
    for (text, fake, working) in cases {
        assert_eq!(Fake::parse(text), fake, "{text:?}");
        let mut agents = ShellAgents::new(fake);
        agents.note_fake(Some(7));
        assert_eq!(agents.found(7).is_some(), fake != Fake::Off);
        assert_eq!(agents.found(7).is_some_and(|d| d.working), working);
        assert!(agents.found(8).is_none());
        assert_eq!(agents.next_in(0.0, true, 1).is_none(), fake != Fake::Off);
    }
}

#[test]
fn a_session_that_is_not_running_is_not_detected() {
    let mut session = Session {
        cli: Cli::Claude,
        session_id: None,
        cwd: PathBuf::from("/work/web"),
        name: None,
        flags: tidewatch_core::elsewhere_flags::Flags::default(),
        presence: tidewatch_core::elsewhere::Presence::NotRunning,
        since_ms: None,
    };
    assert_eq!(Detected::of(&session), None);
    session.presence =
        tidewatch_core::elsewhere::Presence::Running(tidewatch_core::elsewhere::Running {
            pid: 1,
            proc_start: "x".into(),
            host: tidewatch_core::elsewhere::Host::Other,
            activity: Activity::Working,
        });
    assert!(Detected::of(&session).unwrap().working);
}
