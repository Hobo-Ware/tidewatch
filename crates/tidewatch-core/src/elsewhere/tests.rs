use super::*;

const PS: &str = "  PID  PPID STARTED                      ARGS
    1     0 Wed Oct  7 09:29:17 2026     /sbin/launchd
90775     1 Wed Oct  7 09:29:20 2026     /Applications/stdusk.app/Contents/MacOS/stdusk
11662 90775 Fri Oct 16 13:20:00 2026     /bin/zsh
19821 11662 Fri Oct 16 13:23:31 2026     claude --permission-mode bypassPermissions
25004     1 Wed Oct 14 09:50:00 2026     /bin/zsh
25746 25004 Wed Oct 14 09:59:03 2026     claude --permission-mode acceptEdits --model opus
";

fn agents() -> AgentTable {
    AgentTable::builtin()
}

fn row(pid: u32, ppid: u32, started: &str, command: &str) -> ProcessRow {
    ProcessRow {
        pid,
        ppid,
        started: started.to_owned(),
        argv: command.split_whitespace().map(str::to_owned).collect(),
    }
}

fn entry(pid: u32, id: &str, start: &str, status: &str) -> RegistryEntry {
    RegistryEntry {
        pid,
        session_id: id.to_owned(),
        cwd: PathBuf::from("/work/acme"),
        started_at_ms: Some(1_792_157_013_549),
        proc_start: start.to_owned(),
        kind: Some("interactive".to_owned()),
        name: Some("retry-stage-gates".to_owned()),
        status: Some(status.to_owned()),
    }
}

const ID_A: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";
const ID_B: &str = "79fdd8f6-5176-4a47-a46f-8db10ec660b5";
const OWN: u32 = 424_242;
const START_A: &str = "Fri Oct 16 13:23:31 2026";

#[test]
fn ps_output_is_parsed_into_rows_and_the_header_is_skipped() {
    let rows = parse_ps(PS);
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0], row(1, 0, "Wed Oct 7 09:29:17 2026", "/sbin/launchd"));
    assert_eq!(rows[3].started, START_A);
    assert_eq!(rows[3].argv, ["claude", "--permission-mode", "bypassPermissions"]);
    assert_eq!(rows[5].command(), "claude --permission-mode acceptEdits --model opus");
}

#[test]
fn lines_that_are_not_rows_are_skipped() {
    let junk =
        "garbage\n12 x Wed Oct 7 09:29:17 2026 cmd\n12 1 short\n\n12 1 Wed Oct 7 nine 2026 cmd\n";
    assert_eq!(parse_ps(junk), []);
}

#[test]
fn a_start_time_has_one_spelling_whatever_the_spaces() {
    assert_eq!(normalize_start("Thu Oct  8 21:57:15 2026"), "Thu Oct 8 21:57:15 2026");
    assert_eq!(normalize_start(" Thu Oct 8 21:57:15 2026 "), "Thu Oct 8 21:57:15 2026");
}

#[test]
fn lstart_text_is_unix_millis_in_utc() {
    let cases = [
        ("Thu Jan  1 00:00:00 1970", Some(0)),
        ("Wed Mar  1 00:00:00 2000", Some(951_868_800_000)),
        ("Mon Jan  5 23:59:58 2026", Some(1_767_657_598_000)),
        ("Thu Feb 29 12:00:00 2024", Some(1_709_208_000_000)),
        ("Fri Oct 16 13:23:31 2026", Some(1_792_157_011_000)),
        ("not a date", None),
        ("Fri Foo 16 13:23:31 2026", None),
        ("Fri Oct 16 13:23 2026", None),
    ];
    for (text, want) in cases {
        assert_eq!(parse_lstart_ms(text), want, "{text}");
    }
}

#[test]
fn ancestors_walk_up_to_the_top_and_stop_on_a_loop() {
    let table = ProcessTable::new(parse_ps(PS));
    let chain: Vec<u32> = table.ancestors(19821).iter().map(|r| r.pid).collect();
    assert_eq!(chain, [11662, 90775, 1]);
    assert!(table.descends_from(19821, 90775));
    assert!(!table.descends_from(25746, 90775));
    let looped = ProcessTable::new(vec![
        row(2, 3, "a b c 1:00:00 d", "x"),
        row(3, 2, "a b c 1:00:00 d", "y"),
    ]);
    assert_eq!(looped.ancestors(2).len(), 1);
}

#[test]
fn the_host_is_the_nearest_known_app_above_the_session() {
    let cases = [
        ("/Applications/stdusk.app/Contents/MacOS/stdusk", Host::Stdusk),
        ("/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", Host::Terminal),
        ("/Applications/iTerm.app/Contents/MacOS/iTerm2", Host::ITerm2),
        (
            "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper (Plugin).app/Contents/MacOS/Code Helper (Plugin)",
            Host::VsCode,
        ),
        ("/Applications/Ghostty.app/Contents/MacOS/ghostty", Host::Ghostty),
        ("/usr/bin/wezterm-gui start", Host::WezTerm),
        ("/Applications/Warp.app/Contents/MacOS/stable", Host::Warp),
        ("tmux: server", Host::Tmux),
        ("/usr/libexec/something", Host::Other),
    ];
    for (command, want) in cases {
        let table = ProcessTable::new(vec![
            row(10, 0, "a b c 1:00:00 d", command),
            row(11, 10, "a b c 1:00:00 d", "/bin/zsh"),
            row(12, 11, "a b c 1:00:00 d", "claude"),
        ]);
        assert_eq!(host_of(&table, 12), want, "{command}");
    }
}

#[test]
fn the_nearest_host_wins_when_two_are_in_the_chain() {
    let table = ProcessTable::new(vec![
        row(10, 0, "a b c 1:00:00 d", "/Applications/iTerm.app/Contents/MacOS/iTerm2"),
        row(11, 10, "a b c 1:00:00 d", "tmux: server"),
        row(12, 11, "a b c 1:00:00 d", "claude"),
    ]);
    assert_eq!(host_of(&table, 12), Host::Tmux);
    assert_eq!(Host::Other.label(), "another terminal");
    assert_eq!(Host::VsCode.label(), "VS Code");
}

#[test]
fn registry_files_are_digits_dot_json_and_nothing_else() {
    let cases = [
        ("19821.json", Some(19821)),
        ("5.json", Some(5)),
        ("19821.abc.key", None),
        ("19821.json.key", None),
        ("19821.key", None),
        ("abc.json", None),
        (".json", None),
        ("19821", None),
        ("-5.json", None),
        ("99999999999.json", None),
    ];
    for (name, want) in cases {
        assert_eq!(registry_pid(name), want, "{name}");
    }
}

const REGISTRY: &str = r#"{"pid":19821,"sessionId":"7d198e16-6157-439f-a389-34a4951f5dd8","cwd":"/Users/me/Git/stdusk","startedAt":1792157013549,"procStart":"Fri Oct 16 13:23:31 2026","version":"2.1.301","peerProtocol":1,"kind":"interactive","entrypoint":"cli","name":"stdusk-8e","nameSource":"derived","status":"idle","updatedAt":1792157418763,"messagingSocketPath":"/tmp/x.sock","bridgeSessionId":null}"#;

#[test]
fn a_registry_file_of_claude_2_1_301_is_read() {
    let got = parse_registry_entry(REGISTRY).unwrap();
    assert_eq!(got.pid, 19821);
    assert_eq!(got.session_id, ID_A);
    assert_eq!(got.cwd, PathBuf::from("/Users/me/Git/stdusk"));
    assert_eq!(got.started_at_ms, Some(1_792_157_013_549));
    assert_eq!(got.proc_start, START_A);
    assert_eq!(got.kind.as_deref(), Some("interactive"));
    assert_eq!(got.name.as_deref(), Some("stdusk-8e"));
    assert_eq!(got.status.as_deref(), Some("idle"));
}

#[test]
fn a_registry_file_missing_what_matters_is_not_a_session() {
    let cases = [
        r#"{"sessionId":"7d198e16-6157-439f-a389-34a4951f5dd8","cwd":"/a","procStart":"x"}"#,
        r#"{"pid":1,"cwd":"/a","procStart":"Fri Oct 16 13:23:31 2026"}"#,
        r#"{"pid":1,"sessionId":"7d198e16-6157-439f-a389-34a4951f5dd8","procStart":"Fri Oct 16 13:23:31 2026"}"#,
        r#"{"pid":1,"sessionId":"7d198e16-6157-439f-a389-34a4951f5dd8","cwd":"/a"}"#,
        r#"{"pid":1,"sessionId":"x y; rm -rf","cwd":"/a","procStart":"Fri Oct 16 13:23:31 2026"}"#,
        r#"{"pid":1,"sessionId":"7d198e16-6157-439f-a389-34a4951f5dd8","cwd":"relative","procStart":"Fri Oct 16 13:23:31 2026"}"#,
        r#"{"pid":-4,"sessionId":"7d198e16-6157-439f-a389-34a4951f5dd8","cwd":"/a","procStart":"Fri Oct 16 13:23:31 2026"}"#,
        "not json",
        "[]",
    ];
    for text in cases {
        assert_eq!(parse_registry_entry(text), None, "{text}");
    }
}

#[test]
fn a_name_loses_control_characters_and_is_cut() {
    let long = "n".repeat(200);
    let text = format!(
        r#"{{"pid":1,"sessionId":"{ID_A}","cwd":"/a","procStart":"{START_A}","name":"a\u001b[31mb{long}"}}"#
    );
    let name = parse_registry_entry(&text).unwrap().name.unwrap();
    assert_eq!(name.chars().count(), NAME_MAX_CHARS);
    assert!(name.starts_with("a[31mb"));
}

#[test]
fn registry_statuses_map_to_activity() {
    let cases = [
        (Some("idle"), Activity::Idle),
        (Some("busy"), Activity::Working),
        (Some("shell"), Activity::Working),
        (Some("waiting"), Activity::Quiet),
        (None, Activity::Quiet),
    ];
    for (status, want) in cases {
        assert_eq!(Activity::of_registry_status(status), want, "{status:?}");
    }
}

#[test]
fn only_a_live_process_with_the_recorded_start_is_a_claude_session() {
    let table = ProcessTable::new(parse_ps(PS));
    let entries = [
        entry(19821, ID_A, START_A, "idle"),
        entry(25746, ID_B, "Wed Oct 14 09:59:03 2026", "busy"),
        entry(8173, "11111111-2222-3333-4444-555555555555", "Sun Oct 4 20:17:36 2026", "idle"),
        entry(25746, ID_B, "Mon Oct 12 18:01:24 2026", "idle"),
        entry(404, "22222222-2222-3333-4444-555555555555", START_A, "idle"),
    ];
    let got = claude_sessions(&entries, &table, OWN, &agents());
    let pids: Vec<u32> = got.iter().filter_map(|s| s.running().map(|r| r.pid)).collect();
    assert_eq!(pids, [19821, 25746]);
}

#[test]
fn a_claude_session_carries_its_host_activity_level_and_flags() {
    let table = ProcessTable::new(parse_ps(PS));
    let entries = [
        entry(19821, ID_A, START_A, "idle"),
        entry(25746, ID_B, "Wed Oct 14 09:59:03 2026", "busy"),
    ];
    let got = claude_sessions(&entries, &table, OWN, &agents());
    let first = got[0].running().unwrap();
    assert_eq!((first.host, first.activity), (Host::Stdusk, Activity::Idle));
    assert_eq!(got[0].flags.level, Some(crate::agents::Level::Yolo));
    assert_eq!(got[0].session_id.as_deref(), Some(ID_A));
    assert_eq!(got[0].since_ms, Some(1_792_157_013_549));
    let second = got[1].running().unwrap();
    assert_eq!((second.host, second.activity), (Host::Other, Activity::Working));
    assert_eq!(got[1].flags.level, Some(crate::agents::Level::Edits));
    assert_eq!(got[1].flags.model.as_deref(), Some("opus"));
}

#[test]
fn another_kind_of_session_is_left_out() {
    let table = ProcessTable::new(parse_ps(PS));
    let mut other = entry(19821, ID_A, START_A, "idle");
    other.kind = Some("sdk".to_owned());
    assert_eq!(claude_sessions(&[other.clone()], &table, OWN, &agents()), []);
    other.kind = None;
    assert_eq!(claude_sessions(&[other], &table, OWN, &agents()).len(), 1);
}

#[test]
fn tidewatchs_own_tabs_are_not_running_elsewhere() {
    let table = ProcessTable::new(parse_ps(PS));
    let entries = [entry(19821, ID_A, START_A, "idle")];
    assert_eq!(claude_sessions(&entries, &table, 90775, &agents()), []);
    let handed_over = ProcessTable::new(vec![row(
        30,
        1,
        START_A,
        "claude --mcp-config=/home/me/.tidewatch/.state/web/x/mcp.json --permission-mode acceptEdits",
    )]);
    let entries = [entry(30, ID_A, START_A, "idle")];
    assert_eq!(claude_sessions(&entries, &handed_over, 99, &agents()), []);
}

#[test]
fn codex_gemini_and_copilot_are_found_by_program_name() {
    let table = ProcessTable::new(vec![
        row(
            20,
            1,
            "Fri Oct 16 15:09:00 2026",
            "/opt/homebrew/bin/codex --sandbox workspace-write --ask-for-approval on-request -m gpt-5",
        ),
        row(
            21,
            1,
            "Fri Oct 16 15:09:00 2026",
            "node /opt/homebrew/bin/gemini --approval-mode auto_edit",
        ),
        row(
            22,
            1,
            "Fri Oct 16 15:09:00 2026",
            "/usr/local/bin/copilot --resume=3f2a9c1e-1111-2222-3333-444455556666 --allow-all-tools",
        ),
        row(23, 1, "Fri Oct 16 15:09:00 2026", "/opt/homebrew/bin/codex exec fix the bug"),
        row(24, 1, "Fri Oct 16 15:09:00 2026", "gemini -p summarize"),
        row(25, 1, "Fri Oct 16 15:09:00 2026", "copilot --prompt hello"),
        row(26, 1, "Fri Oct 16 15:09:00 2026", "node /tmp/server/index.js --codex"),
        row(27, 1, "Fri Oct 16 15:09:00 2026", "/opt/homebrew/bin/codex app-server --listen"),
        row(
            28,
            1,
            "Fri Oct 16 15:09:00 2026",
            "codex resume 0f5e4aa6-cdf3-464c-8123-4ddb654ee653 -m gpt-5",
        ),
    ]);
    let got = other_candidates(&table, OWN, &agents());
    let seen: Vec<(u32, Cli)> = got.iter().map(|c| (c.pid, c.cli)).collect();
    assert_eq!(seen, [(20, Cli::Codex), (21, Cli::Gemini), (22, Cli::Copilot), (28, Cli::Codex)]);
    assert_eq!(got[0].flags.level, Some(crate::agents::Level::Edits));
    assert_eq!(got[0].flags.model.as_deref(), Some("gpt-5"));
    assert_eq!(got[1].flags.level, Some(crate::agents::Level::Edits));
    assert_eq!(got[2].resume_id.as_deref(), Some("3f2a9c1e-1111-2222-3333-444455556666"));
    assert_eq!(got[2].flags.level, Some(crate::agents::Level::Auto));
    assert_eq!(got[3].resume_id.as_deref(), Some("0f5e4aa6-cdf3-464c-8123-4ddb654ee653"));
    assert_eq!(got[0].started_ms, Some(parse_lstart_ms("Fri Oct 16 15:09:00 2026").unwrap()));
}

#[test]
fn a_wrapper_and_the_binary_it_starts_count_once_as_the_outer_one() {
    let table = ProcessTable::new(vec![
        row(40, 1, "Fri Oct 16 15:09:00 2026", "node /opt/homebrew/bin/codex"),
        row(41, 40, "Fri Oct 16 15:09:01 2026", "/opt/homebrew/lib/vendor/codex/codex"),
    ]);
    let got = other_candidates(&table, OWN, &agents());
    assert_eq!(got.iter().map(|c| c.pid).collect::<Vec<_>>(), [40]);
}

#[test]
fn tidewatchs_own_codex_tab_is_not_a_candidate() {
    let table = ProcessTable::new(vec![
        row(
            50,
            1,
            "Fri Oct 16 15:09:00 2026",
            "/Applications/Tidewatch.app/Contents/MacOS/tidewatch",
        ),
        row(51, 50, "Fri Oct 16 15:09:00 2026", "/bin/zsh"),
        row(52, 51, "Fri Oct 16 15:09:01 2026", "codex --sandbox read-only"),
    ]);
    assert_eq!(other_candidates(&table, 50, &agents()), []);
}

#[test]
fn ages_read_as_the_card_shows_them() {
    let cases = [
        (0, "just now"),
        (59_000, "just now"),
        (14 * 60_000, "14 min ago"),
        (59 * 60_000, "59 min ago"),
        (2 * 3_600_000, "2h ago"),
        (47 * 3_600_000, "47h ago"),
        (72 * 3_600_000, "3d ago"),
        (-5000, "just now"),
    ];
    for (ms, want) in cases {
        assert_eq!(age_text(ms), want, "{ms}");
    }
}

#[test]
fn a_session_is_named_by_claude_else_by_its_cli_and_age() {
    let flags = Flags::default();
    let base = Session {
        cli: Cli::Codex,
        session_id: None,
        cwd: PathBuf::from("/a"),
        name: None,
        flags,
        presence: Presence::Running(Running {
            pid: 1,
            proc_start: "x".into(),
            host: Host::Other,
            activity: Activity::Quiet,
        }),
        since_ms: Some(1_000_000),
    };
    let now = 1_000_000 + 14 * 60_000;
    assert_eq!(base.display_name(now), "codex · 14 min ago");
    let ended = Session { presence: Presence::NotRunning, cli: Cli::Gemini, ..base.clone() };
    assert_eq!(ended.display_name(1_000_000 + 2 * 3_600_000), "gemini · ended 2h ago");
    let named = Session { name: Some("retry-stage-gates".into()), ..base.clone() };
    assert_eq!(named.display_name(now), "retry-stage-gates");
    let unknown_age = Session { since_ms: None, ..base };
    assert_eq!(unknown_age.display_name(now), "codex");
}

#[test]
fn a_session_key_is_its_process_or_its_stored_id() {
    let running = Session {
        cli: Cli::Claude,
        session_id: Some(ID_A.into()),
        cwd: PathBuf::from("/a"),
        name: None,
        flags: Flags::default(),
        presence: Presence::Running(Running {
            pid: 7,
            proc_start: "x".into(),
            host: Host::Other,
            activity: Activity::Idle,
        }),
        since_ms: None,
    };
    assert_eq!(running.key(), Some(Key::Process { pid: 7, proc_start: "x".into() }));
    let stored = Session { presence: Presence::NotRunning, ..running.clone() };
    assert_eq!(stored.key(), Some(Key::Stored { cli: Cli::Claude, id: ID_A.into() }));
    let anonymous = Session { session_id: None, ..stored };
    assert_eq!(anonymous.key(), None);
}

#[test]
fn the_ids_of_the_agent_table_map_to_a_cli_and_a_transcript_source() {
    for cli in Cli::ALL {
        assert_eq!(Cli::from_id(cli.id()), Some(cli));
        assert_eq!(Source::from_agent_id(cli.id()), Some(cli.source()));
    }
    assert_eq!(Cli::from_id("mine"), None);
}
