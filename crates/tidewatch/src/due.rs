//! Items that are due: which ones a tab's Reader offers to start, which ones the desktop is told
//! about (once per item per day), the snooze, and the message that asks an agent to begin (pure).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tidewatch_core::civil_date::Date;
use tidewatch_core::progress::{Ledger, Mark};
use tidewatch_core::status::AgentStatus;

use crate::coming_up::{Group, Row};
use crate::note_send::{ReviewGate, review_gate};
use crate::notify::{Notification, for_due};

const STEP_CHARS: usize = 140;

/// One scheduled item of a tab: where it is and what it says.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub(crate) repo: PathBuf,
    pub(crate) tab: String,
    pub(crate) what: String,
}

impl Key {
    pub(crate) fn new(repo: &Path, tab: &str, what: &str) -> Self {
        Self { repo: repo.to_path_buf(), tab: tab.to_owned(), what: what.to_owned() }
    }
}

/// What was snoozed and on which day: it is hidden that day and back on the next. Kept in memory
/// only: the schedule index has no field for it.
#[derive(Debug, Default)]
pub(crate) struct Snoozes {
    on: HashMap<Key, Date>,
}

impl Snoozes {
    pub(crate) fn snooze(&mut self, key: Key, today: Date) {
        self.on.insert(key, today);
    }

    pub(crate) fn is_snoozed(&self, key: &Key, today: Date) -> bool {
        self.on.get(key) == Some(&today)
    }
}

/// A due item as the Reader's card shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Due {
    pub(crate) what: String,
    /// Negative when overdue.
    pub(crate) days: i64,
    pub(crate) steps: Vec<(Mark, String)>,
}

impl Due {
    pub(crate) fn pill(&self) -> String {
        if self.days < 0 { format!("{} d late", -self.days) } else { "Due today".to_owned() }
    }
}

/// The open items of `ledger` whose date is today or past and that are not snoozed.
pub(crate) fn due_items(
    today: Date,
    ledger: &Ledger,
    (repo, tab): (&Path, &str),
    snoozes: &Snoozes,
) -> Vec<Due> {
    ledger
        .scheduled
        .iter()
        .filter(|item| matches!(item.mark, Mark::Todo | Mark::Doing | Mark::Blocked))
        .filter_map(|item| {
            let days = item.when.days_until(today).filter(|days| *days <= 0)?;
            let key = Key::new(repo, tab, &item.what);
            (!snoozes.is_snoozed(&key, today)).then(|| Due {
                what: item.what.clone(),
                days,
                steps: item.steps.iter().map(|step| (step.mark, step.text.clone())).collect(),
            })
        })
        .collect()
}

fn one_line(text: &str) -> String {
    let line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= STEP_CHARS {
        line
    } else {
        format!("{}...", line.chars().take(STEP_CHARS - 3).collect::<String>())
    }
}

/// The message pasted into the agent when the user asks it to start.
pub(crate) fn ask_text(due: &Due) -> String {
    let open: Vec<String> = due
        .steps
        .iter()
        .filter(|(mark, _)| !matches!(mark, Mark::Done | Mark::Skipped))
        .map(|(_, text)| one_line(text))
        .collect();
    let steps =
        if open.is_empty() { String::new() } else { format!(" Steps: {}.", open.join("; ")) };
    format!(
        "Due today: {}.{steps} Start with the first open step and update the ledger.",
        one_line(due.what.trim().trim_end_matches('.'))
    )
}

/// Why Ask is off, or `None` when the agent is idle and may be given the message.
pub(crate) fn ask_blocked(status: Option<&AgentStatus>) -> Option<&'static str> {
    match review_gate(status) {
        ReviewGate::Now => None,
        ReviewGate::WhenIdle => Some("The agent is busy. Ask when it is idle"),
        ReviewGate::Refused(reason) => Some(reason),
    }
}

/// Which items were already told to the desktop on which day.
#[derive(Debug, Default)]
pub(crate) struct Told {
    sent: HashSet<(Key, Date)>,
}

impl Told {
    pub(crate) fn has(&self, key: &Key, today: Date) -> bool {
        self.sent.contains(&(key.clone(), today))
    }

    pub(crate) fn mark(&mut self, key: Key, today: Date) {
        self.sent.retain(|(_, day)| *day == today);
        self.sent.insert((key, today));
    }
}

/// The due items across every tab, open or closed, that the desktop has not been told about today
/// and the user has not snoozed.
fn to_tell<'a>(
    today: Date,
    rows: &'a [Row],
    snoozes: &Snoozes,
    told: &Told,
) -> Vec<(Key, &'a Row)> {
    rows.iter()
        .filter(|row| row.group == Group::Due)
        .map(|row| (Key::new(&row.repo, &row.tab, &row.what), row))
        .filter(|(key, _)| !snoozes.is_snoozed(key, today) && !told.has(key, today))
        .collect()
}

/// Offers the desktop each due item that was not told today. `offer` says whether it went (it does
/// not while the window is in front), and only an item that went is marked, so it still goes when
/// the window is away later the same day. Returns how many went.
pub(crate) fn tell_all(
    today: Date,
    rows: &[Row],
    snoozes: &Snoozes,
    told: &mut Told,
    mut offer: impl FnMut(Notification) -> bool,
) -> usize {
    let mut sent = 0;
    for (key, row) in to_tell(today, rows, snoozes, told) {
        if offer(for_due(&row.what, &row.tab)) {
            told.mark(key, today);
            sent += 1;
        }
    }
    sent
}

/// Seconds from `unix_secs` to the next midnight UTC, the moment a date turns due.
pub(crate) fn secs_to_midnight(unix_secs: i64) -> u64 {
    (86_400 - unix_secs.rem_euclid(86_400)) as u64
}

#[cfg(test)]
mod tests;
