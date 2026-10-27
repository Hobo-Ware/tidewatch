use super::*;

#[test]
fn gh_json_becomes_a_state() {
    let cases = [
        (r#"{"state":"OPEN"}"#, LinkState::Open),
        (r#"{"state":"MERGED"}"#, LinkState::Merged),
        (r#"{"state":"CLOSED"}"#, LinkState::Closed),
        (r#"{"state":"closed"}"#, LinkState::Closed),
        ("{\"state\": \"MERGED\"}\n", LinkState::Merged),
        (r#"{"state":"DRAFT"}"#, LinkState::Unknown),
        (r#"{"state":3}"#, LinkState::Unknown),
        (r#"{"other":"OPEN"}"#, LinkState::Unknown),
        ("not json", LinkState::Unknown),
        ("", LinkState::Unknown),
    ];
    for (json, want) in cases {
        assert_eq!(parse_state(json), want, "{json}");
    }
}

#[test]
fn a_link_that_is_not_a_pull_request_or_issue_is_never_looked_up() {
    let nowhere = Tools::with_path("/nonexistent");
    for url in ["https://example.com/a", "https://claude.ai/artifact/x", "https://github.com/o/r"] {
        assert_eq!(state_of(&nowhere, Path::new("/"), url), LinkState::Unknown, "{url}");
    }
}

#[test]
fn a_missing_gh_is_unknown_not_a_hang() {
    let nowhere = Tools::with_path("/nonexistent");
    assert_eq!(
        state_of(&nowhere, Path::new("/"), "https://github.com/o/r/pull/1"),
        LinkState::Unknown
    );
}
