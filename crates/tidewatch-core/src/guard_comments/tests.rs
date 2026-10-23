use std::ffi::OsString;
use std::process::Command;

use serde_json::json;

use super::*;
use crate::comments::Kind;
use crate::guards::Rule;

fn call(tool: &str, input: &Value) -> HookCall {
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": tool,
        "tool_input": input,
        "cwd": "/work/web",
        "session_id": "s1",
    });
    parse_hook(&payload.to_string()).unwrap()
}

#[test]
fn the_payload_is_read_and_a_broken_one_is_not() {
    let parsed = parse_hook(
        r#"{"hook_event_name":"Stop","stop_hook_active":true,"cwd":"/w","tool_name":"Write","tool_input":{"file_path":"/w/a.rs","content":"x"}}"#,
    )
    .unwrap();
    assert!(parsed.stop_hook_active);
    assert_eq!(parsed.cwd, Some(PathBuf::from("/w")));
    assert_eq!(parsed.tool_name, "Write");
    assert_eq!(parse_hook("{}").unwrap().tool_name, "");
    assert_eq!(parse_hook("not json"), None);
    assert_eq!(parse_hook(""), None);
}

#[test]
fn a_write_is_scanned_from_line_one() {
    let input = json!({"file_path": "/w/src/a.rs", "content": "fn a() {}\n// loop over the items\nfn b() {}\n"});
    let (path, flags) = flags_of_write(&call("Write", &input), |_| None).unwrap();
    assert_eq!(path, "/w/src/a.rs");
    assert_eq!(flags.len(), 1);
    assert_eq!((flags[0].line, flags[0].kind), (2, Kind::Narration));
}

#[test]
fn an_edit_is_placed_in_the_file_it_changed() {
    let file = "fn a() {}\nfn b() {}\n// set b to two\nfn c() {}\n";
    let input = json!({"file_path": "/w/a.rs", "old_string": "fn c() {}", "new_string": "// set b to two\nfn c() {}"});
    let (_, flags) = flags_of_write(&call("Edit", &input), |_| Some(file.to_owned())).unwrap();
    assert_eq!((flags[0].line, flags[0].kind), (3, Kind::Narration));
    let unplaced = flags_of_write(&call("Edit", &input), |_| None).unwrap().1;
    assert_eq!(unplaced[0].line, 1);
}

#[test]
fn a_multiedit_scans_every_new_string() {
    let input = json!({"file_path": "/w/a.py", "edits": [
        {"old_string": "a", "new_string": "# initialize the list\nxs = []"},
        {"old_string": "b", "new_string": "ys = []  # hack"},
    ]});
    let (_, flags) = flags_of_write(&call("MultiEdit", &input), |_| None).unwrap();
    let kinds: Vec<Kind> = flags.iter().map(|f| f.kind).collect();
    assert_eq!(kinds, [Kind::Narration, Kind::Alibi]);
}

#[test]
fn other_tools_other_files_and_clean_writes_are_not_judged() {
    let write = |path: &str, content: &str| {
        flags_of_write(&call("Write", &json!({"file_path": path, "content": content})), |_| None)
    };
    assert_eq!(write("/w/README.md", "// loop over"), None);
    assert_eq!(write("/w/node_modules/x/a.js", "// loop over"), None);
    assert_eq!(write("/w/a.rs", "fn a() {}\n").unwrap().1, vec![]);
    assert_eq!(flags_of_write(&call("Bash", &json!({"command": "ls"})), |_| None), None);
    assert_eq!(flags_of_write(&call("Write", &json!({"content": "x"})), |_| None), None);
}

#[test]
fn the_edit_message_names_the_file_the_count_the_lines_and_the_advice() {
    let flags = scan_text("fn a() {}\n// loop over the items\n");
    let message = edit_message("/w/src/a.rs", &flags);
    assert!(
        message.starts_with("COMMENT GUARD - a.rs: 1 suspect comment(s) in what you just wrote.\n"),
        "{message}"
    );
    assert!(message.contains(
        "  L2  [narration] narrates what the next line already says: loop over the items"
    ));
    assert!(message.ends_with(ADVICE));
}

fn scan_text(text: &str) -> Vec<Flag> {
    scan(text, Syntax::CLike, 1)
}

#[test]
fn a_long_edit_report_is_cut_after_fifteen_lines() {
    let text = (0..20).fold(String::new(), |mut all, n| {
        let _ = writeln!(all, "// fetch thing {n}\nlet a{n} = 1;");
        all
    });
    let message = edit_message("a.rs", &scan_text(&text));
    assert_eq!(
        message
            .lines()
            .filter(|line| line.starts_with("  L") || line.starts_with("  file"))
            .count(),
        15
    );
    assert!(message.contains("... and more"));
}

#[test]
fn the_sweep_message_groups_by_file_and_the_digest_changes_with_the_report() {
    let files = vec![
        FileFlags { path: "src/a.rs".into(), flags: scan_text("// loop over a\nfn a() {}") },
        FileFlags { path: "src/b.rs".into(), flags: scan_text("// hack\nfn b() {}") },
    ];
    let message = sweep_message(&files);
    assert!(message.starts_with("COMMENT GUARD (branch sweep) - noisy comments still in the diff:\nsrc/a.rs: 1 suspect\n  L1  [narration]"), "{message}");
    assert!(message.contains("src/b.rs: 1 suspect"));
    assert_eq!(digest(&message), digest(&message));
    assert_ne!(digest(&message), digest(&sweep_message(&files[..1])));
    assert_eq!(digest("x").len(), 12);
}

#[test]
fn records_keep_the_kind_tag_the_place_and_the_origin() {
    let flags = scan_text("// loop over a\nfn a() {}");
    let made = records(Origin::Sweep, "src/a.rs", &flags, 7);
    assert_eq!(
        made,
        [Record {
            at_ms: 7,
            origin: Origin::Sweep,
            file: "src/a.rs".into(),
            line: 1,
            kind: "narration".into(),
            text: "loop over a".into()
        }]
    );
}

struct Lab {
    root: tempfile::TempDir,
}

impl Lab {
    fn new(change: impl FnOnce(&mut crate::guards::GuardPrefs)) -> Self {
        let lab = Self { root: tempfile::tempdir().unwrap() };
        lab.configure(change);
        lab
    }

    fn configure(&self, change: impl FnOnce(&mut crate::guards::GuardPrefs)) {
        let paths = crate::paths::TidewatchPaths::new(&self.root.path().join("home"));
        let mut prefs = Prefs::default();
        change(&mut prefs.guards);
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        prefs.save(&paths).unwrap();
    }

    fn env(&self) -> GuardEnv {
        let paths = crate::paths::TidewatchPaths::new(&self.root.path().join("home"));
        GuardEnv {
            config: paths.prefs_file(),
            repo: self.root.path().join("repo"),
            cache: None,
            shim_dir: PathBuf::new(),
            tab: Some(self.root.path().join("tab")),
            path: OsString::new(),
        }
    }

    fn tab(&self) -> TabFiles {
        TabFiles::new(&self.root.path().join("tab"))
    }
}

fn payload(tool: &str, input: &Value) -> String {
    json!({"tool_name": tool, "tool_input": input, "cwd": "/nowhere"}).to_string()
}

#[test]
fn a_noisy_write_exits_two_and_is_recorded_and_a_clean_one_exits_zero() {
    let lab = Lab::new(|_| {});
    let noisy = payload(
        "Write",
        &json!({"file_path": "/w/a.rs", "content": "// loop over a\nfn a() {}\n"}),
    );
    assert_eq!(run(false, &noisy, &lab.env()), 2);
    let kept = lab.tab().load();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        (kept[0].origin, kept[0].kind.as_str(), kept[0].line),
        (Origin::Edit, "narration", 1)
    );
    let clean = payload("Write", &json!({"file_path": "/w/a.rs", "content": "fn a() {}\n"}));
    assert_eq!(run(false, &clean, &lab.env()), 0);
    assert_eq!(lab.tab().load().len(), 1);
}

#[test]
fn the_rule_off_in_settings_or_for_the_repo_or_the_tab_silences_the_hook() {
    let noisy = payload(
        "Write",
        &json!({"file_path": "/w/a.rs", "content": "// loop over a\nfn a() {}\n"}),
    );
    let off = Lab::new(|guards| guards.rules.set(Rule::Comments, false));
    assert_eq!(run(false, &noisy, &off.env()), 0);

    let lab = Lab::new(|_| {});
    let repo = lab.env().repo;
    lab.configure(|guards| guards.set_rule(Some(&repo), Rule::Comments, false));
    assert_eq!(run(false, &noisy, &lab.env()), 0);
    lab.configure(|guards| guards.set_rule(Some(Path::new("/other")), Rule::Comments, false));
    assert_eq!(run(false, &noisy, &lab.env()), 2);

    let tab_off = Lab::new(|_| {});
    tab_off.tab().set_off(true).unwrap();
    assert_eq!(run(false, &noisy, &tab_off.env()), 0);
    tab_off.tab().set_off(false).unwrap();
    assert_eq!(run(false, &noisy, &tab_off.env()), 2);
}

#[test]
fn a_payload_that_does_not_parse_or_a_tool_it_does_not_know_is_exit_zero() {
    let lab = Lab::new(|_| {});
    assert_eq!(run(false, "garbage", &lab.env()), 0);
    assert_eq!(run(false, &payload("Bash", &json!({"command": "ls"})), &lab.env()), 0);
    assert_eq!(run(true, "garbage", &lab.env()), 0);
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn branch_with_noise(lab: &Lab) -> PathBuf {
    let repo = lab.root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    git(&repo, &["switch", "-q", "-c", "agent/work"]);
    std::fs::write(repo.join("a.rs"), "// loop over the items\nfn a() {}\n").unwrap();
    repo
}

fn stop_payload(repo: &Path, active: bool) -> String {
    json!({"hook_event_name": "Stop", "cwd": repo, "stop_hook_active": active}).to_string()
}

#[test]
fn the_stop_sweep_reports_a_branch_diff_once_and_again_when_it_changes() {
    let lab = Lab::new(|_| {});
    let repo = branch_with_noise(&lab);
    assert_eq!(run(true, &stop_payload(&repo, false), &lab.env()), 2);
    let kept = lab.tab().load();
    assert_eq!((kept[0].origin, kept[0].file.as_str()), (Origin::Sweep, "a.rs"));
    assert_eq!(
        run(true, &stop_payload(&repo, false), &lab.env()),
        0,
        "the same report is not sent twice"
    );

    std::fs::write(repo.join("b.rs"), "fn b() {}\n// hack\n").unwrap();
    assert_eq!(run(true, &stop_payload(&repo, false), &lab.env()), 2);

    std::fs::write(repo.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(repo.join("b.rs"), "fn b() {}\n").unwrap();
    assert_eq!(run(true, &stop_payload(&repo, false), &lab.env()), 0);
    assert_eq!(
        lab.tab().swept(),
        None,
        "a clean sweep forgets the digest, so a regression is told again"
    );
}

#[test]
fn a_stop_that_already_continued_for_a_hook_is_left_alone() {
    let lab = Lab::new(|_| {});
    let repo = branch_with_noise(&lab);
    assert_eq!(run(true, &stop_payload(&repo, true), &lab.env()), 0);
    assert_eq!(lab.tab().load(), vec![]);
}

#[test]
fn a_stop_in_a_folder_with_nothing_to_compare_with_is_exit_zero() {
    let lab = Lab::new(|_| {});
    let plain = lab.root.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(run(true, &stop_payload(&plain, false), &lab.env()), 0);
}
