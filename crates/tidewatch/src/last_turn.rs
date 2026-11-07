//! The done check of the agent's last turn, as plain data for the Progress panel (pure).

use std::hash::{DefaultHasher, Hash, Hasher};

use tidewatch_core::civil_date::Date;

use crate::done_check_glue::Look;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LastTurn {
    pub(crate) check: Look,
    pub(crate) can_ask: Result<(), &'static str>,
    pub(crate) owner_asks: bool,
    pub(crate) today: Date,
}

impl LastTurn {
    /// The last turn of a tab, `None` when it has no done check.
    pub(crate) fn of(
        check: Option<Look>,
        can_ask: Result<(), &'static str>,
        today: Date,
    ) -> Option<Self> {
        Some(Self { check: check?, can_ask, owner_asks: true, today })
    }

    /// Changes when the report is new to the owner, so the panel's tab can carry a dot.
    pub(crate) fn attention(&self) -> Option<u64> {
        let report = match &self.check {
            Look::Report(report, _) | Look::Checking(Some(report)) => report.on(self.today),
            Look::Checking(None) => return None,
        };
        let mut hasher = DefaultHasher::new();
        format!("{report:?}").hash(&mut hasher);
        Some(hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::done_check_glue::Checked;
    use tidewatch_core::done_check::{Kind, Line, Report, Status, Unverified};

    fn day(text: &str) -> Date {
        Date::parse(text).unwrap()
    }

    fn of(check: Option<Look>, can_ask: Result<(), &'static str>) -> Option<LastTurn> {
        LastTurn::of(check, can_ask, day("2026-10-27"))
    }

    const STAMP: Checked = Checked { at: 0, same: false };

    fn report_of(detail: &str) -> Report {
        Report {
            head: String::new(),
            lines: vec![Line {
                kind: Kind::Pushed,
                status: Status::Fail,
                detail: detail.to_owned(),
            }],
            unverified: Vec::new(),
            ..Report::default()
        }
    }

    fn report(detail: &str) -> Look {
        Look::Report(report_of(detail), STAMP)
    }

    #[test]
    fn a_tab_with_no_check_has_no_last_turn() {
        assert_eq!(of(None, Ok(())), None);
    }

    #[test]
    fn attention_is_there_for_reports_and_moves_when_they_change() {
        assert_eq!(of(Some(Look::Checking(None)), Ok(())).unwrap().attention(), None);
        let checked = of(Some(report("one")), Ok(())).unwrap().attention();
        assert!(checked.is_some());
        let again = of(Some(report("one")), Ok(())).unwrap().attention();
        let other = of(Some(report("two")), Ok(())).unwrap().attention();
        assert_eq!(checked, again);
        assert_ne!(checked, other);
    }

    #[test]
    fn whether_the_agent_can_be_asked_does_not_make_the_turn_new() {
        let ready = of(Some(report("x")), Ok(())).unwrap();
        let busy = of(Some(report("x")), Err("working")).unwrap();
        assert_eq!(ready.attention(), busy.attention());
    }

    #[test]
    fn a_check_that_runs_again_keeps_the_number_of_the_report_it_replaces() {
        let shown = of(Some(report("one")), Ok(())).unwrap().attention();
        let rerun = Look::Checking(Some(report_of("one")));
        assert_eq!(of(Some(rerun), Ok(())).unwrap().attention(), shown);
        assert_eq!(of(Some(Look::Checking(None)), Ok(())).unwrap().attention(), None);
    }

    #[test]
    fn a_stamp_does_not_make_the_report_new_but_a_waiting_report_coming_due_does() {
        let stamped = |at, same| Look::Report(report_of("one"), Checked { at, same });
        let first = of(Some(stamped(1, false)), Ok(())).unwrap().attention();
        let later = of(Some(stamped(99, true)), Ok(())).unwrap().attention();
        assert_eq!(first, later);

        let waiting = Report {
            head: String::new(),
            lines: vec![Line {
                kind: Kind::Unverified,
                status: Status::Waiting,
                detail: "1 item waiting until 2026-10-28".to_owned(),
            }],
            unverified: vec![Unverified::until("live list", day("2026-10-28"))],
            ..Report::default()
        };
        let look = || Some(Look::Report(waiting.clone(), STAMP));
        let on = |today| LastTurn::of(look(), Ok(()), day(today)).unwrap();
        assert_eq!(on("2026-10-27").attention(), on("2026-10-27").attention());
        assert_ne!(on("2026-10-27").attention(), on("2026-10-28").attention());
        assert_eq!(on("2026-10-28").attention(), on("2026-11-09").attention());
    }
}
