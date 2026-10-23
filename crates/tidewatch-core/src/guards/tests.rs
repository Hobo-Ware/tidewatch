use super::*;

fn hit(line: usize, column: usize, found: &str) -> Hit {
    Hit { line, column, found: found.to_owned() }
}

#[test]
fn dashes_are_found_with_their_place() {
    let cases = [
        ("plain - hyphen only", None),
        ("", None),
        ("a \u{2014} b", Some(hit(1, 3, "an em dash"))),
        ("first\nsecond \u{2013} third", Some(hit(2, 8, "an en dash"))),
        ("\u{2013}\u{2014}", Some(hit(1, 1, "an en dash"))),
        ("-- and \u{2212} minus", None),
    ];
    for (text, want) in cases {
        assert_eq!(find_dash(text), want, "text {text:?}");
    }
}

#[test]
fn trailers_and_footers_are_whole_lines() {
    let cases = [
        ("fix: x\n\nCo-Authored-By: Someone <a@b.c>", Some(hit(3, 1, "a Co-Authored-By trailer"))),
        ("co-authored-by: lower", Some(hit(1, 1, "a Co-Authored-By trailer"))),
        ("body\n  Co-authored-by: indented", Some(hit(2, 3, "a Co-Authored-By trailer"))),
        (
            "text\n\n\u{1F916} Generated with [Claude Code](https://claude.com/claude-code)",
            Some(hit(3, 1, "a Generated with footer")),
        ),
        ("Generated with the tool", Some(hit(1, 1, "a Generated with footer"))),
        ("> *Generated with care*", Some(hit(1, 1, "a Generated with footer"))),
        ("the file is generated with protoc", None),
        ("mentions Co-Authored-By: mid line", None),
        ("co authored by me", None),
        ("", None),
    ];
    for (text, want) in cases {
        assert_eq!(find_trailer(text), want, "text {text:?}");
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| (*n).to_owned()).collect()
}

#[test]
fn a_private_name_is_found_only_as_a_whole_name() {
    let private = names(&["acme/vault", "ledger-api"]);
    let cases = [
        ("see acme/vault#12", Some(hit(1, 5, "the private repo acme/vault"))),
        ("See ACME/Vault", Some(hit(1, 5, "the private repo acme/vault"))),
        ("x\nuses ledger-api.", Some(hit(2, 6, "the private repo ledger-api"))),
        ("(ledger-api)", Some(hit(1, 2, "the private repo ledger-api"))),
        ("ledger-api-extra and ledger_api_extra", None),
        ("xledger-api", None),
        ("acme/vaulted", None),
        ("nothing here", None),
    ];
    for (text, want) in cases {
        assert_eq!(find_private_name(text, &private), want, "text {text:?}");
    }
}

#[test]
fn the_earliest_private_name_wins_and_an_empty_list_finds_nothing() {
    let private = names(&["late", "early"]);
    assert_eq!(
        find_private_name("early then late", &private).map(|h| h.found),
        Some("the private repo early".to_owned())
    );
    assert_eq!(find_private_name("anything", &[]), None);
    assert_eq!(find_private_name("anything", &names(&[""])), None);
}

#[test]
fn a_name_with_non_ascii_text_before_it_reports_the_right_column() {
    let hit = find_private_name("caf\u{e9} \u{2014} acme-web", &names(&["acme-web"])).unwrap();
    assert_eq!((hit.line, hit.column), (1, 8));
}

#[test]
fn typed_names_split_on_lines_commas_and_spaces_without_repeats() {
    assert_eq!(
        parse_private_repos("acme/vault, Ledger-API\n\n  acme/vault  other"),
        ["acme/vault", "Ledger-API", "other"]
    );
    assert_eq!(parse_private_repos("  \n "), Vec::<String>::new());
}

#[test]
fn every_rule_has_a_key_a_title_and_a_reason_and_only_drafts_start_off() {
    let keys: Vec<&str> = Rule::ALL.iter().map(|r| r.key()).collect();
    assert_eq!(keys, ["dashes", "trailers", "private_names", "draft_prs", "rerun_failed"]);
    for rule in Rule::ALL {
        assert_ne!(rule.title().len(), 0);
        assert_ne!(rule.reason().len(), 0);
        assert_eq!(rule.default_on(), Rules::default().is_on(rule), "{rule:?}");
        assert!(!rule.reason().contains('\u{2014}') && !rule.title().contains('\u{2014}'));
    }
    assert_eq!(Rule::ALL.iter().filter(|r| !r.default_on()).count(), 1);
}

#[test]
fn a_repo_answer_beats_the_default_and_only_for_that_repo() {
    let mut prefs = GuardPrefs::default();
    prefs.set_rule(Some(Path::new("/work/web")), Rule::DraftPrs, true);
    prefs.set_rule(Some(Path::new("/work/web")), Rule::Dashes, false);
    let web = prefs.rules_for(Path::new("/work/web"));
    assert!(web.draft_prs && !web.dashes && web.trailers);
    assert_eq!(prefs.rules_for(Path::new("/work/api")), Rules::default());
}

#[test]
fn a_repo_answer_equal_to_the_default_is_dropped() {
    let mut prefs = GuardPrefs::default();
    prefs.set_rule(Some(Path::new("/work/web")), Rule::Dashes, false);
    prefs.set_rule(Some(Path::new("/work/web")), Rule::Dashes, true);
    assert_eq!(prefs, GuardPrefs::default());
}

#[test]
fn changing_the_default_moves_every_repo_that_has_no_answer() {
    let mut prefs = GuardPrefs::default();
    prefs.set_rule(Some(Path::new("/a")), Rule::Trailers, false);
    prefs.set_rule(None, Rule::Trailers, false);
    prefs.set_rule(None, Rule::DraftPrs, true);
    assert!(
        !prefs.rules_for(Path::new("/b")).trailers && prefs.rules_for(Path::new("/b")).draft_prs
    );
    prefs.set_rule(None, Rule::Trailers, true);
    assert!(!prefs.rules_for(Path::new("/a")).trailers);
    assert!(prefs.rules_for(Path::new("/b")).trailers);
}

#[test]
fn the_table_reads_missing_keys_as_defaults_and_survives_a_round_trip() {
    let empty: GuardPrefs = toml::from_str("").unwrap();
    assert_eq!(empty, GuardPrefs::default());
    let partial: GuardPrefs =
        toml::from_str("draft_prs = true\nprivate_repos = [\"acme/vault\"]").unwrap();
    assert!(partial.rules.draft_prs && partial.rules.dashes);
    assert_eq!(partial.private_repos, ["acme/vault"]);

    let mut prefs = GuardPrefs::default();
    prefs.rules.set(Rule::RerunFailed, false);
    prefs.private_repos = names(&["acme/vault"]);
    prefs.set_rule(Some(Path::new("/work/web")), Rule::DraftPrs, true);
    let text = toml::to_string(&prefs).unwrap();
    assert_eq!(toml::from_str::<GuardPrefs>(&text).unwrap(), prefs);
}

#[test]
fn the_guards_table_lives_under_guards_in_settings_and_a_missing_one_is_the_defaults() {
    let prefs: crate::prefs::Prefs = toml::from_str(
        "[guards]\ndashes = false\n\n[guards.repos.\"/work/web\"]\ndraft_prs = true\n",
    )
    .unwrap();
    assert!(!prefs.guards.rules.dashes);
    assert!(prefs.guards.rules_for(Path::new("/work/web")).draft_prs);
    let none: crate::prefs::Prefs = toml::from_str("restore_session = true").unwrap();
    assert_eq!(none.guards, GuardPrefs::default());
}
