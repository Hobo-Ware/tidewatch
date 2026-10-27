//! Parked tabs: which quiet tabs leave the strip, why, and what brings them back (pure).

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use tidewatch_core::civil_date::Date;
use tidewatch_core::job::{Job, JobBoard, JobState};
use tidewatch_core::progress::{Ledger, Mark, When};
pub(crate) use tidewatch_core::session::Why;

use crate::job_model::duration_text;

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const WORK_QUIET_MS: u64 = 30 * MINUTE_MS;
const LONG_JOB_SECS: u64 = 30 * 60;
const MONTHS: [&str; 12] =
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct Park {
    pub(crate) why: Why,
    pub(crate) since_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Work {
    #[default]
    None,
    Running,
    Long,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Schedule {
    #[default]
    None,
    Waiting(Date),
    Due,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Pinned,
    Draft,
    Plain,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts {
    pub(crate) id: u64,
    pub(crate) group: usize,
    pub(crate) role: Role,
    pub(crate) active: bool,
    pub(crate) keep: bool,
    pub(crate) needs_you: bool,
    pub(crate) park: Option<Park>,
    pub(crate) touched_ms: u64,
    pub(crate) work: Work,
    pub(crate) schedule: Schedule,
}

impl Facts {
    fn parkable(&self) -> bool {
        self.role == Role::Plain && !(self.active || self.keep || self.needs_you)
    }

    fn quiet_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.touched_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fits {
    pub(crate) alone: usize,
    pub(crate) with_pill: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rules {
    pub(crate) after_ms: Option<u64>,
    pub(crate) fits: Fits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Park(u64, Why),
    Unpark(u64),
    Refill(u64),
}

fn auto_why(tab: &Facts, now_ms: u64, rules: &Rules) -> Option<Why> {
    let quiet = tab.quiet_ms(now_ms);
    if matches!(tab.schedule, Schedule::Waiting(_)) && quiet >= WORK_QUIET_MS {
        Some(Why::Waiting)
    } else if tab.work == Work::Long && quiet >= WORK_QUIET_MS {
        Some(Why::Job)
    } else if rules.after_ms.is_some_and(|after| quiet >= after) {
        Some(Why::Idle)
    } else {
        None
    }
}

fn comes_back(tab: &Facts, why: Why) -> bool {
    match why {
        Why::Job => tab.work == Work::None,
        Why::Waiting => !matches!(tab.schedule, Schedule::Waiting(_)),
        Why::Idle | Why::Manual | Why::Overflow => false,
    }
}

fn step(tab: &Facts, now_ms: u64, rules: &Rules) -> Option<Change> {
    match tab.park {
        Some(park) if tab.active || tab.needs_you || comes_back(tab, park.why) => {
            Some(Change::Unpark(tab.id))
        }
        Some(Park { why: Why::Overflow, .. }) => {
            auto_why(tab, now_ms, rules).map(|why| Change::Park(tab.id, why))
        }
        None if tab.parkable() => auto_why(tab, now_ms, rules).map(|why| Change::Park(tab.id, why)),
        Some(_) | None => None,
    }
}

pub(crate) fn decide(tabs: &[Facts], now_ms: u64, rules: &Rules) -> Vec<Change> {
    let mut changes: Vec<Change> = tabs.iter().filter_map(|tab| step(tab, now_ms, rules)).collect();
    let mut parked: Vec<Option<Why>> = tabs.iter().map(|tab| tab.park.map(|p| p.why)).collect();
    let mut touched: Vec<u64> = tabs.iter().map(|tab| tab.touched_ms).collect();
    for change in &changes {
        let at = |id| tabs.iter().position(|tab| tab.id == id).unwrap_or_default();
        match *change {
            Change::Park(id, why) => parked[at(id)] = Some(why),
            Change::Unpark(id) | Change::Refill(id) => {
                parked[at(id)] = None;
                touched[at(id)] = now_ms;
            }
        }
    }
    let groups: BTreeSet<usize> = tabs.iter().map(|tab| tab.group).collect();
    for group in groups {
        let members: Vec<usize> = (0..tabs.len())
            .filter(|i| tabs[*i].group == group && tabs[*i].role != Role::Pinned)
            .collect();
        let shown = members.iter().filter(|i| parked[**i].is_none()).count();
        let away = members.len() - shown;
        let cap = if away > 0 { rules.fits.with_pill } else { rules.fits.alone };
        if shown > cap {
            let mut order: Vec<usize> = members
                .iter()
                .copied()
                .filter(|i| parked[*i].is_none() && tabs[*i].parkable())
                .collect();
            order.sort_by_key(|i| (touched[*i], tabs[*i].id));
            let mut shown = shown;
            for i in order {
                if shown <= rules.fits.with_pill {
                    break;
                }
                changes.push(Change::Park(tabs[i].id, Why::Overflow));
                shown -= 1;
            }
        } else if away > 0 && shown < rules.fits.with_pill {
            let mut order: Vec<usize> = members
                .iter()
                .copied()
                .filter(|i| parked[*i] == Some(Why::Overflow) && !tabs[*i].needs_you)
                .collect();
            order.sort_by_key(|i| (std::cmp::Reverse(touched[*i]), tabs[*i].id));
            let room = rules.fits.with_pill - shown;
            changes.extend(order.into_iter().take(room).map(|i| Change::Refill(tabs[i].id)));
        }
    }
    changes
}

pub(crate) fn next_look_ms(tabs: &[Facts], now_ms: u64, rules: &Rules) -> Option<u64> {
    tabs.iter()
        .filter(|tab| tab.park.is_none() && tab.parkable())
        .filter_map(|tab| {
            let quiet = tab.quiet_ms(now_ms);
            [
                rules.after_ms,
                (tab.work == Work::Long).then_some(WORK_QUIET_MS),
                matches!(tab.schedule, Schedule::Waiting(_)).then_some(WORK_QUIET_MS),
            ]
            .into_iter()
            .flatten()
            .map(|limit| limit.saturating_sub(quiet))
            .filter(|left| *left > 0)
            .min()
        })
        .min()
        .map(|left| left + 500)
}

#[derive(Debug, Default)]
pub(crate) struct Touches {
    last: HashMap<u64, u64>,
    active: Option<u64>,
}

impl Touches {
    pub(crate) fn observe(&mut self, active: u64, now_ms: u64) {
        if self.active != Some(active) {
            if let Some(before) = self.active {
                self.last.insert(before, now_ms);
            }
            self.active = Some(active);
        }
        self.last.insert(active, now_ms);
    }

    pub(crate) fn of(&mut self, id: u64, now_ms: u64) -> u64 {
        *self.last.entry(id).or_insert(now_ms)
    }

    pub(crate) fn set(&mut self, id: u64, now_ms: u64) {
        self.last.insert(id, now_ms);
    }

    pub(crate) fn forget(&mut self, id: u64) {
        self.last.remove(&id);
    }
}

pub(crate) fn work_of(board: Option<&JobBoard>, now_ms: i64) -> Work {
    let Some(board) = board else { return Work::None };
    let mut running = board.jobs.iter().filter(|job| job.state == JobState::Running).peekable();
    if running.peek().is_none() {
        return Work::None;
    }
    let long = running.any(|job| {
        job.eta_left(now_ms).is_some_and(|secs| secs >= LONG_JOB_SECS)
            || job.elapsed_ms(now_ms) >= i64::try_from(WORK_QUIET_MS).unwrap_or(i64::MAX)
    });
    if long { Work::Long } else { Work::Running }
}

pub(crate) fn schedule_of(ledger: &Ledger, today: Date) -> Schedule {
    let mut next: Option<Date> = None;
    for item in &ledger.scheduled {
        if !matches!(item.mark, Mark::Todo | Mark::Doing) {
            continue;
        }
        let (When::Date(date) | When::About(date)) = &item.when else { continue };
        if item.when.due_on(today) {
            return Schedule::Due;
        }
        next = Some(next.map_or(*date, |soonest| soonest.min(*date)));
    }
    next.map_or(Schedule::None, Schedule::Waiting)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Teal,
    Blue,
    Muted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) tone: Tone,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct JobLine {
    pub(crate) percent: Option<u32>,
    pub(crate) eta_secs: Option<u64>,
}

pub(crate) fn job_line(board: Option<&JobBoard>, now_ms: i64) -> JobLine {
    let lead = board.and_then(JobBoard::lead);
    JobLine {
        percent: lead.and_then(Job::percent).map(|percent| percent.round() as u32),
        eta_secs: lead.and_then(|job| job.eta_left(now_ms)),
    }
}

fn idle_text(ms: u64) -> String {
    match ms {
        0..HOUR_MS => format!("{}m", (ms / MINUTE_MS).max(1)),
        HOUR_MS..172_800_000 => format!("{}h", ms / HOUR_MS),
        _ => format!("{}d", ms / (24 * HOUR_MS)),
    }
}

pub(crate) fn date_text(date: Date) -> String {
    let month = MONTHS[usize::from(date.month().clamp(1, 12)) - 1];
    format!("{} {} {month}", date.weekday().short(), date.day())
}

pub(crate) fn reason_line(park: Park, now_ms: u64, job: JobLine, schedule: Schedule) -> Line {
    let (text, tone) = match park.why {
        Why::Job => {
            let mut parts = vec!["Running".to_owned()];
            if let Some(percent) = job.percent {
                parts[0] = format!("Running {percent}%");
            }
            if let Some(secs) = job.eta_secs {
                parts.push(format!("ETA {}", duration_text(secs)));
            }
            (parts.join(" \u{b7} "), Tone::Teal)
        }
        Why::Waiting => match schedule {
            Schedule::Waiting(date) => (format!("Waiting until {}", date_text(date)), Tone::Blue),
            _ => ("Waiting".to_owned(), Tone::Blue),
        },
        Why::Idle => {
            (format!("Idle {}", idle_text(now_ms.saturating_sub(park.since_ms))), Tone::Muted)
        }
        Why::Manual => ("Parked by you".to_owned(), Tone::Muted),
        Why::Overflow => ("No room in the strip".to_owned(), Tone::Muted),
    };
    Line { text, tone }
}

#[cfg(test)]
mod tests;
