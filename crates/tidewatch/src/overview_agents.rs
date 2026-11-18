//! The Agents panel and the Finished today panel of the Overview as plain data (pure): the groups
//! an agent sits in, the lines its row shows, how tall the row is, whether every row can show its
//! steps in the room there is, the columns of a row, and the steps ticked today.

use egui::Color32;
use jiff::tz::TimeZone;
use tidewatch_core::panel_view::Tone;
use tidewatch_core::progress::Mark;

use crate::brand;
use crate::overview::{Row, card_view, idle_line, needs_line, waiting_line};
use crate::progress_model::{CardChecklist, CardStep};

/// The state group an agent is listed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    Needs,
    Working,
    Waiting,
    Idle,
}

impl Lane {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Needs => "Needs you",
            Self::Working => "Working",
            Self::Waiting => "Waiting",
            Self::Idle => "Idle",
        }
    }

    pub(crate) fn caption(self) -> &'static str {
        match self {
            Self::Waiting => "soonest first",
            Self::Idle => "nothing asked",
            _ => "",
        }
    }

    pub(crate) fn dot(self) -> Color32 {
        match self {
            Self::Needs => brand::needs(),
            Self::Working => brand::working(),
            Self::Waiting => brand::waiting(),
            Self::Idle => brand::faint(),
        }
    }
}

/// Steps a row shows when it has room for them all.
pub(crate) const STEP_LINES: usize = 4;
pub(crate) const LINE_H: f32 = 22.0;
pub(crate) const ROW_PAD: f32 = 10.0;
pub(crate) const MIN_ROW_H: f32 = 58.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Density {
    /// Up to four step lines.
    Expanded,
    /// What it does now and what comes next.
    Compact,
}

/// Expanded when everything above the rows (`fixed`) and every expanded row fit the `viewport`.
pub(crate) fn density(fixed: f32, expanded: f32, viewport: f32) -> Density {
    if fixed + expanded <= viewport { Density::Expanded } else { Density::Compact }
}

pub(crate) fn row_height(lines: usize) -> f32 {
    (ROW_PAD * 2.0 + lines.max(1) as f32 * LINE_H).max(MIN_ROW_H)
}

/// What the steps column of a row holds, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lines {
    /// The state line (what is asked, what it waits on, why it is idle), over the steps.
    pub(crate) lead: Option<(String, Color32)>,
    pub(crate) steps: CardChecklist,
    /// `next: ...` under the step in progress.
    pub(crate) next: Option<String>,
}

impl Lines {
    pub(crate) fn count(&self) -> usize {
        usize::from(self.lead.is_some()) + self.steps.steps.len() + usize::from(self.next.is_some())
    }
}

/// The state line of a row in its group. Empty when the report and the status say nothing.
pub(crate) fn lead_of(row: &Row, lane: Lane) -> (String, Color32) {
    let view = card_view(row);
    match lane {
        Lane::Needs => (needs_line(row, &view.line), brand::needs()),
        Lane::Working => (view.line, brand::muted()),
        Lane::Waiting => (waiting_line(row, &view.line), brand::waiting()),
        Lane::Idle => {
            let ink = if row.tone == Tone::Failed { brand::red() } else { brand::muted() };
            (idle_line(row), ink)
        }
    }
}

fn open(step: &CardStep) -> bool {
    !matches!(step.mark, Mark::Done | Mark::Skipped)
}

pub(crate) fn lines(row: &Row, lane: Lane, density: Density) -> Lines {
    let list = row.look.as_ref().map(|look| &look.checklist).filter(|list| !list.steps.is_empty());
    let (text, ink) = lead_of(row, lane);
    let said = (!text.is_empty()).then_some((text, ink));
    let lead = if lane == Lane::Working && list.is_some() { None } else { said };
    let Some(list) = list else {
        return Lines { lead, steps: CardChecklist::default(), next: None };
    };
    match density {
        Density::Expanded => {
            let room = STEP_LINES - usize::from(lead.is_some());
            let steps = list.steps.iter().take(room).cloned().collect();
            Lines { lead, steps: CardChecklist { steps, more: 0 }, next: None }
        }
        Density::Compact => {
            let open: Vec<&CardStep> = list.steps.iter().filter(|step| open(step)).collect();
            let doing = open.iter().position(|step| step.mark == Mark::Doing).unwrap_or(0);
            let next = open.get(doing + 1).map(|step| format!("next: {}", step.text));
            if lead.is_some() {
                let next = open.first().map(|step| format!("next: {}", step.text));
                return Lines { lead, steps: CardChecklist::default(), next };
            }
            let steps = open.get(doing).map_or_default(|step| vec![(*step).clone()]);
            Lines { lead, steps: CardChecklist { steps, more: 0 }, next }
        }
    }
}

/// The columns of an agent row for `inner` points of width: the face, the name and its meta line,
/// the steps (what is left over), the progress and the time of the last word.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Columns {
    pub(crate) name: f32,
    pub(crate) steps: f32,
    /// Zero when the window is too narrow for it.
    pub(crate) progress: f32,
    pub(crate) time: f32,
}

pub(crate) const FACE: f32 = 34.0;
pub(crate) const COL_GAP: f32 = 14.0;
const MIN_STEPS_W: f32 = 120.0;

pub(crate) fn columns(inner: f32) -> Columns {
    let name = if inner >= 760.0 { 230.0 } else { 170.0 };
    let time = if inner >= 600.0 { 92.0 } else { 64.0 };
    let progress = if inner >= 640.0 { 150.0 } else { 0.0 };
    let gaps = COL_GAP * if progress > 0.0 { 4.0 } else { 3.0 };
    let steps = (inner - FACE - name - progress - time - gaps).max(MIN_STEPS_W);
    Columns { name, steps, progress, time }
}

/// `9 of 18 \u{b7} 567M tokens`, whichever parts the row has.
pub(crate) fn progress_text(row: &Row) -> Option<String> {
    let steps = row
        .look
        .as_ref()
        .filter(|look| look.ring.total > 0)
        .map(|look| format!("{} of {}", look.ring.done, look.ring.total));
    let tokens = row.tokens.as_ref().map(|tokens| tokens.short.clone());
    let parts: Vec<String> = steps.into_iter().chain(tokens).collect();
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// How many of the mini bar's segments there are and how many are filled, for `done` of `total`
/// steps: one segment per step up to 24, and a step done always fills one.
pub(crate) fn segments(done: usize, total: usize) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let count = total.min(24);
    let filled = (done * count).div_ceil(total).min(count);
    (filled, count)
}

/// A step ticked today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Finished {
    pub(crate) at: u64,
    pub(crate) text: String,
    pub(crate) agent: String,
    pub(crate) tab_id: u64,
}

fn local_date(secs: i64, zone: &TimeZone) -> Option<jiff::civil::Date> {
    Some(jiff::Timestamp::from_second(secs).ok()?.to_zoned(zone.clone()).date())
}

/// Every step the agents ticked today (by the clock of `zone`), newest first.
pub(crate) fn finished_today<'a>(
    rows: impl Iterator<Item = &'a Row>,
    now_secs: i64,
    zone: &TimeZone,
) -> Vec<Finished> {
    let today = local_date(now_secs, zone);
    let mut found: Vec<Finished> = rows
        .flat_map(|row| {
            row.look.iter().flat_map(|look| &look.ticked).map(move |step| Finished {
                at: step.at,
                text: step.text.clone(),
                agent: row.title.clone(),
                tab_id: row.tab_id,
            })
        })
        .filter(|done| {
            i64::try_from(done.at)
                .ok()
                .and_then(|at| local_date(at, zone))
                .is_some_and(|d| Some(d) == today)
        })
        .collect();
    found.sort_by_key(|done| std::cmp::Reverse(done.at));
    found
}

/// `14:02`.
pub(crate) fn clock(at: u64, zone: &TimeZone) -> String {
    i64::try_from(at)
        .ok()
        .and_then(|at| jiff::Timestamp::from_second(at).ok())
        .map_or_else(String::new, |stamp| {
            stamp.to_zoned(zone.clone()).strftime("%H:%M").to_string()
        })
}

pub(crate) const FINISHED_ROW_H: f32 = 26.0;
pub(crate) const FINISHED_MIN: usize = 3;

/// How many finished rows to draw in `room` points of height (the rows only): all of them when
/// `all`, else as many as fit and never fewer than three.
pub(crate) fn finished_shown(total: usize, room: f32, all: bool) -> usize {
    if all {
        return total;
    }
    let fit = (room.max(0.0) / FINISHED_ROW_H).floor() as usize;
    total.min(fit.max(FINISHED_MIN))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overview::test_row;
    use crate::progress_model::look;
    use tidewatch_core::progress::Ledger;
    use tidewatch_core::status::AgentStatus;

    const STEPS: &str = "# Goal\nx\n\n## Status\nstate: working\nnote: building\n\n## Checklist\n### 1. Build\n- [x] Read the code\n- [~] Write the reader\n- [ ] Add tests\n- [ ] Open the PR\n- [ ] Merge\n";

    fn working(markdown: &str) -> Row {
        let mut row = test_row(1, "album-covers", "acme-api", AgentStatus::Working);
        row.look = Some(look(&Ledger::parse(markdown), false));
        row
    }

    fn texts(list: &CardChecklist) -> Vec<&str> {
        list.steps.iter().map(|step| step.text.as_str()).collect()
    }

    #[test]
    fn density_is_expanded_only_when_everything_fits() {
        let cases = [
            (300.0, 400.0, 700.0, Density::Expanded),
            (300.0, 401.0, 700.0, Density::Compact),
            (0.0, 100.0, 50.0, Density::Compact),
        ];
        for (fixed, expanded, viewport, want) in cases {
            assert_eq!(density(fixed, expanded, viewport), want, "{fixed} {expanded} {viewport}");
        }
    }

    #[test]
    fn a_row_is_as_tall_as_its_lines_and_never_under_the_minimum() {
        assert_eq!(row_height(0), MIN_ROW_H);
        assert_eq!(row_height(2), MIN_ROW_H.max(ROW_PAD * 2.0 + 2.0 * LINE_H));
        assert_eq!(row_height(4), ROW_PAD * 2.0 + 4.0 * LINE_H);
    }

    #[test]
    fn an_expanded_working_row_shows_the_last_ticked_step_and_the_open_ones_up_to_four() {
        let lines = lines(&working(STEPS), Lane::Working, Density::Expanded);
        assert_eq!(lines.lead, None);
        assert_eq!(
            texts(&lines.steps),
            ["Read the code", "Write the reader", "Add tests", "Open the PR"]
        );
        assert_eq!((lines.next.clone(), lines.count()), (None, 4));
    }

    #[test]
    fn a_compact_working_row_shows_what_it_does_now_and_the_next_step() {
        let lines = lines(&working(STEPS), Lane::Working, Density::Compact);
        assert_eq!(texts(&lines.steps), ["Write the reader"]);
        assert_eq!(lines.next.as_deref(), Some("next: Add tests"));
        assert_eq!(lines.count(), 2);
    }

    #[test]
    fn a_waiting_row_keeps_its_state_line_and_gives_a_step_the_room_it_leaves() {
        let mut row = working(STEPS);
        row.tone = Tone::Waiting;
        let expanded = lines(&row, Lane::Waiting, Density::Expanded);
        assert!(expanded.lead.is_some());
        assert_eq!(expanded.steps.steps.len(), STEP_LINES - 1);
        let compact = lines(&row, Lane::Waiting, Density::Compact);
        assert_eq!(
            (compact.steps.steps.len(), compact.next.as_deref()),
            (0, Some("next: Write the reader"))
        );
    }

    #[test]
    fn a_row_with_no_steps_is_one_state_line() {
        let mut row = test_row(1, "x", "acme-api", AgentStatus::Done);
        row.idle_secs = 7200.0;
        for density in [Density::Expanded, Density::Compact] {
            let lines = lines(&row, Lane::Idle, density);
            assert_eq!(lines.count(), 1, "{density:?}");
        }
    }

    #[test]
    fn the_columns_fill_the_width_and_the_progress_goes_first_when_it_is_narrow() {
        for inner in [840.0, 1000.0, 1500.0, 1900.0] {
            let cols = columns(inner);
            let used = FACE + cols.name + cols.progress + cols.time + COL_GAP * 4.0 + cols.steps;
            assert!((used - inner).abs() < 0.01, "{inner}: {used}");
        }
        assert_eq!(columns(500.0).progress, 0.0);
        assert!(columns(500.0).steps >= MIN_STEPS_W);
    }

    #[test]
    fn progress_says_steps_and_tokens_and_the_bar_fills_a_segment_per_step() {
        let mut row = working(STEPS);
        assert_eq!(progress_text(&row).as_deref(), Some("1 of 5"));
        row.tokens = Some(crate::token_budget::TabTokens {
            short: "567M tokens".into(),
            tip: String::new(),
        });
        assert_eq!(progress_text(&row).as_deref(), Some("1 of 5 \u{b7} 567M tokens"));
        let cases = [
            ((0, 0), (0, 0)),
            ((0, 5), (0, 5)),
            ((1, 5), (1, 5)),
            ((9, 18), (9, 18)),
            ((1, 100), (1, 24)),
            ((100, 100), (24, 24)),
        ];
        for ((done, total), want) in cases {
            assert_eq!(segments(done, total), want, "{done} of {total}");
        }
    }

    fn ticked(row: &mut Row, steps: &[(u64, &str)]) {
        let look = row.look.as_mut().expect("a look");
        look.ticked = steps
            .iter()
            .map(|(at, text)| crate::progress_model::Ticked { at: *at, text: (*text).to_owned() })
            .collect();
    }

    #[test]
    fn finished_today_keeps_only_todays_steps_newest_first() {
        let zone = TimeZone::UTC;
        let now = 1_800_000_000_i64;
        let midnight = now - now.rem_euclid(86_400);
        let (at, early, yesterday) = (now as u64, midnight as u64 + 60, midnight as u64 - 60);
        let mut one = working(STEPS);
        ticked(&mut one, &[(early, "Early"), (yesterday, "Yesterday"), (at - 30, "Recent")]);
        let mut two = working(STEPS);
        two.tab_id = 2;
        two.title = "import-backfill".into();
        ticked(&mut two, &[(at, "Newest")]);
        let found = finished_today([&one, &two].into_iter(), now, &zone);
        let shown: Vec<(&str, &str)> =
            found.iter().map(|done| (done.text.as_str(), done.agent.as_str())).collect();
        assert_eq!(
            shown,
            [("Newest", "import-backfill"), ("Recent", "album-covers"), ("Early", "album-covers")]
        );
        assert_eq!(clock(midnight as u64 + 14 * 3600 + 2 * 60, &zone), "14:02");
    }

    #[test]
    fn the_finished_panel_shows_what_fits_but_never_under_three() {
        let cases = [
            (10, 520.0, false, 10),
            (10, 130.0, false, 5),
            (10, 10.0, false, 3),
            (2, 10.0, false, 2),
            (10, 10.0, true, 10),
        ];
        for (total, room, all, want) in cases {
            assert_eq!(finished_shown(total, room, all), want, "{total} {room} {all}");
        }
    }
}
