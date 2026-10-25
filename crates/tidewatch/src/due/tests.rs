use tidewatch_core::progress::Ledger;
use tidewatch_core::schedule::{Entry, Index, Owner, Presence};
use tidewatch_core::status::Reason;

use super::*;
use crate::coming_up::coming_up;

fn day(d: u8) -> Date {
    Date::new(2026, 10, d).unwrap()
}

const SCHEDULED: &str = "- [ ] 2026-10-17 -- Switch off the old webhook -- the milestone\n  - [x] Route check\n  - [ ] Merge the switch\n  - [-] Watch writes -- not needed\n  - [~] A week of traffic\n- [ ] ~2026-10-15 -- Late about item\n- [ ] 2026-10-20 -- Later one\n- [ ] on: sync ships -- Event\n- [x] 2026-10-10 -- Already done\n";

fn ledger() -> Ledger {
    Ledger::parse(&format!("# Goal\nG\n\n## Scheduled\n{SCHEDULED}"))
}

fn whats(items: &[Due]) -> Vec<&str> {
    items.iter().map(|due| due.what.as_str()).collect()
}

fn at() -> (&'static Path, &'static str) {
    (Path::new("/work/web"), "heron")
}

#[test]
fn an_item_is_due_on_its_date_or_after_and_never_when_done_dated_later_or_an_event() {
    let items = due_items(day(17), &ledger(), at(), &Snoozes::default());
    assert_eq!(whats(&items), ["Switch off the old webhook", "Late about item"]);
    assert_eq!(items.iter().map(|due| due.days).collect::<Vec<_>>(), [0, -2]);
    assert_eq!(
        whats(&due_items(day(16), &ledger(), at(), &Snoozes::default())),
        ["Late about item"]
    );
    assert_eq!(
        whats(&due_items(day(20), &ledger(), at(), &Snoozes::default())),
        ["Switch off the old webhook", "Late about item", "Later one"]
    );
}

#[test]
fn an_item_carries_its_steps_with_their_marks() {
    let items = due_items(day(17), &ledger(), at(), &Snoozes::default());
    let marks: Vec<_> = items[0].steps.iter().map(|(mark, text)| (*mark, text.as_str())).collect();
    assert_eq!(
        marks,
        [
            (Mark::Done, "Route check"),
            (Mark::Todo, "Merge the switch"),
            (Mark::Skipped, "Watch writes"),
            (Mark::Doing, "A week of traffic"),
        ]
    );
}

#[test]
fn snooze_hides_an_item_for_the_rest_of_the_day_and_it_is_back_tomorrow() {
    let mut snoozes = Snoozes::default();
    let key = Key::new(at().0, at().1, "Switch off the old webhook");
    assert!(!snoozes.is_snoozed(&key, day(17)));
    snoozes.snooze(key.clone(), day(17));
    let cases = [(16, false), (17, true), (18, false), (19, false)];
    for (today, want) in cases {
        assert_eq!(snoozes.is_snoozed(&key, day(today)), want, "day {today}");
    }
    assert_eq!(whats(&due_items(day(17), &ledger(), at(), &snoozes)), ["Late about item"]);
    assert_eq!(due_items(day(18), &ledger(), at(), &snoozes).len(), 2);
    assert!(
        !snoozes.is_snoozed(&Key::new(at().0, "other-tab", "Switch off the old webhook"), day(17))
    );
}

#[test]
fn the_pill_says_today_or_how_late() {
    let due = |days| Due { what: String::new(), days, steps: Vec::new() };
    assert_eq!(due(0).pill(), "Due today");
    assert_eq!(due(-3).pill(), "3 d late");
}

#[test]
fn the_message_names_the_item_and_its_open_steps_and_asks_for_the_first() {
    let due = Due {
        what: "Switch off the old webhook (#42)".into(),
        days: 0,
        steps: vec![
            (Mark::Done, "Route check".into()),
            (Mark::Todo, "Merge the switch".into()),
            (Mark::Skipped, "Watch writes".into()),
            (Mark::Doing, "A week of traffic\nthen remove".into()),
        ],
    };
    assert_eq!(
        ask_text(&due),
        "Due today: Switch off the old webhook (#42). Steps: Merge the switch; A week of traffic then remove. Start with the first open step and update the ledger."
    );
    let bare = Due { what: "Ship it.".into(), days: 0, steps: Vec::new() };
    assert_eq!(
        ask_text(&bare),
        "Due today: Ship it. Start with the first open step and update the ledger."
    );
    let long = Due { what: "x".repeat(400), days: 0, steps: Vec::new() };
    assert!(ask_text(&long).contains(&format!("{}...", "x".repeat(137))));
}

#[test]
fn ask_is_open_only_for_an_idle_agent() {
    let cases = [
        (Some(AgentStatus::Done), false),
        (Some(AgentStatus::Working), true),
        (Some(AgentStatus::NeedsYou(Reason::Question)), true),
        (Some(AgentStatus::SettingUp), true),
        (Some(AgentStatus::Failed("x".into())), true),
        (None, true),
    ];
    for (status, blocked) in cases {
        assert_eq!(ask_blocked(status.as_ref()).is_some(), blocked, "{status:?}");
    }
}

fn index(scheduled: &str, presence: Presence) -> Index {
    let owner =
        Owner { repo: "/work/web".into(), tab: "heron".into(), session_id: None, profile_id: None };
    let ledger = Ledger::parse(&format!("# Goal\nG\n\n## Scheduled\n{scheduled}"));
    let mut index = Index::default();
    index.set(Entry::of(owner, &ledger, presence));
    index
}

#[test]
fn the_desktop_hears_of_each_due_item_once_a_day_even_for_a_closed_tab() {
    let rows = coming_up(
        day(17),
        &index(
            "- [ ] 2026-10-17 -- A\n- [ ] 2026-10-18 -- B\n- [ ] on: x -- C\n",
            Presence::Closed,
        ),
    );
    let mut told = Told::default();
    let snoozes = Snoozes::default();
    let first = to_tell(day(17), &rows, &snoozes, &told);
    assert_eq!(first.iter().map(|(key, _)| key.what.as_str()).collect::<Vec<_>>(), ["A"]);
    told.mark(first[0].0.clone(), day(17));
    assert_eq!(to_tell(day(17), &rows, &snoozes, &told).len(), 0);

    let tomorrow = coming_up(
        day(18),
        &index("- [ ] 2026-10-17 -- A\n- [ ] 2026-10-18 -- B\n", Presence::Closed),
    );
    let next = to_tell(day(18), &tomorrow, &snoozes, &told);
    assert_eq!(next.iter().map(|(key, _)| key.what.as_str()).collect::<Vec<_>>(), ["A", "B"]);
}

#[test]
fn a_snoozed_item_is_not_told_and_an_old_day_is_forgotten() {
    let rows = coming_up(day(17), &index("- [ ] 2026-10-17 -- A\n", Presence::Open));
    let mut snoozes = Snoozes::default();
    snoozes.snooze(Key::new(Path::new("/work/web"), "heron", "A"), day(17));
    assert_eq!(to_tell(day(17), &rows, &snoozes, &Told::default()).len(), 0);

    let mut told = Told::default();
    told.mark(Key::new(Path::new("/w"), "t", "old"), day(10));
    told.mark(Key::new(Path::new("/w"), "t", "new"), day(17));
    assert_eq!(told.sent.len(), 1);
}

#[test]
fn midnight_is_the_next_multiple_of_a_day() {
    let cases = [(0, 86_400), (1, 86_399), (86_399, 1), (86_400, 86_400), (-1, 1)];
    for (now, want) in cases {
        assert_eq!(secs_to_midnight(now), want, "{now}");
    }
}

#[test]
fn the_desktop_is_offered_each_item_until_one_goes_and_then_never_again_that_day() {
    let rows = coming_up(
        day(17),
        &index("- [ ] 2026-10-17 -- A\n- [ ] 2026-10-17 -- B\n", Presence::Open),
    );
    let (snoozes, mut told) = (Snoozes::default(), Told::default());
    let mut offered = Vec::new();
    let mut away = false;
    let mut run = |told: &mut Told, away: bool| {
        tell_all(day(17), &rows, &snoozes, told, |note| {
            offered.push((note.title.clone(), note.body.clone()));
            away
        })
    };
    assert_eq!(run(&mut told, away), 0, "in front: offered, not sent, not marked");
    away = true;
    assert_eq!(run(&mut told, away), 2, "away later the same day: both go");
    assert_eq!(run(&mut told, away), 0, "and never again today");
    assert_eq!(offered.len(), 4);
    assert_eq!(offered[0], ("Due today".to_owned(), "A\nheron".to_owned()));
}

#[test]
fn a_tab_on_its_own_starts_each_due_item_once_a_day_one_at_a_time() {
    let dues = due_items(day(17), &ledger(), at(), &Snoozes::default());
    let key = |due: &Due| Key::new(Path::new("/work/web"), "heron", &due.what);
    let mut started = Told::default();
    let (first_key, first) = next_to_start(day(17), &dues, key, &started).unwrap();
    assert_eq!(first.what, "Switch off the old webhook");
    started.mark(first_key, day(17));
    let (second_key, second) = next_to_start(day(17), &dues, key, &started).unwrap();
    assert_eq!(second.what, "Late about item");
    started.mark(second_key, day(17));
    assert!(next_to_start(day(17), &dues, key, &started).is_none(), "both were asked today");
    assert!(next_to_start(day(18), &dues, key, &started).is_some(), "a new day asks again");
    assert!(next_to_start(day(17), &[], key, &started).is_none());
}

#[test]
fn a_snoozed_item_is_not_started_for_a_tab_on_its_own_either() {
    let mut snoozes = Snoozes::default();
    snoozes.snooze(Key::new(at().0, at().1, "Switch off the old webhook"), day(17));
    let dues = due_items(day(17), &ledger(), at(), &snoozes);
    let key = |due: &Due| Key::new(at().0, at().1, &due.what);
    let (_, next) = next_to_start(day(17), &dues, key, &Told::default()).unwrap();
    assert_eq!(next.what, "Late about item");
}
