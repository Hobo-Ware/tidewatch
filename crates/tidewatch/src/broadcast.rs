//! Message all agents: which tabs the sheet lists, which start ticked, what the owner changed, the
//! draft that outlives Esc and the words of the Send button. Pure.

use std::collections::HashSet;

use tidewatch_core::panel_view::Tone;
use tidewatch_core::status::AgentStatus;

use crate::review::plural;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Agent,
    Meeseeks,
    Other,
}

/// What the app knows of one tab of the repo on show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) kind: Kind,
    pub(crate) status: AgentStatus,
    pub(crate) parked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Live {
    Parked,
    Starting,
    Working,
    Idle,
    NeedsYou,
    Failed,
    Stopped,
}

impl Live {
    fn of(candidate: &Candidate) -> Self {
        if candidate.parked {
            return Self::Parked;
        }
        match Tone::of(&candidate.status) {
            Tone::Setup => Self::Starting,
            Tone::Working => Self::Working,
            Tone::Needs => Self::NeedsYou,
            Tone::Failed => Self::Failed,
            Tone::Stopped | Tone::Shell => Self::Stopped,
            Tone::Done | Tone::Waiting | Tone::Blocked => Self::Idle,
        }
    }

    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Parked => "parked",
            Self::Starting => "starting",
            Self::Working => "working",
            Self::Idle => "idle",
            Self::NeedsYou => "needs you",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) live: Live,
    pub(crate) ticked: bool,
}

/// The sheet while it is open: the draft, and the tabs whose tick the owner flipped from its
/// default (every listed tab ticked, a parked one not).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Sheet {
    pub(crate) draft: String,
    pub(crate) focus: bool,
    flipped: HashSet<u64>,
}

impl Sheet {
    pub(crate) fn open(draft: String) -> Self {
        Self { draft, focus: true, flipped: HashSet::new() }
    }

    pub(crate) fn rows(&self, candidates: &[Candidate]) -> Vec<Row> {
        candidates
            .iter()
            .filter(|candidate| candidate.kind == Kind::Agent)
            .map(|candidate| {
                let live = Live::of(candidate);
                let by_default = live != Live::Parked;
                Row {
                    id: candidate.id,
                    name: candidate.name.clone(),
                    live,
                    ticked: by_default != self.flipped.contains(&candidate.id),
                }
            })
            .collect()
    }

    pub(crate) fn toggle(&mut self, id: u64) {
        if !self.flipped.remove(&id) {
            self.flipped.insert(id);
        }
    }

    pub(crate) fn can_send(&self, rows: &[Row]) -> bool {
        !self.draft.trim().is_empty() && rows.iter().any(|row| row.ticked)
    }
}

pub(crate) fn ticked(rows: &[Row]) -> Vec<u64> {
    rows.iter().filter(|row| row.ticked).map(|row| row.id).collect()
}

pub(crate) fn send_label(rows: &[Row]) -> String {
    format!("Send to {}", plural(rows.iter().filter(|row| row.ticked).count(), "agent"))
}

/// What the toast says once the copies are in: `Sent to 3 agents`, or `Sent to 2 of 3 agents` when
/// a tab's line was full and its copy went back to that tab's box.
pub(crate) fn sent_toast(went: usize, asked: usize) -> String {
    if went == asked {
        format!("Sent to {}", plural(went, "agent"))
    } else {
        format!("Sent to {went} of {}", plural(asked, "agent"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: u64, kind: Kind, status: AgentStatus, parked: bool) -> Candidate {
        Candidate { id, name: format!("tab-{id}"), kind, status, parked }
    }

    fn mixed() -> Vec<Candidate> {
        vec![
            tab(1, Kind::Agent, AgentStatus::Working, false),
            tab(2, Kind::Agent, AgentStatus::Done, true),
            tab(3, Kind::Other, AgentStatus::Shell, false),
            tab(4, Kind::Meeseeks, AgentStatus::Working, false),
            tab(
                5,
                Kind::Agent,
                AgentStatus::NeedsYou(tidewatch_core::status::Reason::Input),
                false,
            ),
        ]
    }

    #[test]
    fn only_agent_tabs_are_listed_and_only_a_parked_one_starts_unticked() {
        let rows = Sheet::open(String::new()).rows(&mixed());
        let seen: Vec<_> = rows.iter().map(|row| (row.id, row.live, row.ticked)).collect();
        assert_eq!(
            seen,
            [(1, Live::Working, true), (2, Live::Parked, false), (5, Live::NeedsYou, true)]
        );
    }

    #[test]
    fn each_state_has_its_word() {
        let cases = [
            (AgentStatus::Working, false, "working"),
            (AgentStatus::Done, false, "idle"),
            (AgentStatus::NeedsYou(tidewatch_core::status::Reason::Quiet), false, "idle"),
            (AgentStatus::NeedsYou(tidewatch_core::status::Reason::Permission), false, "needs you"),
            (AgentStatus::SettingUp, false, "starting"),
            (AgentStatus::Failed("x".into()), false, "failed"),
            (AgentStatus::Stopped, false, "stopped"),
            (AgentStatus::Working, true, "parked"),
        ];
        for (status, parked, word) in cases {
            let rows =
                Sheet::open(String::new()).rows(&[tab(1, Kind::Agent, status.clone(), parked)]);
            assert_eq!(rows[0].live.word(), word, "{status:?} parked {parked}");
        }
    }

    #[test]
    fn a_flipped_tick_inverts_its_default_and_flips_back() {
        let mut sheet = Sheet::open(String::new());
        sheet.toggle(1);
        sheet.toggle(2);
        let ticks: Vec<_> = sheet.rows(&mixed()).iter().map(|row| row.ticked).collect();
        assert_eq!(ticks, [false, true, true]);
        sheet.toggle(1);
        let ticks: Vec<_> = sheet.rows(&mixed()).iter().map(|row| row.ticked).collect();
        assert_eq!(ticks, [true, true, true]);
    }

    #[test]
    fn send_needs_text_and_a_ticked_agent() {
        let mut sheet = Sheet::open("  ".into());
        let rows = sheet.rows(&mixed());
        assert!(!sheet.can_send(&rows), "blank text");
        sheet.draft = "rebase on main".into();
        assert!(sheet.can_send(&rows));
        let none: Vec<Row> = rows.iter().cloned().map(|row| Row { ticked: false, ..row }).collect();
        assert!(!sheet.can_send(&none), "nobody ticked");
        assert_eq!(ticked(&none), Vec::<u64>::new());
    }

    #[test]
    fn the_button_and_the_toast_count_the_ticked_agents() {
        let rows = Sheet::open(String::new()).rows(&mixed());
        assert_eq!(send_label(&rows), "Send to 2 agents");
        assert_eq!(ticked(&rows), [1, 5]);
        assert_eq!(send_label(&rows[..1]), "Send to 1 agent");
        assert_eq!(send_label(&rows[1..2]), "Send to 0 agents");
        assert_eq!(sent_toast(3, 3), "Sent to 3 agents");
        assert_eq!(sent_toast(1, 1), "Sent to 1 agent");
        assert_eq!(sent_toast(2, 3), "Sent to 2 of 3 agents");
    }
}
