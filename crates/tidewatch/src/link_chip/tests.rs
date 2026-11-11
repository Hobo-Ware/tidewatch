use super::*;

fn pull(repo: &str, number: u64) -> Chip {
    Chip::Pull { repo: repo.to_owned(), number }
}

fn found(title: Option<&str>, state: LinkState) -> Info {
    Info { title: title.map(str::to_owned), state }
}

#[test]
fn only_a_whole_pull_request_issue_or_artifact_address_is_a_chip() {
    let cases = [
        (
            "https://github.com/trakt/trakt-workers/pull/2903",
            Some(pull("trakt/trakt-workers", 2903)),
        ),
        (
            "https://github.com/o/r/issues/38",
            Some(Chip::Issue { repo: "o/r".to_owned(), number: 38 }),
        ),
        ("https://github.com/o/r/pull/12/files", Some(pull("o/r", 12))),
        ("https://github.com/o/r/pull/12#issuecomment-5", Some(pull("o/r", 12))),
        ("https://github.com/o/r/pull/12?x=1", Some(pull("o/r", 12))),
        ("https://claude.ai/artifact/abc-123", Some(Chip::Artifact)),
        ("https://claude.ai/code/artifact/0a1b", Some(Chip::Artifact)),
        ("https://claude.ai/public/artifacts/x", Some(Chip::Artifact)),
        ("https://claude.ai/artifact/", None),
        ("https://github.com/o/r", None),
        ("https://github.com/o/r/pull/abc", None),
        ("https://github.com/o/r/tree/main", None),
        ("https://example.com/o/r/pull/1", None),
        ("see https://github.com/o/r/pull/1", None),
        ("https://github.com/o/r/pull/1 ", None),
        ("github.com/o/r/pull/1", None),
        ("", None),
    ];
    for (url, want) in cases {
        assert_eq!(parse(url), want, "{url:?}");
    }
}

#[test]
fn a_chip_reads_repo_and_number_until_the_title_arrives_then_number_and_title() {
    let chip = pull("trakt/trakt-workers", 2903);
    let cases = [
        (None, "trakt-workers #2903"),
        (Some(found(None, LinkState::Open)), "trakt-workers #2903"),
        (Some(found(Some("  "), LinkState::Open)), "trakt-workers #2903"),
        (Some(found(Some("fix: round totals"), LinkState::Merged)), "#2903 fix: round totals"),
        (Some(found(Some("#2903 fix: round totals"), LinkState::Open)), "#2903 fix: round totals"),
        (Some(found(Some("#2903: fix"), LinkState::Open)), "#2903 fix"),
        (Some(found(Some("#12 other"), LinkState::Open)), "#2903 #12 other"),
    ];
    for (known, want) in cases {
        assert_eq!(label(&chip, known.as_ref()), want);
    }
}

#[test]
fn an_artifact_chip_is_artifact_and_gains_its_title() {
    assert_eq!(label(&Chip::Artifact, None), "Artifact");
    assert_eq!(
        label(&Chip::Artifact, Some(&found(Some("Spend board"), LinkState::Unknown))),
        "Artifact \u{b7} Spend board"
    );
}

#[test]
fn a_long_title_is_cut_to_one_short_label() {
    let long = "x".repeat(200);
    let words = label(&pull("o/r", 1), Some(&found(Some(&long), LinkState::Open)));
    assert_eq!(words.chars().count(), MAX_LABEL_CHARS + 3);
    assert!(words.ends_with("..."));
}

#[test]
fn what_the_box_wants_is_taken_once_and_what_the_app_publishes_is_read_by_link_key() {
    let ctx = Context::default();
    let url = "https://github.com/o/r/pull/9";
    want(&ctx, url);
    want(&ctx, url);
    want(&ctx, "https://claude.ai/artifact/a");
    assert_eq!(take_wants(&ctx), [url, "https://claude.ai/artifact/a"]);
    assert_eq!(take_wants(&ctx), Vec::<String>::new());
    assert_eq!(info(&ctx, url), None);
    let known: Known = HashMap::from([(
        key_of(url),
        Info { title: Some("t".to_owned()), state: LinkState::Open },
    )]);
    publish(&ctx, Arc::new(known));
    assert_eq!(
        info(&ctx, "https://github.com/o/r/pull/9/").map(|i| i.state),
        Some(LinkState::Open)
    );
    assert_eq!(info(&ctx, "https://github.com/o/r/pull/10"), None);
}
