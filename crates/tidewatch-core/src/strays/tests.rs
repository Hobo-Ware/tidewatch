use super::*;
use std::process::Command;

const PS: &str = "\
    1     0 ??       /sbin/launchd
  500     1 ??       node
  501     1 ttys004  /bin/zsh
  502   600 ??       node
  503   700 ?        /usr/bin/python3
  600     1 ??       /bin/zsh
  700     1 ?        /usr/lib/systemd/systemd
  900     1 ??       tidewatch
";

#[test]
fn only_orphans_without_a_terminal_escaped() {
    let rows = parse_ps(PS);
    let cases = [
        (500, true, "orphan with no terminal: a nohup'd dev server"),
        (501, false, "has a terminal: another tab's adopted shell"),
        (502, false, "parent alive: inside some tree"),
        (503, true, "re-parented to a Linux user systemd"),
        (900, false, "Tidewatch itself"),
        (1, false, "init"),
        (42, false, "gone from the table"),
    ];
    for (pid, want, why) in cases {
        assert_eq!(escaped(&[pid], &rows, 900) == [pid], want, "{why}");
    }
}

#[test]
fn parse_ps_skips_malformed_lines() {
    let rows = parse_ps("garbage\n 12 1 ?? a b\n\n 13\n");
    assert_eq!(rows, [Row { pid: 12, ppid: 1, has_terminal: false, program: "a b".into() }]);
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn stop_escaped_ends_a_real_orphan_in_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new("perl")
        .arg("-e")
        .arg(
            "use POSIX; $|=1; my $p = fork; if ($p) { print $p; exit } \
             POSIX::setsid(); close STDOUT; close STDERR; close STDIN; chdir $ARGV[0]; exec 'sleep', '300'",
        )
        .arg(dir.path())
        .output()
        .unwrap();
    let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while system_cwd_misses(pid, dir.path()) && Instant::now() < deadline {
        std::thread::sleep(POLL);
    }

    let stopped = stop_escaped(dir.path());

    let gone = !alive(pid);
    if !gone {
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
    }
    assert_eq!(stopped, [pid]);
    assert!(gone, "the orphan outlived stop_escaped");
}

fn system_cwd_misses(pid: u32, dir: &Path) -> bool {
    !RealSystem::new().cwd_holders(dir).is_some_and(|h| h.contains(&pid))
}
