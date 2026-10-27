use tidewatch_core::link_board::TitleFrom;
use tidewatch_core::pr_status::State;

use super::*;

fn link(url: &str, title: &str, note: Option<&str>, ago_min: i64) -> Link {
    Link {
        url: url.to_owned(),
        title: title.to_owned(),
        note: note.map(str::to_owned),
        first_seen_ms: 100 * MIN - ago_min * MIN,
        title_from: TitleFrom::Agent,
        state: LinkState::Unknown,
    }
}

fn with_state(url: &str, ago_min: i64, state: LinkState) -> Link {
    Link { state, ..link(url, "t", None, ago_min) }
}

fn titles(links: &[&Link]) -> Vec<String> {
    links.iter().map(|link| link.title.clone()).collect()
}

fn board(count: usize) -> LinkBoard {
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
fn three_rows_show_newest_first_and_all_shows_every_row() {
    let board = board(5);
    let (main, _) = split(&board);
    assert_eq!(titles(listed(&main, false)), ["L4", "L3", "L2"]);
    assert_eq!(listed(&main, true).len(), 5);
    let small = self::board(2);
    assert_eq!(listed(&split(&small).0, false).len(), 2);
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
    assert_eq!(meta(&with_for, now), "claude.ai \u{b7} for: Which split? \u{b7} 12m ago");
    let bare = link("https://www.github.com/o/r/pull/1", "t", None, 40);
    assert_eq!(meta(&bare, now), "github.com \u{b7} 40m ago");
    let blank = link("https://a.dev/x", "t", Some("  "), 0);
    assert_eq!(meta(&blank, now), "a.dev \u{b7} just now");
    let day = link("https://a.dev/x", "t", None, 3000);
    assert_eq!(meta(&day, now), "a.dev \u{b7} 2d ago");
    for line in [meta(&with_for, now), meta(&bare, now)] {
        assert!(!line.contains(['\u{2014}', '\u{2013}']), "{line}");
    }
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
    OwnPr { number, url, title, state: LinkState::Unknown }
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
fn the_fake_board_has_five_links_in_the_main_list_and_three_merged_or_closed() {
    let board = fake_board(1_000 * MIN);
    assert_eq!(board.len(), 8);
    let (main, archived) = split(&board);
    assert_eq!(
        titles(listed(&main, false)),
        [
            "Split options compared",
            "#42 feat: profit and spend dashboard",
            "Mercury spend export, October"
        ]
    );
    assert_eq!(read_first(&board, "Which split for the bonus pool?").len(), 1);
    let kinds: Vec<Kind> = main.iter().map(|link| link.kind()).collect();
    assert_eq!(kinds, [Kind::Artifact, Kind::Pr, Kind::Doc, Kind::Issue, Kind::Web]);
    let tags: Vec<_> = archived.iter().map(|link| state_tag(link)).collect();
    assert_eq!(tags, [Some("Merged"), Some("Merged"), Some("Closed")]);
}

#[test]
fn merged_and_closed_links_leave_the_main_list_and_keep_the_newest_first_order() {
    let mut board = LinkBoard::default();
    for (url, ago, state) in [
        ("https://github.com/o/r/pull/1", 50, LinkState::Merged),
        ("https://github.com/o/r/pull/2", 40, LinkState::Open),
        ("https://github.com/o/r/issues/3", 30, LinkState::Closed),
        ("https://github.com/o/r/pull/4", 20, LinkState::Unknown),
        ("https://github.com/o/r/pull/5", 10, LinkState::Closed),
        ("https://a.dev/6", 5, LinkState::Unknown),
    ] {
        board.absorb(&with_state(url, ago, state));
    }
    let (main, archived) = split(&board);
    let urls = |links: &[&Link]| -> Vec<String> {
        links.iter().map(|link| link.url.rsplit('/').next().unwrap().to_owned()).collect()
    };
    assert_eq!(urls(&main), ["6", "4", "2"]);
    assert_eq!(urls(&archived), ["5", "3", "1"]);
    assert_eq!(main.len() + archived.len(), board.len());
    assert!(fold_label(main.len(), false).is_none(), "the fold counts the main list only");
}

#[test]
fn a_board_of_only_merged_links_has_an_empty_main_list() {
    let mut board = LinkBoard::default();
    board.absorb(&with_state("https://github.com/o/r/pull/1", 5, LinkState::Merged));
    let (main, archived) = split(&board);
    assert_eq!(main.len(), 0);
    assert_eq!(archived.len(), 1);
    assert_eq!(listed(&main, false).len(), 0);
}

#[test]
fn the_group_header_counts_its_links_and_is_absent_when_empty() {
    assert_eq!(archive_label(0), None);
    assert_eq!(archive_label(1).as_deref(), Some("Merged and closed (1)"));
    assert_eq!(archive_label(12).as_deref(), Some("Merged and closed (12)"));
}

#[test]
fn a_merged_or_closed_row_carries_its_tag_and_no_other_row_does() {
    let tags: Vec<_> = [LinkState::Unknown, LinkState::Open, LinkState::Merged, LinkState::Closed]
        .map(|state| state_tag(&with_state("https://github.com/o/r/pull/1", 1, state)))
        .into();
    assert_eq!(tags, [None, None, Some("Merged"), Some("Closed")]);
}

#[test]
fn a_lookup_is_due_for_an_open_or_unknown_pull_request_or_issue_never_looked_up_or_looked_up_30_minutes_ago()
 {
    let pr = "https://github.com/o/r/pull/1";
    let issue = "https://github.com/o/r/issues/2";
    let now = 1_000 * MIN;
    let cases = [
        (pr, LinkState::Unknown, None, true),
        (issue, LinkState::Unknown, None, true),
        (pr, LinkState::Open, None, true),
        (pr, LinkState::Open, Some(now - 29 * MIN), false),
        (pr, LinkState::Open, Some(now - 30 * MIN), true),
        (pr, LinkState::Open, Some(now - 31 * MIN), true),
        (pr, LinkState::Open, Some(now), false),
        (pr, LinkState::Merged, None, false),
        (issue, LinkState::Closed, Some(now - 90 * MIN), false),
        ("https://a.dev/1", LinkState::Unknown, None, false),
        ("https://claude.ai/artifact/x", LinkState::Unknown, None, false),
        ("https://docs.google.com/document/d/1", LinkState::Unknown, None, false),
    ];
    for (url, state, last, want) in cases {
        assert_eq!(
            check_due(&with_state(url, 1, state), last, now),
            want,
            "{url} {state:?} {last:?}"
        );
    }
}

#[test]
fn the_watchs_state_maps_to_the_links_state() {
    assert_eq!(link_state(State::Open), LinkState::Open);
    assert_eq!(link_state(State::Merged), LinkState::Merged);
    assert_eq!(link_state(State::Closed), LinkState::Closed);
}

#[test]
fn the_watchs_summary_gives_the_pull_request_its_title_else_the_lookup_gives_it_bare() {
    use tidewatch_core::pr_status::{Decision, State, SummaryChecks};
    let summary = Summary {
        number: 42,
        title: "feat: dashboard".into(),
        url: "https://github.com/o/r/pull/42".into(),
        state: State::Open,
        checks: SummaryChecks::None,
        comments: 0,
        decision: Decision::None,
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
    let from_lookup = own_pr(None, Some(&lookup)).unwrap();
    assert_eq!(
        (from_lookup.number, from_lookup.url, from_lookup.title, from_lookup.state),
        (7, "https://github.com/o/r/pull/7", None, LinkState::Unknown)
    );
    assert_eq!(own_pr(None, None), None);
    let merged = Summary { state: State::Merged, ..summary };
    assert_eq!(own_pr(Some(&merged), None).unwrap().state, LinkState::Merged);
}
