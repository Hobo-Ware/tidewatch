use std::collections::HashSet;

use super::*;
use tidewatch_core::agents::AgentTable;

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
fn the_tests_row_tip_names_the_command_and_what_picked_it() {
    let found = RepoFacts {
        test_command: Some("bun test".into()),
        test_why: Some("bun.lock".into()),
        ..RepoFacts::default()
    };
    assert_eq!(
        tests_tip(Some(&found)).as_deref(),
        Some(
            "Runs bun test (bun.lock). Wrong command? Set test = \"...\" in .tidewatch.toml at the repo root."
        )
    );
    assert_eq!(tests_tip(Some(&facts(None))), None);
    assert_eq!(tests_tip(None), None);
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

type Entry<'a> = (&'a str, &'a Profile, &'a Agent);

fn named(agent: &str, name: &str) -> Profile {
    Profile { name: name.into(), ..Profile::defaults_for(agent, Level::Edits) }
}

#[test]
fn the_profile_of_a_meeseeks_is_the_builder_else_the_first_else_the_cli_defaults_always_at_yolo() {
    let table = AgentTable::builtin();
    let claude = table.get("claude").unwrap();
    let codex = table.get("codex").unwrap();
    let first = named("claude", "Scout");
    let upper = named("claude", "BUILDER");
    let other = named("codex", "Builder");
    let seeded = named("claude", "Codex CLI Builder");
    let zed = named("claude", "Z");
    let cases: [(&str, Vec<Entry>, &str); 7] = [
        ("claude", vec![("scout", &first, claude), ("build", &upper, claude)], "build"),
        ("claude", vec![("scout", &first, claude), ("x", &other, codex)], "scout"),
        ("claude", vec![("b", &first, claude), ("a", &zed, claude)], "b"),
        ("claude", vec![("Builder", &first, claude), ("o", &first, claude)], "Builder"),
        (
            "claude",
            vec![("o", &first, claude), ("claude-builder", &first, claude)],
            "claude-builder",
        ),
        ("claude", vec![("o", &first, claude), ("p", &seeded, claude)], "p"),
        ("gemini", vec![("scout", &first, claude), ("x", &other, codex)], Profile::DEFAULTS_ID),
    ];
    for (agent, entries, want) in cases {
        let (key, profile) = profile_for(agent, &entries);
        assert_eq!(key, want, "{agent} {entries:?}");
        assert_eq!(profile.level, Level::Yolo);
        assert_eq!(profile.agent, agent);
    }
}

#[test]
fn the_cli_defaults_have_no_model_and_keep_the_house_style_and_never_push_on() {
    let (key, profile) = profile_for("codex", &[]);
    assert_eq!(key, Profile::DEFAULTS_ID);
    assert_eq!((profile.model, profile.effort, profile.setup.as_str()), (None, None, ""));
    assert!(profile.never_push && profile.house_style);
}

#[test]
fn the_yolo_line_names_the_worktree() {
    assert_eq!(YOLO_LINE, "Meeseeks always run in YOLO, in their own worktree.");
    assert_eq!(LEVEL, Level::Yolo);
}

#[test]
fn the_pr_row_updates_the_pull_request_a_task_names_and_opens_one_otherwise() {
    let link = "Sensei https://github.com/trakt/trakt-workers/pull/2437 and fold the fix in";
    let cases = [
        (lines(&[]), "Open a PR"),
        (lines(&["Bump vite to 6.3"]), "Open a PR"),
        (lines(&["Bump vite", "Rename the helper"]), "Open a PR"),
        (lines(&[link]), "Update PR #2437"),
        (lines(&["rebase PR 2437"]), "Update PR #2437"),
        (lines(&[link, "also PR #2437 again"]), "Update PR #2437"),
        (lines(&[link, "rebase PR 12"]), "Update their PRs"),
        (lines(&[link, "Bump vite"]), "Open or update a PR"),
        (lines(&["Bump vite", link]), "Open or update a PR"),
        (lines(&["explain issue #12"]), "Open a PR"),
    ];
    for (tasks, want) in cases {
        assert_eq!(pr_label(&tasks), want, "{tasks:?}");
    }
}
