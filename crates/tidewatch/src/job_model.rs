//! What the job screens say about a tab's jobs, as plain data: cards for the Progress panel, the
//! tab's bar and percent, the line under the agent's name (pure).

use jiff::tz::TimeZone;
use tidewatch_core::background::Launch;
use tidewatch_core::job::{Job, JobBoard, JobState};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Card {
    Bar(BarCard),
    Segments(SegmentsCard),
    Quiet(QuietLine),
    Stalled(StalledCard),
    Finished(FinishedLine),
}

/// A job measured against a total, or against the time its reporter expects it to take.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BarCard {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) right: Option<String>,
    pub(crate) fraction: f32,
    pub(crate) detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    Done,
    Active,
    Todo,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SegmentsCard {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) segments: Vec<Segment>,
    pub(crate) sub: String,
}

/// A job with nothing to measure by: its name and how long it has been going.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QuietLine {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) elapsed: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StalledCard {
    pub(crate) id: u64,
    pub(crate) line: String,
    pub(crate) ask: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FinishedLine {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) failed: bool,
    pub(crate) ended: bool,
    pub(crate) detail: String,
}

/// What a tab shows of its leading job: the bar along its bottom edge and the small percent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabJob {
    pub(crate) fraction: f32,
    pub(crate) label: String,
    pub(crate) stalled: bool,
    pub(crate) tip: String,
}

pub(crate) fn duration_text(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => match (secs % 3600) / 60 {
            0 => format!("{}h", secs / 3600),
            minutes => format!("{}h {minutes}m", secs / 3600),
        },
        _ => match (secs % 86_400) / 3600 {
            0 => format!("{}d", secs / 86_400),
            hours => format!("{}d {hours}h", secs / 86_400),
        },
    }
}

fn ms_text(ms: i64) -> String {
    duration_text(u64::try_from(ms.max(0) / 1000).unwrap_or(0))
}

pub(crate) fn count_text(value: f64) -> String {
    if value.fract() != 0.0 {
        return format!("{value:.1}");
    }
    let digits = format!("{value:.0}");
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn counts_text(job: &Job) -> String {
    let unit = unit_suffix(job);
    match job.total {
        Some(total) => format!("{} / {}{unit}", count_text(job.done), count_text(total)),
        None => format!("{}{unit}", count_text(job.done)),
    }
}

pub(crate) fn clock_label(at_ms: i64, zone: &TimeZone) -> Option<String> {
    let stamp = jiff::Timestamp::from_millisecond(at_ms).ok()?;
    Some(stamp.to_zoned(zone.clone()).strftime("%H:%M").to_string())
}

/// The percent to show, whole and never 0: a job that has not moved shows none, one under 1% says
/// so, and one short of the total never reads 100.
pub(crate) fn percent_text(percent: f64) -> Option<String> {
    if !percent.is_finite() || percent <= 0.0 {
        None
    } else if percent < 1.0 {
        Some("<1%".to_owned())
    } else if percent >= 100.0 {
        Some("100%".to_owned())
    } else {
        Some(format!("{}%", percent.round().min(99.0)))
    }
}

fn percent_label(job: &Job) -> Option<String> {
    job.percent().and_then(percent_text)
}

fn unit_suffix(job: &Job) -> String {
    if job.unit.is_empty() || job.unit == "%" { String::new() } else { format!(" {}", job.unit) }
}

fn left_text(secs: u64) -> String {
    format!("about {} left", duration_text(secs))
}

fn stalled_line(job: &Job, now_ms: i64) -> String {
    let quiet = ms_text(job.quiet_ms(now_ms));
    match job.percent().and_then(percent_text) {
        Some(percent) => format!("{} stuck at {percent} for {quiet}", job.name),
        None => format!("{} stuck for {quiet}", job.name),
    }
}

pub(crate) fn ask_text(job: &Job, now_ms: i64) -> String {
    format!("{} has not moved for {}. Check on it.", job.name, ms_text(job.quiet_ms(now_ms)))
}

fn counted_detail(job: &Job, total: f64, now_ms: i64) -> String {
    let mut parts = Vec::new();
    if !job.phase.is_empty() {
        parts.push(job.phase.clone());
    }
    if job.done <= 0.0 {
        parts.push("Starting".to_owned());
    } else if job.unit != "%" {
        parts.push(format!(
            "{} of {}{}",
            count_text(job.done),
            count_text(total),
            unit_suffix(job)
        ));
    }
    parts.extend(job.eta_left(now_ms).map(left_text));
    parts.join(" \u{b7} ")
}

fn timed_detail(job: &Job, now_ms: i64) -> String {
    let mut parts = Vec::new();
    if !job.phase.is_empty() {
        parts.push(job.phase.clone());
    }
    parts.extend(job.eta_left(now_ms).map(left_text));
    parts.extend(job.overdue_ms(now_ms).map(|late| format!("overdue by {}", ms_text(late))));
    parts.join(" \u{b7} ")
}

fn bar_card(job: &Job, total: f64, now_ms: i64) -> BarCard {
    BarCard {
        id: job.id,
        name: job.name.clone(),
        right: percent_label(job),
        fraction: job.fraction().unwrap_or(0.0),
        detail: counted_detail(job, total, now_ms),
    }
}

fn timed_card(job: &Job, fraction: f32, now_ms: i64) -> BarCard {
    BarCard {
        id: job.id,
        name: job.name.clone(),
        right: Some(ms_text(job.elapsed_ms(now_ms))),
        fraction,
        detail: timed_detail(job, now_ms),
    }
}

fn segments_card(job: &Job, total: f64, now_ms: i64) -> SegmentsCard {
    let count = total as usize;
    let done = (job.done as usize).min(count);
    let segments = (0..count)
        .map(|index| match index.cmp(&done) {
            std::cmp::Ordering::Less => Segment::Done,
            std::cmp::Ordering::Equal => Segment::Active,
            std::cmp::Ordering::Greater => Segment::Todo,
        })
        .collect();
    let mut parts = Vec::new();
    if !job.phase.is_empty() {
        parts.push(job.phase.clone());
    }
    parts.extend(job.eta_left(now_ms).map(|secs| left_text(secs.max(1))));
    let label = if job.done > 0.0 {
        format!("{} of {}{}", count_text(job.done), count_text(total), unit_suffix(job))
    } else {
        String::new()
    };
    SegmentsCard {
        id: job.id,
        name: job.name.clone(),
        label,
        segments,
        sub: parts.join(" \u{b7} "),
    }
}

fn quiet_line(job: &Job, now_ms: i64) -> QuietLine {
    QuietLine { id: job.id, name: job.name.clone(), elapsed: ms_text(job.elapsed_ms(now_ms)) }
}

fn finished_line(job: &Job, now_ms: i64) -> FinishedLine {
    let failed = job.state == JobState::Failed;
    let result = if failed {
        "failed".to_owned()
    } else if job.ended {
        "ended, no final report".to_owned()
    } else if job.done > 0.0 && !job.unit.is_empty() && job.unit != "%" {
        format!("{} {}", count_text(job.done), job.unit)
    } else {
        "done".to_owned()
    };
    FinishedLine {
        id: job.id,
        name: job.name.clone(),
        failed,
        ended: job.ended,
        detail: format!("{result} \u{b7} {}", ms_text(job.elapsed_ms(now_ms))),
    }
}

/// The cards of a board: running jobs in the order they began, quiet lines for the ones with
/// nothing to measure, stalled ones, then finished ones with the latest first.
pub(crate) fn cards(board: &JobBoard, now_ms: i64) -> Vec<Card> {
    let mut running = Vec::new();
    let mut quiet = Vec::new();
    let mut stalled = Vec::new();
    let mut finished: Vec<&Job> = Vec::new();
    for job in &board.jobs {
        match (job.state, job.total) {
            (JobState::Running, Some(total)) if job.is_segmented() => {
                running.push(Card::Segments(segments_card(job, total, now_ms)));
            }
            (JobState::Running, Some(total)) => {
                running.push(Card::Bar(bar_card(job, total, now_ms)));
            }
            (JobState::Running, None) => match job.time_fraction(now_ms) {
                Some(fraction) => running.push(Card::Bar(timed_card(job, fraction, now_ms))),
                None => quiet.push(Card::Quiet(quiet_line(job, now_ms))),
            },
            (JobState::Stalled, Some(_)) => stalled.push(Card::Stalled(StalledCard {
                id: job.id,
                line: stalled_line(job, now_ms),
                ask: ask_text(job, now_ms),
            })),
            (JobState::Stalled, None) => quiet.push(Card::Quiet(quiet_line(job, now_ms))),
            (JobState::Done | JobState::Failed, _) => finished.push(job),
        }
    }
    finished.sort_by_key(|job| std::cmp::Reverse(job.finished_ms));
    running
        .into_iter()
        .chain(quiet)
        .chain(stalled)
        .chain(finished.into_iter().map(|job| Card::Finished(finished_line(job, now_ms))))
        .collect()
}

/// A quiet line for each background launch that reported nothing, before the finished ones.
pub(crate) fn add_background(cards: &mut Vec<Card>, launches: &[Launch], now_ms: i64) {
    let at = cards.iter().position(|card| matches!(card, Card::Finished(_))).unwrap_or(cards.len());
    let lines = launches.iter().map(|launch| {
        Card::Quiet(QuietLine {
            id: 0,
            name: format!("{} running in the background, no progress reported", launch.command),
            elapsed: ms_text(now_ms - launch.at_ms),
        })
    });
    cards.splice(at..at, lines);
}

/// The leading job's bar for the tab. `None` when no job has a total to measure by.
pub(crate) fn tab_job(board: &JobBoard, now_ms: i64) -> Option<TabJob> {
    let job = board.lead()?;
    let percent = job.percent()?;
    let label = percent_text(percent)?;
    let mut parts = vec![format!("{}: {}", job.name, counts_text(job))];
    parts.extend(job.eta_left(now_ms).map(|secs| format!("{} left", duration_text(secs))));
    if job.state == JobState::Stalled {
        parts.push(format!("stuck for {}", ms_text(job.quiet_ms(now_ms))));
    }
    let tip = parts.join(" \u{b7} ");
    Some(TabJob {
        fraction: (percent / 100.0) as f32,
        label,
        stalled: job.state == JobState::Stalled,
        tip,
    })
}

/// The status line of an agent row: `21% \u{b7} 1h 52m`.
pub(crate) fn lane_line(board: &JobBoard, now_ms: i64) -> Option<String> {
    let job = board.lead()?;
    let mut parts = Vec::new();
    parts.extend(job.percent().and_then(percent_text));
    match job.state {
        JobState::Stalled => parts.push(format!("stuck {}", ms_text(job.quiet_ms(now_ms)))),
        _ => parts.extend(job.eta_left(now_ms).map(duration_text)),
    }
    if parts.is_empty() {
        parts.push(job.name.clone());
    }
    Some(parts.join(" \u{b7} "))
}

#[cfg(test)]
mod tests;
