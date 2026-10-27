use tidewatch_core::link_board::TitleFrom;

use super::*;

fn link(url: &str, title: &str, note: Option<&str>, ago_min: i64) -> Link {
    Link {
        url: url.to_owned(),
        title: title.to_owned(),
        note: note.map(str::to_owned),
        first_seen_ms: 100 * MIN - ago_min * MIN,
        title_from: TitleFrom::Agent,
    }
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
    let shown: Vec<&str> = listed(&board, false).iter().map(|link| link.title.as_str()).collect();
    assert_eq!(shown, ["L4", "L3", "L2"]);
    assert_eq!(listed(&board, true).len(), 5);
    assert_eq!(listed(&self::board(2), false).len(), 2);
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

#[test]
fn the_tabs_pull_request_is_a_link_titled_with_its_number_and_title_when_known() {
    let named = pr_link(42, "https://github.com/o/r/pull/42", Some("feat: dashboard"), 9);
    assert_eq!((named.title.as_str(), named.title_from), ("#42 feat: dashboard", TitleFrom::Seen));
    for title in [None, Some(""), Some("  ")] {
        let bare = pr_link(42, "https://github.com/o/r/pull/42", title, 9);
        assert_eq!((bare.title.as_str(), bare.title_from), ("PR #42", TitleFrom::Guess));
    }
    let long = pr_link(7, "https://github.com/o/r/pull/7", Some(&"w".repeat(300)), 9);
    assert_eq!(long.title.chars().count(), 120);
    assert_eq!(named.kind(), Kind::Pr);
}

#[test]
fn the_fake_board_is_the_mockups_five_links_with_the_question_link_among_them() {
    let board = fake_board(1_000 * MIN);
    assert_eq!(board.len(), 5);
    let newest: Vec<&str> = listed(&board, false).iter().map(|link| link.title.as_str()).collect();
    assert_eq!(
        newest,
        [
            "Split options compared",
            "#42 feat: profit and spend dashboard",
            "Mercury spend export, October"
        ]
    );
    assert_eq!(read_first(&board, "Which split for the bonus pool?").len(), 1);
    let kinds: Vec<Kind> = board.newest_first().iter().map(|link| link.kind()).collect();
    assert_eq!(kinds, [Kind::Artifact, Kind::Pr, Kind::Doc, Kind::Issue, Kind::Web]);
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
    assert_eq!(
        own_pr(Some(&summary), Some(&lookup)),
        Some((42, "https://github.com/o/r/pull/42", Some("feat: dashboard")))
    );
    assert_eq!(own_pr(None, Some(&lookup)), Some((7, "https://github.com/o/r/pull/7", None)));
    assert_eq!(own_pr(None, None), None);
}
