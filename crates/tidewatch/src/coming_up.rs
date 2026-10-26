//! The Coming up list: what every tab has scheduled, in the order it needs looking at (pure).

use std::path::PathBuf;

use tidewatch_core::civil_date::Date;
use tidewatch_core::progress::{Ledger, When};
use tidewatch_core::schedule::{Index, Owner, Presence};

/// Which part of the list a row sits in; the list is sorted by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Group {
    /// A date or an about-date that is today or past.
    Due,
    Dated,
    About,
    On,
    Parked,
    You,
    /// A `when` the ledger wrote that is none of the kinds.
    Other,
}

impl Group {
    fn of(when: &When, today: Date) -> Self {
        match when {
            When::Date(_) | When::About(_) if when.due_on(today) => Self::Due,
            When::Date(_) => Self::Dated,
            When::About(_) => Self::About,
            When::On(_) => Self::On,
            When::Parked(_) => Self::Parked,
            When::You => Self::You,
            When::Unknown(_) => Self::Other,
        }
    }
}

/// Where a row came from. `Next` rows are the Next entries of a ledger that stopped with nothing
/// scheduled; they are never written to the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Source {
    Scheduled,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) group: Group,
    pub(crate) source: Source,
    pub(crate) when: When,
    /// Days from today to the date, negative when overdue; `None` for `On`, `Parked`, `You`.
    pub(crate) days: Option<i64>,
    pub(crate) what: String,
    pub(crate) why: Option<String>,
    pub(crate) repo: PathBuf,
    pub(crate) tab: String,
    /// The goal of the ledger the item is in.
    pub(crate) goal: String,
    pub(crate) session_id: Option<String>,
    pub(crate) profile_id: Option<String>,
    pub(crate) presence: Presence,
}

/// Every open scheduled item of `index`: due or overdue first (oldest first), then dated, then
/// about-dates (soonest first), then the ones with no date in the order `On`, `Parked`, `You`,
/// each kept in index order.
pub(crate) fn coming_up(today: Date, index: &Index) -> Vec<Row> {
    let mut rows: Vec<Row> = index
        .tabs
        .iter()
        .flat_map(|entry| {
            entry.items.iter().map(move |item| Row {
                group: Group::of(&item.when, today),
                source: Source::Scheduled,
                days: item.when.days_until(today),
                when: item.when.clone(),
                what: item.what.clone(),
                why: item.why.clone(),
                repo: entry.repo.clone(),
                tab: entry.tab.clone(),
                goal: entry.goal.clone(),
                session_id: entry.session_id.clone(),
                profile_id: entry.profile_id.clone(),
                presence: entry.presence,
            })
        })
        .collect();
    rows.sort_by_key(sort_key);
    rows
}

fn sort_key(row: &Row) -> (Group, Source, Option<i64>) {
    (row.group, row.source, row.when.date().map(Date::day_number))
}

/// The Next entries of a ledger that stopped with nothing open in Scheduled, each as a row in the
/// due group: they were meant to happen next.
pub(crate) fn next_rows(owner: &Owner, ledger: &Ledger) -> Vec<Row> {
    ledger
        .stranded_next()
        .into_iter()
        .map(|entry| Row {
            group: Group::Due,
            source: Source::Next,
            when: When::Unknown(NEXT_STEP.to_owned()),
            days: None,
            what: entry.text.clone(),
            why: entry.why.clone(),
            repo: owner.repo.clone(),
            tab: owner.tab.clone(),
            goal: ledger.goal.clone(),
            session_id: owner.session_id.clone(),
            profile_id: owner.profile_id.clone(),
            presence: Presence::Open,
        })
        .collect()
}

pub(crate) const NEXT_STEP: &str = "next step";

/// `rows` with the `extra` ones placed where their group and source put them; the order inside a
/// group and source is kept.
pub(crate) fn with_extra(mut rows: Vec<Row>, extra: Vec<Row>) -> Vec<Row> {
    rows.extend(extra);
    rows.sort_by_key(sort_key);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::progress::Ledger;
    use tidewatch_core::schedule::{Entry, Owner};

    fn today() -> Date {
        Date::new(2026, 10, 17).unwrap()
    }

    fn index(tabs: &[(&str, Presence, &str)]) -> Index {
        let mut index = Index::default();
        for (tab, presence, scheduled) in tabs {
            let owner = Owner {
                repo: "/work/web".into(),
                tab: (*tab).into(),
                session_id: Some(format!("session-{tab}")),
                profile_id: Some("builder".into()),
            };
            let ledger =
                Ledger::parse(&format!("# Goal\nGoal of {tab}\n\n## Scheduled\n{scheduled}"));
            index.set(Entry::of(owner, &ledger, *presence));
        }
        index
    }

    fn whats(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.what.as_str()).collect()
    }

    #[test]
    fn due_first_then_dated_then_about_then_on_parked_and_you() {
        let rows = coming_up(
            today(),
            &index(&[
                (
                    "otter",
                    Presence::Open,
                    "- [ ] you -- you 1\n- [ ] parked: later -- parked 1\n- [ ] on: x -- on 1\n- [ ] ~2026-11-20 -- about far\n- [ ] 2026-12-01 -- dated far\n- [ ] 2026-10-20 -- dated soon\n- [ ] ~2026-10-18 -- about soon\n- [ ] someday -- other 1\n",
                ),
                (
                    "heron",
                    Presence::Closed,
                    "- [ ] 2026-10-10 -- overdue a week\n- [ ] ~2026-10-16 -- about yesterday\n- [ ] 2026-10-17 -- due today\n- [ ] on: y -- on 2\n- [ ] you -- you 2\n",
                ),
            ]),
        );
        assert_eq!(
            whats(&rows),
            [
                "overdue a week",
                "about yesterday",
                "due today",
                "dated soon",
                "dated far",
                "about soon",
                "about far",
                "on 1",
                "on 2",
                "parked 1",
                "you 1",
                "you 2",
                "other 1",
            ]
        );
        let groups: Vec<_> = rows.iter().map(|row| row.group).collect();
        assert_eq!(
            groups,
            [
                Group::Due,
                Group::Due,
                Group::Due,
                Group::Dated,
                Group::Dated,
                Group::About,
                Group::About,
                Group::On,
                Group::On,
                Group::Parked,
                Group::You,
                Group::You,
                Group::Other,
            ]
        );
    }

    #[test]
    fn a_row_says_how_far_off_it_is_and_how_to_get_back_to_its_tab() {
        let rows = coming_up(
            today(),
            &index(&[(
                "heron",
                Presence::Closed,
                "- [ ] 2026-10-10 -- late -- why late\n- [ ] 2026-11-01 -- soon\n- [ ] you -- mine\n",
            )]),
        );
        assert_eq!(rows.iter().map(|row| row.days).collect::<Vec<_>>(), [Some(-7), Some(15), None]);
        let late = &rows[0];
        assert_eq!(late.why.as_deref(), Some("why late"));
        assert_eq!(
            (late.repo.as_path(), late.tab.as_str()),
            (std::path::Path::new("/work/web"), "heron")
        );
        assert_eq!(late.goal, "Goal of heron");
        assert_eq!(late.session_id.as_deref(), Some("session-heron"));
        assert_eq!(late.profile_id.as_deref(), Some("builder"));
        assert_eq!(late.presence, Presence::Closed);
    }

    #[test]
    fn nothing_scheduled_is_an_empty_list_and_the_same_date_keeps_index_order() {
        assert_eq!(coming_up(today(), &Index::default()), Vec::new());
        let rows = coming_up(
            today(),
            &index(&[
                ("a", Presence::Open, "- [ ] 2026-11-01 -- first\n"),
                ("b", Presence::Open, "- [ ] 2026-11-01 -- second\n- [ ] 2026-11-01 -- third\n"),
            ]),
        );
        assert_eq!(whats(&rows), ["first", "second", "third"]);
    }

    #[test]
    fn the_same_ledger_moves_a_row_from_dated_to_due_as_the_days_pass() {
        let index = index(&[("a", Presence::Open, "- [ ] 2026-10-18 -- tomorrow\n")]);
        let groups: Vec<_> = [17, 18, 19]
            .into_iter()
            .map(|day| coming_up(Date::new(2026, 10, day).unwrap(), &index)[0].group)
            .collect();
        assert_eq!(groups, [Group::Dated, Group::Due, Group::Due]);
    }

    fn owner() -> Owner {
        Owner {
            repo: "/work/api".into(),
            tab: "pkce".into(),
            session_id: Some("s1".into()),
            profile_id: Some("builder".into()),
        }
    }

    fn ledger(state: &str, next: &str, scheduled: &str) -> Ledger {
        Ledger::parse(&format!(
            "# Goal\nShip pkce\n\n## Status\nstate: {state}\n\n## Next\n{next}\n## Scheduled\n{scheduled}"
        ))
    }

    #[test]
    fn next_entries_of_a_stopped_ledger_with_nothing_scheduled_become_due_rows() {
        let cases = [
            ("blocked", "- After approval: merge -- ci is green\n- Watch deploy\n", "", 2),
            ("waiting", "- Watch deploy\n", "", 1),
            ("done", "- Check logs\n", "", 1),
            ("working", "- Write the test\n", "", 0),
            ("blocked", "- nothing\n", "", 0),
            ("blocked", "- Watch deploy\n", "- [ ] on: merge -- Deploy\n", 0),
            ("blocked", "- Watch deploy\n", "- [x] 2026-10-01 -- Old\n", 1),
        ];
        for (state, next, scheduled, want) in cases {
            let rows = next_rows(&owner(), &ledger(state, next, scheduled));
            assert_eq!(rows.len(), want, "{state} {next} {scheduled}");
            for row in &rows {
                assert_eq!((row.group, row.source), (Group::Due, Source::Next));
                assert_eq!((row.repo.as_path(), row.tab.as_str()), ("/work/api".as_ref(), "pkce"));
                assert_eq!(row.goal, "Ship pkce");
                assert_eq!(row.days, None);
                assert_eq!(row.presence, Presence::Open);
            }
        }
        let rows = next_rows(&owner(), &ledger("blocked", "- Merge -- ci is green\n", ""));
        assert_eq!(rows[0].what, "Merge");
        assert_eq!(rows[0].why.as_deref(), Some("ci is green"));
    }

    #[test]
    fn next_rows_sit_after_the_scheduled_rows_of_the_due_group_and_before_dated_ones() {
        let rows = with_extra(
            coming_up(
                today(),
                &index(&[(
                    "otter",
                    Presence::Open,
                    "- [ ] 2026-10-18 -- dated\n- [ ] 2026-10-17 -- due today\n- [ ] on: x -- on one\n",
                )]),
            ),
            next_rows(&owner(), &ledger("blocked", "- first next\n- second next\n", "")),
        );
        assert_eq!(whats(&rows), ["due today", "first next", "second next", "dated", "on one"]);
        assert_eq!(
            rows.iter().map(|row| row.source).collect::<Vec<_>>(),
            [Source::Scheduled, Source::Next, Source::Next, Source::Scheduled, Source::Scheduled]
        );
    }
}
