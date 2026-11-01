use super::*;
use egui::{Context, Event, PointerButton, Pos2, RawInput};

fn summary() -> Summary {
    Summary {
        number: 42,
        title: "Retry a stage once when its tests fail".into(),
        url: "https://github.com/acme/web/pull/42".into(),
        state: State::Open,
        checks: SummaryChecks::Running,
        comments: 1,
        decision: Decision::None,
        conflicting: false,
    }
}

fn texts(summary: &Summary) -> Vec<(String, Tone)> {
    pills(summary).into_iter().map(|pill| (pill.text, pill.tone)).collect()
}

#[test]
fn the_pills_follow_the_checks_the_review_conflicts_and_comments() {
    let mut s = summary();
    assert_eq!(
        texts(&s),
        [("Checks running".to_owned(), Tone::Warn), ("1 comment".to_owned(), Tone::Quiet)]
    );
    s.checks = SummaryChecks::Failed;
    s.decision = Decision::ChangesRequested;
    s.conflicting = true;
    s.comments = 3;
    assert_eq!(
        texts(&s),
        [
            ("Checks failed".to_owned(), Tone::Bad),
            ("Changes requested".to_owned(), Tone::Warn),
            ("Conflicts".to_owned(), Tone::Warn),
            ("3 comments".to_owned(), Tone::Quiet),
        ]
    );
    s.checks = SummaryChecks::None;
    s.decision = Decision::Approved;
    s.conflicting = false;
    s.comments = 0;
    assert_eq!(texts(&s), [("Approved".to_owned(), Tone::Good)]);
    s.decision = Decision::ReviewRequired;
    assert_eq!(texts(&s), []);
}

#[test]
fn a_merged_pull_request_leads_with_merged_and_claims_no_open_checks_or_review() {
    let mut s = summary();
    s.state = State::Merged;
    s.checks = SummaryChecks::Passed;
    s.decision = Decision::Approved;
    s.conflicting = true;
    assert_eq!(
        texts(&s),
        [("Merged".to_owned(), Tone::Good), ("1 comment".to_owned(), Tone::Quiet)]
    );
    assert!(pills(&s)[0].outlined);
    assert!(!pills(&s)[1].outlined);
}

#[test]
fn a_closed_pull_request_leads_with_closed_and_claims_no_open_checks_or_review() {
    let mut s = summary();
    s.state = State::Closed;
    s.checks = SummaryChecks::Failed;
    s.decision = Decision::ChangesRequested;
    s.comments = 0;
    assert_eq!(texts(&s), [("Closed".to_owned(), Tone::Bad)]);
}

#[test]
fn the_chip_names_the_mode_in_few_words() {
    assert_eq!(mode_label(PrMode::Ask), "Ask me");
    assert_eq!(mode_label(PrMode::Auto), "Auto");
    assert_eq!(mode_label(PrMode::Off), "Off");
    for (mode, _) in MODES {
        assert_ne!(mode_help(mode), "");
    }
}

#[test]
fn the_tones_read_apart() {
    let inks = [Tone::Bad, Tone::Warn, Tone::Good, Tone::Quiet].map(tone_ink);
    for (n, ink) in inks.iter().enumerate() {
        assert!(!inks[n + 1..].contains(ink), "{ink:?}");
    }
}

#[test]
fn a_merged_or_closed_strip_goes_an_hour_after_github_ended_it_and_an_open_one_stays() {
    const MINUTE: i64 = 60 * 1000;
    let now = 1_793_563_198_000;
    let cases = [
        ("open with no end time", State::Open, None, true),
        ("reopened after a close two hours ago", State::Open, Some(now - 120 * MINUTE), true),
        ("merged 10 minutes ago", State::Merged, Some(now - 10 * MINUTE), true),
        ("merged 59 minutes ago", State::Merged, Some(now - 59 * MINUTE), true),
        ("merged an hour ago", State::Merged, Some(now - 60 * MINUTE), false),
        ("merged 61 minutes ago", State::Merged, Some(now - 61 * MINUTE), false),
        ("closed 10 minutes ago", State::Closed, Some(now - 10 * MINUTE), true),
        ("closed 2 hours ago", State::Closed, Some(now - 120 * MINUTE), false),
        ("merged with no known time", State::Merged, None, false),
        ("closed with no known time", State::Closed, None, false),
        ("merged a few seconds ahead of this clock", State::Merged, Some(now + 5_000), true),
    ];
    for (name, state, ended_at, want) in cases {
        assert_eq!(shows(state, ended_at, now), want, "{name}");
    }
}

/// Runs the strip over a few frames of pointer events and returns the last action it reported.
fn frames(events: Vec<Vec<Event>>, strip: &Strip<'_>) -> (Vec<Option<Action>>, Rect) {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let band = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, HEIGHT));
    let mut seen = Vec::new();
    for batch in std::iter::once(Vec::new()).chain(events) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 200.0))),
            events: batch,
            focused: true,
            ..RawInput::default()
        };
        let mut action = None;
        let _ = ctx.run_ui(raw, |ui| action = show(ui, band, strip));
        seen.push(action);
    }
    (seen, band)
}

fn click_at(pos: Pos2) -> Vec<Vec<Event>> {
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![vec![Event::PointerMoved(pos)], vec![button(true)], vec![button(false)]]
}

#[test]
fn a_click_on_open_on_github_asks_for_the_page_and_nothing_else_does() {
    let summary = summary();
    let strip = Strip {
        summary: &summary,
        policy: GithubPrefs::default(),
        dev_open_popover: false,
        updates: updates_line(false, None),
    };
    let (quiet, band) = frames(vec![vec![]], &strip);
    assert_eq!(quiet, [None, None]);
    let link = pos2(band.right() - INSET - 14.0 - 10.0, band.center().y);
    let (clicked, _) = frames(click_at(link), &strip);
    assert_eq!(clicked.last().cloned().flatten(), Some(Action::Open(summary.url.clone())));
    let on_the_title = pos2(band.left() + 200.0, band.center().y);
    let (clicked, _) = frames(click_at(on_the_title), &strip);
    assert!(clicked.iter().all(Option::is_none));
}

#[test]
fn the_strip_draws_for_a_merged_closed_and_long_titled_pull_request() {
    let mut s = summary();
    s.title = "x".repeat(400);
    for state in [State::Open, State::Merged, State::Closed] {
        s.state = state;
        let strip = Strip {
            summary: &s,
            policy: GithubPrefs::default(),
            dev_open_popover: true,
            updates: updates_line(true, None),
        };
        let (seen, _) = frames(vec![vec![], vec![]], &strip);
        assert!(seen.iter().all(Option::is_none), "{state:?}");
    }
}

#[test]
fn the_popover_says_auto_and_lists_the_four_jobs_with_push_last_and_explained() {
    let labels: Vec<&str> = MODES.iter().map(|(_, label)| *label).collect();
    assert_eq!(labels, ["Ask me", "Auto", "Off"]);
    assert!(mode_help(PrMode::Auto).starts_with("Auto does the ticked jobs by itself"));
    let jobs: Vec<&str> = JOBS.iter().map(|job| job.label).collect();
    assert_eq!(
        jobs,
        [
            "Fix failed checks, with the failing log lines",
            "Answer review comments and requested changes",
            "Fix merge conflicts with the base branch",
            "Ask the agent to open the pull request",
        ]
    );
    assert_eq!(JOBS.iter().filter(|job| job.caption.is_some()).count(), 1);
    assert_eq!(JOBS[3].which, AutoJob::Push);
}

#[test]
fn the_number_tooltip_says_live_or_how_often_tidewatch_looks_and_why_not_live() {
    let cases = [
        (true, None, "Live from GitHub"),
        (false, None, "Checking every 2 min"),
        (
            false,
            Some(Why::NoAdmin),
            "Checking every 2 min (it needs admin rights on the repository)",
        ),
        (
            false,
            Some(Why::HookTaken),
            "Checking every 2 min (another forwarder holds the repository's webhook)",
        ),
    ];
    for (live, why, want) in cases {
        assert_eq!(updates_line(live, why), want);
    }
}
