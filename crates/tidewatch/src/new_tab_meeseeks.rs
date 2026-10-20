//! The Meeseeks side of the new-tab form, pure: the ticks of "Done means", the worktree name of
//! each task line and the Summon label.

use tidewatch_core::branch_name;
use tidewatch_core::meeseeks::Criteria;
use tidewatch_core::names;

use crate::repo_base::RepoFacts;

pub(crate) const NO_TESTS: &str = "needs a test command, none found";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tick {
    Tests,
    Committed,
    Pr,
}

impl Tick {
    pub(crate) const ALL: [Self; 3] = [Self::Tests, Self::Committed, Self::Pr];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Tests => "Tests pass",
            Self::Committed => "Work committed",
            Self::Pr => "Open a PR",
        }
    }
}

/// What the repo says about running tests, once it has been looked at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TestsOffer {
    Command(String),
    Missing,
    Unknown,
}

impl TestsOffer {
    pub(crate) fn of(facts: Option<&RepoFacts>) -> Self {
        match facts {
            None => Self::Unknown,
            Some(facts) => facts.test_command.clone().map_or(Self::Missing, Self::Command),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) tick: Tick,
    pub(crate) on: bool,
    /// Why the row cannot be ticked, when it cannot.
    pub(crate) disabled: Option<&'static str>,
}

pub(crate) fn rows(ticks: Criteria, offer: &TestsOffer) -> [Row; 3] {
    Tick::ALL.map(|tick| match tick {
        Tick::Tests if *offer == TestsOffer::Missing => {
            Row { tick, on: false, disabled: Some(NO_TESTS) }
        }
        Tick::Tests => Row { tick, on: ticks.tests, disabled: None },
        Tick::Committed => Row { tick, on: ticks.committed, disabled: None },
        Tick::Pr => Row { tick, on: ticks.pr, disabled: None },
    })
}

/// The ticks after one click. A disabled row stays as it was.
pub(crate) fn toggled(ticks: Criteria, tick: Tick, offer: &TestsOffer) -> Criteria {
    match tick {
        Tick::Tests if *offer == TestsOffer::Missing => ticks,
        Tick::Tests => Criteria { tests: !ticks.tests, ..ticks },
        Tick::Committed => Criteria { committed: !ticks.committed, ..ticks },
        Tick::Pr => Criteria { pr: !ticks.pr, ..ticks },
    }
}

/// What the new Meeseeks are held to: the ticks, minus tests where the repo has no command.
pub(crate) fn criteria(ticks: Criteria, offer: &TestsOffer) -> Criteria {
    Criteria { tests: ticks.tests && *offer != TestsOffer::Missing, ..ticks }
}

/// One worktree name per task line: the slug of the line, `-2`, `-3` when something has it
/// already, which counts the names this batch has handed out. A line with nothing to slug gets an
/// adjective-noun.
pub(crate) fn names_for(lines: &[String], seed: u64, taken: impl Fn(&str) -> bool) -> Vec<String> {
    let mut chosen: Vec<String> = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let used = |name: &str| taken(name) || chosen.iter().any(|done| done == name);
        let slug = branch_name::slug(line);
        let name = if slug.is_empty() {
            names::pick(seed.wrapping_add(index as u64), used)
        } else if used(&slug) {
            branch_name::next_free(&slug, used)
        } else {
            slug
        };
        chosen.push(name);
    }
    chosen
}

pub(crate) fn summon_label(tasks: usize) -> String {
    if tasks == 0 { "Summon".to_owned() } else { format!("Summon {tasks}") }
}

#[cfg(test)]
mod tests;
