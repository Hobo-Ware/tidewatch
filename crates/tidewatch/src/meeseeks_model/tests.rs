use tidewatch_core::kind::Kind;
use tidewatch_core::meeseeks::{Criteria, Event};

use super::*;

const MIN: i64 = 60_000;

fn running() -> Meeseeks {
    Meeseeks { kind: Kind::Edit, ..Meeseeks::summon("Bump vite to 6.3", Criteria::default(), 0) }
}

fn text(card: &Card) -> String {
    card.status.iter().map(|span| span.text.as_str()).collect()
}

#[test]
fn the_meta_line_is_the_kind_and_the_elapsed_time_and_never_a_turn_count() {
    let mut meeseeks = running();
    assert_eq!(card(&meeseeks, "vite", 12 * MIN, None).meta, "Edit \u{b7} 12m");
    meeseeks.turns = 8;
    assert_eq!(card(&meeseeks, "vite", 70 * MIN, None).meta, "Edit \u{b7} 1h 10m");
    meeseeks.kind = Kind::ReadOnly;
    assert_eq!(card(&meeseeks, "vite", 5 * MIN, None).meta, "Read-only \u{b7} 5m");
}

#[test]
fn a_read_only_card_has_no_check_bars_and_says_it_poofs_with_its_answer() {
    let mut meeseeks = Meeseeks { kind: Kind::ReadOnly, ..running() };
    let working = card(&meeseeks, "x", 0, None);
    assert_eq!(working.bars.len(), 0);
    assert!(text(&working).starts_with("Read-only."));
    assert_eq!(working.shape, StageShape { kind: Kind::ReadOnly, no_diff: false });
    meeseeks.apply(Event::Said("It is a cache".into()), 0);
    let poofing = card(&meeseeks, "x", 0, None);
    assert_eq!(poofing.bars.len(), 0);
    assert_eq!(text(&poofing), "Answered. Poofing...");
}

#[test]
fn an_edit_with_nothing_to_ship_says_so_and_shapes_the_stage_bars() {
    let mut meeseeks = running();
    meeseeks.apply(Event::Looked { edited: false }, 0);
    meeseeks.apply(Event::Said("nothing needed".into()), 0);
    let shown = card(&meeseeks, "x", 0, None);
    assert_eq!(shown.shape, StageShape { kind: Kind::Edit, no_diff: true });
    assert_eq!(text(&shown), "Nothing changed. No checks to run. Poofing...");
    assert!(shown.bars.iter().all(|(_, look)| *look == Look::Skipped));
}

#[test]
fn a_working_meeseeks_shows_no_stage_bars_and_a_checking_one_shows_the_four_in_order() {
    let mut meeseeks = running();
    let working = card(&meeseeks, "vite", 0, None);
    assert_eq!(working.title, "Bump vite to 6.3");
    assert_eq!(working.bars.len(), 0);
    meeseeks.apply(Event::Said("done".into()), 0);
    let checking = card(&meeseeks, "vite", 0, None);
    let names: Vec<&str> = checking.bars.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["Change", "Tests", "Proof", "Hand off"]);
    let looks: Vec<Look> = checking.bars.iter().map(|(_, look)| *look).collect();
    assert_eq!(looks, [Look::Active, Look::Pending, Look::Pending, Look::Pending]);
}

#[test]
fn while_a_check_runs_the_status_box_names_it_and_the_bars_move_on() {
    let mut meeseeks = running();
    meeseeks.apply(Event::Said("done".into()), 0);
    meeseeks.apply(Event::Passed { step: Step::Change, pr_url: None, detail: String::new() }, 0);
    let shown = card(&meeseeks, "vite", 0, Some("npm test"));
    assert_eq!(text(&shown), "Says it's done. Tidewatch is checking: npm test running...");
    assert_eq!(shown.status[1], Span { text: "npm test".into(), strong: true });
    assert_eq!(shown.bars[0].1, Look::Done);
    assert_eq!(shown.bars[1].1, Look::Active);
    let unnamed = card(&meeseeks, "vite", 0, None);
    assert!(text(&unnamed).contains("the tests running"));
}

#[test]
fn each_phase_has_its_own_words() {
    let mut meeseeks = running();
    assert!(text(&card(&meeseeks, "x", 0, None)).starts_with("Working."));
    meeseeks.phase = Phase::Checking(Step::Change);
    assert!(text(&card(&meeseeks, "x", 0, None)).contains("commit and clean tree"));
    meeseeks.phase = Phase::Checking(Step::HandOff);
    assert!(text(&card(&meeseeks, "x", 0, None)).contains("pull request"));
    meeseeks.criteria.pr = false;
    assert!(text(&card(&meeseeks, "x", 0, None)).contains("hand off"));
    meeseeks.phase = Phase::Poofing;
    assert_eq!(text(&card(&meeseeks, "x", 0, None)), "Checks passed. Poofing...");
}

#[test]
fn after_a_failed_check_the_note_stays_up_until_the_agent_calls_done_again() {
    let mut meeseeks = running();
    meeseeks.note = Some("Tests failed: exit 1 from `npm test`".into());
    assert_eq!(
        text(&card(&meeseeks, "x", 0, None)),
        "Tests failed: exit 1 from `npm test`. Waiting for the fix."
    );
}

#[test]
fn a_struggling_meeseeks_gets_the_card_with_its_reason_and_stops_saying_it_is_checking() {
    let mut meeseeks = running();
    assert!(card(&meeseeks, "rename", 0, None).struggling.is_none());
    meeseeks.failures = 3;
    let shown = card(&meeseeks, "rename", 0, None);
    assert_eq!(
        shown.struggling,
        Some(Struggling { title: "rename is struggling".into(), reason: "3 failed checks".into() })
    );
    assert!(text(&shown).contains("Waiting for you"));
    meeseeks.failures = 0;
    let slow = card(&meeseeks, "rename", 60 * MIN, None);
    assert_eq!(slow.struggling.unwrap().reason, "60 minutes");
    assert_eq!(STRUGGLE_BODY, "It can't finish alone. It won't poof until you decide.");
}

#[test]
fn a_blocked_meeseeks_shows_the_reason_instead_of_running_checks() {
    let mut meeseeks = running();
    let command = meeseeks.apply(Event::Said("Blocked: the staging key is missing".into()), 0);
    assert!(matches!(command, tidewatch_core::meeseeks::Command::Poof { .. }));
    let shown = card(&meeseeks, "x", 0, None);
    assert_eq!(text(&shown), "Blocked. the staging key is missing. Poofing...");
    assert!(shown.bars.iter().all(|(_, look)| *look == Look::Skipped));
}

#[test]
fn a_task_that_names_a_pull_request_checks_and_labels_the_update_of_it() {
    let mut meeseeks = Meeseeks {
        kind: Kind::Edit,
        ..Meeseeks::summon(
            "Sensei https://github.com/trakt/trakt-workers/pull/2437",
            Criteria::default(),
            0,
        )
    };
    meeseeks.phase = Phase::Checking(Step::HandOff);
    let shown = card(&meeseeks, "x", 0, None);
    assert_eq!(text(&shown), "Says it's done. Tidewatch is checking: PR #2437 update running...");
    let names: Vec<&str> = shown.bars.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["Change", "Tests", "Proof", "Update PR"]);
    let plain = card(&running(), "x", 0, None);
    assert_eq!(plain.bars.len(), 0);
    meeseeks.task = "Bump vite to 6.3".into();
    let names: Vec<&str> = card(&meeseeks, "x", 0, None).bars.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["Change", "Tests", "Proof", "Hand off"]);
}
