use tidewatch_core::link_board::{Access, Checks, LinkState, Review, Status, TitleFrom};

use super::*;

fn link(url: &str, title: &str, state: LinkState, status: Status, seen: i64) -> Link {
    Link {
        url: url.to_owned(),
        title: title.to_owned(),
        note: None,
        first_seen_ms: seen,
        title_from: TitleFrom::Agent,
        state,
        status,
    }
}

fn failing() -> Status {
    Status { checks: Some(Checks::Fail), ..Status::default() }
}

fn board() -> LinkBoard {
    let mut board = LinkBoard::default();
    let rows = [
        link("https://github.com/o/r/pull/1", "email digest", LinkState::Open, failing(), 10),
        link(
            "https://github.com/o/r/pull/2",
            "paraglide cache",
            LinkState::Open,
            Status { updated_ms: Some(500), ..Status::default() },
            20,
        ),
        link(
            "https://github.com/o/r/pull/3",
            "client secret",
            LinkState::Merged,
            Status::default(),
            30,
        ),
        link(
            "https://github.com/o/r/issues/4",
            "season recap",
            LinkState::Open,
            Status::default(),
            40,
        ),
        link(
            "https://github.com/o/r/issues/5",
            "old bug",
            LinkState::Closed,
            Status::default(),
            50,
        ),
        link(
            "https://claude.ai/artifact/abc",
            "voice review",
            LinkState::Unknown,
            Status::default(),
            60,
        ),
        link(
            "https://docs.google.com/document/d/1",
            "export",
            LinkState::Unknown,
            Status::default(),
            70,
        ),
        link("https://example.com/notes", "notes", LinkState::Unknown, Status::default(), 80),
    ];
    for row in &rows {
        board.absorb(row);
    }
    board
}

fn titles(links: &[&Link]) -> Vec<String> {
    links.iter().map(|link| link.title.clone()).collect()
}

#[test]
fn each_type_chip_keeps_its_kind_and_docs_and_web_go_together() {
    let board = board();
    let cases =
        [(Type::All, 8), (Type::Prs, 3), (Type::Issues, 2), (Type::Artifacts, 1), (Type::Docs, 2)];
    for (kind, want) in cases {
        let filter = Filter { kind, ..Filter::default() };
        assert_eq!(shown(&board, &filter).len(), want, "{kind:?}");
        assert_eq!(counts(&board, &Filter::default()).of_kind(kind), want, "{kind:?}");
    }
}

#[test]
fn the_state_chips_split_open_needs_action_and_done_and_a_link_without_state_is_open() {
    let board = board();
    let cases = [
        (
            Bucket::Open,
            vec![
                "email digest",
                "paraglide cache",
                "season recap",
                "voice review",
                "export",
                "notes",
            ],
        ),
        (Bucket::NeedsAction, vec!["email digest"]),
        (Bucket::Done, vec!["client secret", "old bug"]),
    ];
    for (bucket, want) in cases {
        let filter = Filter { bucket: Some(bucket), ..Filter::default() };
        let mut got = titles(&shown(&board, &filter));
        got.sort();
        let mut want: Vec<String> = want.into_iter().map(str::to_owned).collect();
        want.sort();
        assert_eq!(got, want, "{bucket:?}");
        assert_eq!(counts(&board, &Filter::default()).of_bucket(bucket), want.len(), "{bucket:?}");
    }
}

#[test]
fn the_needs_action_links_come_first_then_open_by_last_change_then_done() {
    let board = board();
    let order = titles(&shown(&board, &Filter::default()));
    assert_eq!(
        order,
        [
            "email digest",
            "paraglide cache",
            "notes",
            "export",
            "voice review",
            "season recap",
            "old bug",
            "client secret",
        ]
    );
}

#[test]
fn githubs_update_time_decides_the_order_over_the_first_sight() {
    let mut board = LinkBoard::default();
    let at = |url: &str, seen: i64, updated: Option<i64>| {
        link(url, url, LinkState::Open, Status { updated_ms: updated, ..Status::default() }, seen)
    };
    board.absorb(&at("https://github.com/o/r/pull/1", 100, Some(900)));
    board.absorb(&at("https://github.com/o/r/pull/2", 500, None));
    board.absorb(&at("https://github.com/o/r/pull/3", 200, Some(700)));
    let shown = shown(&board, &Filter::default());
    let order: Vec<&str> = shown.iter().filter_map(|link| link.url.rsplit('/').next()).collect();
    assert_eq!(order, ["1", "3", "2"]);
}

#[test]
fn the_chips_combine_and_each_count_follows_the_other_chips_and_the_search() {
    let board = board();
    let filter = Filter { kind: Type::Prs, bucket: Some(Bucket::Done), search: String::new() };
    assert_eq!(titles(&shown(&board, &filter)), ["client secret"]);
    let seen = counts(&board, &filter);
    assert_eq!(seen.of_kind(Type::Issues), 1, "issues that are done");
    assert_eq!(seen.of_kind(Type::All), 2, "everything that is done");
    assert_eq!(seen.of_bucket(Bucket::Open), 2, "open pull requests");
    assert_eq!(seen.of_bucket(Bucket::NeedsAction), 1);
}

#[test]
fn the_search_matches_title_address_and_for_note_ignoring_case() {
    let mut board = board();
    board.absorb(&Link {
        note: Some("the opt-out flow".to_owned()),
        ..link("https://github.com/o/r/pull/1", "", LinkState::Unknown, Status::default(), 1)
    });
    let find = |query: &str| {
        titles(&shown(&board, &Filter { search: query.to_owned(), ..Filter::default() }))
    };
    assert_eq!(find("PARAGLIDE"), ["paraglide cache"]);
    assert_eq!(find("claude.ai"), ["voice review"]);
    assert_eq!(find("opt-out"), ["email digest"]);
    assert_eq!(find("  recap "), ["season recap"]);
    assert_eq!(find("zzz"), Vec::<String>::new());
    assert_eq!(find("").len(), 8);
}

#[test]
fn a_deleted_link_is_done_and_a_link_without_access_stays_open() {
    let mut board = LinkBoard::default();
    let url = "https://github.com/o/r/pull/9";
    board.absorb(&link(
        url,
        "gone",
        LinkState::Open,
        Status { access: Access::Gone, checks: Some(Checks::Fail), ..Status::default() },
        1,
    ));
    board.absorb(&link(
        "https://github.com/o/r/pull/10",
        "private",
        LinkState::Unknown,
        Status { access: Access::NoAccess, ..Status::default() },
        2,
    ));
    let done = Filter { bucket: Some(Bucket::Done), ..Filter::default() };
    assert_eq!(titles(&shown(&board, &done)), ["gone"]);
    let open = Filter { bucket: Some(Bucket::Open), ..Filter::default() };
    assert_eq!(titles(&shown(&board, &open)), ["private"]);
    assert_eq!(counts(&board, &Filter::default()).of_bucket(Bucket::NeedsAction), 0);
}

#[test]
fn changes_requested_and_conflicts_need_action_like_failing_checks() {
    let mut board = LinkBoard::default();
    let rows = [
        ("1", Status { review: Some(Review::ChangesRequested), ..Status::default() }),
        ("2", Status { conflicts: true, ..Status::default() }),
        ("3", Status { review: Some(Review::Approved), ..Status::default() }),
    ];
    for (n, status) in rows {
        board.absorb(&link(
            &format!("https://github.com/o/r/pull/{n}"),
            n,
            LinkState::Open,
            status,
            1,
        ));
    }
    let filter = Filter { bucket: Some(Bucket::NeedsAction), ..Filter::default() };
    let mut got = titles(&shown(&board, &filter));
    got.sort();
    assert_eq!(got, ["1", "2"]);
}

#[test]
fn a_filter_that_keeps_nothing_says_what_it_asked_for() {
    let with = |kind, bucket: Option<Bucket>, search: &str| {
        empty_line(&Filter { kind, bucket, search: search.to_owned() })
    };
    let cases = [
        (with(Type::Prs, Some(Bucket::Open), ""), "No open PRs"),
        (with(Type::Issues, None, ""), "No issues"),
        (with(Type::All, Some(Bucket::NeedsAction), ""), "Nothing needs action"),
        (
            with(Type::Docs, Some(Bucket::NeedsAction), ""),
            "Nothing needs action in docs and web links",
        ),
        (with(Type::Artifacts, Some(Bucket::Done), ""), "Nothing is done in artifacts"),
        (with(Type::All, Some(Bucket::Done), ""), "Nothing is done"),
        (with(Type::Prs, Some(Bucket::Open), " abc "), "Nothing matches \"abc\""),
    ];
    for (got, want) in cases {
        assert_eq!(got, want);
    }
}
