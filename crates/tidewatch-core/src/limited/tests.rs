use super::*;

fn sh(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn a_fast_child_keeps_its_output_and_status() {
    let out = output(&mut sh("echo out; echo err 1>&2; exit 3"), Duration::from_secs(30)).unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "out\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "err\n");
}

#[test]
fn a_child_that_never_ends_is_stopped_with_a_message_naming_program_and_limit() {
    let started = Instant::now();
    let error = output(&mut sh("sleep 30"), Duration::from_millis(200)).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(is_stopped(&error));
    assert_eq!(error.to_string(), "sh took longer than 200 ms and was stopped");
    assert_eq!(failure("sh", &error), error.to_string());
}

#[test]
fn a_stopped_run_keeps_what_the_child_had_printed() {
    let Ended::Stopped(out) =
        run(&mut sh("echo before; echo err 1>&2; sleep 30"), Duration::from_millis(500)).unwrap()
    else {
        panic!("a sleeping child cannot have exited");
    };
    assert_eq!(String::from_utf8_lossy(&out.stdout), "before\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "err\n");
}

#[test]
fn a_grandchild_in_the_group_is_killed_with_the_child() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
    let started = Instant::now();
    let error = output(&mut sh(&script), Duration::from_millis(500)).unwrap_err();
    assert!(is_stopped(&error));
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    let pid = std::fs::read_to_string(&pid_file).unwrap().trim().to_owned();
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(&pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(&pid), "the grandchild {pid} outlived its group");
}

#[test]
fn a_program_that_does_not_exist_is_not_a_stop() {
    let error = output(&mut Command::new("tidewatch-no-such-program"), DEFAULT).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(!is_stopped(&error));
    assert!(failure("x", &error).starts_with("cannot run x: "), "{}", failure("x", &error));
}

#[test]
fn a_child_that_leaves_a_daemon_holding_the_pipe_still_returns_its_output() {
    let started = Instant::now();
    let out = output(&mut sh("echo done; sleep 4 &"), Duration::from_secs(30)).unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "done\n");
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
}

#[test]
fn limits_are_worded_in_seconds_or_whole_minutes() {
    let cases = [(90, "90 s"), (60, "60 s"), (120, "2 min"), (180, "3 min"), (600, "10 min")];
    assert_eq!(describe(Duration::from_millis(300)), "300 ms");
    for (secs, want) in cases {
        assert_eq!(describe(Duration::from_secs(secs)), want);
    }
    assert_eq!(
        Stopped { program: "git".into(), limit: WORKTREE }.to_string(),
        "git took longer than 3 min and was stopped"
    );
}
