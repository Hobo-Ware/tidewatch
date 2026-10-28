//! Tidewatch's reminders to the agent as quiet lines (pure, table-tested): which timeline places are
//! reminders, and the folding of back-to-back ones of one reason into a single line with a count.

use jiff::tz::TimeZone;
use tidewatch_core::sent::Record;

use crate::pr_timeline::Place;
use crate::reader_turns::span_label;

/// One thing to draw: a place of the timeline as it is, or a run of reminders as one line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Entry<'a> {
    Place(Place),
    Quiet(Vec<&'a Record>),
}

/// Folds each run of reminders of one reason with nothing else between them into one `Quiet`.
/// `quiet_of` gives the record of a place when that place is a reminder.
pub(crate) fn fold<'a>(
    places: impl IntoIterator<Item = Place>,
    quiet_of: impl Fn(Place) -> Option<&'a Record>,
) -> Vec<Entry<'a>> {
    let mut out: Vec<Entry<'a>> = Vec::new();
    for place in places {
        let Some(record) = quiet_of(place) else {
            out.push(Entry::Place(place));
            continue;
        };
        match out.last_mut() {
            Some(Entry::Quiet(run)) if run[0].reason == record.reason => run.push(record),
            _ => out.push(Entry::Quiet(vec![record])),
        }
    }
    out
}

/// The key of a run in the set of open lists and in the heights: its first record.
pub(crate) fn key(run: &[&Record]) -> u64 {
    run.first().map_or(0, |record| record.id)
}

/// The sentence of a run: what Tidewatch asked for.
pub(crate) fn words(run: &[&Record]) -> &'static str {
    run.first().and_then(|record| record.reason.quiet_words()).unwrap_or("Tidewatch told the agent")
}

/// `4` for several reminders, nothing for one.
pub(crate) fn count(run: &[&Record]) -> Option<String> {
    (run.len() > 1).then(|| run.len().to_string())
}

/// `23:44` for one minute, `23:22 to 23:42` for a run that spans two.
pub(crate) fn span(run: &[&Record], zone: &TimeZone) -> Option<String> {
    let first = run.first()?.at_ms;
    let last = run.last()?.at_ms;
    span_label(Some(first), Some(last), zone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::sent::Reason;

    const MINUTE: i64 = 60_000;

    fn record(reason: Reason, at_ms: i64) -> Record {
        Record { at_ms, ..Record::new(reason, "a rule fired", "text", None) }
    }

    fn quiet_of<'a>(records: &'a [Record]) -> impl Fn(Place) -> Option<&'a Record> {
        move |place| match place {
            Place::Said(n) => records.get(n).filter(|record| record.quiet()),
            _ => None,
        }
    }

    fn shape(entries: &[Entry<'_>]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Place(Place::Row(n)) => format!("row{n}"),
                Entry::Place(Place::Card(n)) => format!("card{n}"),
                Entry::Place(Place::Said(n)) => format!("said{n}"),
                Entry::Quiet(run) => format!("quiet{}", run.len()),
            })
            .collect()
    }

    #[test]
    fn back_to_back_reminders_of_one_reason_fold_and_anything_between_ends_the_run() {
        let records: Vec<Record> = vec![
            record(Reason::LedgerGuard, 0),
            record(Reason::LedgerGuard, MINUTE),
            record(Reason::LedgerGuard, 2 * MINUTE),
            record(Reason::Avatar, 3 * MINUTE),
            record(Reason::LedgerGuard, 4 * MINUTE),
            record(Reason::LedgerGuard, 5 * MINUTE),
            record(Reason::DueToday, 6 * MINUTE),
        ];
        let cases: [(&str, Vec<Place>, Vec<&str>); 6] = [
            (
                "three in a row",
                vec![Place::Said(0), Place::Said(1), Place::Said(2)],
                vec!["quiet3"],
            ),
            (
                "another reason splits",
                vec![Place::Said(1), Place::Said(3), Place::Said(4)],
                vec!["quiet1", "quiet1", "quiet1"],
            ),
            (
                "a row between splits",
                vec![Place::Said(0), Place::Row(0), Place::Said(1)],
                vec!["quiet1", "row0", "quiet1"],
            ),
            (
                "a card between splits",
                vec![Place::Said(0), Place::Card(0), Place::Said(1), Place::Said(2)],
                vec!["quiet1", "card0", "quiet2"],
            ),
            ("a loud reminder keeps its place", vec![Place::Said(6)], vec!["said6"]),
            ("none", vec![], vec![]),
        ];
        for (what, places, want) in cases {
            assert_eq!(shape(&fold(places, quiet_of(&records))), want, "{what}");
        }
    }

    #[test]
    fn a_run_says_what_was_asked_counts_itself_and_names_its_time() {
        let zone = TimeZone::UTC;
        let one = [record(Reason::Avatar, 23 * 60 * MINUTE + 44 * MINUTE)];
        let run: Vec<&Record> = one.iter().collect();
        assert_eq!(words(&run), "Tidewatch asked for an avatar");
        assert_eq!(count(&run), None);
        assert_eq!(span(&run, &zone).as_deref(), Some("23:44"));
        let four: Vec<Record> = (0..4)
            .map(|n| record(Reason::LedgerGuard, 22 * 60 * MINUTE + 22 * MINUTE + n * 6 * MINUTE))
            .collect();
        let run: Vec<&Record> = four.iter().collect();
        assert_eq!(words(&run), "Tidewatch reminded the agent to update its report");
        assert_eq!(count(&run).as_deref(), Some("4"));
        assert_eq!(span(&run, &zone).as_deref(), Some("22:22 to 22:40"));
        assert_eq!(key(&run), four[0].id);
    }
}
