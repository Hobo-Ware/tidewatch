//! New-tab dialog state and rules, without drawing (YOLO only in a worktree tab, D11).

use std::path::PathBuf;

use tidewatch_core::agents::{Agent, Level};
use tidewatch_core::profiles::Profile;
use tidewatch_core::workflows::Workflow;

use crate::catalog::{Availability, Catalog};
use crate::new_tab_workflow;
use crate::workflow_glue::WorkflowRequest;

const COPILOT_TRUST_HINT: &str = "Copilot asks to trust a new worktree on its first start. Trust ~/.tidewatch once to skip that.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Repo,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Agent,
    Workflow,
    Shell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Choose,
    Agent,
    Workflow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentRequest {
    pub(crate) profile_id: String,
    pub(crate) profile: Profile,
    pub(crate) agent: Agent,
    /// The first message, sent from the tab and never from the dialog: empty for a tab the dialog opens.
    pub(crate) task: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    Shell,
    Agent(Box<AgentRequest>),
    Workflow(Box<WorkflowRequest>),
}

/// Why `level` cannot be picked for `agent` in `group`, or `None` when it can.
pub(crate) fn level_refusal(group: Group, agent: &Agent, level: Level) -> Option<String> {
    if agent.level_args(level).is_none() {
        return Some(format!(
            "{} has no flags for {} in profiles.toml.",
            agent.name,
            level.label()
        ));
    }
    (level == Level::Yolo && group == Group::Other)
        .then(|| "YOLO only runs in an agent worktree tab, not in Other.".to_owned())
}

pub(crate) fn level_hint(level: Level) -> &'static str {
    match level {
        Level::Plan => "Reads and searches only. It cannot change files or run commands.",
        Level::Ask => "Asks before every edit and every command.",
        Level::Edits => "Edits files on its own, asks before running commands.",
        Level::Auto => "Edits files and runs everyday commands without asking.",
        Level::Yolo => "Skips every permission check. Only in an agent worktree.",
    }
}

fn opening_level(group: Group, agent: &Agent, wanted: Level) -> Level {
    [wanted, Level::Auto]
        .into_iter()
        .chain(Level::ALL)
        .find(|level| level_refusal(group, agent, *level).is_none())
        .unwrap_or(wanted)
}

fn matches_filter(filter: &str, texts: &[&str]) -> bool {
    let needle = filter.trim().to_lowercase();
    needle.is_empty() || texts.iter().any(|text| text.to_lowercase().contains(&needle))
}

pub(crate) struct NewTab {
    group: Group,
    label: String,
    repo: PathBuf,
    base: Option<String>,
    step: Step,
    highlight: Kind,
    from_chooser: bool,
    last_profile: Option<String>,
    filter: String,
    selected: usize,
    level: Level,
    running_here: Vec<String>,
    pub(crate) scroll_to_selected: bool,
    pub(crate) error: Option<String>,
}

impl NewTab {
    pub(crate) fn open(
        group: Group,
        label: String,
        repo: PathBuf,
        running_here: Vec<String>,
        last_profile: Option<&str>,
    ) -> Self {
        Self {
            group,
            label,
            repo,
            base: None,
            step: Step::Choose,
            highlight: Kind::Agent,
            from_chooser: true,
            last_profile: last_profile.map(str::to_owned),
            filter: String::new(),
            selected: 0,
            level: Level::Auto,
            running_here,
            scroll_to_selected: true,
            error: None,
        }
    }

    pub(crate) fn group(&self) -> Group {
        self.group
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn repo(&self) -> &std::path::Path {
        &self.repo
    }

    pub(crate) fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    pub(crate) fn set_base(&mut self, base: Option<String>) {
        self.base = base;
    }

    pub(crate) fn step(&self) -> Step {
        self.step
    }

    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    pub(crate) fn level(&self) -> Level {
        self.level
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn highlight(&self) -> Kind {
        self.highlight
    }

    /// Left and Right on the first screen: the next card that is on offer, staying put at the ends.
    pub(crate) fn move_highlight(&mut self, forward: bool) {
        let order = [Kind::Agent, Kind::Workflow, Kind::Shell];
        let at = order.iter().position(|kind| *kind == self.highlight).unwrap_or_default();
        let next: Vec<Kind> = if forward {
            order[at + 1..].to_vec()
        } else {
            order[..at].iter().rev().copied().collect()
        };
        if let Some(kind) = next.into_iter().find(|kind| self.offers(*kind)) {
            self.highlight = kind;
        }
    }

    /// Whether the first screen offers `kind` here: a workflow needs a worktree to run in.
    pub(crate) fn offers(&self, kind: Kind) -> bool {
        kind != Kind::Workflow || self.group == Group::Repo
    }

    pub(crate) fn jump(&mut self, preselect: &str, catalog: &Catalog) {
        self.from_chooser = false;
        match preselect.strip_prefix("workflow:") {
            Some(id) if self.offers(Kind::Workflow) => {
                self.enter(Step::Workflow, catalog);
                self.select_id(id, catalog);
            }
            Some(_) => {}
            None => {
                self.enter(Step::Agent, catalog);
                self.select_id(preselect, catalog);
            }
        }
    }

    pub(crate) fn choose(&mut self, kind: Kind, catalog: &Catalog) -> Option<Request> {
        if !self.offers(kind) {
            return None;
        }
        match kind {
            Kind::Shell => return Some(Request::Shell),
            Kind::Agent => self.enter(Step::Agent, catalog),
            Kind::Workflow => self.enter(Step::Workflow, catalog),
        }
        self.from_chooser = true;
        None
    }

    fn enter(&mut self, step: Step, catalog: &Catalog) {
        self.step = step;
        self.filter.clear();
        self.selected = 0;
        self.error = None;
        if step == Step::Agent
            && let Some(last) = self.last_profile.clone()
        {
            self.select_id(&last, catalog);
        }
        self.apply_selection(catalog);
    }

    /// One step back to the first screen. `false` when there is none to go back to.
    pub(crate) fn back(&mut self) -> bool {
        if self.step == Step::Choose || !self.from_chooser {
            return false;
        }
        self.step = Step::Choose;
        self.filter.clear();
        self.error = None;
        true
    }

    pub(crate) fn set_filter(&mut self, text: &str, catalog: &Catalog) {
        if self.filter != text {
            text.clone_into(&mut self.filter);
            self.selected = 0;
            self.apply_selection(catalog);
        }
    }

    pub(crate) fn agent_groups(&self, catalog: &Catalog) -> Vec<(String, Vec<String>)> {
        let entries = catalog.entries();
        catalog
            .agent_names()
            .into_iter()
            .filter_map(|(agent_id, _)| {
                let ids: Vec<String> = entries
                    .iter()
                    .filter(|(_, profile, agent)| {
                        agent.id == agent_id
                            && matches_filter(
                                &self.filter,
                                &[
                                    &profile.name,
                                    &agent.name,
                                    profile.model.as_deref().unwrap_or_default(),
                                    profile.level.label(),
                                ],
                            )
                    })
                    .map(|(id, _, _)| (*id).to_owned())
                    .collect();
                (!ids.is_empty()).then_some((agent_id, ids))
            })
            .collect()
    }

    pub(crate) fn workflow_ids(&self, catalog: &Catalog) -> Vec<String> {
        let Some(workflows) = catalog.workflows() else { return Vec::new() };
        new_tab_workflow::ordered(workflows)
            .into_iter()
            .filter(|workflow| {
                matches_filter(&self.filter, &[&workflow.name, &workflow.description])
            })
            .map(|workflow| workflow.id.clone())
            .collect()
    }

    pub(crate) fn visible(&self, catalog: &Catalog) -> Vec<String> {
        match self.step {
            Step::Choose => Vec::new(),
            Step::Agent => {
                self.agent_groups(catalog).into_iter().flat_map(|(_, ids)| ids).collect()
            }
            Step::Workflow => self.workflow_ids(catalog),
        }
    }

    pub(crate) fn selected_id(&self, catalog: &Catalog) -> Option<String> {
        self.visible(catalog).into_iter().nth(self.selected)
    }

    pub(crate) fn selected_entry<'a>(
        &self,
        catalog: &'a Catalog,
    ) -> Option<(&'a Profile, &'a Agent)> {
        if self.step != Step::Agent {
            return None;
        }
        let id = self.selected_id(catalog)?;
        catalog.entries().into_iter().find(|(x, _, _)| *x == id).map(|(_, p, a)| (p, a))
    }

    pub(crate) fn selected_workflow<'a>(&self, catalog: &'a Catalog) -> Option<&'a Workflow> {
        if self.step != Step::Workflow {
            return None;
        }
        let id = self.selected_id(catalog)?;
        catalog.workflows()?.workflow.get(&id)
    }

    pub(crate) fn select_id(&mut self, id: &str, catalog: &Catalog) {
        if let Some(index) = self.visible(catalog).iter().position(|x| x == id) {
            self.select(index, catalog);
        }
    }

    pub(crate) fn select(&mut self, index: usize, catalog: &Catalog) {
        if index < self.visible(catalog).len() && index != self.selected {
            self.selected = index;
            self.apply_selection(catalog);
        } else {
            self.scroll_to_selected = true;
        }
    }

    pub(crate) fn step_selection(&mut self, down: bool, catalog: &Catalog) {
        let count = self.visible(catalog).len();
        let next = if down {
            (self.selected + 1).min(count.saturating_sub(1))
        } else {
            self.selected.saturating_sub(1)
        };
        self.select(next, catalog);
    }

    fn apply_selection(&mut self, catalog: &Catalog) {
        self.error = None;
        self.scroll_to_selected = true;
        if let Some((profile, agent)) = self.selected_entry(catalog) {
            self.level = opening_level(self.group, agent, profile.level);
        }
    }

    /// Picks `level` when it is open for the selected agent. Returns whether it was taken.
    pub(crate) fn set_level(&mut self, level: Level, catalog: &Catalog) -> bool {
        let Some((_, agent)) = self.selected_entry(catalog) else { return false };
        if level_refusal(self.group, agent, level).is_some() {
            return false;
        }
        self.level = level;
        self.error = None;
        true
    }

    /// Left and Right: the next open level in that direction, staying put at the end.
    pub(crate) fn step_level(&mut self, forward: bool, catalog: &Catalog) -> bool {
        let Some((_, agent)) = self.selected_entry(catalog) else { return false };
        let at = Level::ALL.iter().position(|level| *level == self.level).unwrap_or_default();
        let candidates: Vec<Level> = if forward {
            Level::ALL[at + 1..].to_vec()
        } else {
            Level::ALL[..at].iter().rev().copied().collect()
        };
        let next =
            candidates.into_iter().find(|level| level_refusal(self.group, agent, *level).is_none());
        next.is_some_and(|level| self.set_level(level, catalog))
    }

    /// What Enter does: the request, or the reason it cannot start.
    pub(crate) fn request(&self, catalog: &Catalog) -> Result<Request, String> {
        match self.step {
            Step::Choose => Err("Pick Agent, Workflow or Plain shell first.".to_owned()),
            Step::Agent => self.agent_request(catalog),
            Step::Workflow => self.workflow_request(catalog),
        }
    }

    fn agent_request(&self, catalog: &Catalog) -> Result<Request, String> {
        let Some(id) = self.selected_id(catalog) else {
            return Err("No profile to start. Add one in Settings.".to_owned());
        };
        let (profile, agent) = self
            .selected_entry(catalog)
            .ok_or_else(|| format!("profile {id} is no longer available"))?;
        if let Some(reason) = level_refusal(self.group, agent, self.level) {
            return Err(reason);
        }
        Ok(Request::Agent(Box::new(AgentRequest {
            profile_id: id,
            profile: Profile { level: self.level, ..profile.clone() },
            agent: agent.clone(),
            task: String::new(),
        })))
    }

    fn workflow_request(&self, catalog: &Catalog) -> Result<Request, String> {
        let Some(id) = self.selected_id(catalog) else {
            return Err("No workflow to start. Add one in Settings.".to_owned());
        };
        let (Some(workflow), Some(profiles)) =
            (self.selected_workflow(catalog), catalog.profiles())
        else {
            return Err(format!("workflow {id} is no longer available"));
        };
        new_tab_workflow::build(workflow, profiles).map(|r| Request::Workflow(Box::new(r)))
    }

    pub(crate) fn warnings(&self, catalog: &Catalog) -> Vec<String> {
        if let Some(workflow) = self.selected_workflow(catalog)
            && let Some(profiles) = catalog.profiles()
        {
            return new_tab_workflow::missing_agents(workflow, profiles, |agent| {
                catalog.availability(agent) != Availability::Missing
            });
        }
        let Some((_, agent)) = self.selected_entry(catalog) else { return Vec::new() };
        let mut warnings = Vec::new();
        if catalog.availability(&agent.id) == Availability::Missing {
            warnings.push(format!(
                "{} was not found on PATH. Starting will still try your login shell.",
                agent.bin
            ));
        }
        if !agent.verified {
            warnings.push(format!("{} support is experimental.", agent.name));
        }
        if agent.id == "copilot" && self.group == Group::Repo {
            warnings.push(COPILOT_TRUST_HINT.to_owned());
        }
        if self.group == Group::Other && !self.running_here.is_empty() {
            warnings.push(same_folder_warning(&self.running_here));
        }
        warnings
    }
}

pub(crate) fn same_folder_warning(names: &[String]) -> String {
    match names {
        [one] => format!(
            "The {one} tab already has an agent in this folder. They can overwrite each other's files."
        ),
        many => format!(
            "The {} tabs already have an agent in this folder. They can overwrite each other's files.",
            many.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests;
