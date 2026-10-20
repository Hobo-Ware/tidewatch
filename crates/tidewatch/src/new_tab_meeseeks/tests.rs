use std::collections::HashSet;

use super::*;

fn facts(command: Option<&str>) -> RepoFacts {
    RepoFacts { test_command: command.map(str::to_owned), ..RepoFacts::default() }
}

fn lines(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| (*text).to_owned()).collect()
}

#[test]
fn the_offer_follows_what_the_repo_says_and_waits_while_it_is_unknown() {
    assert_eq!(TestsOffer::of(None), TestsOffer::Unknown);
    assert_eq!(TestsOffer::of(Some(&facts(None))), TestsOffer::Missing);
    assert_eq!(
        TestsOffer::of(Some(&facts(Some("npm test")))),
        TestsOffer::Command("npm test".into())
    );
}

#[test]
fn every_tick_shows_on_by_default_and_the_tests_row_is_off_and_says_why_without_a_command() {
    let on = rows(Criteria::default(), &TestsOffer::Command("npm test".into()));
    assert!(on.iter().all(|row| row.on && row.disabled.is_none()));
    let labels = on.each_ref().map(|row| row.tick.label());
    assert_eq!(labels, ["Tests pass", "Work committed", "Open a PR"]);
    let missing = rows(Criteria::default(), &TestsOffer::Missing);
    assert_eq!(missing[0], Row { tick: Tick::Tests, on: false, disabled: Some(NO_TESTS) });
    assert!(missing[1].on && missing[2].on);
    assert!(rows(Criteria::default(), &TestsOffer::Unknown)[0].disabled.is_none());
}

#[test]
fn a_click_flips_one_tick_and_a_disabled_row_stays_as_it_was() {
    let command = TestsOffer::Command("make test".into());
    let cases = [
        (Tick::Tests, Criteria { tests: false, committed: true, pr: true }),
        (Tick::Committed, Criteria { tests: true, committed: false, pr: true }),
        (Tick::Pr, Criteria { tests: true, committed: true, pr: false }),
    ];
    for (tick, want) in cases {
        assert_eq!(toggled(Criteria::default(), tick, &command), want, "{tick:?}");
        assert_eq!(toggled(want, tick, &command), Criteria::default(), "{tick:?} back");
    }
    assert_eq!(
        toggled(Criteria::default(), Tick::Tests, &TestsOffer::Missing),
        Criteria::default()
    );
}

#[test]
fn the_ticks_a_meeseeks_is_held_to_drop_tests_only_where_the_repo_has_none() {
    let ticks = Criteria { tests: true, committed: true, pr: false };
    assert_eq!(criteria(ticks, &TestsOffer::Command("x".into())), ticks);
    assert_eq!(criteria(ticks, &TestsOffer::Unknown), ticks);
    assert_eq!(criteria(ticks, &TestsOffer::Missing), Criteria { tests: false, ..ticks });
}

#[test]
fn each_line_is_named_from_itself_and_a_repeat_or_a_taken_name_takes_the_next_free_one() {
    let taken: HashSet<&str> = ["add-test-expired-promo"].into_iter().collect();
    let names = names_for(
        &lines(&[
            "Bump vite to 6.3 and fix the build",
            "Add a test for expired promo codes",
            "Bump vite to 6.3 and fix the build",
            "Bump vite to 6.3 and fix the build",
        ]),
        1,
        |name| taken.contains(name),
    );
    assert_eq!(names[0], "bump-vite-fix-build");
    assert_eq!(names[1], "add-test-expired-promo-2");
    assert_eq!(names[2], "bump-vite-fix-build-2");
    assert_eq!(names[3], "bump-vite-fix-build-3");
}

#[test]
fn a_line_with_nothing_to_slug_gets_a_random_name_that_is_not_taken_or_repeated() {
    let names = names_for(&lines(&["???", "!!!", "..."]), 5, |_| false);
    let distinct: HashSet<&String> = names.iter().collect();
    assert_eq!(distinct.len(), 3, "{names:?}");
    assert!(names.iter().all(|name| name.contains('-')), "{names:?}");
}

#[test]
fn the_summon_label_counts_the_tasks() {
    assert_eq!(summon_label(0), "Summon");
    assert_eq!(summon_label(1), "Summon 1");
    assert_eq!(summon_label(3), "Summon 3");
}
