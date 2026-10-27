use super::*;

fn said(role: Role, text: &str, ts: Option<i64>) -> Entry {
    Entry { role, text: text.to_owned(), tool: None, ts, images: Vec::new() }
}

fn agent(text: &str) -> Vec<Entry> {
    vec![said(Role::Assistant, text, Some(1_000))]
}

fn pairs(text: &str) -> Vec<(String, String)> {
    found(&agent(text), 0).into_iter().map(|link| (link.url, link.title)).collect()
}

fn one(url: &str, title: &str) -> Vec<(String, String)> {
    vec![(url.to_owned(), title.to_owned())]
}

#[test]
fn a_markdown_link_gives_its_text_as_the_title() {
    assert_eq!(
        pairs(
            "See [Split options compared](https://claude.ai/artifact/4JqXyaNvpN1W289NyzXiT9) first."
        ),
        one("https://claude.ai/artifact/4JqXyaNvpN1W289NyzXiT9", "Split options compared")
    );
    assert_eq!(
        pairs("Opened [#42 feat: dashboard](https://github.com/Hobo-Ware/tidewatch/pull/42)"),
        one("https://github.com/Hobo-Ware/tidewatch/pull/42", "#42 feat: dashboard")
    );
}

#[test]
fn a_bare_link_gets_a_plain_fallback_title() {
    assert_eq!(
        pairs("board: https://claude.ai/artifact/abc123"),
        one("https://claude.ai/artifact/abc123", "Artifact")
    );
    assert_eq!(
        pairs("PR https://github.com/o/r/pull/42 and https://github.com/o/r/issues/38"),
        [
            ("https://github.com/o/r/pull/42".to_owned(), "PR #42".to_owned()),
            ("https://github.com/o/r/issues/38".to_owned(), "Issue #38".to_owned()),
        ]
    );
}

#[test]
fn trailing_punctuation_and_closing_brackets_are_not_part_of_the_url() {
    let want = one("https://github.com/o/r/pull/42", "PR #42");
    for text in [
        "done https://github.com/o/r/pull/42.",
        "(see https://github.com/o/r/pull/42)",
        "[https://github.com/o/r/pull/42]",
        "it is https://github.com/o/r/pull/42, ok",
        "really? https://github.com/o/r/pull/42!",
        "<https://github.com/o/r/pull/42>",
        "\"https://github.com/o/r/pull/42\"",
        "`https://github.com/o/r/pull/42`:",
        "https://github.com/o/r/pull/42;",
    ] {
        assert_eq!(pairs(text), want, "{text}");
    }
    assert_eq!(
        pairs("([Board](https://claude.ai/artifact/abc))."),
        one("https://claude.ai/artifact/abc", "Board")
    );
}

#[test]
fn a_pull_request_link_with_a_tail_or_a_fragment_is_the_pull_request() {
    for tail in ["/files", "/commits/abc", "#issuecomment-9", "?diff=split"] {
        assert_eq!(
            pairs(&format!("https://github.com/o/r/pull/42{tail}")),
            one("https://github.com/o/r/pull/42", "PR #42"),
            "{tail}"
        );
    }
}

#[test]
fn all_three_artifact_shapes_are_found() {
    for path in ["artifact/abc-1_2", "code/artifact/abc-1_2", "public/artifacts/abc-1_2"] {
        let url = format!("https://claude.ai/{path}");
        assert_eq!(pairs(&format!("see {url}")), one(&url, "Artifact"), "{path}");
    }
}

#[test]
fn only_the_agents_own_messages_are_read() {
    let url = "https://claude.ai/artifact/abc";
    let entries = [
        said(Role::User, url, Some(1)),
        said(Role::Tool, url, Some(2)),
        said(Role::System, url, Some(3)),
        said(Role::Command, url, Some(4)),
        said(Role::CommandOutput, url, Some(5)),
        said(Role::Compacted, url, Some(6)),
    ];
    assert_eq!(found(&entries, 0), Vec::new());
    let mut mixed = entries.to_vec();
    mixed.push(said(Role::Assistant, url, Some(7)));
    assert_eq!(found(&mixed, 0).len(), 1);
}

#[test]
fn other_hosts_and_other_github_pages_are_ignored() {
    for text in [
        "https://example.com/artifact/abc",
        "https://claude.ai.evil.com/artifact/abc",
        "https://notclaude.ai/artifact/abc",
        "https://claude.ai/chat/abc",
        "https://claude.ai/artifact/",
        "https://github.com/o/r",
        "https://github.com/o/r/pulls",
        "https://github.com/o/r/pull/abc",
        "https://github.com/o/r/pull/42abc",
        "https://gitlab.com/o/r/pull/42",
        "https://docs.google.com/document/d/1",
        "ftp://github.com/o/r/pull/42",
        "claude.ai/artifact/abc",
        "xhttps://github.com/o/r/pull/42",
    ] {
        assert_eq!(pairs(text), Vec::new(), "{text}");
    }
}

#[test]
fn a_link_said_twice_is_one_link_and_the_markdown_text_wins() {
    let text = "https://claude.ai/artifact/abc then [The board](https://claude.ai/artifact/abc) and https://claude.ai/artifact/abc";
    let links = found(&agent(text), 0);
    assert_eq!(links.len(), 1);
    assert_eq!((links[0].title.as_str(), links[0].title_from), ("The board", TitleFrom::Seen));
}

#[test]
fn the_time_is_the_messages_own_and_else_now() {
    let entries = [
        said(Role::Assistant, "https://claude.ai/artifact/a", Some(5_000)),
        said(Role::Assistant, "https://claude.ai/artifact/b", None),
    ];
    let seen: Vec<i64> = found(&entries, 9_000).iter().map(|link| link.first_seen_ms).collect();
    assert_eq!(seen, [5_000, 9_000]);
}

#[test]
fn the_first_message_that_said_a_link_gives_its_time() {
    let entries = [
        said(Role::Assistant, "https://claude.ai/artifact/a", Some(5_000)),
        said(Role::Assistant, "again https://claude.ai/artifact/a", Some(8_000)),
    ];
    assert_eq!(found(&entries, 0)[0].first_seen_ms, 5_000);
}

#[test]
fn a_long_or_odd_markdown_text_is_one_short_line_and_a_url_text_is_no_title() {
    let long = "w ".repeat(100);
    let links = found(&agent(&format!("[{long}](https://claude.ai/artifact/abc)")), 0);
    assert!(links[0].title.chars().count() <= 120 && links[0].title.ends_with("..."));
    assert_eq!(
        pairs("[https://claude.ai/artifact/abc](https://claude.ai/artifact/abc)"),
        one("https://claude.ai/artifact/abc", "Artifact")
    );
    assert_eq!(
        pairs("[  ](https://claude.ai/artifact/abc)"),
        one("https://claude.ai/artifact/abc", "Artifact")
    );
    assert_eq!(
        pairs("[a\nb](https://claude.ai/artifact/abc)"),
        one("https://claude.ai/artifact/abc", "Artifact")
    );
}

#[test]
fn http_is_read_too_and_stored_as_https() {
    assert_eq!(
        pairs("http://github.com/o/r/issues/7"),
        one("https://github.com/o/r/issues/7", "Issue #7")
    );
}
