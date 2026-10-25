use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::*;
use crate::paths::TidewatchPaths;

const SYSTEM: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

fn without_main(script: &str) -> &str {
    script.strip_suffix("main \"$@\"\n").expect("the script ends by calling main")
}

fn accepts(check: &str, argument: &str) -> bool {
    let script = helper_script();
    let program = format!("{}\n{check} \"$1\"", without_main(&script));
    Command::new("/bin/sh")
        .args(["-c", &program, "check", argument])
        .stdin(Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn the_helper_accepts_only_the_token_file_of_the_state_folder() {
    let cases = [
        ("/Users/vlad/.tidewatch/.state/amphetamine.token", true),
        ("/tmp/ta/.state/amphetamine.token", true),
        ("/home/a_b-c.d/.tidewatch/.state/amphetamine.token", true),
        ("", false),
        ("amphetamine.token", false),
        (".state/amphetamine.token", false),
        ("/etc/passwd", false),
        ("/Users/vlad/.tidewatch/.state/other.token", false),
        ("/Users/vlad/.tidewatch/.state/amphetamine.token.bak", false),
        ("/Users/vlad/.tidewatch/.state/../../../etc/shadow", false),
        ("/Users/vlad/.tidewatch/.state/amphetamine.token; rm -rf /", false),
        ("/Users/vlad/.tidewatch/.state/amphetamine.token && id", false),
        ("/Users/vlad/$(id)/.state/amphetamine.token", false),
        ("/Users/vlad/`id`/.state/amphetamine.token", false),
        ("/Users/vlad/a b/.state/amphetamine.token", false),
        ("/Users/vlad/a\nb/.state/amphetamine.token", false),
        ("/Users/vlad/a'b/.state/amphetamine.token", false),
        ("/Users/vlad/a|b/.state/amphetamine.token", false),
        ("/Users/vlad/a*/.state/amphetamine.token", false),
    ];
    for (token, want) in cases {
        assert_eq!(accepts("valid_token", token), want, "{token:?}");
    }
}

#[test]
fn the_helper_accepts_the_token_the_app_really_writes() {
    let paths = TidewatchPaths::new(std::path::Path::new("/Users/vlad/.tidewatch"));
    assert!(accepts("valid_token", &paths.amphetamine_token_file().to_string_lossy()));
}

#[test]
fn the_helper_accepts_only_a_plain_number_as_the_pid() {
    let cases = [
        ("1", true),
        ("4242", true),
        ("", false),
        ("-1", false),
        ("12a", false),
        ("1 2", false),
        ("1;id", false),
        ("$$", false),
        ("1.5", false),
    ];
    for (pid, want) in cases {
        assert_eq!(accepts("valid_pid", pid), want, "{pid:?}");
    }
}

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let rig = Self { dir };
        rig.executable("pmset", &format!("#!/bin/sh\necho \"$@\" >> '{}'\n", rig.log().display()));
        rig.executable("sleep", "#!/bin/sh\nexec /bin/sleep 0.05\n");
        std::fs::create_dir_all(rig.dir.path().join(".state")).unwrap();
        rig
    }

    fn executable(&self, name: &str, text: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.dir.path().join(name);
        let source = path.with_extension("src");
        std::fs::write(&source, text).unwrap();
        assert!(Command::new("cp").arg(&source).arg(&path).status().unwrap().success());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join("pmset.log")
    }

    fn token(&self) -> PathBuf {
        self.dir.path().join(".state/amphetamine.token")
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.log()).unwrap_or_default().lines().map(str::to_owned).collect()
    }

    fn script(&self) -> String {
        let script = helper_script();
        let path = format!("{}:/usr/bin:/bin", self.dir.path().display());
        let swapped = script.replace(&format!("PATH={SYSTEM}\n"), &format!("PATH={path}\n"));
        assert_ne!(swapped, script, "the rig puts its fake pmset first");
        let own_sudoers = self.dir.path().join("sudoers");
        let own_self = self.dir.path().join("helper");
        let swapped = swapped
            .replace(&shell_quote(SUDOERS_PATH), &shell_quote(&own_sudoers.to_string_lossy()))
            .replace(&shell_quote(HELPER_PATH), &shell_quote(&own_self.to_string_lossy()));
        assert!(!swapped.contains(SUDOERS_PATH) && !swapped.contains(HELPER_PATH));
        swapped
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new("/bin/sh")
            .args(["-c", &self.script(), "helper"])
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn wait_for(&self, calls: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.calls().len() >= calls {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

#[test]
fn start_turns_sleep_off_reports_the_watcher_and_the_watcher_restores_when_the_token_goes() {
    let rig = Rig::new();
    std::fs::write(rig.token(), "0").unwrap();
    let token = rig.token();
    let out = rig.run(&["start", &token.to_string_lossy(), &std::process::id().to_string()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let pid = parse_pid(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert!(pid > 1);
    assert_eq!(rig.calls(), ["-a disablesleep 1"]);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rig.calls().len(), 1, "the watcher holds while the token stands");
    std::fs::remove_file(&token).unwrap();
    assert!(rig.wait_for(2));
    assert_eq!(rig.calls(), ["-a disablesleep 1", "-a disablesleep 0"]);
}

#[test]
fn start_refuses_a_bad_token_or_pid_before_it_touches_pmset() {
    let rig = Rig::new();
    let good = rig.token().to_string_lossy().into_owned();
    let pid = std::process::id().to_string();
    let cases: [&[&str]; 7] = [
        &["start", "/etc/passwd", &pid],
        &["start", &good, "12a"],
        &["start", &good],
        &["start"],
        &["start", "/tmp/x; id", &pid],
        &["pmset", "-a", "disablesleep", "1"],
        &[],
    ];
    for args in cases {
        let out = rig.run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(out.stdout, b"", "{args:?}");
    }
    assert_eq!(rig.calls().len(), 0);
}

#[test]
fn stop_turns_sleep_back_on_and_uninstall_removes_both_files() {
    let rig = Rig::new();
    assert!(rig.run(&["stop"]).status.success());
    assert_eq!(rig.calls(), ["-a disablesleep 0"]);
    let (sudoers, helper) = (rig.dir.path().join("sudoers"), rig.dir.path().join("helper"));
    std::fs::write(&sudoers, "x").unwrap();
    std::fs::write(&helper, "x").unwrap();
    assert!(rig.run(&["uninstall"]).status.success());
    assert!(!sudoers.exists() && !helper.exists());
}

#[test]
fn the_sudoers_rule_names_one_user_and_only_the_helper() {
    let text = sudoers_text("vlad").unwrap();
    let rules: Vec<_> = text.lines().filter(|line| !line.starts_with('#')).collect();
    assert_eq!(rules, [format!("vlad ALL=(root) NOPASSWD: {HELPER_PATH}")]);
    assert!(text.ends_with('\n'));
}

#[test]
fn only_a_plain_account_name_gets_a_sudoers_rule() {
    let cases = [
        ("vlad", true),
        ("_www", true),
        ("john.smith", true),
        ("a-b_c", true),
        ("", false),
        ("%admin", false),
        ("#501", false),
        ("-x", false),
        (".hidden", false),
        ("a b", false),
        ("a\nb ALL=(ALL) NOPASSWD: ALL", false),
        ("a,b", false),
        ("a!b", false),
        ("a:b", false),
        ("ALL", false),
        ("ADMINS_1", false),
        ("Vlad", true),
        (&"x".repeat(33), false),
    ];
    for (name, want) in cases {
        assert_eq!(valid_user(name), want, "{name:?}");
        assert_eq!(sudoers_text(name).is_some(), want, "{name:?}");
        assert_eq!(install_command(name, SYSTEM).is_some(), want, "{name:?}");
    }
}

#[test]
fn the_sudoers_text_passes_visudo_when_it_is_installed() {
    let visudo = std::path::Path::new("/usr/sbin/visudo");
    if !visudo.exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("sudoers");
    std::fs::write(&file, sudoers_text("vlad").unwrap()).unwrap();
    let out = Command::new(visudo).arg("-cf").arg(&file).stdin(Stdio::null()).output().unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(&file, "vlad ALL=(root) NOPASSWD ALL ALL\n").unwrap();
    let bad = Command::new(visudo).arg("-cf").arg(&file).stdin(Stdio::null()).output().unwrap();
    assert!(!bad.status.success(), "visudo is the gate the install relies on");
}

#[test]
fn the_install_command_checks_the_drop_in_before_any_file_moves() {
    let command = install_command("vlad", SYSTEM).unwrap();
    let check = command.find("visudo -cf").unwrap();
    for later in ["mkdir -p", "install -o root", "mv -f"] {
        assert!(check < command.find(later).unwrap(), "{later}");
    }
    assert!(command.contains("set -e"));
    assert!(command.contains("-m 0755") && command.contains("-m 0440"));
    assert!(command.contains("-o root -g wheel"));
    assert!(command.contains("stat -f %u"), "the helper's folder must belong to root");
    assert!(command.contains(&format!("{}.new", shell_quote(SUDOERS_PATH))));
    assert!(command.contains(&shell_quote(&helper_script())));
}

#[test]
fn the_install_command_is_a_script_the_shell_can_parse() {
    let command = install_command("vlad", SYSTEM).unwrap();
    let out = Command::new("/bin/sh").args(["-n", "-c", &command]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = Command::new("/bin/sh").args(["-n", "-c", &remove_command(SYSTEM)]).output().unwrap();
    assert!(out.status.success());
}

const FIFO: &str = "/Users/vlad/.tidewatch/.state/amphetamine.serve";

fn first_run_command() -> String {
    let token = PathBuf::from("/Users/vlad/.tidewatch/.state/amphetamine.token");
    install_and_serve_command("vlad", &token, 77, &PathBuf::from(FIFO), SYSTEM).unwrap()
}

#[test]
fn the_first_run_command_is_one_script_that_installs_before_it_serves() {
    let command = first_run_command();
    let out = Command::new("/bin/sh").args(["-n", "-c", &command]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let install = install_command("vlad", SYSTEM).unwrap();
    assert!(command.starts_with(&install));
    let serve = &command[install.len()..];
    assert!(install.ends_with(&format!(
        "mv -f {}.new {}",
        shell_quote(SUDOERS_PATH),
        shell_quote(SUDOERS_PATH)
    )));
    assert!(
        serve.starts_with(&format!("; {} serve ", shell_quote(HELPER_PATH))),
        "the serve follows the whole install"
    );
    assert!(command.contains("set -e"));
    assert!(command.ends_with(&shell_quote(FIFO)));
    assert_eq!(command.matches("visudo -cf").count(), 1);
}

#[test]
fn the_first_run_command_is_refused_for_an_account_a_sudoers_line_could_misread() {
    let token = PathBuf::from("/Users/vlad/.tidewatch/.state/amphetamine.token");
    for user in ["", "ALL", "a b", "root\nALL ALL=(ALL) NOPASSWD: ALL"] {
        let fifo = PathBuf::from(FIFO);
        assert_eq!(install_and_serve_command(user, &token, 1, &fifo, SYSTEM), None, "{user:?}");
    }
}

#[test]
fn the_helper_script_is_a_script_the_shell_can_parse() {
    let out = Command::new("/bin/sh").args(["-n", "-c", &helper_script()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn the_serve_lines_name_a_deadline_or_an_off() {
    assert_eq!(Serve::On(Some(1_729_000_000.0)).line(), "on 1729000000");
    assert_eq!(Serve::On(None).line(), "on 0");
    assert_eq!(Serve::On(None).answer(), "0");
    assert_eq!(Serve::Off.line(), "off");
    assert_eq!(Serve::Off.answer(), "off");
}

#[test]
fn the_command_pipe_is_made_private_to_the_owner() {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let dir = std::env::temp_dir().join(format!("tidewatch-pipe-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fifo = dir.join("amphetamine.serve");
    make_command_fifo(&fifo).unwrap();
    make_command_fifo(&fifo).unwrap();
    let meta = std::fs::metadata(&fifo).unwrap();
    assert!(meta.file_type().is_fifo());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_remove_command_removes_the_rule_before_the_helper() {
    let command = remove_command(SYSTEM);
    assert!(command.find(SUDOERS_PATH).unwrap() < command.find(HELPER_PATH).unwrap());
    assert!(command.contains("rm -f"));
}

#[test]
fn the_sudo_arguments_never_prompt_and_name_the_helper_by_its_full_path() {
    let token = PathBuf::from("/Users/vlad/.tidewatch/.state/amphetamine.token");
    assert_eq!(
        sudo_serve_args(&token, 77, &PathBuf::from(FIFO)),
        ["-n", HELPER_PATH, "serve", "/Users/vlad/.tidewatch/.state/amphetamine.token", "77", FIFO]
    );
    assert_eq!(sudo_verb_args("stop"), ["-n", HELPER_PATH, "stop"]);
    assert_eq!(sudo_verb_args("uninstall"), ["-n", HELPER_PATH, "uninstall"]);
}

#[test]
fn the_helper_pid_is_read_from_its_output() {
    assert_eq!(parse_pid("4242\n"), Ok(4242));
    assert!(parse_pid("").is_err());
    assert!(parse_pid("sudo: a password is required").is_err());
}

fn pipe_of(rig: &Rig) -> PathBuf {
    rig.dir.path().join(".state/amphetamine.serve")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn send(pipe: &Path, line: &str) {
    use std::io::Write as _;
    let mut writer = std::fs::OpenOptions::new().read(true).write(true).open(pipe).unwrap();
    writeln!(writer, "{line}").unwrap();
}

fn wait_until(what: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn token_says(rig: &Rig, want: &str) -> bool {
    std::fs::read_to_string(rig.token()).is_ok_and(|text| text.trim() == want)
}

fn gone(pid: u32) -> bool {
    wait_until(|| {
        let out = Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .stderr(Stdio::null())
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        state.is_empty() || state.starts_with('Z')
    })
}

fn owner_child() -> std::process::Child {
    Command::new("/bin/sleep").arg("60").stdin(Stdio::null()).spawn().unwrap()
}

fn serve_under(rig: &Rig, owner: u32) -> u32 {
    std::fs::write(rig.token(), "0").unwrap();
    make_command_fifo(&pipe_of(rig)).unwrap();
    let out = rig.run(&[
        "serve",
        &rig.token().to_string_lossy(),
        &owner.to_string(),
        &pipe_of(rig).to_string_lossy(),
    ]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    parse_pid(&String::from_utf8_lossy(&out.stdout)).unwrap()
}

fn finish(owner: &mut std::process::Child, helper: u32) {
    owner.kill().unwrap();
    owner.wait().unwrap();
    assert!(gone(helper), "the helper exits with its owner");
}

#[test]
fn serve_takes_on_and_off_from_its_pipe_and_writes_each_answer() {
    let rig = Rig::new();
    let mut owner = owner_child();
    let helper = serve_under(&rig, owner.id());
    assert_eq!(rig.calls(), ["-a disablesleep 1"]);
    let until = now_secs() + 3600;
    send(&pipe_of(&rig), &format!("on {until}"));
    assert!(wait_until(|| token_says(&rig, &until.to_string())));
    assert_eq!(rig.calls(), ["-a disablesleep 1", "-a disablesleep 1"]);
    send(&pipe_of(&rig), "off");
    assert!(wait_until(|| token_says(&rig, "off")));
    assert_eq!(rig.calls().last().map(String::as_str), Some("-a disablesleep 0"));
    finish(&mut owner, helper);
}

#[test]
fn serve_ignores_every_line_but_on_with_a_number_and_off() {
    let rig = Rig::new();
    let mut owner = owner_child();
    let helper = serve_under(&rig, owner.id());
    for line in ["on soon", "on 12;id", "on", "on $(id)", "reboot"] {
        send(&pipe_of(&rig), line);
    }
    send(&pipe_of(&rig), "off");
    assert!(wait_until(|| token_says(&rig, "off")));
    assert_eq!(rig.calls(), ["-a disablesleep 1", "-a disablesleep 0"]);
    finish(&mut owner, helper);
}

#[test]
fn serve_restores_sleep_at_the_deadline_and_keeps_serving() {
    let rig = Rig::new();
    let mut owner = owner_child();
    let helper = serve_under(&rig, owner.id());
    send(&pipe_of(&rig), &format!("on {}", now_secs() + 1));
    assert!(wait_until(|| token_says(&rig, "off")));
    assert_eq!(rig.calls().last().map(String::as_str), Some("-a disablesleep 0"));
    send(&pipe_of(&rig), "on 0");
    assert!(wait_until(|| rig.calls().len() == 4));
    assert_eq!(rig.calls().last().map(String::as_str), Some("-a disablesleep 1"));
    finish(&mut owner, helper);
}

#[test]
fn serve_restores_sleep_and_exits_when_the_app_is_gone() {
    let rig = Rig::new();
    let mut owner = owner_child();
    let helper = serve_under(&rig, owner.id());
    send(&pipe_of(&rig), "on 0");
    assert!(wait_until(|| rig.calls().len() == 2));
    owner.kill().unwrap();
    owner.wait().unwrap();
    assert!(wait_until(|| rig.calls().last().map(String::as_str) == Some("-a disablesleep 0")));
    assert!(gone(helper));
}

#[test]
fn serve_restores_sleep_and_exits_when_its_token_is_removed() {
    let rig = Rig::new();
    let mut owner = owner_child();
    let helper = serve_under(&rig, owner.id());
    std::fs::remove_file(rig.token()).unwrap();
    assert!(wait_until(|| rig.calls().last().map(String::as_str) == Some("-a disablesleep 0")));
    assert!(gone(helper));
    finish(&mut owner, helper);
}

#[test]
fn serve_refuses_a_pipe_outside_the_state_folder_or_a_link_to_one_before_pmset() {
    let rig = Rig::new();
    std::fs::write(rig.token(), "0").unwrap();
    make_command_fifo(&pipe_of(&rig)).unwrap();
    let owner = std::process::id().to_string();
    let token = rig.token().to_string_lossy().into_owned();
    let linked_dir = rig.dir.path().join("linked/.state");
    std::fs::create_dir_all(&linked_dir).unwrap();
    let link = linked_dir.join("amphetamine.serve");
    std::os::unix::fs::symlink(pipe_of(&rig), &link).unwrap();
    for pipe in ["/etc/amphetamine.serve".to_owned(), link.to_string_lossy().into_owned()] {
        let out = rig.run(&["serve", &token, &owner, &pipe]);
        assert!(!out.status.success(), "{pipe}");
    }
    assert_eq!(rig.calls().len(), 0);
}

#[test]
fn no_text_for_the_user_holds_a_dash() {
    for text in [SETUP_PROMPT, SETUP_AND_START_PROMPT, REMOVE_PROMPT, TOUCH_ID_REASON] {
        assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
    }
}
