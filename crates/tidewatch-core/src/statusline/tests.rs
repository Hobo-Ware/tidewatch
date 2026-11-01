use super::*;

const NOW: i64 = 1_793_000_000_000;

const SAMPLE: &str = r#"{
  "session_id": "abc",
  "model": {"id": "claude-opus-4-7", "display_name": "Opus"},
  "workspace": {"current_dir": "/work/web", "project_dir": "/work/web"},
  "version": "2.1.316",
  "cost": {"total_cost_usd": 20.51, "total_duration_ms": 91000, "total_lines_added": 4},
  "context_window": {"used_percentage": 12},
  "rate_limits": {
    "five_hour": {"used_percentage": 31.4, "resets_at": 1793007800},
    "seven_day": {"used_percentage": 84, "resets_at": 1793260000}
  }
}"#;

fn window(used_pct: f64, resets_at_ms: i64) -> Window {
    Window { used_pct, resets_at_ms: Some(resets_at_ms) }
}

#[test]
fn the_real_field_names_of_claude_code_2_1_316_give_both_windows_and_the_cost() {
    let reading = parse(SAMPLE, NOW).unwrap();
    assert_eq!(
        reading,
        Reading {
            limits: PlanLimits {
                five_hour: Some(window(31.4, 1_793_007_800_000)),
                week: Some(window(84.0, 1_793_260_000_000)),
            },
            cost_usd: Some(20.51),
            read_ms: NOW,
        }
    );
}

#[test]
fn a_field_that_is_missing_or_not_a_number_is_left_out() {
    let cases = [
        (r#"{"cost":{"total_cost_usd":1.5}}"#, PlanLimits::default(), Some(1.5)),
        (r#"{"rate_limits":{}}"#, PlanLimits::default(), None),
        (
            r#"{"rate_limits":{"five_hour":{"used_percentage":"high","resets_at":5}}}"#,
            PlanLimits::default(),
            None,
        ),
        (
            r#"{"rate_limits":{"seven_day":{"used_percentage":10}},"cost":{"total_cost_usd":"x"}}"#,
            PlanLimits {
                five_hour: None,
                week: Some(Window { used_pct: 10.0, resets_at_ms: None }),
            },
            None,
        ),
        (r#"{"cost":{"total_cost_usd":-3}}"#, PlanLimits::default(), None),
    ];
    for (input, limits, cost_usd) in cases {
        let reading = parse(input, NOW).unwrap();
        assert_eq!((reading.limits, reading.cost_usd), (limits, cost_usd), "{input}");
    }
}

#[test]
fn a_percent_outside_the_scale_is_held_to_it() {
    let high = parse(r#"{"rate_limits":{"five_hour":{"used_percentage":140}}}"#, NOW).unwrap();
    assert_eq!(high.limits.five_hour.map(|window| window.used_pct), Some(100.0));
}

#[test]
fn input_that_is_not_a_json_object_is_no_reading() {
    for input in ["", "not json", "[1,2]", "42", "{\"cost\":"] {
        assert_eq!(parse(input, NOW), None, "{input:?}");
    }
}

#[test]
fn an_input_with_no_limit_and_no_cost_has_nothing_to_save() {
    assert!(!parse("{}", NOW).unwrap().has_data());
    assert!(!parse(r#"{"model":{"id":"x"}}"#, NOW).unwrap().has_data());
    assert!(parse(SAMPLE, NOW).unwrap().has_data());
}

#[test]
fn what_the_new_input_lacks_stays_as_it_was_saved() {
    let saved = parse(SAMPLE, NOW - 5000).unwrap();
    let early = parse(r#"{"cost":{"total_cost_usd":0.2}}"#, NOW).unwrap();
    let merged = early.over(Some(saved));
    assert_eq!(merged.limits, saved.limits);
    assert_eq!(merged.cost_usd, Some(0.2));
    assert_eq!(merged.read_ms, NOW);
    assert_eq!(early.over(None), early);
}

#[test]
fn a_new_window_replaces_the_saved_one() {
    let saved = parse(SAMPLE, NOW - 5000).unwrap();
    let newer = parse(
        r#"{"rate_limits":{"five_hour":{"used_percentage":50,"resets_at":1793009000}}}"#,
        NOW,
    )
    .unwrap();
    let merged = newer.over(Some(saved));
    assert_eq!(merged.limits.five_hour, Some(window(50.0, 1_793_009_000_000)));
    assert_eq!(merged.limits.week, saved.limits.week);
}

#[test]
fn the_reading_goes_to_disk_and_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plan-limits.json");
    assert_eq!(load(&file), None);
    let reading = parse(SAMPLE, NOW).unwrap();
    save(&file, &reading).unwrap();
    assert_eq!(load(&file), Some(reading));
    std::fs::write(&file, "{ broken").unwrap();
    assert_eq!(load(&file), None);
}

#[test]
fn the_newest_reading_with_a_limit_gives_the_limits() {
    let old = parse(SAMPLE, NOW - 9000).unwrap();
    let new = parse(
        r#"{"rate_limits":{"five_hour":{"used_percentage":70,"resets_at":1793009000}}}"#,
        NOW,
    )
    .unwrap();
    let cost_only = parse(r#"{"cost":{"total_cost_usd":1}}"#, NOW + 1000).unwrap();
    assert_eq!(newest_limits([&old, &new, &cost_only]), Some(new.limits));
    assert_eq!(newest_limits([&cost_only]), None);
    assert_eq!(newest_limits([]), None);
}

const FIRST_SCREEN: &str = r#"{"session_id":"bcd87bd1-d8ea-4b17-8eff-898d9fc25c09","transcript_path":"/home/me/.claude/projects/-work-web/bcd87bd1.jsonl","cwd":"/work/web","effort":{"level":"xhigh"},"model":{"id":"claude-haiku-5-5","display_name":"Haiku 5.5"},"workspace":{"current_dir":"/work/web","project_dir":"/work/web","added_dirs":[],"git_worktree":"web"},"version":"2.1.316","output_style":{"name":"default"},"cost":{"total_cost_usd":0,"total_duration_ms":378,"total_api_duration_ms":0,"total_lines_added":0,"total_lines_removed":0},"context_window":{"total_input_tokens":0,"total_output_tokens":0,"context_window_size":1000000,"current_usage":null,"used_percentage":null,"remaining_percentage":null},"exceeds_200k_tokens":false,"fast_mode":false,"thinking":{"enabled":true}}"#;

#[test]
fn the_input_of_a_real_session_before_its_first_answer_has_a_cost_and_no_limits() {
    let reading = parse(FIRST_SCREEN, NOW).unwrap();
    assert_eq!(reading.limits, PlanLimits::default());
    assert_eq!(reading.cost_usd, Some(0.0));
    assert!(reading.has_data());
}
