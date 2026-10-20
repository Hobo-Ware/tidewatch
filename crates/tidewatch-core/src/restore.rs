//! Session restore (PLAN.md, D11): what to do with each saved tab at the next launch, and the
//! command that reopens an agent's session.
//!
//! [`plan`] is pure: it reads the saved [`Session`], the agent table, the profiles and a [`Disk`]
//! that answers what exists now. [`RealDisk`] is the real answer; tests use a fake. The rules:
//!
//! - A tab whose repo, folder or worktree is gone is skipped: nothing is left to open it in.
//! - `resume_agents` and how the agent last ended pick between running the resume command, typing
//!   it for the user to run, and a plain shell (`resume_mode::decide`). A YOLO tab is always typed,
//!   never run on its own. Workflow tabs go through the runner instead; a YOLO workflow run is
//!   not resumed and reopens as a plain shell with a note.
//! - An agent tab resumes only when its agent is installed, its profile still runs that agent, it
//!   has a session id, the CLI's session file exists, and no earlier tab claimed the same session.
//!   Anything else reopens as a plain shell in the worktree, with the reason as a note.
//! - A tab with a workflow run reopens through `Runner::load` and `Runner::resume`.
//! - A moved repo is repaired first with [`repair_moved`], which gives the window its new folder.
//!
//! Resume forms were checked against claude 2.1.296, gemini 0.46.0, copilot 1.0.96 and codex
//! 0.162.1 (`--help`; Claude and Codex also live, see PLAN.md Phase 0 findings).

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::agents::{Agent, AgentTable, Level, ResumeForm};
use crate::launch::{Launch, LaunchCtx, LaunchError, launch_command};
use crate::paths::TidewatchPaths;
use crate::prefs::ResumeAgents;
use crate::profiles::{Profile, Profiles};
use crate::resume_mode::{Ended, Resume, decide};
use crate::runner::{self, Runner};
use crate::session::{SavedAgentTab, SavedPark, SavedTab, SavedWindow, Session};
use crate::tools::Tools;
use crate::transcript::Source;
use crate::transcript_locate::Locator;
use crate::worktree::{self, Worktree};

/// Why a tab did not come back the way it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    RepoMissing,
    FolderMissing,
    WorktreeGone,
    WorktreeBroken,
    Yolo,
    ResumeOff,
    AgentUnknown(String),
    AgentMissing(String),
    ProfileGone(String),
    ProfileChanged { profile: String, agent: String },
    NotResumable(String),
    NoSessionId,
    TranscriptMissing,
    Duplicate,
    WorkflowUnreadable,
}

impl Reason {
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::RepoMissing => "repository folder missing".into(),
            Self::FolderMissing => "folder missing".into(),
            Self::WorktreeGone => "worktree folder gone".into(),
            Self::WorktreeBroken => "worktree no longer belongs to its repository".into(),
            Self::Yolo => "YOLO workflow runs are never resumed automatically".into(),
            Self::ResumeOff => "resuming agents is turned off in Settings".into(),
            Self::AgentUnknown(id) => format!("agent `{id}` is not configured"),
            Self::AgentMissing(id) => format!("`{id}` is not installed"),
            Self::ProfileGone(name) => format!("profile `{name}` no longer exists"),
            Self::ProfileChanged { profile, agent } => {
                format!("profile `{profile}` no longer runs `{agent}`")
            }
            Self::NotResumable(id) => format!("`{id}` cannot resume a session"),
            Self::NoSessionId => "no session id was recorded".into(),
            Self::TranscriptMissing => "its session file is gone".into(),
            Self::Duplicate => "another tab holds the same session".into(),
            Self::WorkflowUnreadable => "its workflow state is unreadable".into(),
        }
    }
}

/// What exists on disk now. Everything the planner needs to know, so it can stay pure.
pub trait Disk {
    fn dir_exists(&self, path: &Path) -> bool;
    /// The folder is a linked git worktree whose repository is reachable.
    fn worktree_usable(&self, path: &Path) -> bool;
    /// The CLI's session file for `session_id` exists, looked up from the tab's folder.
    fn transcript_exists(&self, agent: &str, dir: &Path, session_id: &str) -> bool;
    /// The workflow run at `stages` loads and can be resumed.
    fn workflow_runnable(&self, stages: &Path) -> bool;
}

/// The real disk. Blocks on directory listings and one `git` call per worktree tab: run it off
/// the UI thread.
#[derive(Clone, Debug)]
pub struct RealDisk {
    pub home: PathBuf,
    /// `COPILOT_HOME`, when set.
    pub copilot_home: Option<PathBuf>,
    /// `CODEX_HOME`, when set.
    pub codex_home: Option<PathBuf>,
}

impl RealDisk {
    #[must_use]
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into(), copilot_home: None, codex_home: None }
    }

    #[must_use]
    pub fn from_env() -> Self {
        let var = |key: &str| std::env::var_os(key).map(PathBuf::from);
        Self {
            home: var("HOME").unwrap_or_default(),
            copilot_home: var("COPILOT_HOME"),
            codex_home: var("CODEX_HOME"),
        }
    }
}

impl Disk for RealDisk {
    fn dir_exists(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn worktree_usable(&self, path: &Path) -> bool {
        path.join(".git").is_file()
            && Tools::system()
                .git(path)
                .args(["rev-parse", "--git-dir"])
                .output()
                .is_ok_and(|out| out.status.success())
    }

    fn transcript_exists(&self, agent: &str, dir: &Path, session_id: &str) -> bool {
        let Some(source) = Source::from_agent_id(agent) else {
            return false;
        };
        let mut locator = Locator::new(&self.home, dir);
        locator.session_id = Some(session_id.to_owned());
        locator.copilot_home.clone_from(&self.copilot_home);
        locator.codex_home.clone_from(&self.codex_home);
        locator.locate(source).is_some()
    }

    fn workflow_runnable(&self, stages: &Path) -> bool {
        Runner::load(stages).is_some()
    }
}

/// What the planner knows besides the disk.
pub struct RestoreEnv<'a> {
    pub agents: &'a AgentTable,
    pub profiles: &'a Profiles,
    /// Ids of the agents whose binary was found (`AgentTable::found_on_path`).
    pub installed: &'a [String],
    pub resume: ResumeAgents,
    pub app_crashed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabAction {
    /// Reopen a plain shell: a main or shell tab.
    Shell,
    /// An agent tab that could not resume: reopen a plain shell in its folder, with the reason as a note.
    ShellInstead(Reason),
    /// Start the agent with `resume_command`, or type it and leave Enter to the user (`how`).
    /// `profile` carries the level the tab ran at.
    ResumeAgent { agent: String, profile: Box<Profile>, session_id: String, how: Resume },
    /// `restore_workflow(stages)` gives the runner and the command it was waiting on.
    ResumeWorkflow { stages: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPlan {
    pub saved: SavedTab,
    /// Where the tab runs: the worktree, the shared folder, or the repo root for the main tab.
    pub dir: PathBuf,
    /// The tab's worktree, for a tab in one.
    pub worktree: Option<Worktree>,
    pub action: TabAction,
    pub park: Option<SavedPark>,
}

impl TabPlan {
    /// What to print for a tab that did not simply come back: above the plain shell that stands in
    /// for its agent, or on a tab whose resume command is typed and waits for Enter.
    #[must_use]
    pub fn note(&self) -> Option<String> {
        match &self.action {
            TabAction::ResumeAgent { how: Resume::TypeOnly(why), .. } => {
                Some(why.note().to_owned())
            }
            TabAction::ShellInstead(Reason::Yolo) => Some(
                "This tab ran at YOLO, so Tidewatch did not restart its agent. Start it again yourself."
                    .to_owned(),
            ),
            TabAction::ShellInstead(reason) => {
                Some(format!("The agent was not resumed: {}.", reason.text()))
            }
            _ => None,
        }
    }
}

/// A tab that is not reopened at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub label: String,
    pub reason: Reason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowPlan {
    pub dir: PathBuf,
    pub other: bool,
    pub focus_mode: bool,
    /// Index into `tabs`: the saved active tab, or the nearest kept one before it.
    pub active: usize,
    /// The tabs to reopen, in their saved order.
    pub tabs: Vec<TabPlan>,
    pub skipped: Vec<Skipped>,
}

impl WindowPlan {
    /// One message for the toast, or `None` when everything came back as it was.
    #[must_use]
    pub fn toast(&self) -> Option<String> {
        let dropped: Vec<String> =
            self.skipped.iter().map(|s| format!("{} ({})", s.label, s.reason.text())).collect();
        let degraded: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|tab| match &tab.action {
                TabAction::ShellInstead(Reason::ResumeOff) => None,
                TabAction::ShellInstead(reason) => {
                    Some(format!("{} ({})", tab.saved.label(&self.dir), reason.text()))
                }
                _ => None,
            })
            .collect();
        let mut parts = Vec::new();
        if !dropped.is_empty() {
            parts.push(format!(
                "Could not reopen {}: {}",
                count_tabs(dropped.len()),
                dropped.join(", ")
            ));
        }
        if !degraded.is_empty() {
            parts.push(format!(
                "Reopened {} as a shell: {}",
                count_tabs(degraded.len()),
                degraded.join(", ")
            ));
        }
        (!parts.is_empty()).then(|| parts.join(". "))
    }
}

fn count_tabs(n: usize) -> String {
    format!("{n} tab{}", if n == 1 { "" } else { "s" })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestorePlan {
    pub windows: Vec<WindowPlan>,
}

type Claims = HashSet<(String, String)>;

/// Plans every window of the session. A session id is claimed by the first tab that resumes it.
#[must_use]
pub fn plan(session: &Session, env: &RestoreEnv, disk: &dyn Disk) -> RestorePlan {
    let mut claims = Claims::new();
    RestorePlan {
        windows: session.windows.iter().map(|w| plan_one(w, env, disk, &mut claims)).collect(),
    }
}

/// Plans one window, for an app that opens a single repo.
#[must_use]
pub fn plan_window(window: &SavedWindow, env: &RestoreEnv, disk: &dyn Disk) -> WindowPlan {
    plan_one(window, env, disk, &mut Claims::new())
}

fn plan_one(
    window: &SavedWindow,
    env: &RestoreEnv,
    disk: &dyn Disk,
    claims: &mut Claims,
) -> WindowPlan {
    let window_missing = (!disk.dir_exists(&window.dir)).then_some(if window.other {
        Reason::FolderMissing
    } else {
        Reason::RepoMissing
    });
    let mut tabs = Vec::new();
    let mut skipped = Vec::new();
    let mut kept = Vec::new();
    for (index, tab) in window.tabs.iter().enumerate() {
        let planned = match &window_missing {
            Some(reason) => Err(reason.clone()),
            None => plan_tab(tab, window, env, disk, claims).map(|plan| TabPlan {
                park: window.parked.iter().find(|park| park.tab == index).copied(),
                ..plan
            }),
        };
        kept.push(planned.is_ok());
        match planned {
            Ok(plan) => tabs.push(plan),
            Err(reason) => skipped.push(Skipped { label: tab.label(&window.dir), reason }),
        }
    }
    let upto = window.active.min(kept.len().saturating_sub(1));
    let active = kept.iter().take(upto + 1).filter(|k| **k).count().saturating_sub(1);
    WindowPlan {
        dir: window.dir.clone(),
        other: window.other,
        focus_mode: window.focus_mode,
        active,
        tabs,
        skipped,
    }
}

fn plan_tab(
    tab: &SavedTab,
    window: &SavedWindow,
    env: &RestoreEnv,
    disk: &dyn Disk,
    claims: &mut Claims,
) -> Result<TabPlan, Reason> {
    let dir = tab.dir(&window.dir).to_path_buf();
    let worktree = tab.branch().map(|branch| Worktree {
        repo: window.dir.clone(),
        path: dir.clone(),
        branch: branch.to_owned(),
    });
    if worktree.is_some() {
        if !disk.dir_exists(&dir) {
            return Err(Reason::WorktreeGone);
        }
        if !disk.worktree_usable(&dir) {
            return Err(Reason::WorktreeBroken);
        }
    } else if !disk.dir_exists(&dir) {
        return Err(Reason::FolderMissing);
    }
    let action = match tab {
        SavedTab::Main | SavedTab::Shell { .. } => TabAction::Shell,
        SavedTab::Agent(agent) => plan_agent(agent, env, disk, claims),
    };
    Ok(TabPlan { saved: tab.clone(), dir, worktree, action, park: None })
}

fn plan_agent(
    tab: &SavedAgentTab,
    env: &RestoreEnv,
    disk: &dyn Disk,
    claims: &mut Claims,
) -> TabAction {
    match agent_resume(tab, env, disk, claims) {
        Ok(action) => action,
        Err(reason) => TabAction::ShellInstead(reason),
    }
}

fn agent_resume(
    tab: &SavedAgentTab,
    env: &RestoreEnv,
    disk: &dyn Disk,
    claims: &mut Claims,
) -> Result<TabAction, Reason> {
    if let Some(stages) = &tab.stages {
        if tab.level == Level::Yolo {
            return Err(Reason::Yolo);
        }
        return if disk.workflow_runnable(stages) {
            Ok(TabAction::ResumeWorkflow { stages: stages.clone() })
        } else {
            Err(Reason::WorkflowUnreadable)
        };
    }
    let how = decide(env.resume, Ended::of(env.app_crashed, tab.crashed), tab.level)?;
    let agent =
        env.agents.get(&tab.agent).ok_or_else(|| Reason::AgentUnknown(tab.agent.clone()))?;
    if !env.installed.contains(&tab.agent) {
        return Err(Reason::AgentMissing(tab.agent.clone()));
    }
    let defaults = tab.meeseeks.is_some().then(|| Profile::defaults_for(&tab.agent, tab.level));
    let profile = env
        .profiles
        .profile
        .get(&tab.profile)
        .or_else(|| env.profiles.profile.values().find(|p| p.name == tab.profile))
        .or(defaults.as_ref().filter(|_| tab.profile == Profile::DEFAULTS_ID))
        .ok_or_else(|| Reason::ProfileGone(tab.profile.clone()))?;
    if profile.agent != tab.agent {
        return Err(Reason::ProfileChanged {
            profile: tab.profile.clone(),
            agent: tab.agent.clone(),
        });
    }
    let form = agent.resume.ok_or_else(|| Reason::NotResumable(tab.agent.clone()))?;
    let id = tab.session_id.as_deref().filter(|id| !id.is_empty()).ok_or(Reason::NoSessionId)?;
    if !disk.transcript_exists(&tab.agent, &tab.dir, id) {
        return Err(Reason::TranscriptMissing);
    }
    let claim = match form {
        ResumeForm::FlagLatest(_) => (tab.agent.clone(), tab.dir.display().to_string()),
        _ => (tab.agent.clone(), id.to_owned()),
    };
    if !claims.insert(claim) {
        return Err(Reason::Duplicate);
    }
    Ok(TabAction::ResumeAgent {
        agent: tab.agent.clone(),
        profile: Box::new(Profile { level: tab.level, ..profile.clone() }),
        session_id: id.to_owned(),
        how,
    })
}

/// Startup pass for a repo that moved. When no window of the session has `opened` as its folder
/// and one whose folder is gone has the same name, git's links for that repo's worktrees are
/// repaired from `opened` and the window takes the new folder. Returns the repaired worktrees.
/// Runs `git`: call it off the UI thread, before [`plan`].
pub fn repair_moved(paths: &TidewatchPaths, session: &mut Session, opened: &Path) -> Vec<Worktree> {
    if !opened.exists() || session.windows.iter().any(|w| w.dir == opened) {
        return Vec::new();
    }
    let name = opened.file_name();
    let Some(window) = session
        .windows
        .iter_mut()
        .find(|w| !w.other && !w.dir.exists() && w.dir.file_name() == name)
    else {
        return Vec::new();
    };
    let repaired = worktree::repair(paths, opened);
    window.dir = opened.to_path_buf();
    repaired
}

/// The runner a workflow tab was in, and the command it was waiting on (`None` for a run that
/// never started). The app carries the command out as it would after any event.
#[must_use]
pub fn restore_workflow(stages: &Path) -> Option<(Runner, Option<runner::Command>)> {
    let runner = Runner::load(stages)?;
    let command = runner.resume();
    Some((runner, command))
}

/// The session id Codex put in a rollout file name (`rollout-<timestamp>-<uuid>.jsonl`). Codex takes
/// no id at launch, so the app reads it from the file `Locator` finds and saves it. Checked on
/// codex 0.162.1: `codex resume <id>` with this id continued the same rollout file.
#[must_use]
pub fn codex_session_id(rollout: &Path) -> Option<String> {
    let stem = rollout.file_stem()?.to_str()?.strip_prefix("rollout-")?;
    let id = stem.get(stem.len().checked_sub(UUID_LEN)?..)?;
    let shape = [8, 13, 18, 23];
    let uuid_like = id
        .char_indices()
        .all(|(i, c)| if shape.contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    (uuid_like && stem.as_bytes().get(stem.len() - UUID_LEN - 1) == Some(&b'-'))
        .then(|| id.to_owned())
}

const UUID_LEN: usize = 36;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeError {
    Launch(LaunchError),
    /// The agent has no resume form (a custom agent).
    NotResumable {
        agent: String,
    },
}

impl fmt::Display for ResumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(e) => e.fmt(f),
            Self::NotResumable { agent } => write!(f, "{agent} cannot resume a session"),
        }
    }
}

impl std::error::Error for ResumeError {}

impl From<LaunchError> for ResumeError {
    fn from(e: LaunchError) -> Self {
        Self::Launch(e)
    }
}

/// The command that reopens `session_id`: [`launch_command`] for the same profile and context, so
/// the guard environment, hooks and MCP wiring match a fresh launch, with the CLI's resume
/// arguments in front. `ctx.session_id` is dropped (a CLI given both a new id and a session to
/// resume would start another session). Run it in `ctx.worktree`, the folder the session began in:
/// Claude keeps its session files per folder. Gemini resumes the latest session of the folder,
/// whatever `session_id` is. A YOLO profile resumes with its YOLO flags, and is refused outside an
/// agent worktree tab like a fresh launch.
pub fn resume_command(
    agent: &Agent,
    profile: &Profile,
    session_id: &str,
    ctx: &LaunchCtx,
) -> Result<Launch, ResumeError> {
    let resume = agent
        .resume_args(session_id)
        .ok_or_else(|| ResumeError::NotResumable { agent: agent.id.clone() })?;
    let ctx = LaunchCtx { session_id: None, ..ctx.clone() };
    let mut launch = launch_command(agent, profile, &ctx)?;
    launch.args.splice(0..0, resume);
    Ok(launch)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::agents::{AgentOverride, ArgList};
    use crate::launch::HookListener;
    use crate::resume_mode::Why;

    const ID_C: &str = "0c2cbc96-1111-4222-8333-444455556666";
    const ID_G: &str = "1c2cbc96-1111-4222-8333-444455556666";
    const ID_P: &str = "2c2cbc96-1111-4222-8333-444455556666";
    const ID_X: &str = "3c2cbc96-1111-4222-8333-444455556666";
    const REPO: &str = "/work/web";

    fn wt(name: &str) -> String {
        format!("/root/web-ab12cd/{name}")
    }

    fn profile(agent: &str, level: Level) -> Profile {
        Profile {
            name: format!("{agent} profile"),
            agent: agent.into(),
            model: None,
            effort: None,
            level,
            setup: String::new(),
            never_push: true,
            extra_args: ArgList::default(),
            house_style: true,
        }
    }

    fn profiles() -> Profiles {
        let profile_map: BTreeMap<String, Profile> = [
            ("builder", profile("claude", Level::Edits)),
            ("gem", profile("gemini", Level::Edits)),
            ("cop", profile("copilot", Level::Edits)),
            ("cdx", profile("codex", Level::Edits)),
            ("mine", profile("mine", Level::Edits)),
        ]
        .into_iter()
        .map(|(id, p)| (id.to_owned(), p))
        .collect();
        Profiles { profile: profile_map, agent: BTreeMap::new() }
    }

    fn table() -> AgentTable {
        let custom = AgentOverride { bin: Some("mine-cli".into()), ..AgentOverride::default() };
        AgentTable::with_overrides(&BTreeMap::from([("mine".to_owned(), custom)]))
    }

    fn everything_installed() -> Vec<String> {
        ["claude", "gemini", "copilot", "codex", "mine"].map(str::to_owned).to_vec()
    }

    #[derive(Default)]
    struct FakeDisk {
        dirs: Vec<String>,
        broken: Vec<String>,
        transcripts: Vec<(String, String)>,
        workflows: Vec<String>,
    }

    impl FakeDisk {
        fn with_dirs(dirs: &[&str]) -> Self {
            Self { dirs: dirs.iter().map(|d| (*d).to_owned()).collect(), ..Self::default() }
        }

        fn with_dirs_added(mut self, dirs: &[&str]) -> Self {
            self.dirs.extend(dirs.iter().map(|d| (*d).to_owned()));
            self
        }

        fn transcript(mut self, agent: &str, id: &str) -> Self {
            self.transcripts.push((agent.to_owned(), id.to_owned()));
            self
        }
    }

    impl Disk for FakeDisk {
        fn dir_exists(&self, path: &Path) -> bool {
            self.dirs.iter().any(|d| Path::new(d) == path)
        }

        fn worktree_usable(&self, path: &Path) -> bool {
            !self.broken.iter().any(|d| Path::new(d) == path)
        }

        fn transcript_exists(&self, agent: &str, _dir: &Path, session_id: &str) -> bool {
            self.transcripts.iter().any(|(a, id)| a == agent && id == session_id)
        }

        fn workflow_runnable(&self, stages: &Path) -> bool {
            self.workflows.iter().any(|f| Path::new(f) == stages)
        }
    }

    fn agent_tab(
        name: &str,
        agent: &str,
        profile: &str,
        level: Level,
        id: Option<&str>,
    ) -> SavedTab {
        SavedTab::Agent(SavedAgentTab {
            dir: PathBuf::from(wt(name)),
            branch: Some(format!("agent/{name}")),
            agent: agent.into(),
            profile: profile.into(),
            level,
            session_id: id.map(str::to_owned),
            stages: None,
            crashed: false,
            remote: false,
            meeseeks: None,
        })
    }

    fn window(tabs: Vec<SavedTab>) -> SavedWindow {
        SavedWindow {
            dir: PathBuf::from(REPO),
            other: false,
            active: 0,
            focus_mode: false,
            tabs,
            parked: Vec::new(),
        }
    }

    fn plan_with(window: &SavedWindow, installed: &[String], disk: &FakeDisk) -> WindowPlan {
        plan_as(window, installed, disk, ResumeAgents::Auto, false)
    }

    fn plan_as(
        window: &SavedWindow,
        installed: &[String],
        disk: &FakeDisk,
        resume: ResumeAgents,
        app_crashed: bool,
    ) -> WindowPlan {
        let table = table();
        let profiles = profiles();
        let env =
            RestoreEnv { agents: &table, profiles: &profiles, installed, resume, app_crashed };
        plan_window(window, &env, disk)
    }

    fn actions(plan: &WindowPlan) -> Vec<TabAction> {
        plan.tabs.iter().map(|t| t.action.clone()).collect()
    }

    fn resume(agent: &str, level: Level, id: &str) -> TabAction {
        let key = match agent {
            "claude" => "builder",
            "gemini" => "gem",
            "copilot" => "cop",
            _ => "cdx",
        };
        TabAction::ResumeAgent {
            agent: agent.into(),
            profile: Box::new(Profile { level, ..profiles().profile[key].clone() }),
            session_id: id.into(),
            how: Resume::Run,
        }
    }

    fn all_four_tabs() -> Vec<SavedTab> {
        vec![
            SavedTab::Main,
            agent_tab("a-claude", "claude", "builder", Level::Auto, Some(ID_C)),
            agent_tab("b-gemini", "gemini", "gem", Level::Edits, Some(ID_G)),
            agent_tab("c-copilot", "copilot", "cop", Level::Plan, Some(ID_P)),
            agent_tab("d-codex", "codex", "cdx", Level::Ask, Some(ID_X)),
        ]
    }

    fn disk_for_all() -> FakeDisk {
        let mut disk = FakeDisk::with_dirs(&[
            REPO,
            &wt("a-claude"),
            &wt("b-gemini"),
            &wt("c-copilot"),
            &wt("d-codex"),
        ]);
        disk = disk
            .transcript("claude", ID_C)
            .transcript("gemini", ID_G)
            .transcript("copilot", ID_P)
            .transcript("codex", ID_X);
        disk
    }

    #[test]
    fn everything_present_resumes_every_agent_at_its_saved_level() {
        let plan = plan_with(&window(all_four_tabs()), &everything_installed(), &disk_for_all());
        assert_eq!(plan.skipped, []);
        assert_eq!(
            actions(&plan),
            [
                TabAction::Shell,
                resume("claude", Level::Auto, ID_C),
                resume("gemini", Level::Edits, ID_G),
                resume("copilot", Level::Plan, ID_P),
                resume("codex", Level::Ask, ID_X),
            ]
        );
        assert_eq!(plan.toast(), None);
        let claude = &plan.tabs[1];
        assert_eq!(claude.dir, PathBuf::from(wt("a-claude")));
        assert_eq!(
            claude.worktree,
            Some(Worktree {
                repo: PathBuf::from(REPO),
                path: PathBuf::from(wt("a-claude")),
                branch: "agent/a-claude".into(),
            })
        );
        assert_eq!(plan.tabs[0].dir, PathBuf::from(REPO));
    }

    #[test]
    fn a_tab_keeps_its_parking_through_the_plan_even_when_a_tab_before_it_is_skipped() {
        use crate::session::{SavedPark, Why};
        let mut saved = window(all_four_tabs());
        saved.parked = vec![
            SavedPark { tab: 3, why: Some(Why::Idle), since_ms: 7, keep: false },
            SavedPark { tab: 4, why: None, since_ms: 0, keep: true },
        ];
        let mut disk = disk_for_all();
        disk.dirs.retain(|dir| dir != &wt("a-claude"));
        let plan = plan_with(&saved, &everything_installed(), &disk);
        let parks: Vec<_> = plan.tabs.iter().map(|tab| tab.park).collect();
        assert_eq!(
            parks,
            [None, None, Some(saved.parked[0]), Some(saved.parked[1])],
            "the indices are the saved ones, not the planned ones"
        );
    }

    #[test]
    fn a_gone_or_broken_worktree_is_skipped_and_the_active_tab_moves_to_a_kept_one() {
        let mut disk = disk_for_all();
        disk.dirs.retain(|d| d != &wt("b-gemini"));
        disk.broken.push(wt("c-copilot"));
        let mut saved = window(all_four_tabs());
        saved.active = 3;
        let plan = plan_with(&saved, &everything_installed(), &disk);
        assert_eq!(
            plan.skipped,
            [
                Skipped { label: "b-gemini".into(), reason: Reason::WorktreeGone },
                Skipped { label: "c-copilot".into(), reason: Reason::WorktreeBroken },
            ]
        );
        assert_eq!(plan.tabs.len(), 3);
        assert_eq!(plan.active, 1, "the nearest kept tab before the skipped active one");
        let toast = plan.toast().unwrap();
        assert!(
            toast.starts_with("Could not reopen 2 tabs: b-gemini (worktree folder gone)"),
            "{toast}"
        );
    }

    #[test]
    fn the_active_tab_index_counts_kept_tabs_only() {
        let mut disk = disk_for_all();
        disk.dirs.retain(|d| d != &wt("a-claude"));
        let mut saved = window(all_four_tabs());
        saved.active = 4;
        let plan = plan_with(&saved, &everything_installed(), &disk);
        assert_eq!(plan.active, 3);
        saved.active = 1;
        let plan = plan_with(&saved, &everything_installed(), &disk);
        assert_eq!(plan.active, 0);
        saved.active = 99;
        assert_eq!(plan_with(&saved, &everything_installed(), &disk).active, 3);
    }

    #[test]
    fn a_missing_repo_skips_every_tab_and_a_missing_other_folder_says_folder() {
        let disk = FakeDisk::with_dirs(&[&wt("a-claude")]);
        let plan = plan_with(&window(all_four_tabs()), &everything_installed(), &disk);
        assert_eq!(plan.tabs, []);
        assert!(plan.skipped.iter().all(|s| s.reason == Reason::RepoMissing));
        assert_eq!(plan.skipped.len(), 5);

        let other = SavedWindow { other: true, tabs: vec![SavedTab::Main], ..window(Vec::new()) };
        let plan = plan_with(&other, &everything_installed(), &disk);
        assert_eq!(plan.skipped, [Skipped { label: "main".into(), reason: Reason::FolderMissing }]);
    }

    #[test]
    fn a_missing_binary_reopens_a_shell_in_the_worktree_with_the_reason() {
        let installed: Vec<String> =
            everything_installed().into_iter().filter(|a| a != "claude").collect();
        let plan = plan_with(&window(all_four_tabs()), &installed, &disk_for_all());
        assert_eq!(plan.skipped, []);
        assert_eq!(
            plan.tabs[1].action,
            TabAction::ShellInstead(Reason::AgentMissing("claude".into()))
        );
        assert_eq!(plan.tabs[1].dir, PathBuf::from(wt("a-claude")));
        assert_eq!(
            plan.tabs[1].note().unwrap(),
            "The agent was not resumed: `claude` is not installed."
        );
        assert_eq!(plan.tabs[2].action, resume("gemini", Level::Edits, ID_G));
    }

    #[test]
    fn a_yolo_tab_is_typed_at_the_level_it_ran_at_and_never_run_on_its_own() {
        let tabs = vec![agent_tab("a-claude", "claude", "builder", Level::Yolo, Some(ID_C))];
        let plan = plan_with(&window(tabs), &everything_installed(), &disk_for_all());
        let TabAction::ResumeAgent { profile, how, .. } = &plan.tabs[0].action else {
            panic!("{:?}", plan.tabs[0].action)
        };
        assert_eq!(profile.level, Level::Yolo);
        assert_eq!(*how, Resume::TypeOnly(Why::Yolo));
        assert_eq!(plan.tabs[0].note().as_deref(), Some(Why::Yolo.note()));
    }

    #[test]
    fn a_yolo_tab_keeps_every_other_check_of_a_resume() {
        let yolo =
            |profile: &str| agent_tab("a-claude", "claude", profile, Level::Yolo, Some(ID_C));
        let cases = [
            (yolo("builder"), everything_installed(), disk_for_all(), None),
            (
                yolo("builder"),
                vec!["gemini".to_owned()],
                disk_for_all(),
                Some(Reason::AgentMissing("claude".into())),
            ),
            (
                yolo("gem"),
                everything_installed(),
                disk_for_all(),
                Some(Reason::ProfileChanged { profile: "gem".into(), agent: "claude".into() }),
            ),
            (
                yolo("builder"),
                everything_installed(),
                FakeDisk::with_dirs(&[REPO, &wt("a-claude")]),
                Some(Reason::TranscriptMissing),
            ),
        ];
        for (tab, installed, disk, want) in cases {
            let plan = plan_with(&window(vec![tab]), &installed, &disk);
            match want {
                Some(reason) => assert_eq!(plan.tabs[0].action, TabAction::ShellInstead(reason)),
                None => assert!(matches!(plan.tabs[0].action, TabAction::ResumeAgent { .. })),
            }
        }
    }

    #[test]
    fn two_yolo_tabs_of_one_session_resume_once() {
        let tabs = vec![
            agent_tab("a-claude", "claude", "builder", Level::Yolo, Some(ID_C)),
            agent_tab("b-claude", "claude", "builder", Level::Yolo, Some(ID_C)),
        ];
        let disk = disk_for_all().with_dirs_added(&[&wt("b-claude")]);
        let plan = plan_with(&window(tabs), &everything_installed(), &disk);
        assert!(matches!(plan.tabs[0].action, TabAction::ResumeAgent { .. }));
        assert_eq!(plan.tabs[1].action, TabAction::ShellInstead(Reason::Duplicate));
    }

    #[test]
    fn a_tab_of_a_removed_yolo_profile_reopens_as_a_shell_whatever_level_it_ran_at() {
        for level in [Level::Yolo, Level::Auto] {
            let tabs = vec![agent_tab("a-claude", "claude", "builder-yolo", level, Some(ID_C))];
            let plan = plan_with(&window(tabs), &everything_installed(), &disk_for_all());
            assert_eq!(
                plan.tabs[0].action,
                TabAction::ShellInstead(Reason::ProfileGone("builder-yolo".into())),
                "{level:?}"
            );
            assert!(plan.tabs[0].note().is_some());
        }
    }

    #[test]
    fn a_meeseeks_that_ran_on_its_cli_defaults_resumes_typed_at_yolo() {
        let meeseeks = Some(Box::new(crate::meeseeks::Meeseeks::default()));
        let mut tab =
            agent_tab("a-claude", "claude", Profile::DEFAULTS_ID, Level::Yolo, Some(ID_C));
        let SavedTab::Agent(saved) = &mut tab else { unreachable!("agent_tab makes an agent") };
        saved.meeseeks = meeseeks;
        let plan = plan_with(&window(vec![tab.clone()]), &everything_installed(), &disk_for_all());
        let TabAction::ResumeAgent { profile, how, .. } = &plan.tabs[0].action else {
            panic!("{:?}", plan.tabs[0].action)
        };
        assert_eq!((profile.level, profile.agent.as_str()), (Level::Yolo, "claude"));
        assert_eq!(*how, Resume::TypeOnly(Why::Yolo));
        let SavedTab::Agent(mut plain) = tab else { unreachable!("agent_tab makes an agent") };
        plain.meeseeks = None;
        let plan = plan_with(
            &window(vec![SavedTab::Agent(plain)]),
            &everything_installed(),
            &disk_for_all(),
        );
        assert_eq!(
            plan.tabs[0].action,
            TabAction::ShellInstead(Reason::ProfileGone(Profile::DEFAULTS_ID.into()))
        );
    }

    #[test]
    fn a_yolo_workflow_tab_is_not_resumed_either() {
        let mut tab = SavedAgentTab {
            dir: PathBuf::from(wt("a-claude")),
            branch: Some("agent/a-claude".into()),
            agent: "claude".into(),
            profile: "builder".into(),
            level: Level::Yolo,
            session_id: Some(ID_C.into()),
            stages: Some(PathBuf::from("/state/stages.json")),
            crashed: false,
            remote: false,
            meeseeks: None,
        };
        let mut disk = disk_for_all();
        disk.workflows.push("/state/stages.json".into());
        let yolo =
            plan_with(&window(vec![SavedTab::Agent(tab.clone())]), &everything_installed(), &disk);
        assert_eq!(yolo.tabs[0].action, TabAction::ShellInstead(Reason::Yolo));
        tab.level = Level::Auto;
        let auto = plan_with(&window(vec![SavedTab::Agent(tab)]), &everything_installed(), &disk);
        assert_eq!(
            auto.tabs[0].action,
            TabAction::ResumeWorkflow { stages: PathBuf::from("/state/stages.json") }
        );
    }

    #[test]
    fn a_workflow_tab_resumes_through_its_run_or_falls_back_when_the_file_is_unreadable() {
        let tab = SavedTab::Agent(SavedAgentTab {
            dir: PathBuf::from(wt("a-claude")),
            branch: Some("agent/a-claude".into()),
            agent: "claude".into(),
            profile: "gone-profile".into(),
            level: Level::Auto,
            session_id: None,
            stages: Some(PathBuf::from("/state/stages.json")),
            crashed: false,
            remote: false,
            meeseeks: None,
        });
        let mut disk = disk_for_all();
        let unreadable = plan_with(&window(vec![tab.clone()]), &everything_installed(), &disk);
        assert_eq!(unreadable.tabs[0].action, TabAction::ShellInstead(Reason::WorkflowUnreadable));
        disk.workflows.push("/state/stages.json".into());
        let readable = plan_with(&window(vec![tab]), &everything_installed(), &disk);
        assert_eq!(
            readable.tabs[0].action,
            TabAction::ResumeWorkflow { stages: PathBuf::from("/state/stages.json") },
            "the run decides the profile, so a renamed profile does not matter"
        );
    }

    #[test]
    fn each_way_an_agent_cannot_resume_names_its_reason() {
        let mut disk = disk_for_all();
        disk.dirs.push(wt("e"));
        let cases: Vec<(SavedTab, Reason)> = vec![
            (
                agent_tab("a-claude", "ghost", "builder", Level::Edits, Some(ID_C)),
                Reason::AgentUnknown("ghost".into()),
            ),
            (
                agent_tab("a-claude", "claude", "deleted", Level::Edits, Some(ID_C)),
                Reason::ProfileGone("deleted".into()),
            ),
            (
                agent_tab("a-claude", "claude", "gem", Level::Edits, Some(ID_C)),
                Reason::ProfileChanged { profile: "gem".into(), agent: "claude".into() },
            ),
            (agent_tab("a-claude", "claude", "builder", Level::Edits, None), Reason::NoSessionId),
            (
                agent_tab("a-claude", "claude", "builder", Level::Edits, Some("")),
                Reason::NoSessionId,
            ),
            (
                agent_tab("a-claude", "claude", "builder", Level::Edits, Some(ID_G)),
                Reason::TranscriptMissing,
            ),
            (
                agent_tab("a-claude", "mine", "mine", Level::Edits, Some(ID_C)),
                Reason::NotResumable("mine".into()),
            ),
        ];
        for (tab, reason) in cases {
            let plan = plan_with(&window(vec![tab.clone()]), &everything_installed(), &disk);
            assert_eq!(plan.tabs[0].action, TabAction::ShellInstead(reason), "{tab:?}");
        }
    }

    #[test]
    fn a_profile_is_found_by_its_display_name_too() {
        let tabs =
            vec![agent_tab("a-claude", "claude", "claude profile", Level::Edits, Some(ID_C))];
        let plan = plan_with(&window(tabs), &everything_installed(), &disk_for_all());
        assert_eq!(plan.tabs[0].action, resume("claude", Level::Edits, ID_C));
    }

    #[test]
    fn two_tabs_never_resume_the_same_session() {
        let tabs = vec![
            agent_tab("a-claude", "claude", "builder", Level::Edits, Some(ID_C)),
            agent_tab("b-gemini", "claude", "builder", Level::Edits, Some(ID_C)),
        ];
        let plan = plan_with(&window(tabs), &everything_installed(), &disk_for_all());
        assert_eq!(plan.tabs[0].action, resume("claude", Level::Edits, ID_C));
        assert_eq!(plan.tabs[1].action, TabAction::ShellInstead(Reason::Duplicate));
    }

    #[test]
    fn a_failed_check_does_not_claim_the_session_for_a_later_tab() {
        let tabs = vec![
            agent_tab("a-claude", "claude", "deleted", Level::Edits, Some(ID_C)),
            agent_tab("b-gemini", "claude", "builder", Level::Edits, Some(ID_C)),
        ];
        let plan = plan_with(&window(tabs), &everything_installed(), &disk_for_all());
        assert_eq!(plan.tabs[1].action, resume("claude", Level::Edits, ID_C));
    }

    #[test]
    fn gemini_resumes_the_latest_session_of_a_folder_so_two_tabs_there_cannot_both() {
        let shared = |id: &str| {
            SavedTab::Agent(SavedAgentTab {
                dir: PathBuf::from("/scratch"),
                branch: None,
                agent: "gemini".into(),
                profile: "gem".into(),
                level: Level::Edits,
                session_id: Some(id.into()),
                stages: None,
                crashed: false,
                remote: false,
                meeseeks: None,
            })
        };
        let other = SavedWindow {
            dir: PathBuf::from("/scratch"),
            other: true,
            tabs: vec![shared(ID_G), shared(ID_P)],
            ..window(Vec::new())
        };
        let disk = FakeDisk::with_dirs(&["/scratch"])
            .transcript("gemini", ID_G)
            .transcript("gemini", ID_P);
        let plan = plan_with(&other, &everything_installed(), &disk);
        assert_eq!(plan.tabs[0].action, resume("gemini", Level::Edits, ID_G));
        assert_eq!(plan.tabs[1].action, TabAction::ShellInstead(Reason::Duplicate));
    }

    #[test]
    fn claims_carry_across_windows_of_one_session() {
        let one =
            window(vec![agent_tab("a-claude", "claude", "builder", Level::Edits, Some(ID_C))]);
        let two = SavedWindow { dir: PathBuf::from("/work/api"), ..one.clone() };
        let disk = {
            let mut disk = disk_for_all();
            disk.dirs.push("/work/api".into());
            disk
        };
        let table = table();
        let profiles = profiles();
        let installed = everything_installed();
        let env = RestoreEnv {
            agents: &table,
            profiles: &profiles,
            installed: &installed,
            resume: ResumeAgents::Auto,
            app_crashed: false,
        };
        let all = plan(&Session { windows: vec![one, two] }, &env, &disk);
        assert_eq!(all.windows[0].tabs[0].action, resume("claude", Level::Edits, ID_C));
        assert_eq!(all.windows[1].tabs[0].action, TabAction::ShellInstead(Reason::Duplicate));
    }

    fn how_each_agent_tab_comes_back(plan: &WindowPlan) -> Vec<Option<Resume>> {
        plan.tabs
            .iter()
            .filter_map(|tab| match &tab.action {
                TabAction::ResumeAgent { how, .. } => Some(Some(*how)),
                TabAction::ShellInstead(_) => Some(None),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_mode_and_a_crash_pick_run_type_or_shell_for_every_resumable_tab() {
        let window = window(all_four_tabs());
        let disk = disk_for_all();
        let installed = everything_installed();
        let cases = [
            (ResumeAgents::Auto, false, Some(Resume::Run)),
            (ResumeAgents::Auto, true, Some(Resume::TypeOnly(Why::AppCrashed))),
            (ResumeAgents::Prefill, false, Some(Resume::TypeOnly(Why::Chosen))),
            (ResumeAgents::Prefill, true, Some(Resume::TypeOnly(Why::AppCrashed))),
            (ResumeAgents::Off, false, None),
            (ResumeAgents::Off, true, None),
        ];
        for (mode, crashed, want) in cases {
            let plan = plan_as(&window, &installed, &disk, mode, crashed);
            assert_eq!(how_each_agent_tab_comes_back(&plan), vec![want; 4], "{mode:?} {crashed}");
        }
    }

    #[test]
    fn a_tab_that_crashed_on_its_own_is_typed_and_its_neighbours_are_not() {
        let mut crashed = agent_tab("b-gemini", "gemini", "gem", Level::Edits, Some(ID_G));
        if let SavedTab::Agent(tab) = &mut crashed {
            tab.crashed = true;
        }
        let window = window(vec![
            SavedTab::Main,
            agent_tab("a-claude", "claude", "builder", Level::Edits, Some(ID_C)),
            crashed,
        ]);
        let plan = plan_with(&window, &everything_installed(), &disk_for_all());
        assert_eq!(
            how_each_agent_tab_comes_back(&plan),
            [Some(Resume::Run), Some(Resume::TypeOnly(Why::AgentCrashed))]
        );
    }

    #[test]
    fn yolo_is_typed_in_every_mode_but_off_after_a_crash_too() {
        let window =
            window(vec![agent_tab("a-claude", "claude", "builder", Level::Yolo, Some(ID_C))]);
        for mode in [ResumeAgents::Auto, ResumeAgents::Prefill, ResumeAgents::Off] {
            for crashed in [false, true] {
                let plan =
                    plan_as(&window, &everything_installed(), &disk_for_all(), mode, crashed);
                let want = match mode {
                    ResumeAgents::Off => Err(Reason::ResumeOff),
                    _ => Ok(Resume::TypeOnly(Why::Yolo)),
                };
                assert_eq!(
                    how_each_agent_tab_comes_back(&plan),
                    vec![want.clone().ok()],
                    "{mode:?} {crashed}"
                );
                if let Err(reason) = want {
                    assert_eq!(actions(&plan), vec![TabAction::ShellInstead(reason)]);
                }
            }
        }
    }

    #[test]
    fn a_typed_tab_carries_the_reason_as_its_note_and_off_says_nothing_in_the_toast() {
        let window =
            window(vec![agent_tab("a-claude", "claude", "builder", Level::Edits, Some(ID_C))]);
        let installed = everything_installed();
        let disk = disk_for_all();
        let typed = plan_as(&window, &installed, &disk, ResumeAgents::Prefill, false);
        assert_eq!(typed.tabs[0].note().as_deref(), Some(Why::Chosen.note()));
        assert_eq!(typed.toast(), None, "the tab says it itself");
        let off = plan_as(&window, &installed, &disk, ResumeAgents::Off, true);
        assert_eq!(
            off.tabs[0].note().unwrap(),
            "The agent was not resumed: resuming agents is turned off in Settings."
        );
        assert_eq!(off.toast(), None, "a choice the user made is no news");
        let auto = plan_with(&window, &installed, &disk);
        assert_eq!(auto.tabs[0].note(), None);
    }

    #[test]
    fn a_workflow_tab_is_resumed_by_its_runner_whatever_the_mode() {
        let tab = SavedAgentTab {
            dir: PathBuf::from(wt("a-claude")),
            branch: Some("agent/a-claude".into()),
            agent: "claude".into(),
            profile: "builder".into(),
            level: Level::Edits,
            session_id: None,
            stages: Some(PathBuf::from("/state/stages.json")),
            crashed: true,
            remote: false,
            meeseeks: None,
        };
        let window = window(vec![SavedTab::Agent(tab)]);
        let mut disk = disk_for_all();
        disk.workflows.push("/state/stages.json".into());
        let plan = plan_as(&window, &everything_installed(), &disk, ResumeAgents::Off, true);
        assert_eq!(
            actions(&plan),
            vec![TabAction::ResumeWorkflow { stages: PathBuf::from("/state/stages.json") }]
        );
    }

    #[test]
    fn the_toast_names_dropped_tabs_and_degraded_ones_and_is_silent_otherwise() {
        let mut disk = disk_for_all();
        disk.dirs.retain(|d| d != &wt("b-gemini"));
        let installed: Vec<String> =
            everything_installed().into_iter().filter(|a| a != "codex").collect();
        let plan = plan_with(&window(all_four_tabs()), &installed, &disk);
        assert_eq!(
            plan.toast().unwrap(),
            "Could not reopen 1 tab: b-gemini (worktree folder gone). \
             Reopened 1 tab as a shell: d-codex (`codex` is not installed)"
        );
    }

    #[test]
    fn flags_and_focus_mode_carry_into_the_plan() {
        let mut saved = window(vec![SavedTab::Main]);
        saved.focus_mode = true;
        let plan = plan_with(&saved, &everything_installed(), &FakeDisk::with_dirs(&[REPO]));
        assert!(plan.focus_mode);
        assert!(!plan.other);
        assert_eq!(plan.dir, PathBuf::from(REPO));
    }

    #[test]
    fn codex_session_ids_come_from_the_rollout_file_name() {
        let name = format!("rollout-2026-10-13T12-00-00-{ID_X}.jsonl");
        assert_eq!(
            codex_session_id(Path::new(&format!("/s/2026/10/13/{name}"))),
            Some(ID_X.into())
        );
        let real = "rollout-2026-10-13T19-57-41-01a13669-bce5-7242-a392-fd967b70ea72.jsonl";
        assert_eq!(
            codex_session_id(Path::new(real)).as_deref(),
            Some("01a13669-bce5-7242-a392-fd967b70ea72")
        );
        for bad in ["rollout-2026-10-13T12-00-00.jsonl", "other.jsonl", "rollout-short.jsonl"] {
            assert_eq!(codex_session_id(Path::new(bad)), None, "{bad}");
        }
    }

    fn launch_ctx() -> LaunchCtx {
        LaunchCtx {
            worktree: "/root/web-ab12cd/a-claude".into(),
            worktree_tab: true,
            session_id: Some("99999999-8888-7777-6666-555555555555".into()),
            remote: None,
            hooks: Some(HookListener {
                base_url: "http://127.0.0.1:5555".into(),
                token: "tok123".into(),
            }),
            mcp_config: Some("/state/tab/mcp.json".into()),
            house_style: Some("/cfg/house-style.md".into()),
            copilot_instructions_dir: Some("/cfg/copilot".into()),
            guard_bin_dir: Some("/state/guard-bin".into()),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        }
    }

    fn without_session_id(args: &[String], flag: &str) -> Vec<String> {
        let at = args.iter().position(|a| a == flag).expect("fresh launch carries the flag");
        let mut kept = args.to_vec();
        kept.drain(at..=at + 1);
        kept
    }

    #[test]
    fn resume_keeps_the_fresh_launch_environment_and_flags_and_adds_the_resume_form() {
        let table = AgentTable::builtin();
        let cases: [(&str, &[&str]); 4] = [
            ("claude", &["--resume", ID_C]),
            ("gemini", &["--resume", "latest"]),
            ("copilot", &[&format!("--resume={ID_C}")]),
            ("codex", &["resume", ID_C]),
        ];
        for (id, front) in cases {
            let agent = table.get(id).unwrap();
            let profile = profile(id, Level::Edits);
            let ctx = launch_ctx();
            let fresh = launch_command(agent, &profile, &ctx).unwrap();
            let resumed = resume_command(agent, &profile, ID_C, &ctx).unwrap();
            assert_eq!(resumed.program, fresh.program, "{id}");
            assert_eq!(resumed.env, fresh.env, "{id}");
            assert_eq!(resumed.experimental, fresh.experimental, "{id}");
            let fresh_args = match &agent.session_id_flag {
                Some(flag) => without_session_id(&fresh.args, flag),
                None => fresh.args.clone(),
            };
            let want: Vec<String> =
                front.iter().map(|s| (*s).to_owned()).chain(fresh_args).collect();
            assert_eq!(resumed.args, want, "{id}");
            assert!(!resumed.args.iter().any(|a| a == "--session-id"), "{id}");
        }
    }

    #[test]
    fn every_agent_resumes_a_yolo_tab_with_the_yolo_args_of_a_fresh_launch_and_its_resume_form() {
        let table = AgentTable::builtin();
        let cases: [(&str, &[&str]); 4] = [
            ("claude", &["--resume", ID_C]),
            ("gemini", &["--resume", "latest"]),
            ("copilot", &[&format!("--resume={ID_C}")]),
            ("codex", &["resume", ID_C]),
        ];
        for (id, front) in cases {
            let agent = table.get(id).unwrap();
            let yolo = profile(id, Level::Yolo);
            let ctx = launch_ctx();
            let fresh = launch_command(agent, &yolo, &ctx).unwrap();
            let resumed = resume_command(agent, &yolo, ID_C, &ctx).unwrap();
            let level_args = agent.level_args(Level::Yolo).unwrap();
            assert!(!level_args.is_empty(), "{id}");
            let fresh_args = match &agent.session_id_flag {
                Some(flag) => without_session_id(&fresh.args, flag),
                None => fresh.args.clone(),
            };
            let want: Vec<String> =
                front.iter().map(|s| (*s).to_owned()).chain(fresh_args).collect();
            assert_eq!(resumed.args, want, "{id}");
            assert_eq!(resumed.args[front.len()..front.len() + level_args.len()], *level_args);
            assert_eq!(resumed.program, fresh.program, "{id}");
            assert_eq!(resumed.env, fresh.env, "{id}");
        }
    }

    #[test]
    fn resume_keeps_the_never_push_guard_hooks_and_mcp_wiring_of_a_fresh_launch() {
        let table = AgentTable::builtin();
        let profile = profile("claude", Level::Edits);
        let launch =
            resume_command(table.get("claude").unwrap(), &profile, ID_C, &launch_ctx()).unwrap();
        let env = |key: &str| launch.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        assert_eq!(env("GIT_CONFIG_COUNT"), Some("1"));
        assert!(env("PATH").unwrap().starts_with("/state/guard-bin"));
        assert!(launch.args.iter().any(|a| a == "--settings"));
        assert!(launch.args.iter().any(|a| a == "--mcp-config=/state/tab/mcp.json"));
        assert!(launch.args.iter().any(|a| a == "--append-system-prompt-file"));
        assert_eq!(&launch.args[..2], ["--resume", ID_C]);
    }

    #[test]
    fn resume_refuses_yolo_outside_a_worktree_and_agents_without_a_resume_form() {
        let table = table();
        let claude = table.get("claude").unwrap();
        let yolo = profile("claude", Level::Yolo);
        let outside_yolo = LaunchCtx { worktree_tab: false, ..launch_ctx() };
        assert_eq!(
            resume_command(claude, &yolo, ID_C, &outside_yolo),
            Err(ResumeError::Launch(LaunchError::YoloOutsideWorktree))
        );
        let custom = table.get("mine").unwrap();
        assert_eq!(
            resume_command(custom, &profile("mine", Level::Edits), ID_C, &launch_ctx()),
            Err(ResumeError::NotResumable { agent: "mine".into() })
        );
        let outside = LaunchCtx { worktree_tab: false, ..launch_ctx() };
        assert_eq!(
            resume_command(claude, &profile("claude", Level::Edits), ID_C, &outside)
                .map(|l| l.program),
            Ok("claude".to_owned())
        );
    }

    #[test]
    fn a_resumed_tab_keeps_remote_control_and_the_same_session_name() {
        let table = table();
        let name = "web \u{b7} otter";
        let cases: [(&str, &str, &[&str], &[&str]); 4] = [
            ("claude", ID_C, &["--resume", ID_C], &["--remote-control", name]),
            ("copilot", ID_P, &["--resume=2c2cbc96-1111-4222-8333-444455556666"], &["--remote"]),
            ("codex", ID_X, &["resume", ID_X], &[]),
            ("gemini", ID_G, &["--resume", "latest"], &[]),
        ];
        for (agent, id, reopen, flag) in cases {
            let (agent, profile) = (table.get(agent).unwrap(), profile(agent, Level::Edits));
            let on = LaunchCtx { remote: Some(name.into()), ..launch_ctx() };
            let resumed = resume_command(agent, &profile, id, &on).unwrap();
            assert_eq!(&resumed.args[..reopen.len()], reopen, "{}", agent.id);
            let fresh = launch_command(agent, &profile, &LaunchCtx { session_id: None, ..on });
            assert_eq!(resumed.args[reopen.len()..], fresh.unwrap().args, "{}", agent.id);
            let flagged = resumed.args.windows(flag.len().max(1)).any(|window| window == flag);
            assert!(flagged || flag.is_empty(), "{} carries {flag:?}", agent.id);
            let off = resume_command(agent, &profile, id, &launch_ctx()).unwrap();
            assert!(!off.args.iter().any(|arg| arg.starts_with("--remote")), "{}", agent.id);
        }
    }
}
