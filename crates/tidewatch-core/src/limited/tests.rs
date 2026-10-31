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

fn listed(label: &str) -> Vec<Running> {
    running().into_iter().filter(|call| call.label == label).collect()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(done(), "{what}");
}

#[test]
fn a_cancelled_call_is_killed_well_before_its_limit_and_says_so() {
    let cancel = Cancel::new();
    let options = Options { label: None, cancel: Some(cancel.clone()) };
    let later = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        later.cancel();
    });
    let started = Instant::now();
    let error = output_with(&mut sh("sleep 30"), Duration::from_secs(60), &options).unwrap_err();
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(is_cancelled(&error));
    assert!(is_stopped(&error));
    assert_eq!(error.to_string(), "sh was cancelled");
    assert_eq!(failure("sh", &error), "sh was cancelled");
}

#[test]
fn a_cancel_keeps_what_the_child_had_printed() {
    let cancel = Cancel::new();
    let options = Options { label: None, cancel: Some(cancel.clone()) };
    let later = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        later.cancel();
    });
    let Ended::Cancelled(out) =
        run_with(&mut sh("echo before; sleep 30"), Duration::from_secs(60), &options).unwrap()
    else {
        panic!("a sleeping child cannot have exited");
    };
    canceller.join().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "before\n");
}

#[test]
fn a_running_call_is_listed_until_it_ends_and_cancel_call_reaches_it() {
    let label = "listed-until-cancelled";
    let worker = std::thread::spawn(move || {
        output_with(&mut sh("sleep 30"), Duration::from_secs(60), &Options::labeled(label))
    });
    wait_until("the call is listed", || listed(label).len() == 1);
    let call = listed(label).remove(0);
    assert!(call.started.elapsed() < Duration::from_secs(10));
    assert!(cancel_call(call.id));
    let error = worker.join().unwrap().unwrap_err();
    assert!(is_cancelled(&error));
    assert_eq!(listed(label).len(), 0);
    assert!(!cancel_call(call.id));
}

#[test]
fn a_call_leaves_the_list_however_it_ends() {
    let cases: [(&str, &str, Duration); 3] = [
        ("ends-exited", "exit 0", Duration::from_secs(30)),
        ("ends-failed", "exit 7", Duration::from_secs(30)),
        ("ends-stopped", "sleep 30", Duration::from_millis(100)),
    ];
    for (label, script, limit) in cases {
        let _ = output_with(&mut sh(script), limit, &Options::labeled(label));
        assert_eq!(listed(label).len(), 0, "{label}");
    }
    let _ = output_with(
        &mut Command::new("tidewatch-no-such-program"),
        DEFAULT,
        &Options::labeled("ends-unstartable"),
    );
    assert_eq!(listed("ends-unstartable").len(), 0);
}

#[test]
fn calls_are_listed_oldest_first_with_the_label_they_were_given_or_derived() {
    let older = hold("oldest-fake", Duration::from_secs(9));
    let newer = hold("newest-fake", Duration::from_secs(5));
    let labels: Vec<String> = running()
        .into_iter()
        .map(|call| call.label)
        .filter(|label| label.ends_with("-fake"))
        .collect();
    assert_eq!(labels, ["oldest-fake", "newest-fake"]);
    drop((older, newer));
    assert_eq!(listed("oldest-fake").len() + listed("newest-fake").len(), 0);
}

#[test]
fn the_window_is_woken_when_a_call_turns_slow_and_when_it_ends() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static WAKES: AtomicUsize = AtomicUsize::new(0);
    set_waker(|| {
        WAKES.fetch_add(1, Ordering::SeqCst);
    });
    let before = WAKES.load(Ordering::SeqCst);
    let fast = hold("wake-fast", Duration::from_secs(1));
    drop(fast);
    assert_eq!(WAKES.load(Ordering::SeqCst), before);
    let slow = hold("wake-slow", SLOW_AFTER);
    assert_eq!(WAKES.load(Ordering::SeqCst), before + 1);
    drop(slow);
    assert_eq!(WAKES.load(Ordering::SeqCst), before + 2);
}

#[test]
fn a_label_names_the_tool_and_what_it_was_asked_to_do() {
    let cases: [(&str, &[&str], &str); 11] = [
        ("git", &["worktree", "add", "-b", "agent/x", "/w"], "git worktree add"),
        ("git", &["-C", "/repo", "worktree", "remove", "--force", "/w"], "git worktree remove"),
        ("git", &["-c", "core.x=1", "-C", "/r", "status", "--porcelain"], "git status"),
        ("git", &["fetch", "--quiet", "origin", "main"], "git fetch"),
        ("git", &["rev-parse", "--show-toplevel"], "git rev-parse"),
        ("git", &[], "git"),
        ("gh", &["pr", "view", "12", "--json", "state"], "gh pr view"),
        ("gh", &["run", "view", "--log-failed"], "gh run view"),
        ("gh", &["api", "repos/o/n/hooks"], "gh api"),
        ("sh", &["-c", "cargo test"], "sh"),
        ("curl", &["-sfL", "https://example.com/a.png"], "curl"),
    ];
    for (program, args, want) in cases {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        assert_eq!(label_from(program, &args), want, "{program} {args:?}");
    }
}
