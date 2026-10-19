//! What the job screens say about a tab's jobs, as plain data: cards for the Progress panel, the
//! tab's bar and percent, the line under the agent's name (pure).

use jiff::tz::TimeZone;
use tidewatch_core::job::{Job, JobBoard, JobState};

const SPARK_POINTS: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Card {
    Bar(BarCard),
    Segments(SegmentsCard),
    Stalled(StalledCard),
    Finished(FinishedLine),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BarCard {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) elapsed: String,
    pub(crate) percent: Option<String>,
    pub(crate) counts: String,
    pub(crate) fraction: Option<f32>,
    pub(crate) series: Vec<f32>,
    pub(crate) rate: Option<String>,
    pub(crate) eta: Option<String>,
    pub(crate) done_at: Option<String>,
    pub(crate) phase: String,
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StalledCard {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) line: String,
    pub(crate) ask: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FinishedLine {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) failed: bool,
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

fn rate_text(per_min: f64) -> String {
    if per_min >= 10.0 {
        format!("{}/min", count_text(per_min.round()))
    } else {
        format!("{per_min:.1}/min")
    }
}

pub(crate) fn clock_label(at_ms: i64, zone: &TimeZone) -> Option<String> {
    let stamp = jiff::Timestamp::from_millisecond(at_ms).ok()?;
    Some(stamp.to_zoned(zone.clone()).strftime("%H:%M").to_string())
}

fn counts_text(job: &Job) -> String {
    let unit = if job.unit.is_empty() || job.unit == "%" {
        String::new()
    } else {
        format!(" {}", job.unit)
    };
    match job.total {
        Some(total) => format!("{} / {}{unit}", count_text(job.done), count_text(total)),
        None => format!("{}{unit}", count_text(job.done)),
    }
}

fn percent_label(job: &Job) -> Option<String> {
    job.percent().map(|percent| format!("{percent:.1}%"))
}

fn stalled_line(job: &Job, now_ms: i64) -> String {
    let quiet = ms_text(job.quiet_ms(now_ms));
    match job.percent() {
        Some(percent) => format!("Stuck at {percent:.1}% for {quiet}"),
        None => format!("No movement for {quiet}"),
    }
}

pub(crate) fn ask_text(job: &Job, now_ms: i64) -> String {
    format!("{} has not moved for {}. Check on it.", job.name, ms_text(job.quiet_ms(now_ms)))
}

fn bar_card(job: &Job, now_ms: i64, zone: &TimeZone) -> BarCard {
    let eta = job.eta_left(now_ms);
    BarCard {
        id: job.id,
        name: job.name.clone(),
        elapsed: ms_text(job.elapsed_ms(now_ms)),
        percent: percent_label(job),
        counts: counts_text(job),
        fraction: job.fraction(),
        series: job.rate_series(SPARK_POINTS),
        rate: job.rate_per_min.map(rate_text),
        eta: eta.map(duration_text),
        done_at: eta.and_then(|secs| {
            clock_label(now_ms.saturating_add(i64::try_from(secs).unwrap_or(0) * 1000), zone)
        }),
        phase: job.phase.clone(),
    }
}

fn segments_card(job: &Job, now_ms: i64) -> SegmentsCard {
    let total = job.total.unwrap_or(0.0) as usize;
    let done = (job.done as usize).min(total);
    let segments = (0..total)
        .map(|index| match index.cmp(&done) {
            std::cmp::Ordering::Less => Segment::Done,
            std::cmp::Ordering::Equal => Segment::Active,
            std::cmp::Ordering::Greater => Segment::Todo,
        })
        .collect();
    let unit = if job.unit.is_empty() { String::new() } else { format!(" {}", job.unit) };
    let mut parts = Vec::new();
    if !job.phase.is_empty() {
        parts.push(job.phase.clone());
    }
    if let Some(secs) = job.eta_left(now_ms) {
        parts.push(format!("about {} left", duration_text(secs.max(1))));
    }
    SegmentsCard {
        id: job.id,
        name: job.name.clone(),
        label: format!(
            "{} of {}{unit}",
            count_text(job.done),
            count_text(job.total.unwrap_or(0.0))
        ),
        segments,
        sub: parts.join(" \u{b7} "),
    }
}

fn finished_line(job: &Job, now_ms: i64) -> FinishedLine {
    let failed = job.state == JobState::Failed;
    let mut parts = Vec::new();
    if failed {
        parts.push("failed".to_owned());
    } else if job.done > 0.0 && !job.unit.is_empty() && job.unit != "%" {
        parts.push(format!("{} {}", count_text(job.done), job.unit));
    }
    parts.push(ms_text(job.elapsed_ms(now_ms)));
    FinishedLine { id: job.id, name: job.name.clone(), failed, detail: parts.join(" \u{b7} ") }
}

/// The cards of a board: running jobs in the order they began, then stalled ones, then finished
/// ones with the latest first.
pub(crate) fn cards(board: &JobBoard, now_ms: i64, zone: &TimeZone) -> Vec<Card> {
    let mut running = Vec::new();
    let mut stalled = Vec::new();
    let mut finished: Vec<&Job> = Vec::new();
    for job in &board.jobs {
        match job.state {
            JobState::Running if job.is_segmented() => {
                running.push(Card::Segments(segments_card(job, now_ms)));
            }
            JobState::Running => running.push(Card::Bar(bar_card(job, now_ms, zone))),
            JobState::Stalled => stalled.push(Card::Stalled(StalledCard {
                id: job.id,
                name: job.name.clone(),
                line: stalled_line(job, now_ms),
                ask: ask_text(job, now_ms),
            })),
            JobState::Done | JobState::Failed => finished.push(job),
        }
    }
    finished.sort_by_key(|job| std::cmp::Reverse(job.finished_ms));
    running
        .into_iter()
        .chain(stalled)
        .chain(finished.into_iter().map(|job| Card::Finished(finished_line(job, now_ms))))
        .collect()
}

/// The leading job's bar for the tab. `None` when no job has a total to measure by.
pub(crate) fn tab_job(board: &JobBoard, now_ms: i64) -> Option<TabJob> {
    let job = board.lead()?;
    let percent = job.percent()?;
    let mut parts = vec![format!("{}: {}", job.name, counts_text(job))];
    parts.extend(job.eta_left(now_ms).map(|secs| format!("{} left", duration_text(secs))));
    if job.state == JobState::Stalled {
        parts.push(format!("stuck for {}", ms_text(job.quiet_ms(now_ms))));
    }
    let tip = parts.join(" \u{b7} ");
    Some(TabJob {
        fraction: (percent / 100.0) as f32,
        label: format!("{}%", percent.round()),
        stalled: job.state == JobState::Stalled,
        tip,
    })
}

/// The status line of an agent row: `21% \u{b7} 1h 52m`.
pub(crate) fn lane_line(board: &JobBoard, now_ms: i64) -> Option<String> {
    let job = board.lead()?;
    let mut parts = Vec::new();
    if let Some(percent) = job.percent() {
        parts.push(format!("{}%", percent.round()));
    }
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
