use tidewatch_core::link_board::TitleFrom;

use super::*;

fn link(url: &str, title: &str, note: Option<&str>, ago_min: i64) -> Link {
    Link {
        url: url.to_owned(),
        title: title.to_owned(),
        note: note.map(str::to_owned),
        first_seen_ms: 100 * MIN - ago_min * MIN,
        title_from: TitleFrom::Agent,
        state: LinkState::Unknown,
        status: Status::default(),
    }
}

fn with_state(url: &str, ago_min: i64, state: LinkState) -> Link {
    Link { state, ..link(url, "t", None, ago_min) }
}

fn with_status(state: LinkState, status: Status) -> Link {
    Link { state, status, ..link("https://github.com/o/r/pull/1", "t", None, 1) }
}

fn titles(links: &[&Link]) -> Vec<String> {
    links.iter().map(|link| link.title.clone()).collect()
}

fn rows(count: usize) -> LinkBoard {
    let mut board = LinkBoard::default();
    for n in 0..count {
        board.absorb(&link(&format!("https://a.dev/{n}"), &format!("L{n}"), None, 50 - n as i64));
    }
    board
}

#[test]
fn every_kind_has_its_own_ink_from_the_brand() {
    let inks: Vec<Color32> = Kind::ALL.map(ink).into();
    for (at, color) in inks.iter().enumerate() {
        assert_eq!(inks.iter().filter(|other| *other == color).count(), 1, "{at}");
    }
    assert_eq!(ink(Kind::Artifact), brand::VIOLET_TEXT);
    assert_eq!(ink(Kind::Pr), brand::TEAL);
}

#[test]
fn three_rows_show_and_all_shows_every_row() {
    let board = rows(5);
    let all = board.newest_first();
    assert_eq!(titles(listed(&all, false)), ["L4", "L3", "L2"]);
    assert_eq!(listed(&all, true).len(), 5);
    let small = rows(2);
    assert_eq!(listed(&small.newest_first(), false).len(), 2);
}

#[test]
fn the_fold_says_show_all_then_show_less_and_is_gone_when_nothing_is_hidden() {
    assert_eq!(fold_label(0, false), None);
    assert_eq!(fold_label(3, false), None);
    assert_eq!(fold_label(3, true), None);
    assert_eq!(fold_label(5, false).as_deref(), Some("Show all 5"));
    assert_eq!(fold_label(5, true).as_deref(), Some("Show less"));
}

#[test]
fn the_meta_line_is_host_then_for_then_age() {
    let now = 100 * MIN;
    let with_for = link("https://claude.ai/artifact/x", "t", Some("Which split?"), 12);
    assert_eq!(meta(&with_for, now, false), "claude.ai \u{b7} for: Which split? \u{b7} 12m ago");
    let blank = link("https://a.dev/x", "t", Some("  "), 0);
    assert_eq!(meta(&blank, now, false), "a.dev \u{b7} just now");
    let day = link("https://a.dev/x", "t", None, 3000);
    assert_eq!(meta(&day, now, true), "a.dev \u{b7} 2d ago", "live says nothing about a web page");
    for line in [meta(&with_for, now, false), meta(&blank, now, false)] {
        assert!(!line.contains(['\u{2014}', '\u{2013}']), "{line}");
    }
}

#[test]
fn the_meta_of_a_pull_request_names_the_repo_draft_comments_and_the_update_time() {
    let now = 100 * MIN;
    let status = Status {
        draft: true,
        comments: Some(3),
        updated_ms: Some(now - 14 * MIN),
        checked_ms: Some(now - 2 * MIN),
        ..Status::default()
    };
    let mut pull = with_status(LinkState::Open, status);
    pull.note = Some("the opt-out flow".to_owned());
    assert_eq!(
        meta(&pull, now, true),
        "o/r \u{b7} draft \u{b7} for: the opt-out flow \u{b7} 3 comments \u{b7} updated 14m ago"
    );
    assert_eq!(
        meta(&pull, now, false),
        "o/r \u{b7} draft \u{b7} for: the opt-out flow \u{b7} 3 comments \u{b7} updated 14m ago \u{b7} checked 2m ago",
        "a repo nobody delivers for says when it was last looked at"
    );
    let one = with_status(LinkState::Open, Status { comments: Some(1), ..Status::default() });
    assert_eq!(meta(&one, now, true), "o/r \u{b7} 1 comment \u{b7} 1m ago");
    let unlooked = with_status(LinkState::Unknown, Status::default());
    assert_eq!(meta(&unlooked, now, false), "o/r \u{b7} 1m ago", "nothing to say it was checked");
}

#[test]
fn read_first_holds_the_links_sent_for_the_open_question_only() {
    let mut board = LinkBoard::default();
    board.absorb(&link("https://a.dev/1", "For it", Some("Which split?"), 5));
    board.absorb(&link("https://a.dev/2", "For another", Some("Which date?"), 4));
    board.absorb(&link("https://a.dev/3", "For nothing", None, 3));
    board.absorb(&link("https://a.dev/4", "Also for it", Some(" Which split? "), 2));
    let titles: Vec<&str> =
        read_first(&board, "Which split?").iter().map(|link| link.title.as_str()).collect();
    assert_eq!(titles, ["For it", "Also for it"]);
    assert_eq!(read_first(&board, " Which split? ").len(), 2);
    assert_eq!(read_first(&board, "Which split").len(), 0);
    assert_eq!(read_first(&board, "").len(), 0);
}

fn own<'a>(number: u64, url: &'a str, title: Option<&'a str>) -> OwnPr<'a> {
    OwnPr { number, url, title, state: LinkState::Unknown, status: Status::default() }
}

#[test]
fn the_tabs_pull_request_is_a_link_titled_with_its_number_and_title_when_known() {
    let named = pr_link(&own(42, "https://github.com/o/r/pull/42", Some("feat: dashboard")), 9);
    assert_eq!((named.title.as_str(), named.title_from), ("#42 feat: dashboard", TitleFrom::Seen));
    for title in [None, Some(""), Some("  ")] {
        let bare = pr_link(&own(42, "https://github.com/o/r/pull/42", title), 9);
        assert_eq!((bare.title.as_str(), bare.title_from), ("PR #42", TitleFrom::Guess));
    }
    let long = pr_link(&own(7, "https://github.com/o/r/pull/7", Some(&"w".repeat(300))), 9);
    assert_eq!(long.title.chars().count(), 120);
    assert_eq!(named.kind(), Kind::Pr);
}

#[test]
fn the_tabs_pull_request_link_carries_the_watchs_status_stamped_with_the_time() {
    let status = Status { checks: Some(Checks::Fail), ..Status::default() };
    let watched =
        OwnPr { status: status.clone(), ..own(42, "https://github.com/o/r/pull/42", None) };
    let made = pr_link(&watched, 77);
    assert_eq!(made.status, Status { checked_ms: Some(77), ..status });
}

#[test]
fn the_fake_board_shows_every_state_a_row_can_have() {
    let board = fake_board(1_000 * MIN);
    assert_eq!(board.len(), 10);
    assert_eq!(read_first(&board, "Which split for the bonus pool?").len(), 1);
    let kinds: Vec<Kind> = board.links.iter().map(Link::kind).collect();
    for kind in Kind::ALL {
        assert!(kinds.contains(&kind), "{kind:?}");
    }
    let needing = board.links.iter().filter(|link| link.needs_action()).count();
    assert_eq!(needing, 2);
    let live: Vec<bool> =
        board.links.iter().filter_map(Link::repo).map(|repo| repo == FAKE_LIVE_REPO).collect();
    assert_eq!(live.iter().filter(|live| **live).count(), 6);
    assert_eq!(live.iter().filter(|live| !**live).count(), 1);
}

#[test]
fn a_lookup_is_due_for_a_link_still_going_when_never_looked_up_or_looked_up_long_enough_ago() {
    let pr = "https://github.com/o/r/pull/1";
    let issue = "https://github.com/o/r/issues/2";
    let now = 1_000 * MIN;
    let cases = [
        (pr, LinkState::Unknown, None, Reach::Back, true),
        (issue, LinkState::Unknown, None, Reach::Live, true),
        (pr, LinkState::Open, Some(now - 29 * MIN), Reach::Back, false),
        (pr, LinkState::Open, Some(now - 30 * MIN), Reach::Back, true),
        (pr, LinkState::Open, Some(now - 29 * MIN), Reach::Live, false),
        (pr, LinkState::Open, Some(now - 30 * MIN), Reach::Live, true),
        (pr, LinkState::Open, Some(now - 4 * MIN), Reach::Front, false),
        (pr, LinkState::Open, Some(now - 5 * MIN), Reach::Front, true),
        (pr, LinkState::Open, Some(now), Reach::Front, false),
        (pr, LinkState::Merged, None, Reach::Front, false),
        (issue, LinkState::Closed, Some(now - 90 * MIN), Reach::Front, false),
        ("https://a.dev/1", LinkState::Unknown, None, Reach::Front, false),
        ("https://claude.ai/artifact/x", LinkState::Unknown, None, Reach::Front, false),
        ("https://docs.google.com/document/d/1", LinkState::Unknown, None, Reach::Front, false),
    ];
    for (url, state, last, reach, want) in cases {
        assert_eq!(
            check_due(&with_state(url, 1, state), last, now, reach, false),
            want,
            "{url} {state:?} {last:?} {reach:?}"
        );
    }
}

#[test]
fn news_brings_a_lookup_after_2_seconds_whatever_the_reach() {
    let now = 1_000 * MIN;
    let open = with_state("https://github.com/o/r/pull/1", 1, LinkState::Open);
    for reach in [Reach::Live, Reach::Front, Reach::Back] {
        assert!(!check_due(&open, Some(now - 1_999), now, reach, true), "{reach:?}");
        assert!(check_due(&open, Some(now - 2_000), now, reach, true), "{reach:?}");
        assert!(check_due(&open, None, now, reach, true), "{reach:?}");
    }
    let merged = with_state("https://github.com/o/r/pull/1", 1, LinkState::Merged);
    assert!(!check_due(&merged, None, now, Reach::Live, true), "a merged link is not looked up");
}

#[test]
fn a_link_gone_or_without_access_is_never_looked_up_again() {
    let now = 1_000 * MIN;
    for access in [Access::Gone, Access::NoAccess] {
        let link = with_status(LinkState::Open, Status { access, ..Status::default() });
        assert!(!check_due(&link, None, now, Reach::Front, false), "{access:?}");
        assert!(!check_due(&link, None, now, Reach::Front, true), "{access:?}");
    }
}

fn texts(link: &Link, live: bool) -> Vec<&'static str> {
    pills(link, live).iter().map(|pill| pill.text).collect()
}

#[test]
fn a_pull_request_row_says_what_wants_the_user_first_then_how_it_stands_then_live() {
    let all = Status {
        checks: Some(Checks::Fail),
        review: Some(Review::ChangesRequested),
        conflicts: true,
        ..Status::default()
    };
    assert_eq!(
        texts(&with_status(LinkState::Open, all), true),
        ["checks failing", "changes requested", "conflicts", "live"]
    );
    let good =
        Status { checks: Some(Checks::Pass), review: Some(Review::Approved), ..Status::default() };
    assert_eq!(texts(&with_status(LinkState::Open, good), false), ["approved", "checks pass"]);
    let running = Status { checks: Some(Checks::Running), ..Status::default() };
    assert_eq!(texts(&with_status(LinkState::Open, running), false), ["checks running"]);
    assert_eq!(texts(&with_status(LinkState::Open, Status::default()), true), ["open", "live"]);
    assert_eq!(
        texts(&with_status(LinkState::Unknown, Status::default()), false),
        Vec::<&str>::new()
    );
}

#[test]
fn a_finished_row_says_how_it_ended_and_is_never_live() {
    let issue = |reason| Link {
        url: "https://github.com/o/r/issues/2".to_owned(),
        ..with_status(LinkState::Closed, Status { reason, ..Status::default() })
    };
    assert_eq!(texts(&issue(Some(Reason::Completed)), true), ["completed"]);
    assert_eq!(texts(&issue(Some(Reason::NotPlanned)), true), ["not planned"]);
    assert_eq!(texts(&issue(None), true), ["closed"]);
    assert_eq!(texts(&with_status(LinkState::Merged, Status::default()), true), ["merged"]);
}

#[test]
fn a_link_gone_or_out_of_reach_says_so_and_nothing_else() {
    let failing = Status { checks: Some(Checks::Fail), ..Status::default() };
    let gone = Status { access: Access::Gone, ..failing.clone() };
    let denied = Status { access: Access::NoAccess, ..failing };
    assert_eq!(texts(&with_status(LinkState::Open, gone), true), ["gone"]);
    assert_eq!(texts(&with_status(LinkState::Open, denied), true), ["no access"]);
}

#[test]
fn a_web_page_has_no_pills() {
    assert_eq!(texts(&link("https://a.dev/1", "t", None, 1), true), Vec::<&str>::new());
}

#[test]
fn every_tone_is_used_by_some_pill_and_no_text_holds_a_dash() {
    let rows = [
        with_status(
            LinkState::Open,
            Status {
                checks: Some(Checks::Fail),
                review: Some(Review::Approved),
                conflicts: true,
                ..Status::default()
            },
        ),
        with_status(LinkState::Merged, Status::default()),
        with_status(LinkState::Open, Status::default()),
        with_status(LinkState::Open, Status { access: Access::NoAccess, ..Status::default() }),
    ];
    let mut seen: Vec<Tone> =
        rows.iter().flat_map(|link| pills(link, true)).map(|p| p.tone).collect();
    seen.sort_by_key(|tone| *tone as u8);
    seen.dedup();
    assert_eq!(seen, [Tone::Bad, Tone::Warn, Tone::Good, Tone::Quiet, Tone::Live]);
    let all = rows.iter().flat_map(|link| pills(link, true));
    assert!(all.into_iter().all(|pill| !pill.text.contains(['\u{2014}', '\u{2013}'])));
}

#[test]
fn the_watchs_summary_gives_the_pull_request_its_title_status_and_state_else_the_lookup_gives_it_bare()
 {
    use tidewatch_core::pr_status::{Decision, State, SummaryChecks};
    let summary = Summary {
        number: 42,
        title: "feat: dashboard".into(),
        url: "https://github.com/o/r/pull/42".into(),
        state: State::Open,
        checks: SummaryChecks::Failed,
        comments: 2,
        decision: Decision::ChangesRequested,
        conflicting: false,
    };
    let lookup = PullRequest {
        number: 7,
        url: "https://github.com/o/r/pull/7".into(),
        head_sha: "abc".into(),
    };
    let from_summary = own_pr(Some(&summary), Some(&lookup)).unwrap();
    assert_eq!(
        (from_summary.number, from_summary.url, from_summary.title, from_summary.state),
        (42, "https://github.com/o/r/pull/42", Some("feat: dashboard"), LinkState::Open)
    );
    assert_eq!(
        from_summary.status,
        Status {
            checks: Some(Checks::Fail),
            review: Some(Review::ChangesRequested),
            comments: Some(2),
            ..Status::default()
        },
        "the status carries no look time, so the same summary reads the same on every frame"
    );
    let from_lookup = own_pr(None, Some(&lookup)).unwrap();
    assert_eq!(
        (from_lookup.number, from_lookup.url, from_lookup.title, from_lookup.state),
        (7, "https://github.com/o/r/pull/7", None, LinkState::Unknown)
    );
    assert_eq!(own_pr(None, None), None);
    let merged = Summary { state: State::Merged, ..summary };
    let merged = own_pr(Some(&merged), None).unwrap();
    assert_eq!((merged.state, merged.status.checks), (LinkState::Merged, None));
}
