//! The "Change this tab" popover, the pure half: what the app knows of the tab (`Facts`), what the
//! picker has chosen (`Pick`), what is missing before a change can start (`Fix`), the one sentence
//! that says what will happen, and the `Target` a press of Change hands to `retarget`. Every tab
//! can change and every choice can be picked; `change_tab_view` draws it.

use tidewatch_core::agents::Level;

use crate::new_tab_meeseeks::is_role;
use crate::retarget::{Target, fit_level};

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
    pub(crate) installed: bool,
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

#[allow(clippy::struct_excessive_bools)] // independent facts of one tab, not a mode
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
    /// The folder the found session runs in when it is not the tab's own: the tab moves there.
    pub(crate) follows: Option<String>,
    /// A command other than the shell has the terminal: it stops with the shell.
    pub(crate) busy: bool,
    pub(crate) meeseeks: bool,
    /// The pinned main tab.
    pub(crate) pinned: bool,
    /// What the tab shows now that a change replaces: a form, a failure, a summary.
    pub(crate) replaces: Option<&'static str>,
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

fn level_for(facts: &Facts, card: &Card) -> Level {
    let seen = facts.seen_level.or_else(|| {
        facts.current.as_ref().filter(|(id, _)| *id == card.id).map(|(_, level)| *level)
    });
    fit_level(seen.unwrap_or(card.level), &card.unsupported)
}

impl Pick {
    /// The tab as it is: an agent tab's own profile and level, a shell with a found session on the
    /// Builder profile of its CLI at the level it runs at, a plain shell on the first profile.
    pub(crate) fn start(facts: &Facts, builder: &dyn Fn(&str) -> Option<String>) -> Self {
        let own = facts.current.as_ref().and_then(|(id, _)| card_of(facts, id));
        let by_cli = session_cli(facts).and_then(builder).and_then(|id| card_of(facts, &id));
        let card = own
            .or(by_cli)
            .or_else(|| facts.cards.iter().find(|card| card.installed))
            .or_else(|| facts.cards.first());
        let Some(card) = card else {
            return Self { kind: Kind::Agent, profile: String::new(), level: Level::Edits };
        };
        let kind = match (&facts.running, &facts.current) {
            (Running::No, None) if facts.stored.is_none() => Kind::Shell,
            _ => Kind::Agent,
        };
        Self { kind, profile: card.id.clone(), level: level_for(facts, card) }
    }

    pub(crate) fn pick_profile(&mut self, facts: &Facts, id: &str) {
        if card_of(facts, id).is_some() {
            id.clone_into(&mut self.profile);
        }
    }

    pub(crate) fn pick_level(&mut self, level: Level) {
        self.level = level;
    }

    pub(crate) fn pick_kind(&mut self, kind: Kind) {
        self.kind = kind;
    }
}

pub(crate) fn is_working(facts: &Facts) -> bool {
    matches!(facts.running, Running::Agent { working: true, .. })
}

fn who(card: &Card, level: Level) -> String {
    format!("{} \u{b7} {}", card.name, level.label())
}

/// What has to be set up before Change can start, shown in the popover as a button that opens it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fix {
    Install(String),
    AddProfile,
}

impl Fix {
    pub(crate) fn button(&self) -> String {
        match self {
            Self::Install(cli) => format!("Set up {cli}"),
            Self::AddProfile => "Add a profile".to_owned(),
        }
    }

    pub(crate) fn sentence(&self) -> String {
        match self {
            Self::Install(cli) => {
                format!("{cli} is not installed here. Set it up and this tab switches over.")
            }
            Self::AddProfile => "There is no profile to switch to. Add one in Settings.".to_owned(),
        }
    }
}

pub(crate) fn fix(facts: &Facts, pick: &Pick) -> Option<Fix> {
    if pick.kind != Kind::Agent {
        return None;
    }
    match card_of(facts, &pick.profile) {
        None => Some(Fix::AddProfile),
        Some(card) if !card.installed => Some(Fix::Install(card.cli_name.clone())),
        Some(_) => None,
    }
}

fn notes(facts: &Facts, pick: &Pick, card: Option<&Card>) -> String {
    let mut notes = Vec::new();
    match &facts.follows {
        Some(folder) => {
            notes.push(format!("The tab moves to {folder}, where the conversation runs."));
        }
        None if facts.in_worktree => {
            notes.push("Worktree, branch and history stay as they are.".to_owned());
        }
        None => notes.push("The folder and history stay as they are.".to_owned()),
    }
    if let Some(card) = card.filter(|_| pick.kind == Kind::Agent) {
        let fitted = fit_level(pick.level, &card.unsupported);
        if fitted != pick.level {
            notes.push(format!(
                "{} has no flags for {}, so it runs as {}.",
                card.cli_name,
                pick.level.label(),
                fitted.label()
            ));
        }
    }
    if facts.busy {
        notes.push("The command running in this shell stops.".to_owned());
    }
    if facts.meeseeks {
        notes.push("It stops being a Meeseeks and no longer ends by itself.".to_owned());
    }
    if facts.pinned {
        notes.push("The main tab stays pinned.".to_owned());
    }
    notes.extend(facts.replaces.map(str::to_owned));
    notes.join(" ")
}

/// The sentence that says what Change will do.
pub(crate) fn sentence(facts: &Facts, pick: &Pick) -> String {
    let running = match &facts.running {
        Running::Agent { cli_name, working, .. } => Some((cli_name.as_str(), *working)),
        Running::No => None,
    };
    match (pick.kind, running) {
        (Kind::Shell, Some((cli, _))) => format!(
            "{cli} stops and this tab keeps its shell. The conversation is remembered, so you can switch back from here. {}",
            notes(facts, pick, None)
        ),
        (Kind::Shell, None) => {
            format!("The shell restarts in the same folder. {}", notes(facts, pick, None))
        }
        (Kind::Agent, running) => {
            if let Some(fix) = fix(facts, pick) {
                return fix.sentence();
            }
            let Some(card) = card_of(facts, &pick.profile) else {
                return Fix::AddProfile.sentence();
            };
            let who = who(card, pick.level);
            let other_cli = session_cli(facts).is_some_and(|cli| cli != card.agent_id);
            let head = match (running, &facts.stored) {
                (Some((cli, working)), _) if other_cli => format!(
                    "{cli} {} {} cannot continue its conversation, so it starts a new one as {who}. The old one stays on disk.",
                    if working { "finishes this turn, then stops." } else { "stops." },
                    card.cli_name
                ),
                (Some((cli, true)), _) => {
                    format!(
                        "{cli} finishes this turn, then the same conversation resumes as {who}."
                    )
                }
                (Some((cli, false)), _) => {
                    format!("{cli} stops, then the same conversation resumes as {who}.")
                }
                (None, Some((stored, cli))) if *stored != card.agent_id => format!(
                    "{} cannot continue the conversation {cli} ended in this tab, so it starts a new one as {who}. The old one stays on disk.",
                    card.cli_name
                ),
                (None, Some((_, cli))) => {
                    format!("The conversation {cli} ended in this tab resumes as {who}.")
                }
                (None, None) => {
                    format!("A new {} session starts in this tab as {who}.", card.cli_name)
                }
            };
            format!("{head} {}", notes(facts, pick, Some(card)))
        }
    }
}

/// The button that cuts a turn short, offered only while the agent works and Change can go ahead.
pub(crate) fn offers_stop_now(facts: &Facts, pick: &Pick) -> bool {
    is_working(facts) && fix(facts, pick).is_none()
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
    fix(facts, pick).is_none().then(|| Change {
        tab: facts.tab,
        target: target_of(pick),
        interrupt,
    })
}

#[cfg(test)]
mod tests;
