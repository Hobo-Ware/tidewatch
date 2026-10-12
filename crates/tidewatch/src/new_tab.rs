//! The new-tab dialog's state and rules, without any drawing: what can be picked, which permission
//! levels are open (YOLO only in a worktree tab, D11), the live command preview and the request
//! Enter turns into. `new_tab_dialog` draws it.

use std::path::{Path, PathBuf};

use tidewatch_core::agents::{Agent, Level};
use tidewatch_core::launch::Launch;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prepare::{PrepareInput, preview as preview_launch};
use tidewatch_core::profiles::Profile;

use crate::catalog::Catalog;

const LONG_ARG: usize = 72;
const LONG_ARG_KEEP: usize = 60;
const COPILOT_TRUST_HINT: &str = "Copilot asks to trust a new worktree on its first start. Trust ~/.tidewatch once to skip that.";

/// Where the tab opens: a worktree of the repo, or a plain folder in the Other group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Repo,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    Profile(String),
    Shell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentRequest {
    pub(crate) profile_id: String,
    /// The profile with the level picked in the dialog applied.
    pub(crate) profile: Profile,
    pub(crate) agent: Agent,
    pub(crate) task: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    Shell,
    Agent(Box<AgentRequest>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Preview {
    pub(crate) lines: Vec<String>,
    pub(crate) warnings: Vec<String>,
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

/// The profile's own level when it is open here, else Auto, else the first open one.
fn opening_level(group: Group, agent: &Agent, wanted: Level) -> Level {
    [wanted, Level::Auto]
        .into_iter()
        .chain(Level::ALL)
        .find(|level| level_refusal(group, agent, *level).is_none())
        .unwrap_or(wanted)
}

pub(crate) struct NewTab {
    group: Group,
    label: String,
    repo: PathBuf,
    picks: Vec<Pick>,
    selected: usize,
    level: Level,
    pub(crate) task: String,
    pub(crate) focus_task: bool,
    running_here: Vec<String>,
    preview: Preview,
    pub(crate) error: Option<String>,
    dirty: bool,
}

impl NewTab {
    /// `preselect` is a profile id to start on, for example the one used last. `running_here` names
    /// the tabs that already run an agent in the folder (only matters in Other).
    pub(crate) fn open(
        group: Group,
        label: String,
        repo: PathBuf,
        catalog: &Catalog,
        running_here: Vec<String>,
        preselect: Option<&str>,
    ) -> Self {
        let mut picks: Vec<Pick> =
            catalog.entries().iter().map(|(id, _, _)| Pick::Profile((*id).to_owned())).collect();
        picks.push(Pick::Shell);
        let selected = preselect
            .and_then(|id| picks.iter().position(|p| matches!(p, Pick::Profile(x) if x == id)))
            .unwrap_or(0);
        let mut state = Self {
            group,
            label,
            repo,
            picks,
            selected,
            level: Level::Auto,
            task: String::new(),
            focus_task: true,
            running_here,
            preview: Preview::default(),
            error: None,
            dirty: true,
        };
        state.apply_selection(catalog);
        state
    }

    pub(crate) fn group(&self) -> Group {
        self.group
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn picks(&self) -> &[Pick] {
        &self.picks
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn level(&self) -> Level {
        self.level
    }

    pub(crate) fn selected_pick(&self) -> &Pick {
        &self.picks[self.selected]
    }

    pub(crate) fn selected_entry<'a>(
        &self,
        catalog: &'a Catalog,
    ) -> Option<(&'a Profile, &'a Agent)> {
        let Pick::Profile(id) = self.selected_pick() else { return None };
        catalog.entries().into_iter().find(|(x, _, _)| x == id).map(|(_, p, a)| (p, a))
    }

    pub(crate) fn select(&mut self, index: usize, catalog: &Catalog) {
        if index < self.picks.len() && index != self.selected {
            self.selected = index;
            self.apply_selection(catalog);
        }
    }

    pub(crate) fn step(&mut self, down: bool, catalog: &Catalog) {
        let next = if down {
            (self.selected + 1).min(self.picks.len() - 1)
        } else {
            self.selected.saturating_sub(1)
        };
        self.select(next, catalog);
    }

    fn apply_selection(&mut self, catalog: &Catalog) {
        self.error = None;
        self.dirty = true;
        self.focus_task = true;
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
        self.dirty = true;
        self.focus_task = true;
        true
    }

    pub(crate) fn task_edited(&mut self) {
        self.error = None;
        self.dirty = true;
    }

    /// What Enter does: the request, or the reason it cannot start.
    pub(crate) fn request(&self, catalog: &Catalog) -> Result<Request, String> {
        let Pick::Profile(id) = self.selected_pick() else { return Ok(Request::Shell) };
        let (profile, agent) = self
            .selected_entry(catalog)
            .ok_or_else(|| format!("profile {id} is no longer available"))?;
        if let Some(reason) = level_refusal(self.group, agent, self.level) {
            return Err(reason);
        }
        Ok(Request::Agent(Box::new(AgentRequest {
            profile_id: id.clone(),
            profile: Profile { level: self.level, ..profile.clone() },
            agent: agent.clone(),
            task: self.task.trim().to_owned(),
        })))
    }

    pub(crate) fn preview(&self) -> &Preview {
        &self.preview
    }

    /// Rebuilds the preview when something it shows changed since the last call. Returns whether it did.
    pub(crate) fn refresh_preview(
        &mut self,
        catalog: &Catalog,
        paths: Option<&TidewatchPaths>,
    ) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;
        let chosen = self.selected_entry(catalog).map(|(profile, agent)| {
            (Profile { level: self.level, ..profile.clone() }, agent.clone())
        });
        self.preview = build_preview(
            self.group,
            &self.repo,
            chosen.as_ref().map(|(p, a)| (p, a)),
            &self.task,
            paths,
            &self.running_here,
        );
        true
    }
}

fn build_preview(
    group: Group,
    repo: &Path,
    chosen: Option<(&Profile, &Agent)>,
    task: &str,
    paths: Option<&TidewatchPaths>,
    running_here: &[String],
) -> Preview {
    let home = paths.and_then(|p| p.root().parent()).map(Path::to_path_buf);
    let shown = |text: String| match &home {
        Some(home) => text.replace(&format!("{}/", home.display()), "~/"),
        None => text,
    };
    let mut preview = Preview::default();
    match group {
        Group::Repo => {
            if let Some(paths) = paths {
                let dir = paths.repo_dir(repo);
                preview.lines.push(shown(format!(
                    "git worktree add --lock -b agent/<name> {}/<name>",
                    dir.display()
                )));
            }
        }
        Group::Other => {
            preview.lines.push(shown(format!("No worktree: runs in {}", repo.display())));
        }
    }
    let Some((profile, agent)) = chosen else {
        preview.lines.push("then: your shell".to_owned());
        return preview;
    };
    if !profile.setup.trim().is_empty() {
        preview.lines.push(format!(
            "setup: {}",
            profile.setup.trim().lines().collect::<Vec<_>>().join(" && ")
        ));
    }
    let Some(paths) = paths else { return preview };
    let input = PrepareInput {
        paths,
        agent,
        profile,
        repo,
        tab: "<name>",
        dir: repo,
        worktree_tab: group == Group::Repo,
        task,
        session_id: None,
        path_var: None,
        listener: None,
    };
    match preview_launch(&input) {
        Ok(launch) => {
            preview.lines.push(shown(format!("then: {}", elide(&launch).shell_line())));
            if launch.experimental {
                preview.warnings.push(format!("{} support is experimental.", agent.name));
            }
        }
        Err(e) => preview.warnings.push(e.to_string()),
    }
    if agent.id == "copilot" && group == Group::Repo {
        preview.warnings.push(COPILOT_TRUST_HINT.to_owned());
    }
    if group == Group::Other && !running_here.is_empty() {
        preview.warnings.push(same_folder_warning(running_here));
    }
    preview
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

/// The launch with every very long argument cut short, so a house-style text passed as an argument
/// does not fill the preview.
fn elide(launch: &Launch) -> Launch {
    let cut = |arg: &String| {
        if arg.chars().count() > LONG_ARG {
            format!("{}…", arg.chars().take(LONG_ARG_KEEP).collect::<String>())
        } else {
            arg.clone()
        }
    };
    Launch { args: launch.args.iter().map(cut).collect(), ..launch.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use tidewatch_core::agents::{AgentOverride, ArgList, LevelOverrides};
    use tidewatch_core::profiles::Profiles;

    fn catalog_with(profiles: Profiles) -> Catalog {
        let mut catalog = Catalog::new(OsString::from("/usr/bin"));
        catalog.set_profiles(profiles);
        catalog
    }

    fn catalog() -> Catalog {
        catalog_with(Profiles::seeded(&["claude".to_owned(), "copilot".to_owned()]))
    }

    fn paths() -> TidewatchPaths {
        TidewatchPaths::new(Path::new("/Users/dev"))
    }

    fn open(group: Group, catalog: &Catalog, preselect: Option<&str>) -> NewTab {
        NewTab::open(
            group,
            "web".into(),
            PathBuf::from("/work/web"),
            catalog,
            Vec::new(),
            preselect,
        )
    }

    fn pick_ids(state: &NewTab) -> Vec<String> {
        state
            .picks()
            .iter()
            .map(|p| match p {
                Pick::Profile(id) => id.clone(),
                Pick::Shell => "shell".into(),
            })
            .collect()
    }

    #[test]
    fn profiles_come_first_shell_last_and_the_first_profile_is_selected() {
        let state = open(Group::Repo, &catalog(), None);
        assert_eq!(
            pick_ids(&state),
            [
                "builder",
                "builder-yolo",
                "copilot-builder",
                "copilot-builder-yolo",
                "copilot-reviewer",
                "reviewer",
                "shell"
            ]
        );
        assert_eq!(state.selected(), 0);
        assert_eq!(state.level(), Level::Edits);
    }

    #[test]
    fn with_no_profiles_only_shell_is_offered_and_it_starts_a_shell() {
        let catalog = catalog_with(Profiles::default());
        let state = open(Group::Repo, &catalog, None);
        assert_eq!(pick_ids(&state), ["shell"]);
        assert_eq!(state.request(&catalog), Ok(Request::Shell));
    }

    #[test]
    fn the_last_used_profile_is_preselected_when_it_still_exists() {
        let catalog = catalog();
        assert_eq!(open(Group::Repo, &catalog, Some("reviewer")).selected(), 5);
        assert_eq!(open(Group::Repo, &catalog, Some("deleted")).selected(), 0);
    }

    #[test]
    fn picking_a_profile_takes_its_level_and_clears_the_error() {
        let catalog = catalog();
        let mut state = open(Group::Repo, &catalog, None);
        state.error = Some("old".into());
        state.select(5, &catalog);
        assert_eq!((state.level(), state.error.clone()), (Level::Plan, None));
        state.select(1, &catalog);
        assert_eq!(state.level(), Level::Yolo);
    }

    #[test]
    fn yolo_is_open_in_a_repo_worktree_and_refused_in_other_with_a_reason() {
        let catalog = catalog();
        let agent = catalog.table().get("claude").unwrap();
        assert_eq!(level_refusal(Group::Repo, agent, Level::Yolo), None);
        let reason = level_refusal(Group::Other, agent, Level::Yolo).unwrap();
        assert!(reason.contains("not in Other"), "{reason}");
        for level in [Level::Plan, Level::Ask, Level::Edits, Level::Auto] {
            assert_eq!(level_refusal(Group::Other, agent, level), None, "{level:?}");
        }
    }

    #[test]
    fn in_other_a_yolo_profile_opens_on_auto_and_yolo_cannot_be_picked() {
        let catalog = catalog();
        let mut state = open(Group::Other, &catalog, Some("builder-yolo"));
        assert_eq!(state.level(), Level::Auto);
        assert!(!state.set_level(Level::Yolo, &catalog));
        assert_eq!(state.level(), Level::Auto);
        assert!(state.set_level(Level::Ask, &catalog));
        assert_eq!(state.level(), Level::Ask);
    }

    #[test]
    fn a_yolo_request_in_other_is_refused_even_if_the_level_was_forced() {
        let catalog = catalog();
        let mut state = open(Group::Other, &catalog, Some("builder"));
        state.level = Level::Yolo;
        let error = state.request(&catalog).unwrap_err();
        assert!(error.starts_with("YOLO only runs"), "{error}");
    }

    #[test]
    fn the_request_carries_the_picked_level_and_the_trimmed_task() {
        let catalog = catalog();
        let mut state = open(Group::Repo, &catalog, None);
        assert!(state.set_level(Level::Auto, &catalog));
        state.task = "  fix the flaky test \n".into();
        let Ok(Request::Agent(request)) = state.request(&catalog) else { panic!("agent request") };
        assert_eq!(request.profile_id, "builder");
        assert_eq!(request.profile.level, Level::Auto);
        assert_eq!(request.agent.id, "claude");
        assert_eq!(request.task, "fix the flaky test");
    }

    #[test]
    fn shell_is_always_a_shell_request() {
        let catalog = catalog();
        let mut state = open(Group::Repo, &catalog, None);
        let last = state.picks().len() - 1;
        state.select(last, &catalog);
        assert_eq!(state.request(&catalog), Ok(Request::Shell));
        assert!(!state.set_level(Level::Plan, &catalog));
    }

    #[test]
    fn arrow_steps_stay_inside_the_list() {
        let catalog = catalog();
        let mut state = open(Group::Repo, &catalog, None);
        state.step(false, &catalog);
        assert_eq!(state.selected(), 0);
        for _ in 0..20 {
            state.step(true, &catalog);
        }
        assert_eq!(state.selected(), state.picks().len() - 1);
    }

    fn custom_catalog() -> Catalog {
        let mut profiles = Profiles::default();
        profiles.agent.insert(
            "aider".into(),
            AgentOverride {
                bin: Some("aider".into()),
                permissions: LevelOverrides {
                    ask: Some(ArgList(vec!["--no-auto-commits".into()])),
                    ..LevelOverrides::default()
                },
                ..AgentOverride::default()
            },
        );
        profiles.profile.insert(
            "aider".into(),
            Profile {
                name: "Aider".into(),
                agent: "aider".into(),
                model: None,
                effort: None,
                level: Level::Ask,
                setup: String::new(),
                never_push: true,
                extra_args: ArgList::default(),
                house_style: false,
            },
        );
        catalog_with(profiles)
    }

    #[test]
    fn a_custom_agent_can_only_pick_levels_it_has_flags_for() {
        let catalog = custom_catalog();
        let mut state = open(Group::Repo, &catalog, Some("aider"));
        assert_eq!(state.level(), Level::Ask);
        assert!(!state.set_level(Level::Yolo, &catalog));
        let agent = catalog.table().get("aider").unwrap();
        assert_eq!(
            level_refusal(Group::Repo, agent, Level::Yolo),
            Some("aider has no flags for YOLO in profiles.toml.".to_owned())
        );
    }

    #[test]
    fn a_profile_whose_level_has_no_flags_opens_on_a_level_that_does() {
        let mut catalog = custom_catalog();
        let mut profiles = catalog.profiles().unwrap().clone();
        profiles.profile.get_mut("aider").unwrap().level = Level::Yolo;
        catalog.set_profiles(profiles);
        let state = open(Group::Repo, &catalog, Some("aider"));
        assert_eq!(state.level(), Level::Ask);
    }

    fn previewed(group: Group, preselect: &str, task: &str, level: Option<Level>) -> Preview {
        let catalog = catalog();
        let mut state = open(group, &catalog, Some(preselect));
        if let Some(level) = level {
            state.set_level(level, &catalog);
        }
        state.task = task.into();
        state.task_edited();
        state.refresh_preview(&catalog, Some(&paths()));
        state.preview().clone()
    }

    #[test]
    fn the_preview_of_a_claude_builder_shows_the_worktree_and_the_real_command() {
        let preview = previewed(Group::Repo, "builder", "fix it", None);
        assert!(
            preview.lines[0]
                .starts_with("git worktree add --lock -b agent/<name> ~/.tidewatch/web-"),
            "{:?}",
            preview.lines
        );
        assert!(preview.lines[0].ends_with("/<name>"));
        let command = &preview.lines[1];
        assert!(
            command.starts_with(
                "then: claude --permission-mode acceptEdits --model opus --effort high"
            ),
            "{command}"
        );
        assert!(
            command.contains("--append-system-prompt-file ~/.config/tidewatch/house-style.md"),
            "{command}"
        );
        assert!(command.ends_with("-- 'fix it'"), "{command}");
        assert_eq!(preview.warnings, Vec::<String>::new());
    }

    #[test]
    fn the_preview_follows_the_level_and_shows_the_setup_command() {
        let preview = previewed(Group::Repo, "builder", "", Some(Level::Plan));
        assert!(
            preview
                .lines
                .iter()
                .any(|l| l.contains("--permission-mode dontAsk --allowedTools=Read,Grep,Glob")),
            "{:?}",
            preview.lines
        );

        let mut profiles = Profiles::seeded(&["claude".to_owned()]);
        profiles.profile.get_mut("builder").unwrap().setup = "npm ci\nnpm run build".into();
        let catalog = catalog_with(profiles);
        let mut state = open(Group::Repo, &catalog, None);
        state.refresh_preview(&catalog, Some(&paths()));
        assert_eq!(state.preview().lines[1], "setup: npm ci && npm run build");
    }

    #[test]
    fn the_preview_in_other_names_the_folder_and_warns_about_another_agent_there() {
        let catalog = catalog();
        let mut state = NewTab::open(
            Group::Other,
            "Other: notes".into(),
            PathBuf::from("/Users/dev/notes"),
            &catalog,
            vec!["Builder".into()],
            Some("builder"),
        );
        state.refresh_preview(&catalog, Some(&paths()));
        let preview = state.preview();
        assert_eq!(preview.lines[0], "No worktree: runs in ~/notes");
        assert_eq!(
            preview.warnings,
            [
                "The Builder tab already has an agent in this folder. They can overwrite each other's files."
            ]
        );
    }

    #[test]
    fn the_same_folder_warning_names_every_agent_tab() {
        let two = same_folder_warning(&["a".into(), "b".into()]);
        assert!(two.starts_with("The a, b tabs already have an agent in this folder."), "{two}");
    }

    #[test]
    fn a_shell_preview_has_no_command_and_other_agents_get_their_notes() {
        let catalog = catalog();
        let mut state = open(Group::Repo, &catalog, None);
        let last = state.picks().len() - 1;
        state.select(last, &catalog);
        state.refresh_preview(&catalog, Some(&paths()));
        assert_eq!(state.preview().lines.last().unwrap(), "then: your shell");

        let copilot = previewed(Group::Repo, "copilot-builder", "", None);
        assert!(
            copilot.warnings.iter().any(|w| w.starts_with("Copilot asks to trust a new worktree")),
            "{:?}",
            copilot.warnings
        );
        let other = previewed(Group::Other, "copilot-builder", "", None);
        assert_eq!(other.warnings, Vec::<String>::new());
    }

    #[test]
    fn a_refused_launch_shows_the_reason_instead_of_a_command() {
        let catalog = catalog();
        let mut state = open(Group::Other, &catalog, Some("builder"));
        state.level = Level::Yolo;
        state.dirty = true;
        state.refresh_preview(&catalog, Some(&paths()));
        assert_eq!(state.preview().lines.len(), 1);
        assert!(state.preview().warnings[0].starts_with("YOLO only runs"));
    }

    #[test]
    fn very_long_arguments_are_cut_in_the_preview_only() {
        let long = "x".repeat(200);
        let launch = Launch {
            program: "codex".into(),
            args: vec!["-c".into(), long.clone(), "short".into()],
            env: Vec::new(),
            experimental: true,
        };
        let cut = elide(&launch);
        assert_eq!(cut.args[1].chars().count(), LONG_ARG_KEEP + 1);
        assert!(cut.args[1].ends_with('…'));
        assert_eq!((cut.args[0].as_str(), cut.args[2].as_str()), ("-c", "short"));
        assert_eq!(launch.args[1], long);
    }

    #[test]
    fn codex_is_flagged_experimental_in_the_preview() {
        let catalog = catalog_with(Profiles::seeded(&["codex".to_owned()]));
        let mut state = open(Group::Repo, &catalog, None);
        state.refresh_preview(&catalog, Some(&paths()));
        assert_eq!(state.preview().warnings, ["Codex support is experimental."]);
    }
}
