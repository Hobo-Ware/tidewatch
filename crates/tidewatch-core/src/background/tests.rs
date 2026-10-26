use serde_json::json;

use super::*;

const T0: i64 = 1_000_000;

fn launch_payload(command: &str) -> Value {
    json!({
        "session_id": "9bfa0f2f", "cwd": "/w", "permission_mode": "bypassPermissions",
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": command, "description": "bg probe", "run_in_background": true},
        "tool_use_id": "toolu_1"
    })
}

fn notice(id: &str, status: &str) -> Value {
    json!({
        "hook_event_name": "UserPromptSubmit",
        "prompt": format!("<task-notification>\n<task-id>b67</task-id>\n<tool-use-id>{id}</tool-use-id>\n<output-file>/tmp/x.output</output-file>\n<status>{status}</status>\n<summary>Background command \"bg probe\" completed (exit code 0)</summary>\n</task-notification>")
    })
}

fn started(id: &str) -> Signal {
    Signal::Started { tool_use_id: id.into(), command: format!("run {id}") }
}

fn ended(id: &str) -> Signal {
    Signal::Ended { tool_use_id: id.into() }
}

#[test]
fn a_background_bash_call_is_a_launch_and_a_foreground_one_is_not() {
    assert_eq!(
        from_hook("claude", &launch_payload("./import.sh --all")),
        Some(Signal::Started {
            tool_use_id: "toolu_1".into(),
            command: "./import.sh --all".into()
        })
    );
    let mut foreground = launch_payload("ls");
    foreground["tool_input"]["run_in_background"] = json!(false);
    let mut other_tool = launch_payload("ls");
    other_tool["tool_name"] = json!("Monitor");
    let mut post = launch_payload("ls");
    post["hook_event_name"] = json!("PostToolUse");
    let mut plain = launch_payload("ls");
    plain["tool_input"].as_object_mut().unwrap().remove("run_in_background");
    for payload in [foreground, other_tool, post, plain] {
        assert_eq!(from_hook("claude", &payload), None, "{payload}");
    }
    assert_eq!(from_hook("codex", &launch_payload("ls")), None);
    assert_eq!(from_hook("claude", &json!([1])), None);
}

#[test]
fn the_launch_names_the_command_on_one_short_line() {
    let long = format!("cd /w &&\n  ./run.sh {}", "x".repeat(200));
    let Some(Signal::Started { command, .. }) = from_hook("claude", &launch_payload(&long)) else {
        panic!("not a launch");
    };
    assert!(command.starts_with("cd /w && ./run.sh xxx"), "{command}");
    assert_eq!(command.chars().count(), COMMAND_CHARS + 3);
    assert!(command.ends_with("..."));
    assert!(!command.contains('\n'));
}

#[test]
fn a_finished_task_notification_names_the_launch_that_ended() {
    for status in ["completed", "failed", "killed"] {
        assert_eq!(from_hook("claude", &notice("toolu_1", status)), Some(ended("toolu_1")));
    }
    assert_eq!(from_hook("claude", &notice("toolu_1", "running")), None);
    let typed = json!({"hook_event_name": "UserPromptSubmit", "prompt": "fix the build"});
    assert_eq!(from_hook("claude", &typed), None);
}

#[test]
fn a_launch_is_quiet_after_ninety_seconds_with_no_report_since() {
    let mut watch = Watch::default();
    watch.on_signal(started("a"), T0);
    assert_eq!(watch.silent(None, T0 + QUIET_MS - 1).len(), 0);
    assert_eq!(watch.due(None, true, T0 + QUIET_MS - 1), None);
    assert_eq!(watch.silent(None, T0 + QUIET_MS).len(), 1);
    assert_eq!(watch.due(None, true, T0 + QUIET_MS).map(|l| l.tool_use_id.as_str()), Some("a"));
}

#[test]
fn a_line_or_report_after_the_launch_silences_it_and_one_before_does_not() {
    let mut watch = Watch::default();
    watch.on_signal(started("a"), T0);
    let late = T0 + QUIET_MS + 1;
    assert_eq!(watch.due(Some(T0 - 5_000), true, late).map(|l| l.at_ms), Some(T0));
    for at in [T0, T0 + 10_000, late] {
        assert_eq!(watch.due(Some(at), true, late), None, "{at}");
        assert_eq!(watch.silent(Some(at), late).len(), 0, "{at}");
    }
}

#[test]
fn a_launch_waits_while_the_agent_works_and_is_nudged_once() {
    let mut watch = Watch::default();
    watch.on_signal(started("a"), T0);
    let late = T0 + 10 * QUIET_MS;
    assert_eq!(watch.due(None, false, late), None);
    assert!(watch.due(None, true, late).is_some());
    watch.nudged("a");
    assert_eq!(watch.due(None, true, late + QUIET_MS), None);
    assert_eq!(watch.silent(None, late).len(), 1, "the card stays until it reports or ends");
}

#[test]
fn a_task_that_ended_needs_no_nudge_and_leaves_the_card() {
    let mut watch = Watch::default();
    watch.on_signal(started("a"), T0);
    watch.on_signal(started("b"), T0 + 1_000);
    watch.on_signal(ended("a"), T0 + 5_000);
    let late = T0 + 10 * QUIET_MS;
    assert_eq!(watch.due(None, true, late).map(|l| l.tool_use_id.as_str()), Some("b"));
    watch.on_signal(ended("b"), late);
    assert_eq!(watch.due(None, true, late), None);
    assert_eq!(watch.silent(None, late).len(), 0);
    watch.on_signal(ended("never launched"), late);
}

#[test]
fn launches_are_nudged_oldest_first_one_at_a_time_and_kept_to_a_cap() {
    let mut watch = Watch::default();
    watch.on_signal(started("a"), T0);
    watch.on_signal(started("a"), T0 + 5);
    watch.on_signal(started("b"), T0 + 10);
    let late = T0 + 10 * QUIET_MS;
    assert_eq!(watch.silent(None, late).len(), 2);
    assert_eq!(watch.due(None, true, late).map(|l| l.tool_use_id.as_str()), Some("a"));
    watch.nudged("a");
    assert_eq!(watch.due(None, true, late).map(|l| l.tool_use_id.as_str()), Some("b"));
    for n in 0..40 {
        watch.on_signal(started(&format!("x{n}")), T0 + 100 + n);
    }
    assert_eq!(watch.silent(None, T0 + 100 * QUIET_MS).len(), MAX_LAUNCHES);
}

#[test]
fn the_next_wake_is_the_quiet_moment_then_every_minute() {
    let mut watch = Watch::default();
    assert_eq!(watch.next_change_ms(None, T0), None);
    watch.on_signal(started("a"), T0);
    assert_eq!(watch.next_change_ms(None, T0 + 1_000), Some(T0 + QUIET_MS));
    let quiet = T0 + QUIET_MS + 5_000;
    assert_eq!(watch.next_change_ms(None, quiet), Some(quiet + 60_000));
    assert_eq!(watch.next_change_ms(Some(T0 + 1), quiet), None);
}

#[test]
fn the_nudge_names_the_command_and_carries_the_one_liner() {
    let text = nudge_text("./import.sh --all");
    assert!(text.contains("`./import.sh --all`"));
    assert!(text.contains(PROGRESS_EXAMPLE));
    assert!(text.contains("$TIDEWATCH_PROGRESS") && text.contains("tail the file"));
    assert!(!text.contains('\n') && !text.contains('\u{2014}') && !text.contains('\u{2013}'));
}
