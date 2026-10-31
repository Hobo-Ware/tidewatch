use super::*;
use crate::civil_date::{Date, Weekday};
use crate::panel_event::Fact;

fn monday() -> i64 {
    let day = 20_000_i64;
    (day - (day + 3).rem_euclid(7)) * DAY_MS
}

fn delivery(late_by: i64, changed: bool) -> Delivery {
    Delivery::of(
        Source::Hook,
        &Fact::Output,
        false,
        monday() + 1_000,
        monday() + 1_000 + late_by,
        changed,
    )
}

#[test]
fn a_week_starts_on_monday_utc() {
    let weekday = Date::from_day_number(week_of(monday() + 3 * DAY_MS)).weekday();
    assert_eq!(weekday, Weekday::Mon);
    assert_eq!(week_of(monday()), monday() / DAY_MS);
    assert_eq!(week_of(monday() + 7 * DAY_MS - 1), monday() / DAY_MS);
    assert_eq!(week_of(monday() + 7 * DAY_MS), monday() / DAY_MS + 7);
    assert_eq!(week_of(monday() - 1), monday() / DAY_MS - 7);
}

#[test]
fn nothing_counted_reads_no_percent_and_no_worst_source() {
    let counter = Counter::new();
    assert_eq!(counter.percent(monday()), None);
    assert_eq!(counter.worst(monday()), None);
    assert_eq!(counter.counts(monday()), Counts::default());
}

#[test]
fn the_share_on_time_counts_late_and_missed_against_it() {
    let mut counter = Counter::new();
    for _ in 0..997 {
        counter.record(monday(), Source::Hook, Outcome::OnTime);
    }
    counter.record(monday(), Source::Hook, Outcome::Late);
    counter.record(monday(), Source::Transcript, Outcome::Late);
    counter.record(monday(), Source::Hook, Outcome::Missed);
    assert_eq!(counter.counts(monday()), Counts { on_time: 997, late: 2, missed: 1 });
    assert_eq!(counter.percent(monday()), Some(99.7));
    assert_eq!(counter.worst(monday()), Some((Source::Hook, 2)));
}

#[test]
fn a_delivery_counts_as_on_time_or_late_by_its_limit_and_a_non_change_counts_for_nothing() {
    let cases = [
        ("on time", delivery(100, true), Counts { on_time: 1, late: 0, missed: 0 }),
        ("late", delivery(5_000, true), Counts { on_time: 0, late: 1, missed: 0 }),
        ("changed nothing", delivery(5_000, false), Counts::default()),
    ];
    for (name, delivery, want) in cases {
        let mut counter = Counter::new();
        counter.record_delivery(Source::Hook, &delivery);
        assert_eq!(counter.counts(monday() + 5_000), want, "{name}");
    }
}

#[test]
fn a_delivery_stamped_before_its_event_is_not_late() {
    let mut counter = Counter::new();
    counter.record_delivery(Source::Hook, &delivery(-500, true));
    assert_eq!(counter.counts(monday()), Counts { on_time: 1, late: 0, missed: 0 });
}

#[test]
fn the_tally_starts_over_in_a_new_week_and_this_week_reads_empty_after_the_old_one() {
    let mut counter = Counter::new();
    counter.record(monday(), Source::Pr, Outcome::Late);
    counter.record(monday() + 6 * DAY_MS, Source::Pr, Outcome::OnTime);
    assert_eq!(counter.counts(monday()), Counts { on_time: 1, late: 1, missed: 0 });
    let next_week = monday() + 7 * DAY_MS;
    assert_eq!(counter.counts(next_week), Counts::default());
    assert_eq!(counter.percent(next_week), None);
    assert_eq!(counter.worst(next_week), None);
    counter.record(next_week, Source::Hook, Outcome::OnTime);
    assert_eq!(counter.counts(next_week), Counts { on_time: 1, late: 0, missed: 0 });
    assert_eq!(counter.worst(next_week), None);
}

#[test]
fn a_clock_that_jumps_back_keeps_the_week_and_the_counts() {
    let mut counter = Counter::new();
    counter.record(monday() + 3 * DAY_MS, Source::Hook, Outcome::Late);
    counter.record(monday() - 30 * DAY_MS, Source::Hook, Outcome::OnTime);
    assert_eq!(counter.counts(monday() + 3 * DAY_MS), Counts { on_time: 1, late: 1, missed: 0 });
    assert_eq!(counter.counts(monday() - 30 * DAY_MS), Counts { on_time: 1, late: 1, missed: 0 });
}

#[test]
fn a_tie_names_the_first_source_and_a_clean_week_names_none() {
    let mut counter = Counter::new();
    counter.record(monday(), Source::Job, Outcome::Late);
    counter.record(monday(), Source::Hook, Outcome::Late);
    assert_eq!(counter.worst(monday()), Some((Source::Hook, 1)));
    let mut clean = Counter::new();
    clean.record(monday(), Source::Hook, Outcome::OnTime);
    assert_eq!(clean.worst(monday()), None);
}

#[test]
fn two_tallies_of_one_week_add_and_the_newer_week_wins() {
    let mut mine = Counter::new();
    mine.record(monday(), Source::Hook, Outcome::OnTime);
    let mut theirs = Counter::new();
    theirs.record(monday() + DAY_MS, Source::Hook, Outcome::Late);
    mine.merge(theirs);
    assert_eq!(mine.counts(monday()), Counts { on_time: 1, late: 1, missed: 0 });

    let mut newer = Counter::new();
    newer.record(monday() + 8 * DAY_MS, Source::Job, Outcome::OnTime);
    mine.merge(newer.clone());
    assert_eq!(mine, newer);

    let mut older = Counter::new();
    older.record(monday(), Source::Hook, Outcome::Late);
    mine.merge(older);
    assert_eq!(mine, newer);

    let mut empty = Counter::new();
    empty.merge(newer.clone());
    assert_eq!(empty, newer);
    mine.merge(Counter::new());
    assert_eq!(mine, newer);
}

#[test]
fn a_tally_survives_the_file_and_a_bad_file_is_an_empty_tally() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state").join("panel-ontime.json");
    assert_eq!(load(&file), Counter::new());
    let mut counter = Counter::new();
    counter.record(monday(), Source::Pr, Outcome::Missed);
    counter.record(monday(), Source::Hook, Outcome::OnTime);
    save(&file, &counter).unwrap();
    assert_eq!(load(&file), counter);
    std::fs::write(&file, "{\"week\": 1, \"sour").unwrap();
    assert_eq!(load(&file), Counter::new());
    assert_eq!(Counter::parse("not json"), Counter::new());
}

#[test]
fn every_source_has_a_label_for_the_settings_line() {
    let words: Vec<&str> = Source::ALL.iter().map(|source| label(*source)).collect();
    let mut unique = words.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), Source::ALL.len());
    assert_eq!(label(Source::Hook), "hooks");
}
