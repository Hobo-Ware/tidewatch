//! The Coming up rail of the Overview: the rows of the Coming up list grouped by day (pure).

use std::path::Path;

use tidewatch_core::civil_date::Date;
use tidewatch_core::progress::When;

use crate::coming_up::{Group, NEXT_STEP, Row, Source};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    Plain,
    Parked,
    Decide,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    /// Which of the rows it was built from.
    pub(crate) row: usize,
    pub(crate) what: String,
    pub(crate) place: String,
    pub(crate) tab: String,
    /// `3 d late`, `on: deploy`, `parked`: what the second line adds after the tab.
    pub(crate) note: Option<String>,
    pub(crate) mark: Mark,
    /// Scheduled for about the day rather than on it.
    pub(crate) about: bool,
    pub(crate) tip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Day {
    /// `TODAY \u{b7} MON 26`, `TUE 27`, `NO DATE`.
    pub(crate) label: String,
    pub(crate) today: bool,
    /// `October`, `January 2027` (the year when it is not the current one) for a dated day.
    pub(crate) month: Option<String>,
    /// The month header is drawn before this day: it is the first of its month on the rail.
    pub(crate) starts_month: bool,
    pub(crate) items: Vec<Item>,
}

fn weekday_and_day(date: Date) -> String {
    format!("{} {}", date.weekday().short().to_uppercase(), date.day())
}

fn month_of(date: Date, today: Date) -> String {
    let name = MONTHS[usize::from(date.month().clamp(1, 12)) - 1];
    if date.year() == today.year() { name.to_owned() } else { format!("{name} {}", date.year()) }
}

/// The day a row sits under: `Today` for what is due or overdue, `Dated` by date (exact and about
/// alike), and `Nothing` for what waits on an event, a pause, the owner or something unreadable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Today,
    Dated(Date),
    Nothing,
}

fn slot_of(row: &Row) -> Slot {
    match (&row.when, row.group) {
        (_, Group::Due) => Slot::Today,
        (When::Date(date) | When::About(date), _) => Slot::Dated(*date),
        _ => Slot::Nothing,
    }
}

fn label_of(slot: Slot, today: Date) -> String {
    match slot {
        Slot::Today => format!("TODAY \u{b7} {}", weekday_and_day(today)),
        Slot::Dated(date) => weekday_and_day(date),
        Slot::Nothing => "NO DATE".to_owned(),
    }
}

fn note_of(row: &Row) -> Option<String> {
    match (&row.when, row.days) {
        _ if row.source == Source::Next => Some(NEXT_STEP.to_owned()),
        (When::Parked(_), _) => Some("parked".to_owned()),
        (When::On(event), _) => Some(format!("on: {event}")),
        (_, Some(days)) if days < 0 => Some(format!("{} d late", -days)),
        _ => None,
    }
}

fn item_of(index: usize, row: &Row, place: String) -> Item {
    let tip = match &row.why {
        Some(why) => format!("{}\n{why}", row.what),
        None => row.what.clone(),
    };
    let mark = match row.group {
        Group::Parked => Mark::Parked,
        Group::You => Mark::Decide,
        _ => Mark::Plain,
    };
    Item {
        row: index,
        what: row.what.clone(),
        place,
        tab: row.tab.clone(),
        note: note_of(row),
        mark,
        about: matches!(row.when, When::About(_)),
        tip,
    }
}

/// The days of the rail, in the order of `rows` (already sorted by `coming_up`): one block per
/// day, `TODAY` first and `NO DATE` last; an about item sits after the exact ones of its day, and
/// the first dated day of each month starts a month. `place_of` names the repo of a row.
pub(crate) fn days(rows: &[Row], today: Date, place_of: impl Fn(&Path) -> String) -> Vec<Day> {
    let mut out: Vec<(Slot, Day)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let place = place_of(&row.repo);
        let slot = slot_of(row);
        let item = item_of(index, row, place);
        match out.iter_mut().find(|(known, _)| *known == slot) {
            Some((_, day)) => day.items.push(item),
            None => out.push((
                slot,
                Day {
                    label: label_of(slot, today),
                    today: slot == Slot::Today,
                    month: match slot {
                        Slot::Dated(date) => Some(month_of(date, today)),
                        Slot::Today | Slot::Nothing => None,
                    },
                    starts_month: false,
                    items: vec![item],
                },
            )),
        }
    }
    mark_months(out.into_iter().map(|(_, day)| day).collect())
}

fn mark_months(mut days: Vec<Day>) -> Vec<Day> {
    let mut last: Option<String> = None;
    for day in &mut days {
        day.starts_month = false;
        if let Some(month) = &day.month {
            day.starts_month = last.as_ref() != Some(month);
            last = Some(month.clone());
        }
    }
    days
}

/// The days with only the items of one repo (`None` keeps everything); a day left empty goes, and
/// the month headers follow the days that stay.
pub(crate) fn narrow(days: &[Day], only: Option<&str>) -> Vec<Day> {
    let kept = days
        .iter()
        .filter_map(|day| {
            let items: Vec<Item> = day
                .items
                .iter()
                .filter(|item| only.is_none_or(|only| item.place == only))
                .cloned()
                .collect();
            (!items.is_empty()).then(|| Day { items, ..day.clone() })
        })
        .collect();
    mark_months(kept)
}

pub(crate) fn count(days: &[Day]) -> usize {
    days.iter().map(|day| day.items.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coming_up::coming_up;
    use tidewatch_core::progress::Ledger;
    use tidewatch_core::schedule::{Entry, Index, Owner, Presence};

    fn today() -> Date {
        Date::new(2026, 10, 26).unwrap()
    }

    fn index(repo: &str, tab: &str, scheduled: &str) -> Entry {
        let owner =
            Owner { repo: repo.into(), tab: tab.into(), session_id: None, profile_id: None };
        Entry::of(
            owner,
            &Ledger::parse(&format!("# Goal\nG\n\n## Scheduled\n{scheduled}")),
            Presence::Open,
        )
    }

    fn rows() -> Vec<Row> {
        let mut index_ = Index::default();
        index_.set(index(
            "/work/api",
            "pkce",
            "- [ ] 2026-10-26 -- rerun check\n- [ ] 2026-10-20 -- overdue one\n- [ ] 2026-10-27 -- tuesday one\n- [ ] ~2026-10-31 -- about one\n- [ ] 2026-11-03 -- next month\n",
        ));
        index_.set(index(
            "/work/web",
            "otter",
            "- [ ] 2026-10-27 -- tuesday two\n- [ ] on: deploy -- after deploy\n- [ ] parked: later -- parked one\n- [ ] you -- decide this\n",
        ));
        coming_up(today(), &index_)
    }

    fn place(repo: &Path) -> String {
        repo.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }

    fn shape(days: &[Day]) -> Vec<(String, Vec<String>)> {
        days.iter()
            .map(|day| (day.label.clone(), day.items.iter().map(|i| i.what.clone()).collect()))
            .collect()
    }

    #[test]
    fn rows_group_under_today_each_day_and_no_date() {
        let all = days(&rows(), today(), place);
        assert_eq!(
            shape(&all),
            [
                (
                    "TODAY \u{b7} MON 26".to_owned(),
                    vec!["overdue one".to_owned(), "rerun check".to_owned()]
                ),
                ("TUE 27".to_owned(), vec!["tuesday one".to_owned(), "tuesday two".to_owned()]),
                ("SAT 31".to_owned(), vec!["about one".to_owned()]),
                ("TUE 3".to_owned(), vec!["next month".to_owned()]),
                (
                    "NO DATE".to_owned(),
                    vec![
                        "after deploy".to_owned(),
                        "parked one".to_owned(),
                        "decide this".to_owned()
                    ]
                ),
            ]
        );
        assert_eq!(count(&all), 9);
        assert!(all[0].today && !all[1].today);
    }

    fn owner_rows() -> Vec<Row> {
        let mut index_ = Index::default();
        index_.set(index(
            "/work/api",
            "pkce",
            "- [ ] 2026-10-28 -- exact wed\n- [ ] 2026-10-29 -- exact thu\n- [ ] 2026-11-01 -- exact nov 1\n- [ ] 2026-11-08 -- exact nov 8\n- [ ] 2026-11-15 -- exact nov 15\n- [ ] 2027-01-15 -- exact jan 15\n- [ ] ~2026-10-28 -- about wed one\n- [ ] ~2026-11-10 -- about nov 10\n",
        ));
        index_.set(index("/work/web", "otter", "- [ ] ~2026-10-28 -- about wed two\n"));
        coming_up(today(), &index_)
    }

    #[test]
    fn exact_and_about_items_of_a_day_share_one_header_in_date_order() {
        let all = days(&owner_rows(), today(), place);
        assert_eq!(
            shape(&all),
            [
                (
                    "WED 28".to_owned(),
                    vec![
                        "exact wed".to_owned(),
                        "about wed one".to_owned(),
                        "about wed two".to_owned()
                    ]
                ),
                ("THU 29".to_owned(), vec!["exact thu".to_owned()]),
                ("SUN 1".to_owned(), vec!["exact nov 1".to_owned()]),
                ("SUN 8".to_owned(), vec!["exact nov 8".to_owned()]),
                ("TUE 10".to_owned(), vec!["about nov 10".to_owned()]),
                ("SUN 15".to_owned(), vec!["exact nov 15".to_owned()]),
                ("FRI 15".to_owned(), vec!["exact jan 15".to_owned()]),
            ]
        );
        let about: Vec<_> = all
            .iter()
            .flat_map(|day| &day.items)
            .filter(|item| item.about)
            .map(|item| item.what.as_str())
            .collect();
        assert_eq!(about, ["about wed one", "about wed two", "about nov 10"]);
    }

    #[test]
    fn a_month_header_starts_each_month_and_names_the_year_only_when_it_is_not_this_one() {
        let all = days(&owner_rows(), today(), place);
        let months: Vec<_> = all
            .iter()
            .map(|day| (day.label.as_str(), day.month.as_deref(), day.starts_month))
            .collect();
        assert_eq!(
            months,
            [
                ("WED 28", Some("October"), true),
                ("THU 29", Some("October"), false),
                ("SUN 1", Some("November"), true),
                ("SUN 8", Some("November"), false),
                ("TUE 10", Some("November"), false),
                ("SUN 15", Some("November"), false),
                ("FRI 15", Some("January 2027"), true),
            ]
        );
    }

    #[test]
    fn today_and_the_undated_days_get_no_month_header() {
        let all = days(&rows(), today(), place);
        let first = &all[0];
        assert!(first.today && first.month.is_none() && !first.starts_month);
        let last = all.last().unwrap();
        assert_eq!(last.label, "NO DATE");
        assert!(last.month.is_none() && !last.starts_month);
        assert_eq!(all.iter().filter(|day| day.starts_month).count(), 2);
    }

    #[test]
    fn narrowing_moves_a_month_header_to_the_first_day_that_stays() {
        let mut index_ = Index::default();
        index_.set(index(
            "/work/api",
            "pkce",
            "- [ ] 2026-10-27 -- api oct\n- [ ] 2026-11-02 -- api nov\n- [ ] 2026-11-04 -- api nov late\n",
        ));
        index_.set(index("/work/web", "otter", "- [ ] 2026-11-03 -- web nov\n"));
        let all = days(&coming_up(today(), &index_), today(), place);
        let starts = |days: &[Day]| -> Vec<(String, bool)> {
            days.iter().map(|day| (day.label.clone(), day.starts_month)).collect()
        };
        assert_eq!(
            starts(&all),
            [
                ("TUE 27".into(), true),
                ("MON 2".into(), true),
                ("TUE 3".into(), false),
                ("WED 4".into(), false)
            ]
        );
        assert_eq!(starts(&narrow(&all, Some("web"))), [("TUE 3".to_owned(), true)]);
        assert_eq!(
            starts(&narrow(&all, Some("api"))),
            [("TUE 27".to_owned(), true), ("MON 2".to_owned(), true), ("WED 4".to_owned(), false)]
        );
        assert_eq!(narrow(&all, None), all);
    }

    #[test]
    fn parked_and_owner_items_are_marked_and_late_ones_say_how_late() {
        let all = days(&rows(), today(), place);
        let item = |what: &str| {
            all.iter().flat_map(|day| &day.items).find(|item| item.what == what).unwrap().clone()
        };
        assert_eq!(
            (item("parked one").mark, item("parked one").note.as_deref()),
            (Mark::Parked, Some("parked"))
        );
        assert_eq!(item("decide this").mark, Mark::Decide);
        assert_eq!(item("overdue one").note.as_deref(), Some("6 d late"));
        assert_eq!(item("after deploy").note.as_deref(), Some("on: deploy"));
        assert_eq!((item("rerun check").note, item("rerun check").mark), (None, Mark::Plain));
        assert_eq!(item("rerun check").place, "api");
        assert_eq!(item("rerun check").tab, "pkce");
    }

    #[test]
    fn an_item_keeps_the_index_of_its_row_whatever_the_filter() {
        let rows = rows();
        let all = days(&rows, today(), place);
        let web = narrow(&all, Some("web"));
        assert_eq!(count(&web), 4);
        for item in web.iter().flat_map(|day| &day.items) {
            assert_eq!(rows[item.row].what, item.what);
            assert_eq!(item.place, "web");
        }
        assert_eq!(narrow(&all, None), all);
        assert_eq!(narrow(&all, Some("gone")), []);
    }

    #[test]
    fn no_rows_make_no_days() {
        assert_eq!(days(&[], today(), place), []);
    }

    #[test]
    fn a_next_row_sits_under_today_with_a_next_step_note() {
        use crate::coming_up::{next_rows, with_extra};
        let owner = Owner {
            repo: "/work/api".into(),
            tab: "pkce".into(),
            session_id: None,
            profile_id: None,
        };
        let stopped = Ledger::parse(
            "# Goal\nG\n\n## Status\nstate: blocked\n\n## Next\n- After approval: merge\n",
        );
        let rows = with_extra(rows(), next_rows(&owner, &stopped));
        let all = days(&rows, today(), place);
        assert_eq!(
            all[0].items.iter().map(|item| item.what.as_str()).collect::<Vec<_>>(),
            ["overdue one", "rerun check", "After approval: merge"]
        );
        let item = &all[0].items[2];
        assert_eq!((item.note.as_deref(), item.mark), (Some("next step"), Mark::Plain));
        assert_eq!(rows[item.row].tab, "pkce");
    }
}
