use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::*;
use crate::progress::Ledger;

const MINUTE_MS: i64 = 60_000;

#[test]
fn goal_words_pick_the_icon_of_their_group_and_anything_else_gets_the_generic_one() {
    let cases = [
        ("Add tests for the parser", "flask-conical"),
        ("Fix the flaky login test", "flask-conical"),
        ("Fix the crash on startup", "bug"),
        ("Release 0.1.54", "rocket"),
        ("Deploy the worker", "rocket"),
        ("Write the docs for profiles", "book-open"),
        ("Polish the settings screen UI", "layout-dashboard"),
        ("Migrate the database schema", "database"),
        ("Show the usage meter", "gauge"),
        ("Harden security of the listener", "shield"),
        ("Refactor the parser glue", "refresh-cw"),
        ("Refactor the reader glue", "message-square"),
        ("Performance of the graph pane", "layout-dashboard"),
        ("Performance of the graph", "zap"),
        ("Rebase the branch onto main", "git-branch"),
        ("Avatar fallback", "image"),
        ("Quick-reply chips in the chat", "message-square"),
        ("A shell prompt glitch", "terminal"),
        ("Plan the next quarter", GENERIC),
        ("", GENERIC),
        ("attestation", GENERIC),
    ];
    for (goal, want) in cases {
        assert_eq!(pick(goal).0, want, "{goal}");
    }
}

#[test]
fn every_icon_the_picker_can_name_is_in_the_bundled_set_and_draws_a_readable_face() {
    let icons = KEYWORDS.iter().map(|(_, icon)| *icon).chain([GENERIC]);
    for icon in icons {
        assert!(avatar_icons::names().contains(&icon), "{icon} is not in the pack");
        for color in avatar_icons::color_names() {
            let svg = avatar_icons::svg(icon, &avatar_icons::color_hex(color).unwrap()).unwrap();
            let avatar = clean(&svg, REASON).unwrap();
            assert_eq!(avatar_check::check(&avatar.svg), Ok(()), "{icon} in {color}");
        }
    }
}

#[test]
fn the_color_is_from_the_list_and_stays_the_same_for_the_same_goal() {
    let names = avatar_icons::color_names();
    let goals = ["Fix the crash", "Release 0.1.54", "Plan the next quarter", "a", ""];
    for goal in goals {
        let (_, color) = pick(goal);
        assert!(names.contains(&color), "{goal}: {color}");
        assert_eq!(pick(goal), pick(goal));
        assert_eq!(pick(goal).1, pick(&format!("  {goal} ")).1, "trimmed");
    }
    let spread: std::collections::HashSet<&str> =
        (0..40).map(|n| pick(&format!("goal {n}")).1).collect();
    assert!(spread.len() > 3, "{spread:?}");
}

#[test]
fn the_rule_waits_two_minutes_after_the_ask_and_never_touches_a_fixed_or_settled_tab() {
    let ready = Facts {
        face: Face::Letter,
        settled: false,
        has_goal: true,
        asked_at_ms: Some(10 * MINUTE_MS),
    };
    let at = |minutes: i64| 10 * MINUTE_MS + minutes;
    let cases = [
        ("right after the ask", ready, at(0), Due::In(WAIT_MS)),
        ("a minute after", ready, at(MINUTE_MS), Due::In(MINUTE_MS)),
        ("just before two minutes", ready, at(WAIT_MS - 1), Due::In(1)),
        ("at two minutes", ready, at(WAIT_MS), Due::Now),
        ("long after", ready, at(60 * MINUTE_MS), Due::Now),
        ("a Meeseeks", Facts { face: Face::Fixed, ..ready }, at(WAIT_MS), Due::Never),
        ("settled", Facts { settled: true, ..ready }, at(WAIT_MS), Due::Never),
        ("an icon is there", Facts { face: Face::Picture, ..ready }, at(WAIT_MS), Due::Later),
        ("no goal", Facts { has_goal: false, ..ready }, at(WAIT_MS), Due::Later),
        ("never asked", Facts { asked_at_ms: None, ..ready }, at(WAIT_MS), Due::Later),
    ];
    for (what, facts, now, want) in cases {
        assert_eq!(due(&facts, now), want, "{what}");
    }
}

struct Tab {
    _dir: tempfile::TempDir,
    notes: PathBuf,
}

fn tab(goal: &str) -> Tab {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.json");
    let text = format!("# Goal\n{goal}\n\n## Status\nstate: working\n");
    progress_store::save(&progress_file_beside(&notes), &Ledger::parse(&text)).unwrap();
    Tab { _dir: dir, notes }
}

fn ask_at(notes: &Path, ms: i64) {
    let marker = avatar_asked_file_beside(notes);
    avatar_store::mark_asked(&marker).unwrap();
    let at = SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap());
    std::fs::File::options().write(true).open(&marker).unwrap().set_modified(at).unwrap();
}

const ASKED_MS: i64 = 1_800_000_000_000;

#[test]
fn before_two_minutes_nothing_is_saved_and_after_them_the_picked_icon_is() {
    let tab = tab("Fix the crash on startup");
    ask_at(&tab.notes, ASKED_MS);
    let file = avatar_file_beside(&tab.notes);

    assert_eq!(apply(&tab.notes, false, ASKED_MS + MINUTE_MS), Outcome::Wait(MINUTE_MS));
    assert!(!file.exists());

    let Outcome::Saved(avatar) = apply(&tab.notes, false, ASKED_MS + WAIT_MS) else {
        panic!("the icon is due")
    };
    assert_eq!(Some(avatar.clone()), face("Fix the crash on startup"));
    assert_eq!(avatar.reason, REASON);
    assert_eq!(avatar_store::load(&file), Some(avatar));
    assert_eq!(apply(&tab.notes, false, ASKED_MS + 10 * WAIT_MS), Outcome::Never);
}

#[test]
fn an_icon_the_agent_set_is_never_replaced_and_a_later_one_replaces_the_fallback() {
    let tab = tab("Fix the crash on startup");
    ask_at(&tab.notes, ASKED_MS);
    let file = avatar_file_beside(&tab.notes);
    let own = face("Release it").unwrap();
    let own = Avatar { reason: "the agent's own".into(), ..own };
    avatar_store::save(&file, &own).unwrap();
    assert_eq!(apply(&tab.notes, false, ASKED_MS + 5 * WAIT_MS), Outcome::Later);
    assert_eq!(avatar_store::load(&file), Some(own.clone()));

    avatar_store::clear(&file).unwrap();
    assert!(matches!(apply(&tab.notes, false, ASKED_MS + 5 * WAIT_MS), Outcome::Saved(_)));
    avatar_store::save(&file, &own).unwrap();
    assert_eq!(apply(&tab.notes, false, ASKED_MS + 6 * WAIT_MS), Outcome::Never);
    assert_eq!(avatar_store::load(&file), Some(own));
}

#[test]
fn a_meeseeks_a_settled_tab_a_tab_never_asked_and_a_tab_with_no_goal_get_nothing() {
    let later = ASKED_MS + 5 * WAIT_MS;

    let meeseeks = tab("Fix the crash");
    ask_at(&meeseeks.notes, ASKED_MS);
    assert_eq!(apply(&meeseeks.notes, true, later), Outcome::Never);

    let letter = tab("Fix the crash");
    ask_at(&letter.notes, ASKED_MS);
    settle(&letter.notes).unwrap();
    assert_eq!(apply(&letter.notes, false, later), Outcome::Never);

    let never_asked = tab("Fix the crash");
    assert_eq!(apply(&never_asked.notes, false, later), Outcome::Later);

    let no_goal = tab("");
    ask_at(&no_goal.notes, ASKED_MS);
    assert_eq!(apply(&no_goal.notes, false, later), Outcome::Later);

    for tab in [meeseeks, letter, never_asked, no_goal] {
        assert!(!avatar_file_beside(&tab.notes).exists());
    }
}
