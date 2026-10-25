use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::*;

const NOW: u64 = 1_000_000;

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let rig = Self { dir };
        rig.executable("pmset", &format!("#!/bin/sh\necho \"$@\" >> '{}'\n", rig.log().display()));
        rig.executable("sleep", "#!/bin/sh\nexec /bin/sleep 0.05\n");
        rig
    }

    fn executable(&self, name: &str, text: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.dir.path().join(name);
        let source = path.with_extension("src");
        std::fs::write(&source, text).unwrap();
        let copied = Command::new("cp").arg(&source).arg(&path).status().unwrap();
        assert!(copied.success());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join("pmset.log")
    }

    fn token(&self) -> PathBuf {
        self.dir.path().join("token")
    }

    fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.dir.path().display())
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.log()).unwrap_or_default().lines().map(str::to_owned).collect()
    }

    fn helper(&self, owner: u32) -> Child {
        Command::new("/bin/sh")
            .args(["-c", helper_script(), "amphetamine"])
            .arg(self.token())
            .arg(owner.to_string())
            .env("PATH", self.path())
            .stdin(Stdio::null())
            .spawn()
            .unwrap()
    }
}

fn wait_for_exit(child: &mut Child) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    false
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn finished_pid() -> u32 {
    let mut gone = Command::new("/bin/sh").args(["-c", "exit 0"]).spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    pid
}

const RESTORED: [&str; 1] = ["-a disablesleep 0"];

#[test]
fn a_passed_deadline_restores_sleep_and_ends() {
    let rig = Rig::new();
    write_token(&rig.token(), Some((now_secs() - 5) as f64)).unwrap();
    let mut helper = rig.helper(std::process::id());
    assert!(wait_for_exit(&mut helper));
    assert_eq!(rig.calls(), RESTORED);
}

#[test]
fn removing_the_token_restores_sleep_without_a_second_prompt() {
    let rig = Rig::new();
    write_token(&rig.token(), Some((now_secs() + 3600) as f64)).unwrap();
    let mut helper = rig.helper(std::process::id());
    std::thread::sleep(Duration::from_millis(300));
    assert!(helper.try_wait().unwrap().is_none(), "it holds while everything is in order");
    assert_eq!(rig.calls(), Vec::<String>::new());
    remove_token(&rig.token());
    assert!(wait_for_exit(&mut helper));
    assert_eq!(rig.calls(), RESTORED);
}

#[test]
fn a_gone_tidewatch_restores_sleep_even_with_no_deadline() {
    let rig = Rig::new();
    write_token(&rig.token(), None).unwrap();
    let mut helper = rig.helper(finished_pid());
    assert!(wait_for_exit(&mut helper));
    assert_eq!(rig.calls(), RESTORED);
}

#[test]
fn a_token_that_is_not_a_number_restores_sleep() {
    let rig = Rig::new();
    for junk in ["", "soon", "12 34", "-5"] {
        std::fs::write(rig.token(), junk).unwrap();
        let mut helper = rig.helper(std::process::id());
        assert!(wait_for_exit(&mut helper), "{junk:?}");
    }
    assert_eq!(rig.calls().len(), 4);
}

#[test]
fn a_later_deadline_written_to_the_token_keeps_it_holding() {
    let rig = Rig::new();
    write_token(&rig.token(), Some((now_secs() + 1) as f64)).unwrap();
    let mut helper = rig.helper(std::process::id());
    write_token(&rig.token(), Some((now_secs() + 3600) as f64)).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(helper.try_wait().unwrap().is_none());
    remove_token(&rig.token());
    assert!(wait_for_exit(&mut helper));
    assert_eq!(rig.calls(), RESTORED);
}

#[test]
fn being_told_to_stop_restores_sleep_first() {
    let rig = Rig::new();
    write_token(&rig.token(), None).unwrap();
    let mut helper = rig.helper(std::process::id());
    std::thread::sleep(Duration::from_millis(300));
    let killed = Command::new("kill").args(["-TERM", &helper.id().to_string()]).status().unwrap();
    assert!(killed.success());
    assert!(wait_for_exit(&mut helper));
    assert_eq!(rig.calls(), RESTORED);
}

#[test]
fn the_admin_command_sets_lid_mode_then_starts_a_detached_helper_that_reports_its_pid() {
    let rig = Rig::new();
    write_token(&rig.token(), None).unwrap();
    let command = admin_command(&rig.token(), std::process::id(), &rig.path());
    let output =
        Command::new("/bin/sh").args(["-c", &command]).stdin(Stdio::null()).output().unwrap();
    assert!(output.status.success());
    let pid: u32 = String::from_utf8_lossy(&output.stdout).trim().parse().unwrap();
    assert!(pid > 1);
    assert_eq!(rig.calls(), ["-a disablesleep 1"]);
    remove_token(&rig.token());
    let deadline = Instant::now() + Duration::from_secs(30);
    while rig.calls().len() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(rig.calls(), ["-a disablesleep 1", "-a disablesleep 0"]);
}

#[test]
fn a_failed_pmset_starts_no_helper() {
    let rig = Rig::new();
    rig.executable("pmset", "#!/bin/sh\nexit 1\n");
    write_token(&rig.token(), None).unwrap();
    let command = admin_command(&rig.token(), std::process::id(), &rig.path());
    let output = Command::new("/bin/sh").args(["-c", &command]).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "");
}

#[test]
fn quoting_survives_hostile_paths_in_the_shell_and_in_applescript() {
    assert_eq!(shell_quote("it's"), "'it'\\''s'");
    let out = Command::new("/bin/sh")
        .args(["-c", &format!("printf %s {}", shell_quote("a 'b' \"c\" $d `e`"))])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a 'b' \"c\" $d `e`");

    let args = osascript_args("echo \"hi\" \\ done", "Tidewatch \"asks\"");
    assert_eq!(args[0], "-e");
    assert_eq!(
        args[1],
        "do shell script \"echo \\\"hi\\\" \\\\ done\" with prompt \"Tidewatch \\\"asks\\\"\" with administrator privileges"
    );
}

#[test]
fn the_admin_command_hands_root_a_script_with_no_path_spliced_in() {
    let hostile = Path::new("/tmp/a'; rm -rf ~; echo '/token");
    let command = admin_command(hostile, 9, SYSTEM_PATH);
    assert!(command.contains(&shell_quote(&hostile.to_string_lossy())));
    assert!(command.starts_with(
        "PATH='/usr/bin:/bin:/usr/sbin:/sbin'; export PATH; pmset -a disablesleep 1 || exit 1;"
    ));
    assert!(!helper_script().contains("rm -rf"));
    assert_eq!(
        restore_command(SYSTEM_PATH),
        "PATH='/usr/bin:/bin:/usr/sbin:/sbin'; export PATH; pmset -a disablesleep 0"
    );
}

#[test]
fn the_token_holds_the_deadline_or_zero() {
    assert_eq!(token_text(None), "0");
    assert_eq!(token_text(Some(NOW as f64)), "1000000");
    assert_eq!(token_text(Some(-4.0)), "1");
}

#[test]
fn the_marker_round_trips_and_a_bad_file_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("amphetamine.json");
    assert_eq!(read_marker(&file), None);
    write_marker(&file, &Marker { pid: 12, helper: Some(34) });
    assert_eq!(read_marker(&file), Some(Marker { pid: 12, helper: Some(34) }));
    std::fs::write(&file, "{not json").unwrap();
    assert_eq!(read_marker(&file), None);
    remove_marker(&file);
    remove_marker(&file);
    assert!(!file.exists());
}

#[test]
fn a_launch_decides_from_the_marker_pmset_and_the_helper() {
    let marker = Marker { pid: 1, helper: Some(2) };
    let cases = [
        ("no marker, sleep off is not ours", None, Some(true), true, Recovery::Nothing),
        ("no marker, sleep on", None, Some(false), false, Recovery::Nothing),
        ("marker, pmset unreadable", Some(&marker), None, true, Recovery::Nothing),
        ("marker, sleep is back on", Some(&marker), Some(false), false, Recovery::Stale),
        ("sleep off, helper alive", Some(&marker), Some(true), true, Recovery::HelperRestores),
        ("sleep off, helper gone", Some(&marker), Some(true), false, Recovery::NeedsPassword),
    ];
    for (name, marker, disabled, alive, expected) in cases {
        assert_eq!(recovery(marker, disabled, alive), expected, "{name}");
    }
}

#[test]
fn pmset_output_is_read_for_the_system_wide_key() {
    let on = "System-wide power settings:\n SleepDisabled\t\t1\nCurrently in use:\n standby 1\n";
    let off = "System-wide power settings:\n SleepDisabled\t\t0\n";
    assert_eq!(parse_sleep_disabled(on), Some(true));
    assert_eq!(parse_sleep_disabled(off), Some(false));
    assert_eq!(parse_sleep_disabled("Currently in use:\n sleep 1\n"), None);
    assert_eq!(parse_sleep_disabled(" SleepDisabled\n"), None);
    assert_eq!(parse_sleep_disabled(" SleepDisabled maybe\n"), None);
}
