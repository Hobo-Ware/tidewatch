//! The new-tab form's state and rules, without drawing: the kind, the picks, the level (YOLO only
//! in a worktree tab, D11), the worktree name and start branch, and the request Start turns into.

use std::path::PathBuf;

use tidewatch_core::agents::{Agent, Level};
use tidewatch_core::branch_name::{self, Problem};
use tidewatch_core::names;
use tidewatch_core::profiles::Profile;
use tidewatch_core::workflows::Workflow;

use crate::catalog::{Availability, Catalog};
use crate::new_tab_workflow;
use crate::repo_base::RepoFacts;
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

impl Kind {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Agent => "New agent",
            Self::Workflow => "New workflow",
            Self::Shell => "New shell",
        }
    }
}

/// Where a new tab's worktree is cut. The default is the plain way: a name from `names`, off the
/// current commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Placement {
    pub(crate) name: Option<String>,
    /// The branch the worktree starts from; `None` starts from HEAD.
    pub(crate) base: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentRequest {
    pub(crate) profile_id: String,
    pub(crate) profile: Profile,
    pub(crate) agent: Agent,
    /// Given to the agent at launch as its first prompt (`prepare::task_args`); empty starts it idle.
    pub(crate) task: String,
    pub(crate) placement: Placement,
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

/// Whether the worktree name still follows the message, or the user has typed one.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Name {
    Follows,
    Typed(String),
}

/// The branch the worktree starts from: what the repo has checked out, or the user's pick.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Base {
    Default,
    Picked(String),
}

/// What is wrong with the worktree name, if anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NameCheck {
    Fine,
    Invalid(Problem),
    Taken { suggestion: String },
}

impl NameCheck {
    pub(crate) fn is_fine(&self) -> bool {
        *self == Self::Fine
    }
}

fn is_taken(facts: Option<&RepoFacts>, name: &str) -> bool {
    facts.is_some_and(|facts| facts.taken.contains(name))
}

pub(crate) struct NewTab {
    group: Group,
    label: String,
    repo: PathBuf,
    kind: Kind,
    last_profile: Option<String>,
    filter: String,
    agent_pick: usize,
    workflow_pick: usize,
    level: Level,
    running_here: Vec<String>,
    message: String,
    task: String,
    name: Name,
    base: Base,
    seed: u64,
    settled: bool,
    /// The message box takes the keyboard once, when the form opens.
    pub(crate) focus_message: bool,
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
        seed: u64,
    ) -> Self {
        Self {
            group,
            label,
            repo,
            kind: Kind::Agent,
            last_profile: last_profile.map(str::to_owned),
            filter: String::new(),
            agent_pick: 0,
            workflow_pick: 0,
            level: Level::Auto,
            running_here,
            message: String::new(),
            task: String::new(),
            name: Name::Follows,
            base: Base::Default,
            seed,
            settled: false,
            focus_message: true,
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

    pub(crate) fn kind(&self) -> Kind {
        self.kind
    }

    pub(crate) fn title(&self) -> &'static str {
        self.kind.title()
    }

    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    pub(crate) fn level(&self) -> Level {
        self.level
    }

    /// The text the worktree name follows: the first message of an agent, the task of a workflow.
    pub(crate) fn text(&self) -> &str {
        match self.kind {
            Kind::Workflow => &self.task,
            Kind::Agent | Kind::Shell => &self.message,
        }
    }

    pub(crate) fn set_text(&mut self, text: &str) {
        let slot = match self.kind {
            Kind::Workflow => &mut self.task,
            Kind::Agent | Kind::Shell => &mut self.message,
        };
        text.clone_into(slot);
        self.error = None;
    }

    pub(crate) fn selected(&self) -> usize {
        match self.kind {
            Kind::Workflow => self.workflow_pick,
            Kind::Agent | Kind::Shell => self.agent_pick,
        }
    }

    /// Whether the form offers `kind` here: a workflow needs a worktree to run in.
    pub(crate) fn offers(&self, kind: Kind) -> bool {
        kind != Kind::Workflow || self.group == Group::Repo
    }

    /// Whether the form has a worktree to name and a branch to start from.
    pub(crate) fn makes_a_worktree(&self) -> bool {
        self.group == Group::Repo
    }

    /// Settles the form on the catalog the first time it is known: the last used profile, with its
    /// level.
    pub(crate) fn settle(&mut self, catalog: &Catalog) {
        if self.settled || !catalog.is_loaded() {
            return;
        }
        self.settled = true;
        if let Some(last) = self.last_profile.clone() {
            self.select_id(&last, catalog);
        }
        self.apply_selection(catalog);
    }

    pub(crate) fn jump(&mut self, preselect: &str, catalog: &Catalog) {
        match preselect.strip_prefix("workflow:") {
            Some(id) if self.offers(Kind::Workflow) => {
                self.enter(Kind::Workflow);
                self.select_id(id, catalog);
            }
            Some(_) => {}
            None => {
                self.enter(Kind::Agent);
                self.select_id(preselect, catalog);
            }
        }
    }

    /// Shell is a request at once; Agent and Workflow switch the form.
    pub(crate) fn choose(&mut self, kind: Kind) -> Option<Request> {
        if !self.offers(kind) {
            return None;
        }
        match kind {
            Kind::Shell => return Some(Request::Shell),
            Kind::Agent | Kind::Workflow => {
                if kind != self.kind {
                    self.enter(kind);
                }
            }
        }
        None
    }

    fn enter(&mut self, kind: Kind) {
        self.kind = kind;
        self.focus_message &= kind == Kind::Agent;
        self.error = None;
        self.scroll_to_selected = true;
    }

    pub(crate) fn set_filter(&mut self, text: &str, catalog: &Catalog) {
        if self.filter != text {
            text.clone_into(&mut self.filter);
            self.agent_pick = 0;
            self.apply_selection(catalog);
        }
    }

    /// The profiles that match the filter, grouped by CLI in the table's order.
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

    fn workflow_ids(catalog: &Catalog) -> Vec<String> {
        let Some(workflows) = catalog.workflows() else { return Vec::new() };
        new_tab_workflow::ordered(workflows)
            .into_iter()
            .map(|workflow| workflow.id.clone())
            .collect()
    }

    /// The ids of the cards the form shows, in order: what the number keys pick from.
    pub(crate) fn visible(&self, catalog: &Catalog) -> Vec<String> {
        match self.kind {
            Kind::Agent | Kind::Shell => {
                self.agent_groups(catalog).into_iter().flat_map(|(_, ids)| ids).collect()
            }
            Kind::Workflow => Self::workflow_ids(catalog),
        }
    }

    pub(crate) fn selected_id(&self, catalog: &Catalog) -> Option<String> {
        self.visible(catalog).into_iter().nth(self.selected())
    }

    pub(crate) fn selected_entry<'a>(
        &self,
        catalog: &'a Catalog,
    ) -> Option<(&'a Profile, &'a Agent)> {
        if self.kind != Kind::Agent {
            return None;
        }
        let id = self.selected_id(catalog)?;
        catalog.entries().into_iter().find(|(x, _, _)| *x == id).map(|(_, p, a)| (p, a))
    }

    pub(crate) fn selected_workflow<'a>(&self, catalog: &'a Catalog) -> Option<&'a Workflow> {
        if self.kind != Kind::Workflow {
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
        if index < self.visible(catalog).len() && index != self.selected() {
            match self.kind {
                Kind::Workflow => self.workflow_pick = index,
                Kind::Agent | Kind::Shell => self.agent_pick = index,
            }
            self.apply_selection(catalog);
        } else {
            self.scroll_to_selected = true;
        }
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

    /// The name shown in the field: what the user typed, else a slug of the text, else an
    /// adjective-noun nobody has.
    pub(crate) fn name(&self, facts: Option<&RepoFacts>) -> String {
        match &self.name {
            Name::Typed(name) => name.clone(),
            Name::Follows => {
                let slug = branch_name::slug(self.text());
                if slug.is_empty() {
                    names::pick(self.seed, |name| is_taken(facts, name))
                } else {
                    slug
                }
            }
        }
    }

    /// Whether the name is a slug of the text right now, which the field says.
    pub(crate) fn name_follows_text(&self) -> bool {
        self.name == Name::Follows && !branch_name::slug(self.text()).is_empty()
    }

    pub(crate) fn type_name(&mut self, name: &str) {
        self.name = Name::Typed(name.to_owned());
        self.error = None;
    }

    pub(crate) fn name_check(&self, facts: Option<&RepoFacts>) -> NameCheck {
        if !self.makes_a_worktree() {
            return NameCheck::Fine;
        }
        let name = self.name(facts);
        if let Err(problem) = branch_name::check(&name) {
            return NameCheck::Invalid(problem);
        }
        if is_taken(facts, &name) {
            let suggestion = branch_name::next_free(&name, |name| is_taken(facts, name));
            return NameCheck::Taken { suggestion };
        }
        NameCheck::Fine
    }

    /// The branch the worktree starts from, `None` for HEAD.
    pub(crate) fn base(&self, facts: Option<&RepoFacts>) -> Option<String> {
        match &self.base {
            Base::Picked(branch) => Some(branch.clone()),
            Base::Default => facts.and_then(|facts| facts.checked_out.clone()),
        }
    }

    pub(crate) fn pick_base(&mut self, branch: &str) {
        self.base = if branch == HEAD { Base::Default } else { Base::Picked(branch.to_owned()) };
    }

    /// Every branch the field offers: the checked-out one first, or HEAD when none is.
    pub(crate) fn base_options(facts: Option<&RepoFacts>) -> Vec<String> {
        let Some(facts) = facts else { return Vec::new() };
        let first = facts.checked_out.clone().unwrap_or_else(|| HEAD.to_owned());
        let rest = facts.branches.iter().filter(|branch| **branch != first).cloned();
        std::iter::once(first.clone()).chain(rest).collect()
    }

    /// What Start does: the request, or the reason it cannot start.
    pub(crate) fn start_request(
        &self,
        catalog: &Catalog,
        facts: Option<&RepoFacts>,
    ) -> Result<Request, String> {
        if self.kind == Kind::Workflow && self.task.trim().is_empty() {
            return Err("Say what the run should deliver.".to_owned());
        }
        match self.name_check(facts) {
            NameCheck::Fine => {}
            NameCheck::Invalid(problem) => return Err(problem.message()),
            NameCheck::Taken { suggestion } => {
                return Err(format!("{} is taken. Try {suggestion}.", self.name(facts)));
            }
        }
        self.request(catalog, facts)
    }

    /// The request for the picked profile or workflow, whatever the fields say about the name and
    /// the task.
    pub(crate) fn request(
        &self,
        catalog: &Catalog,
        facts: Option<&RepoFacts>,
    ) -> Result<Request, String> {
        match self.kind {
            Kind::Shell => Ok(Request::Shell),
            Kind::Agent => self.agent_request(catalog, facts),
            Kind::Workflow => self.workflow_request(catalog, facts),
        }
    }

    fn placement(&self, facts: Option<&RepoFacts>) -> Placement {
        if self.makes_a_worktree() {
            Placement { name: Some(self.name(facts)), base: self.base(facts) }
        } else {
            Placement::default()
        }
    }

    fn agent_request(
        &self,
        catalog: &Catalog,
        facts: Option<&RepoFacts>,
    ) -> Result<Request, String> {
        if !catalog.is_loaded() {
            return Err("Profiles are still loading.".to_owned());
        }
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
            task: self.message.trim().to_owned(),
            placement: self.placement(facts),
        })))
    }

    fn workflow_request(
        &self,
        catalog: &Catalog,
        facts: Option<&RepoFacts>,
    ) -> Result<Request, String> {
        if !catalog.is_loaded() {
            return Err("Workflows are still loading.".to_owned());
        }
        let Some(id) = self.selected_id(catalog) else {
            return Err("No workflow to start. Add one in Settings.".to_owned());
        };
        let (Some(workflow), Some(profiles)) =
            (self.selected_workflow(catalog), catalog.profiles())
        else {
            return Err(format!("workflow {id} is no longer available"));
        };
        let mut request = new_tab_workflow::build(workflow, profiles)?;
        self.task.trim().clone_into(&mut request.task);
        request.placement = self.placement(facts);
        Ok(Request::Workflow(Box::new(request)))
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

const HEAD: &str = "HEAD";

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
