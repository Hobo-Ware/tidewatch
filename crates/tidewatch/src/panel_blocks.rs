//! What the panel screens say about time: the age of a block (`now`, `2m ago`, `no word since
//! 14:02`), the `out of date` mark of a report the facts contradict, the `3 things happened since`
//! line, the About row's `Panel on time` text, and the few small drawing helpers the Progress panel
//! and the Overview card share. The text is pure; the numbers come from the tab's `PanelView`.

use egui::{Painter, Rect, Ui};
use jiff::tz::TimeZone;
use tidewatch_core::panel_counter::{self, Counter};
use tidewatch_core::panel_event::{Source, StepBy};
use tidewatch_core::panel_state::MarkSeen;
use tidewatch_core::panel_view::{PanelView, Quiet, ReportMark, Why};
use tidewatch_core::progress::Mark;

use crate::brand;
use crate::job_model::clock_label;
use crate::screen_kit::{TEXT_TAG, TagFill, body, paint_tag, tag_width};

pub(crate) const OUT_OF_DATE: &str = "out of date";
pub(crate) const QUIET_PREFIX: &str = "no word";

/// How long ago, in the fewest characters: `now` for under a minute, then `2m ago`, `3h ago`,
/// `4d ago`. A time in the future (the clock moved back) reads `now`.
pub(crate) fn age_text(now_ms: i64, at_ms: i64) -> String {
    let secs = (now_ms - at_ms).max(0) / 1000;
    match secs {
        0..60 => "now".to_owned(),
        60..3_600 => format!("{}m ago", secs / 60),
        3_600..86_400 => format!("{}h ago", secs / 3_600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

pub(crate) fn quiet_text(since_ms: i64, zone: &TimeZone) -> String {
    clock_label(since_ms, zone)
        .map_or_else(|| QUIET_PREFIX.to_owned(), |time| format!("{QUIET_PREFIX} since {time}"))
}

pub(crate) fn since_text(count: u32) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 thing happened since".to_owned()),
        many => Some(format!("{many} things happened since")),
    }
}

/// The sources whose silence belongs to a block of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Block {
    Turn,
    Jobs,
}

fn block_of(source: Source) -> Option<Block> {
    match source {
        Source::Hook | Source::Transcript => Some(Block::Turn),
        Source::Job => Some(Block::Jobs),
        _ => None,
    }
}

fn silent(quiet: &[Quiet], block: Block) -> Option<i64> {
    quiet.iter().filter(|q| block_of(q.source) == Some(block)).map(|q| q.since_ms).min()
}

/// The age line of each block, one per block the panel draws. A source gone quiet past its limit
/// replaces the age of its block with `no word since`. `report` is the agent's report: its plan in
/// its own words, shown with `since` and the `out of date` mark when the facts contradict it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Blocks {
    pub(crate) out_of_date: Option<Why>,
    pub(crate) report: Option<String>,
    pub(crate) turn: Option<String>,
    pub(crate) jobs: Option<String>,
    pub(crate) links: Option<String>,
    pub(crate) since: Option<String>,
}

/// The newest time a block's own data holds, for a block whose source has not spoken since the app
/// started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Kept {
    pub(crate) jobs_ms: Option<i64>,
    pub(crate) links_ms: Option<i64>,
}

impl Blocks {
    pub(crate) fn of(view: &PanelView, now_ms: i64, zone: &TimeZone, kept: Kept) -> Self {
        let age = |at_ms: Option<i64>| at_ms.map(|at| age_text(now_ms, at));
        let quiet = |block, fallback: Option<String>| {
            silent(&view.quiet, block).map(|since| quiet_text(since, zone)).or(fallback)
        };
        let ages = &view.ages;
        let report_at = ages.report.as_ref().map(|age| age.at_ms);
        Self {
            out_of_date: match view.report {
                Some(ReportMark::OutOfDate(why)) => Some(why),
                Some(ReportMark::Fresh) | None => None,
            },
            report: age(report_at),
            turn: quiet(Block::Turn, age(ages.turn.as_ref().map(|age| age.at_ms))),
            jobs: quiet(Block::Jobs, age(ages.jobs.as_ref().map(|age| age.at_ms).or(kept.jobs_ms))),
            links: age(ages.links.as_ref().map(|age| age.at_ms).or(kept.links_ms)),
            since: report_at.and_then(|_| since_text(view.since)),
        }
    }

    /// The age to put on a card that stands for the whole agent: the turn's silence first, else
    /// the report's age, else the turn's.
    pub(crate) fn card_age(&self) -> Option<&str> {
        let silent = self.turn.as_deref().filter(|text| text.starts_with(QUIET_PREFIX));
        silent.or(self.report.as_deref()).or(self.turn.as_deref())
    }
}

/// Whether the facts contradict the tab's report, so its question and its wait are dropped.
pub(crate) fn contradicted(view: Option<&PanelView>) -> bool {
    view.is_some_and(|view| matches!(view.report, Some(ReportMark::OutOfDate(_))))
}

/// The mark a step is drawn with: a step the agent's report leaves open but Tidewatch ticked reads
/// done, and the second value says the tick is Tidewatch's.
pub(crate) fn tick(mark: Mark, ticked: bool) -> (Mark, bool) {
    match (mark, ticked) {
        (Mark::Todo | Mark::Doing, true) => (Mark::Done, true),
        (mark, _) => (mark, false),
    }
}

/// The note at the right of a step row, with `by Tidewatch` after it for a tick Tidewatch made.
pub(crate) fn why_with_tick(why: Option<&str>, by_tidewatch: bool) -> Option<String> {
    match (why, by_tidewatch) {
        (Some(why), true) => Some(format!("{why} \u{b7} by Tidewatch")),
        (None, true) => Some("by Tidewatch".to_owned()),
        (why, false) => why.map(str::to_owned),
    }
}

/// Whether Tidewatch ticked `step` itself: an answered question, a finished job or a passed check.
pub(crate) fn ticked_by_tidewatch(marks: &[MarkSeen], step: &str) -> bool {
    marks.iter().any(|mark| mark.by == StepBy::Tidewatch && mark.step.trim() == step.trim())
}

pub(crate) fn paint_age(painter: &Painter, right: f32, mid: f32, text: &str) -> f32 {
    let galley = painter.layout_no_wrap(text.to_owned(), body(TEXT_TAG), brand::MUTED);
    let left = right - galley.size().x;
    painter.galley(egui::pos2(left, mid - galley.size().y / 2.0), galley, brand::MUTED);
    left
}

pub(crate) fn age_width(painter: &Painter, text: &str) -> f32 {
    painter.layout_no_wrap(text.to_owned(), body(TEXT_TAG), brand::MUTED).size().x
}

pub(crate) fn out_of_date_width(painter: &Painter) -> f32 {
    tag_width(painter, OUT_OF_DATE)
}

/// The muted `out of date` tag, its left edge at `left`, centered on `mid`; the tooltip says why.
pub(crate) fn paint_out_of_date(ui: &Ui, left: f32, mid: f32, why: Why) -> Rect {
    let rect = paint_tag(
        ui.painter(),
        (left, mid),
        OUT_OF_DATE,
        brand::MUTED,
        TagFill::Line(brand::FIELD_LINE),
    );
    let id = ui.id().with(("out-of-date", left.to_bits(), mid.to_bits()));
    ui.interact(rect, id, egui::Sense::hover())
        .on_hover_text(format!("The report {}", why.words()));
    rect
}

/// The About card's `Panel on time` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OnTime {
    pub(crate) line: String,
    pub(crate) late: Option<String>,
}

const NO_CHANGES: &str = "Panel on time: no changes yet this week";

fn percent_text(hundredths: u64) -> String {
    if hundredths >= 10_000 {
        "100%".to_owned()
    } else {
        format!("{}.{:02}%", hundredths / 100, hundredths % 100)
    }
}

impl OnTime {
    pub(crate) fn of(counter: &Counter, now_ms: i64) -> Self {
        let counts = counter.counts(now_ms);
        if counts.total() == 0 {
            return Self::none();
        }
        let hundredths = u64::from(counts.on_time) * 10_000 / u64::from(counts.total());
        let late = counter
            .worst(now_ms)
            .map(|(source, failed)| format!("Late: {} {failed}", panel_counter::label(source)));
        Self { line: format!("Panel on time: {} this week", percent_text(hundredths)), late }
    }

    /// A stand-in for captures: the given percent, with a late line when it is under 100.
    pub(crate) fn fake(percent: f64) -> Self {
        let hundredths = (percent.clamp(0.0, 100.0) * 100.0 + 1e-6).floor() as u64;
        let late = (hundredths < 10_000)
            .then(|| format!("Late: {} {}", panel_counter::label(Source::Hook), 3));
        Self { line: format!("Panel on time: {} this week", percent_text(hundredths)), late }
    }

    pub(crate) fn none() -> Self {
        Self { line: NO_CHANGES.to_owned(), late: None }
    }
}

#[cfg(test)]
mod tests;
