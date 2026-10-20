use tidewatch_core::meeseeks::{Criteria, Event};

use super::*;

const MIN: i64 = 60_000;

fn running() -> Meeseeks {
    Meeseeks::summon("Bump vite to 6.3", Criteria::default(), 0)
}

fn text(card: &Card) -> String {
    card.status.iter().map(|span| span.text.as_str()).collect()
}

#[test]
fn the_meta_line_reads_elapsed_time_and_the_turn_in_progress_out_of_forty() {
    let mut meeseeks = running();
    assert_eq!(card(&meeseeks, "vite", 12 * MIN, None).meta, "12m \u{b7} turn 1 of 40");
    meeseeks.turns = 8;
    assert_eq!(card(&meeseeks, "vite", 12 * MIN, None).meta, "12m \u{b7} turn 9 of 40");
    meeseeks.turns = 40;
    assert_eq!(card(&meeseeks, "vite", 70 * MIN, None).meta, "1h 10m \u{b7} turn 40 of 40");
}

#[test]
fn the_card_is_titled_with_the_task_and_has_the_four_stages_in_order() {
    let card = card(&running(), "vite", 0, None);
    assert_eq!(card.title, "Bump vite to 6.3");
    let names: Vec<&str> = card.bars.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["Change", "Tests", "Proof", "Hand off"]);
    let looks: Vec<Look> = card.bars.iter().map(|(_, look)| *look).collect();
    assert_eq!(looks, [Look::Active, Look::Pending, Look::Pending, Look::Pending]);
}

#[test]
fn while_a_check_runs_the_status_box_names_it_and_the_bars_move_on() {
    let mut meeseeks = running();
    meeseeks.apply(Event::Said("done".into()), 0);
    meeseeks.apply(Event::Passed { step: Step::Change, pr_url: None }, 0);
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
