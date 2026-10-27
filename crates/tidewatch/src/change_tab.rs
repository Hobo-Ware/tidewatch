//! The "Change this tab" popover, the pure half: what the app knows of the tab (`Facts`), what the
//! picker has chosen (`Pick`), which choices are refused and why, the one sentence that says what
//! will happen, and the `Target` a press of Change hands to `retarget`. `change_tab_view` draws it.

use tidewatch_core::agents::Level;
use tidewatch_core::restore::resumed_level_below;

use crate::new_tab_meeseeks::is_role;
use crate::pane::{Body, Pane};
use crate::retarget::Target;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Agent,
    Shell,
}

impl Kind {
    pub(crate) const ALL: [Kind; 2] = [Kind::Agent, Kind::Shell];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Agent => "Agent",
            Kind::Shell => "Shell",
        }
    }
}

/// A profile the picker offers as a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    pub(crate) id: String,
    pub(crate) name: String,
    /// `model · effort`, or what the CLI picks itself.
    pub(crate) detail: String,
    pub(crate) agent_id: String,
    pub(crate) cli_name: String,
    /// Levels the CLI has no flags for.
    pub(crate) unsupported: Vec<Level>,
    pub(crate) below_yolo: Option<Level>,
    pub(crate) level: Level,
}

/// What runs in the tab now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Running {
    /// Nothing: a plain shell, or an agent that exited.
    No,
    Agent {
        agent_id: String,
        cli_name: String,
        working: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) tab: u64,
    pub(crate) in_worktree: bool,
    pub(crate) running: Running,
    /// The session of the tab that can be resumed with nothing running: its CLI id and name.
    pub(crate) stored: Option<(String, String)>,
    /// The profile and level the tab runs now, for an agent tab.
    pub(crate) current: Option<(String, Level)>,
    /// The level a found session last ran at.
    pub(crate) seen_level: Option<Level>,
    pub(crate) cards: Vec<Card>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pick {
    pub(crate) kind: Kind,
    pub(crate) profile: String,
    pub(crate) level: Level,
}

/// The CLI a session in the tab belongs to, when there is one: only its profiles can resume it.
fn session_cli(facts: &Facts) -> Option<&str> {
    match &facts.running {
        Running::Agent { agent_id, .. } => Some(agent_id),
        Running::No => facts.stored.as_ref().map(|(id, _)| id.as_str()),
    }
}

fn role_rank(card: &Card) -> u8 {
    if is_role("builder", &card.id, &card.name) {
        0
    } else if is_role("reviewer", &card.id, &card.name) {
        1
    } else {
        2
    }
}

/// The profiles in the order the grid shows them: the tab's own CLI first, then the others in the
/// order they appear in Settings; with no CLI to start from, Claude first. Inside a CLI, Builder,
/// Reviewer, then the rest in Settings order.
pub(crate) fn ordered_cards(cards: &[Card], own_cli: Option<&str>) -> Vec<Card> {
    let mut clis: Vec<&str> = Vec::new();
    for card in cards {
        if !clis.contains(&card.agent_id.as_str()) {
            clis.push(&card.agent_id);
        }
    }
    let first = own_cli.unwrap_or("claude");
    let cli_rank = |card: &Card| {
        if card.agent_id == first {
            0
        } else {
            1 + clis.iter().position(|cli| *cli == card.agent_id).unwrap_or_default()
        }
    };
    let mut keyed: Vec<(usize, u8, usize, Card)> = cards
        .iter()
        .enumerate()
        .map(|(at, card)| (cli_rank(card), role_rank(card), at, card.clone()))
        .collect();
    keyed.sort_by_key(|(cli, role, at, _)| (*cli, *role, *at));
    keyed.into_iter().map(|(_, _, _, card)| card).collect()
}

fn card_of<'a>(facts: &'a Facts, id: &str) -> Option<&'a Card> {
    facts.cards.iter().find(|card| card.id == id)
}

/// Why a profile card cannot be picked, or `None`.
pub(crate) fn card_refusal(facts: &Facts, card: &Card) -> Option<String> {
    let cli = session_cli(facts)?;
    (cli != card.agent_id)
        .then(|| format!("This conversation runs in another CLI than {}.", card.cli_name))
}

/// Why `level` cannot be picked with `card`, or `None`.
pub(crate) fn level_refusal(facts: &Facts, card: &Card, level: Level) -> Option<String> {
    if card.unsupported.contains(&level) {
        return Some(format!("{} has no flags for {}.", card.cli_name, level.label()));
    }
    (level == Level::Yolo && !facts.in_worktree)
        .then(|| "YOLO only runs in a worktree tab.".to_owned())
}

fn level_for(facts: &Facts, card: &Card) -> Level {
    let seen = facts.seen_level.or_else(|| {
        facts.current.as_ref().filter(|(id, _)| *id == card.id).map(|(_, level)| *level)
    });
    let level = resumed_level_below(card.level, card.below_yolo, facts.in_worktree, seen);
    if level_refusal(facts, card, level).is_some() { card.level } else { level }
}

impl Pick {
    /// The tab as it is: an agent tab's own profile and level, a shell with a found session on the
    /// Builder profile of its CLI at the level it runs at, a plain shell on the first profile.
    pub(crate) fn start(facts: &Facts, builder: &dyn Fn(&str) -> Option<String>) -> Self {
        let own = facts.current.as_ref().and_then(|(id, _)| card_of(facts, id));
        let by_cli = session_cli(facts).and_then(builder).and_then(|id| card_of(facts, &id));
        let card = own
            .or(by_cli)
            .or_else(|| facts.cards.iter().find(|card| card_refusal(facts, card).is_none()));
        let Some(card) = card.or_else(|| facts.cards.first()) else {
            return Self { kind: Kind::Agent, profile: String::new(), level: Level::Edits };
        };
        let kind = match (&facts.running, &facts.current) {
            (Running::No, None) if facts.stored.is_none() => Kind::Shell,
            _ => Kind::Agent,
        };
        Self { kind, profile: card.id.clone(), level: level_for(facts, card) }
    }

    pub(crate) fn pick_profile(&mut self, facts: &Facts, id: &str) {
        let Some(card) = card_of(facts, id).filter(|card| card_refusal(facts, card).is_none())
        else {
            return;
        };
        self.profile.clone_from(&card.id);
        if level_refusal(facts, card, self.level).is_some() {
            self.level = level_for(facts, card);
        }
    }

    pub(crate) fn pick_level(&mut self, facts: &Facts, level: Level) {
        let allowed = card_of(facts, &self.profile)
            .is_none_or(|card| level_refusal(facts, card, level).is_none());
        if allowed {
            self.level = level;
        }
    }

    pub(crate) fn pick_kind(&mut self, facts: &Facts, kind: Kind) {
        if kind_refusal(facts, kind).is_none() {
            self.kind = kind;
        }
    }
}

pub(crate) fn kind_refusal(facts: &Facts, kind: Kind) -> Option<&'static str> {
    (kind == Kind::Shell && facts.running == Running::No).then_some("No agent runs in this tab.")
}

pub(crate) fn is_working(facts: &Facts) -> bool {
    matches!(facts.running, Running::Agent { working: true, .. })
}

fn who(card: &Card, level: Level) -> String {
    format!("{} \u{b7} {}", card.name, level.label())
}

/// Why Change cannot be pressed, or `None`.
pub(crate) fn blocked(facts: &Facts, pick: &Pick) -> Option<String> {
    match pick.kind {
        Kind::Shell => {
            (facts.running == Running::No).then(|| "This tab is already a shell.".to_owned())
        }
        Kind::Agent => {
            let card = card_of(facts, &pick.profile)?;
            if let Some(reason) = card_refusal(facts, card) {
                return Some(reason);
            }
            if let Some(reason) = level_refusal(facts, card, pick.level) {
                return Some(reason);
            }
            let same = matches!(facts.running, Running::Agent { .. })
                && facts.current.as_ref() == Some(&(card.id.clone(), pick.level));
            same.then(|| format!("Already running as {}.", who(card, pick.level)))
        }
    }
}

/// The sentence that says what Change will do.
pub(crate) fn sentence(facts: &Facts, pick: &Pick) -> String {
    let keeps = if facts.in_worktree {
        "Worktree, branch and history stay as they are."
    } else {
        "The folder and history stay as they are."
    };
    let running = match &facts.running {
        Running::Agent { cli_name, working, .. } => Some((cli_name.as_str(), *working)),
        Running::No => None,
    };
    match (pick.kind, running) {
        (Kind::Shell, Some((cli, _))) => format!(
            "{cli} stops and this tab keeps its shell. The conversation is remembered, so you can switch back from here."
        ),
        (Kind::Shell, None) => "This tab is already a shell.".to_owned(),
        (Kind::Agent, running) => {
            let Some(card) = card_of(facts, &pick.profile) else {
                return "No profile to switch to.".to_owned();
            };
            let who = who(card, pick.level);
            match (running, &facts.stored) {
                (Some((cli, true)), _) => format!(
                    "{cli} finishes this turn, then the same conversation resumes as {who}. {keeps}"
                ),
                (Some((cli, false)), _) => {
                    format!("{cli} stops, then the same conversation resumes as {who}. {keeps}")
                }
                (None, Some((_, cli))) => {
                    format!("The conversation {cli} ended in this tab resumes as {who}. {keeps}")
                }
                (None, None) => {
                    format!("A new {} session starts in this tab as {who}. {keeps}", card.cli_name)
                }
            }
        }
    }
}

/// The button that cuts a turn short, offered only while the agent works and Change can go ahead.
pub(crate) fn offers_stop_now(facts: &Facts, pick: &Pick) -> bool {
    is_working(facts) && blocked(facts, pick).is_none()
}

pub(crate) fn target_of(pick: &Pick) -> Target {
    match pick.kind {
        Kind::Shell => Target::Shell,
        Kind::Agent => Target::Agent { profile: pick.profile.clone(), level: Some(pick.level) },
    }
}

/// What a press asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) tab: u64,
    pub(crate) target: Target,
    /// Cut the turn short instead of waiting for it to end.
    pub(crate) interrupt: bool,
}

pub(crate) fn change(facts: &Facts, pick: &Pick, interrupt: bool) -> Option<Change> {
    blocked(facts, pick).is_none().then(|| Change {
        tab: facts.tab,
        target: target_of(pick),
        interrupt,
    })
}

/// Whether a tab can be changed at all, and if not why: the one rule the menu, the chip and the
/// palette share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabKind {
    Main,
    Shell,
    Agent,
    Meeseeks,
    Other,
}

pub(crate) fn tab_kind<T>(pane: &Pane<T>, pinned: bool) -> TabKind {
    match (&pane.body, &pane.agent) {
        _ if pinned => TabKind::Main,
        (Body::Shell(_), None) => TabKind::Shell,
        (Body::Shell(_), Some(agent)) if agent.meeseeks.is_some() => TabKind::Meeseeks,
        (Body::Shell(_), Some(agent)) if agent.launched.is_some() => TabKind::Agent,
        _ => TabKind::Other,
    }
}

pub(crate) fn availability(kind: TabKind) -> Result<(), &'static str> {
    match kind {
        TabKind::Shell | TabKind::Agent => Ok(()),
        TabKind::Main => Err("The main tab cannot change."),
        TabKind::Meeseeks => Err("A Meeseeks tab cannot change."),
        TabKind::Other => Err("This tab is not ready to change."),
    }
}

#[cfg(test)]
mod tests;
