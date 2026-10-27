use super::*;

fn link(url: &str, title: &str, note: Option<&str>, seen: i64, from: TitleFrom) -> Link {
    Link {
        url: url.to_owned(),
        title: title.to_owned(),
        note: note.map(str::to_owned),
        first_seen_ms: seen,
        title_from: from,
    }
}

#[test]
fn the_kind_comes_from_the_address_alone() {
    let cases = [
        ("https://claude.ai/artifact/4JqXyaNvpN1W289NyzXiT9", Kind::Artifact),
        ("https://claude.ai/code/artifact/0b1c-22", Kind::Artifact),
        ("https://claude.ai/public/artifacts/abc", Kind::Artifact),
        ("https://claude.ai/chat/abc", Kind::Web),
        ("https://github.com/Hobo-Ware/tidewatch/pull/42", Kind::Pr),
        ("https://github.com/Hobo-Ware/tidewatch/pull/42/files", Kind::Pr),
        ("https://github.com/Hobo-Ware/tidewatch/issues/38", Kind::Issue),
        ("https://github.com/Hobo-Ware/tidewatch/pulls", Kind::Web),
        ("https://github.com/Hobo-Ware/tidewatch", Kind::Web),
        ("https://docs.google.com/document/d/1", Kind::Doc),
        ("https://www.notion.so/team/Plan-1", Kind::Doc),
        ("https://notion.so/Plan-1", Kind::Doc),
        ("https://acme.notion.so/Plan-1", Kind::Doc),
        ("https://notnotion.so/x", Kind::Web),
        ("https://example.com/artifact/abc", Kind::Web),
        ("HTTPS://CLAUDE.AI/artifact/abc", Kind::Artifact),
        ("not a link", Kind::Web),
    ];
    for (url, want) in cases {
        assert_eq!(kind_of(url), want, "{url}");
    }
    assert_eq!(Kind::ALL.map(Kind::word), ["Artifact", "PR", "Issue", "Doc", "Web"]);
}

#[test]
fn the_host_drops_www_and_case() {
    let at = |url: &str| link(url, "t", None, 0, TitleFrom::Agent).host();
    assert_eq!(at("https://www.Example.com/a?b=1"), "example.com");
    assert_eq!(at("https://claude.ai/artifact/x"), "claude.ai");
    assert_eq!(at("https://localhost:3000"), "localhost:3000");
}

#[test]
fn a_reported_url_must_be_http_or_https_with_a_host_and_no_blanks() {
    for good in ["https://a.dev/x", " http://a.dev ", "HTTPS://A.DEV/x?y=1#z"] {
        assert_eq!(checked_url(good).as_deref(), Some(good.trim()), "{good}");
    }
    let too_long = format!("https://a.dev/{}", "x".repeat(2100));
    for bad in [
        "",
        "ftp://a.dev/x",
        "javascript:alert(1)",
        "a.dev/x",
        "https://",
        "https:///path",
        "https://a.dev/a b",
        "https://a.dev/a\nb",
        too_long.as_str(),
    ] {
        assert_eq!(checked_url(bad), None, "{bad:?}");
    }
}

#[test]
fn one_line_folds_blanks_and_cuts_with_three_dots() {
    assert_eq!(one_line("  a \n b\t c ", 120).as_deref(), Some("a b c"));
    assert_eq!(one_line("   ", 120), None);
    let cut = one_line(&"w".repeat(130), 120).unwrap();
    assert_eq!((cut.chars().count(), cut.ends_with("...")), (120, true));
    assert_eq!(one_line(&"w".repeat(120), 120).unwrap().chars().count(), 120);
}

#[test]
fn the_same_url_is_one_row_however_it_is_spelled() {
    let mut board = LinkBoard::default();
    assert!(board.absorb(&link("https://Claude.ai/artifact/abc", "A", None, 5, TitleFrom::Agent)));
    assert!(!board.absorb(&link(
        "https://claude.ai/artifact/abc/",
        "A",
        None,
        5,
        TitleFrom::Agent
    )));
    assert_eq!(board.len(), 1);
    assert!(board.absorb(&link("https://claude.ai/artifact/abd", "B", None, 6, TitleFrom::Agent)));
    assert_eq!(board.len(), 2);
}

#[test]
fn the_first_seen_time_is_kept_and_an_earlier_one_wins() {
    let mut board = LinkBoard::default();
    board.absorb(&link("https://a.dev/x", "A", None, 100, TitleFrom::Agent));
    board.absorb(&link("https://a.dev/x", "A", None, 900, TitleFrom::Agent));
    assert_eq!(board.links[0].first_seen_ms, 100);
    assert!(board.absorb(&link("https://a.dev/x", "A", None, 40, TitleFrom::Agent)));
    assert_eq!(board.links[0].first_seen_ms, 40);
}

#[test]
fn a_later_non_empty_title_or_for_wins_and_an_empty_one_never_blanks() {
    let mut board = LinkBoard::default();
    board.absorb(&link("https://a.dev/x", "One", Some("first use"), 1, TitleFrom::Agent));
    board.absorb(&link("https://a.dev/x", "Two", Some("second use"), 2, TitleFrom::Agent));
    assert_eq!(board.links[0].title, "Two");
    assert_eq!(board.links[0].note.as_deref(), Some("second use"));
    assert!(!board.absorb(&link("https://a.dev/x", "", None, 3, TitleFrom::Agent)));
    assert!(!board.absorb(&link("https://a.dev/x", "  ", Some(" "), 3, TitleFrom::Agent)));
    assert_eq!(board.links[0].title, "Two");
    assert_eq!(board.links[0].note.as_deref(), Some("second use"));
}

#[test]
fn what_the_agent_reported_wins_over_what_tidewatch_found() {
    let mut board = LinkBoard::default();
    board.absorb(&link("https://a.dev/x", "Reported", Some("why"), 10, TitleFrom::Agent));
    board.absorb(&link("https://a.dev/x", "Seen in chat", None, 5, TitleFrom::Seen));
    board.absorb(&link("https://a.dev/x", "Artifact", None, 6, TitleFrom::Guess));
    let kept = &board.links[0];
    assert_eq!((kept.title.as_str(), kept.title_from), ("Reported", TitleFrom::Agent));
    assert_eq!((kept.first_seen_ms, kept.note.as_deref()), (5, Some("why")));
}

#[test]
fn a_found_title_beats_a_guess_whatever_the_order() {
    for order in [[TitleFrom::Guess, TitleFrom::Seen], [TitleFrom::Seen, TitleFrom::Guess]] {
        let mut board = LinkBoard::default();
        for from in order {
            let title = if from == TitleFrom::Seen { "Plan" } else { "Artifact" };
            board.absorb(&link("https://a.dev/x", title, None, 1, from));
        }
        assert_eq!(board.links[0].title, "Plan", "{order:?}");
    }
}

#[test]
fn a_title_found_later_fills_a_link_that_had_none() {
    let mut board = LinkBoard::default();
    board.absorb(&link("https://a.dev/x", "", None, 1, TitleFrom::Agent));
    board.absorb(&link("https://a.dev/x", "Artifact", None, 2, TitleFrom::Guess));
    assert_eq!(board.links[0].title, "Artifact");
}

#[test]
fn a_board_only_grows_and_lists_the_newest_first() {
    let mut board = LinkBoard::default();
    let inputs = [("https://a.dev/1", 10), ("https://a.dev/2", 30), ("https://a.dev/3", 20)];
    for (url, seen) in inputs {
        board.absorb(&link(url, url, None, seen, TitleFrom::Agent));
    }
    board.absorb(&link("https://a.dev/1", "", None, 99, TitleFrom::Agent));
    assert_eq!(board.len(), 3);
    let order: Vec<&str> = board.newest_first().iter().map(|link| link.url.as_str()).collect();
    assert_eq!(order, ["https://a.dev/2", "https://a.dev/3", "https://a.dev/1"]);
}

#[test]
fn links_seen_at_the_same_time_list_the_later_arrival_first() {
    let mut board = LinkBoard::default();
    for url in ["https://a.dev/1", "https://a.dev/2"] {
        board.absorb(&link(url, url, None, 7, TitleFrom::Agent));
    }
    assert_eq!(board.newest_first()[0].url, "https://a.dev/2");
}

#[test]
fn a_board_round_trips_through_json_with_for_spelled_for() {
    let mut board = LinkBoard::default();
    board.absorb(&link("https://a.dev/x", "A", Some("the question"), 3, TitleFrom::Agent));
    let text = serde_json::to_string(&board).unwrap();
    assert!(text.contains("\"for\":\"the question\""), "{text}");
    assert_eq!(serde_json::from_str::<LinkBoard>(&text).unwrap(), board);
    let bare = r#"{"links":[{"url":"https://a.dev/x","title":"A","first_seen_ms":1}]}"#;
    let loaded: LinkBoard = serde_json::from_str(bare).unwrap();
    assert_eq!(
        (loaded.links[0].note.clone(), loaded.links[0].title_from),
        (None, TitleFrom::Guess)
    );
}

#[test]
fn a_report_becomes_an_agent_link_stamped_with_the_time() {
    let report =
        LinkReport { url: "https://a.dev/x".into(), title: "A".into(), note: Some("n".into()) };
    let made = report.into_link(77);
    assert_eq!((made.first_seen_ms, made.title_from), (77, TitleFrom::Agent));
}
