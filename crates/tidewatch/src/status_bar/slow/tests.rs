use super::*;

fn call(id: u64, label: &str, secs: u64) -> Call {
    Call { id, label: label.to_owned(), elapsed: Duration::from_secs(secs) }
}

#[test]
fn only_a_call_past_ten_seconds_shows() {
    let cases = [
        (vec![], None),
        (vec![call(1, "git status", 3)], None),
        (vec![call(1, "git status", 9)], None),
        (
            vec![call(1, "git fetch", 10)],
            Some(Line { id: 1, text: "git fetch is taking long \u{b7} 10 s".to_owned() }),
        ),
    ];
    for (calls, want) in cases {
        assert_eq!(pick(&calls), want, "{calls:?}");
    }
}

#[test]
fn the_oldest_call_shows_and_the_others_past_the_mark_are_counted() {
    let calls = [
        call(1, "gh pr view", 14),
        call(2, "git worktree add", 61),
        call(3, "git fetch", 12),
        call(4, "git status", 2),
    ];
    assert_eq!(
        pick(&calls),
        Some(Line {
            id: 2,
            text: "git worktree add is taking long \u{b7} 1 min 1 s \u{b7} and 2 more".to_owned()
        })
    );
}

#[test]
fn the_line_is_gone_once_the_slow_call_ends() {
    let mut calls = vec![call(1, "git fetch", 30), call(2, "gh pr view", 11)];
    assert_eq!(pick(&calls).map(|line| line.id), Some(1));
    calls.remove(0);
    assert_eq!(pick(&calls).map(|line| line.id), Some(2));
    calls.remove(0);
    assert_eq!(pick(&calls), None);
}

#[test]
fn elapsed_time_reads_in_seconds_then_minutes() {
    let cases = [(0, "0 s"), (14, "14 s"), (59, "59 s"), (60, "1 min 0 s"), (125, "2 min 5 s")];
    for (secs, want) in cases {
        assert_eq!(elapsed_text(Duration::from_secs(secs)), want);
    }
}

#[test]
fn the_window_wakes_once_a_second_while_the_line_shows_and_at_the_mark_before_it() {
    let cases = [
        (vec![], None),
        (vec![call(1, "git status", 0)], Some(Duration::from_secs(10))),
        (vec![call(1, "git status", 4), call(2, "git fetch", 7)], Some(Duration::from_secs(3))),
        (vec![call(1, "git fetch", 10)], Some(Duration::from_secs(1))),
        (vec![call(1, "git fetch", 40), call(2, "gh pr view", 1)], Some(Duration::from_secs(1))),
    ];
    for (calls, want) in cases {
        assert_eq!(next_look(&calls), want, "{calls:?}");
    }
}

#[test]
fn a_line_fits_only_when_its_cancel_ends_before_the_limit() {
    assert!(fits(100.0, 200.0, 60.0, 400.0));
    assert!(fits(100.0, 200.0, 60.0, 370.0));
    assert!(!fits(100.0, 200.0, 60.0, 369.0));
    assert!(!fits(500.0, 10.0, 60.0, 400.0));
}

#[test]
fn running_calls_become_calls_with_the_time_they_have_run() {
    let now = Instant::now();
    let running = [Running {
        id: 7,
        label: "git fetch".to_owned(),
        started: now.checked_sub(Duration::from_secs(12)).unwrap(),
    }];
    assert_eq!(calls(&running, now), [call(7, "git fetch", 12)]);
}
