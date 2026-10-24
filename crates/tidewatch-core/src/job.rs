//! A long job of one tab: what a script, the agent or the terminal reported about it, and what
//! follows (rate, ETA, stalled). Reports from every source merge by job name. Pure.

use serde::{Deserialize, Serialize};

pub const STALL_AFTER_MS: i64 = 5 * 60 * 1000;
pub const QUIET_GONE_AFTER_MS: i64 = 5 * 60 * 1000;
pub const FINISHED_KEEP_MS: i64 = 10 * 60 * 1000;
pub const MIN_SAMPLES_FOR_ETA: usize = 2;
pub const MAX_JOBS: usize = 24;
pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_WORD_CHARS: usize = 40;
const MAX_SAMPLES: usize = 48;
const RATE_WINDOW: usize = 12;
const MAX_ETA_SECS: f64 = 100.0 * 86_400.0;
const MIN_RATE_SPAN_MS: i64 = 1_000;
const SEGMENT_MAX_TOTAL: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Running,
    Done,
    Failed,
    Stalled,
}

impl JobState {
    /// The state a script or an agent named, in any of the usual words.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "running" | "active" | "started" | "pending" | "in_progress" => Some(Self::Running),
            "done" | "complete" | "completed" | "finished" | "ok" | "success" | "passed" => {
                Some(Self::Done)
            }
            "failed" | "fail" | "error" | "errored" => Some(Self::Failed),
            "stalled" | "stuck" => Some(Self::Stalled),
            _ => None,
        }
    }

    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stalled => "stalled",
        }
    }

    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub at_ms: i64,
    pub done: f64,
}

/// One reading of a job, from any source. A field left `None` keeps what the job has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub name: String,
    pub done: Option<f64>,
    pub total: Option<f64>,
    pub unit: Option<String>,
    pub phase: Option<String>,
    pub state: Option<JobState>,
    pub eta_s: Option<u64>,
}

impl Report {
    #[must_use]
    pub fn named(name: &str) -> Self {
        Self { name: name.to_owned(), ..Self::default() }
    }

    /// The report as the board takes it: the name on one line and cut to length, numbers finite and
    /// not negative, a total of zero or less dropped (a total of 0 says nothing), words cut to
    /// length. `None` when there is no name left.
    #[must_use]
    pub fn cleaned(&self) -> Option<Self> {
        let name = one_line(&self.name, MAX_NAME_CHARS);
        if name.is_empty() {
            return None;
        }
        let finite = |value: f64| value.is_finite().then_some(value.max(0.0));
        Some(Self {
            name,
            done: self.done.and_then(finite),
            total: self.total.and_then(finite).filter(|total| *total > 0.0),
            unit: self.unit.as_deref().map(|text| one_line(text, MAX_WORD_CHARS)),
            phase: self.phase.as_deref().map(|text| one_line(text, MAX_NAME_CHARS)),
            state: self.state,
            eta_s: self.eta_s,
        })
    }
}

fn one_line(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(max).collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub done: f64,
    pub total: Option<f64>,
    pub unit: String,
    pub phase: String,
    pub state: JobState,
    pub started_ms: i64,
    pub updated_ms: i64,
    /// When `done` last moved: the clock the stall rule reads.
    pub changed_ms: i64,
    pub finished_ms: Option<i64>,
    pub rate_per_min: Option<f64>,
    /// Seconds left as of the last sample, from the rate. Read it through [`Job::eta_left`].
    pub eta_s: Option<u64>,
    /// What the reporter said is left, and when it said so.
    pub eta_hint: Option<(i64, u64)>,
    pub samples: Vec<Sample>,
}

impl Job {
    fn new(id: u64, name: String, now_ms: i64) -> Self {
        Self {
            id,
            name,
            done: 0.0,
            total: None,
            unit: String::new(),
            phase: String::new(),
            state: JobState::Running,
            started_ms: now_ms,
            updated_ms: now_ms,
            changed_ms: now_ms,
            finished_ms: None,
            rate_per_min: None,
            eta_s: None,
            eta_hint: None,
            samples: Vec::new(),
        }
    }

    /// How far along, 0 to 100. `None` without a usable total.
    #[must_use]
    pub fn percent(&self) -> Option<f64> {
        let total = self.total.filter(|total| *total > 0.0)?;
        Some((self.done / total * 100.0).clamp(0.0, 100.0))
    }

    #[must_use]
    pub fn fraction(&self) -> Option<f32> {
        self.percent().map(|percent| (percent / 100.0) as f32)
    }

    /// A small whole total (3 of 5 checks) reads better as segments than as a bar.
    #[must_use]
    pub fn is_segmented(&self) -> bool {
        self.total
            .is_some_and(|total| total.fract() == 0.0 && (2.0..=SEGMENT_MAX_TOTAL).contains(&total))
    }

    /// Seconds left at `now_ms`: from the rate when there is one, else what the reporter said.
    /// `None` when neither exists or the job is over.
    #[must_use]
    pub fn eta_left(&self, now_ms: i64) -> Option<u64> {
        if self.state != JobState::Running {
            return None;
        }
        let (base_ms, secs) = match (self.eta_s, self.samples.last(), self.eta_hint) {
            (Some(secs), Some(last), _) => (last.at_ms, secs),
            (_, _, Some((at_ms, secs))) => (at_ms, secs),
            _ => return None,
        };
        let spent = u64::try_from((now_ms - base_ms).max(0) / 1000).unwrap_or(0);
        Some(secs.saturating_sub(spent)).filter(|left| *left > 0)
    }

    /// A job with no total whose reporter gave a time left: its bar is the clock.
    #[must_use]
    pub fn is_timed(&self) -> bool {
        self.total.is_none() && self.eta_hint.is_some()
    }

    /// How much of a timed job's expected run has passed, 0 to 1. `None` for any other job.
    #[must_use]
    pub fn time_fraction(&self, now_ms: i64) -> Option<f32> {
        if !self.is_timed() {
            return None;
        }
        let (hint_ms, secs) = self.eta_hint?;
        let expected_ms = (hint_ms - self.started_ms).max(0) + i64::try_from(secs).ok()? * 1000;
        (expected_ms > 0)
            .then(|| (self.elapsed_ms(now_ms) as f64 / expected_ms as f64).clamp(0.0, 1.0) as f32)
    }

    /// When the board drops this job if nothing more is reported: a finished job after ten minutes,
    /// a job with no total once it has been quiet for five (a timed one counts from the end of its
    /// expected run). A job with a total stays until it finishes.
    #[must_use]
    pub fn gone_at(&self) -> Option<i64> {
        if let Some(finished) = self.finished_ms {
            return Some(finished + FINISHED_KEEP_MS);
        }
        if self.total.is_some() {
            return None;
        }
        let expected_end = self
            .eta_hint
            .and_then(|(at_ms, secs)| Some(at_ms + i64::try_from(secs).ok()? * 1000))
            .unwrap_or(0);
        Some(self.updated_ms.max(expected_end) + QUIET_GONE_AFTER_MS)
    }

    /// How long `done` has stood still.
    #[must_use]
    pub fn quiet_ms(&self, now_ms: i64) -> i64 {
        (now_ms - self.changed_ms).max(0)
    }

    /// How long the job ran or has run so far.
    #[must_use]
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        (self.finished_ms.unwrap_or(now_ms) - self.started_ms).max(0)
    }

    fn refresh_rate(&mut self) {
        self.rate_per_min = rate_per_min(&self.samples);
        self.eta_s = eta_secs(self.done, self.total, self.rate_per_min, self.samples.len());
    }
}

/// Items per minute over the latest samples. `None` with fewer than two, with no time between
/// them, or when nothing moved.
#[must_use]
pub fn rate_per_min(samples: &[Sample]) -> Option<f64> {
    let window = &samples[samples.len().saturating_sub(RATE_WINDOW)..];
    let (first, last) = (window.first()?, window.last()?);
    let span = last.at_ms - first.at_ms;
    let moved = last.done - first.done;
    (span >= MIN_RATE_SPAN_MS && moved > 0.0).then(|| moved / (span as f64 / 60_000.0))
}

/// Seconds to finish at `rate` (per minute). `None` before two samples, without a total, when
/// done has reached the total, without a positive rate, or for a time past a hundred days.
#[must_use]
pub fn eta_secs(done: f64, total: Option<f64>, rate: Option<f64>, samples: usize) -> Option<u64> {
    let total = total.filter(|total| *total > 0.0)?;
    let rate = rate.filter(|rate| *rate > 0.0)?;
    if samples < MIN_SAMPLES_FOR_ETA || done >= total {
        return None;
    }
    let secs = ((total - done) / rate * 60.0).round();
    (secs.is_finite() && secs <= MAX_ETA_SECS).then_some(secs as u64)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JobBoard {
    pub jobs: Vec<Job>,
    next_id: u64,
}

impl JobBoard {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Takes one reading. Jobs merge by name, ignoring case. Returns whether anything changed.
    pub fn report(&mut self, report: &Report, now_ms: i64) -> bool {
        let Some(report) = report.cleaned() else { return false };
        let key = report.name.to_lowercase();
        let index = match self.jobs.iter().position(|job| job.name.to_lowercase() == key) {
            Some(index) => index,
            None => self.add(report.name.clone(), now_ms),
        };
        let before = self.jobs[index].clone();
        apply(&mut self.jobs[index], &report, now_ms);
        let after = &self.jobs[index];
        let moved = after.samples != before.samples
            || after.done != before.done
            || after.total != before.total
            || after.unit != before.unit
            || after.phase != before.phase
            || after.state != before.state
            || after.eta_hint != before.eta_hint;
        moved || before.updated_ms == after.started_ms && before.samples.is_empty()
    }

    /// Takes a reading for a job the board already runs. A reading with no such job (a line
    /// scraped from the terminal) never starts one, and never restarts a finished one.
    pub fn report_known(&mut self, report: &Report, now_ms: i64) -> bool {
        let Some(cleaned) = report.cleaned() else { return false };
        let key = cleaned.name.to_lowercase();
        let known =
            self.jobs.iter().any(|job| job.name.to_lowercase() == key && !job.state.is_finished());
        known && self.report(&cleaned, now_ms)
    }

    fn add(&mut self, name: String, now_ms: i64) -> usize {
        if self.jobs.len() >= MAX_JOBS {
            let drop = self.jobs.iter().position(|job| job.state.is_finished()).unwrap_or(0);
            self.jobs.remove(drop);
        }
        self.next_id += 1;
        self.jobs.push(Job::new(self.next_id, name, now_ms));
        self.jobs.len() - 1
    }

    /// Drops the jobs that are gone by `now_ms` and marks a running job with a total whose `done`
    /// has stood still for five minutes as stalled. Returns whether anything changed.
    pub fn tick(&mut self, now_ms: i64) -> bool {
        let before = self.jobs.len();
        self.jobs.retain(|job| job.gone_at().is_none_or(|at| at > now_ms));
        let mut changed = self.jobs.len() != before;
        for job in &mut self.jobs {
            if job.state == JobState::Running
                && job.total.is_some()
                && job.quiet_ms(now_ms) >= STALL_AFTER_MS
            {
                job.state = JobState::Stalled;
                changed = true;
            }
        }
        changed
    }

    /// The next moment `tick` changes something if nothing is reported: a running job with a
    /// total turning stalled, or a job going away.
    #[must_use]
    pub fn next_change_at(&self) -> Option<i64> {
        let stalls = self
            .jobs
            .iter()
            .filter(|job| job.state == JobState::Running && job.total.is_some())
            .map(|job| job.changed_ms + STALL_AFTER_MS);
        stalls.chain(self.jobs.iter().filter_map(Job::gone_at)).min()
    }

    #[must_use]
    pub fn has_running(&self) -> bool {
        self.jobs.iter().any(|job| job.state == JobState::Running)
    }

    /// The job the tab and the lane show: the one with a percent that moved last, else the one
    /// that moved last. Finished jobs never lead.
    #[must_use]
    pub fn lead(&self) -> Option<&Job> {
        self.jobs
            .iter()
            .filter(|job| !job.state.is_finished())
            .max_by_key(|job| (job.percent().is_some(), job.changed_ms))
    }

    pub fn remove_finished(&mut self) {
        self.jobs.retain(|job| !job.state.is_finished());
    }
}

fn apply(job: &mut Job, report: &Report, now_ms: i64) {
    let restarted = report.done.is_some_and(|done| done < job.done)
        || (job.state.is_finished() && report.state.is_none_or(|state| !state.is_finished()));
    if restarted {
        job.samples.clear();
        job.state = JobState::Running;
        job.started_ms = now_ms;
        job.changed_ms = now_ms;
        job.finished_ms = None;
        job.eta_hint = None;
        if report.done.is_none() {
            job.done = 0.0;
        }
    }
    job.updated_ms = now_ms;
    if let Some(total) = report.total {
        job.total = Some(total);
    }
    if let Some(unit) = &report.unit {
        job.unit.clone_from(unit);
    }
    if let Some(phase) = &report.phase {
        job.phase.clone_from(phase);
    }
    let moved = report.done.is_some_and(|done| done != job.done)
        || (report.done.is_some() && job.samples.is_empty());
    if let Some(done) = report.done {
        job.done = done;
    }
    if moved && !job.state.is_finished() {
        job.samples.push(Sample { at_ms: now_ms, done: job.done });
        if job.samples.len() > MAX_SAMPLES {
            job.samples.remove(0);
        }
        if report.done.is_some() {
            job.changed_ms = now_ms;
            if job.state == JobState::Stalled {
                job.state = JobState::Running;
            }
        }
    }
    if let Some(secs) = report.eta_s {
        job.eta_hint = Some((now_ms, secs));
    }
    match report.state {
        Some(JobState::Done | JobState::Failed) => finish(job, report.state, now_ms),
        Some(JobState::Stalled) => job.state = JobState::Stalled,
        Some(JobState::Running) | None => {}
    }
    if report.state.is_none() && job.state == JobState::Running && job.total == Some(job.done) {
        finish(job, Some(JobState::Done), now_ms);
    }
    job.refresh_rate();
    if job.state.is_finished() {
        job.eta_s = None;
        job.eta_hint = None;
    }
}

fn finish(job: &mut Job, state: Option<JobState>, now_ms: i64) {
    job.state = state.unwrap_or(JobState::Done);
    job.finished_ms.get_or_insert(now_ms);
}

#[cfg(test)]
mod tests;
