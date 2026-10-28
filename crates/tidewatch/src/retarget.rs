//! Changing what a tab runs, the pure half: the target, where the tab lands and the plan.

use std::path::{Path, PathBuf};

use tidewatch_core::agents::{Agent, AgentTable, Level, ResumeForm};
use tidewatch_core::elsewhere::{Activity, Cli, Session, is_session_id};
use tidewatch_core::profiles::{Profile, Profiles};
use tidewatch_core::worktree::Worktree;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Resume the same session under `profile` (a `profiles.toml` key, or `Profile::DEFAULTS_ID`).
    /// `level: None` keeps the level the session last ran at, with the rules a restart uses.
    Agent { profile: String, level: Option<Level> },
    /// End the agent and keep a shell; the session id is kept to resume it later.
    Shell,
}

/// What can still stop a change: a wait that ends by itself, or a real error with what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The agent is working; the change goes ahead when its turn ends.
    MidTurn,
    /// The tab is still being set up or removed; the change goes ahead when it settles.
    Settling,
    ScanFailed,
    Changed,
    AgentMissing(String),
}

impl Failure {
    pub(crate) fn waits(&self) -> bool {
        matches!(self, Self::MidTurn | Self::Settling)
    }

    pub(crate) fn text(&self) -> String {
        match self {
            Self::MidTurn => "The agent is working; the tab changes when its turn ends.".to_owned(),
            Self::Settling => "The tab is still being set up; it changes when that is done.".to_owned(),
            Self::ScanFailed => {
                "The process list could not be read, so nothing was changed. Press Change again."
                    .to_owned()
            }
            Self::Changed => {
                "The agent changed while it was being stopped, so nothing else was touched. Press Change again."
                    .to_owned()
            }
            Self::AgentMissing(agent) => format!(
                "{agent} is not installed. Install it (Settings, Profiles shows how), then change the tab again."
            ),
        }
    }
}

/// What the window holds, to place a tab that follows its session into another folder.
#[derive(Clone, Debug, Default)]
pub(crate) struct Window {
    /// The repo roots of the tab groups.
    pub(crate) repos: Vec<PathBuf>,
    /// The folders the other tabs show.
    pub(crate) shown: Vec<PathBuf>,
}

/// What the app knows of the tab it changes.
#[derive(Clone, Debug)]
pub(crate) struct Subject {
    pub(crate) tab: u64,
    /// The pid of the tab's pty process: the shell, or the agent when the pty runs it directly.
    /// `None` when the tab has no terminal (a form, a failure, a summary).
    pub(crate) shell_pid: Option<u32>,
    /// The repo root, or the folder of the Other group.
    pub(crate) repo: PathBuf,
    pub(crate) worktree: Option<Worktree>,
    /// The folder the tab runs in.
    pub(crate) dir: PathBuf,
    /// The CLI the tab's agent runs, when it is an agent tab.
    pub(crate) cli: Option<Cli>,
    /// The id the tab's agent was launched with, for a CLI whose scan cannot name it.
    pub(crate) known_session_id: Option<String>,
    pub(crate) remote: bool,
    /// The pinned main tab: it stays in its group.
    pub(crate) pinned: bool,
    /// The tab is being set up or removed: nothing can swap its pane yet.
    pub(crate) settling: bool,
    pub(crate) window: Window,
}

/// What git says of a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Folder {
    /// The main checkout of the repo the folder belongs to; `None` outside git.
    pub(crate) main: Option<PathBuf>,
    pub(crate) branch: Option<String>,
    /// A worktree Tidewatch made for that repo.
    pub(crate) made_by_tidewatch: bool,
    /// For a folder outside git: the repo its session worked in most, else the newest repo one
    /// level down.
    pub(crate) worked_in: Option<PathBuf>,
}

/// Where the converted tab runs: the folder, its worktree (when Tidewatch made it and no other tab
/// shows it), and the repo whose group holds the tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Landing {
    pub(crate) dir: PathBuf,
    pub(crate) worktree: Option<Worktree>,
    pub(crate) repo: PathBuf,
}

/// Where an agent starts: in a folder that exists (`Stay`), or in a worktree made for it on a fresh
/// `agent/<name>` branch of `repo` (`Fresh`; `from` is the folder the session ran in).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Home {
    Stay(Landing),
    Fresh { repo: PathBuf, from: PathBuf },
}

/// What the tab has to resume from.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Source<'a> {
    /// An agent process runs in the tab now.
    Running(&'a Session),
    /// Nothing runs; a session the tab ended earlier can be resumed.
    Stored {
        cli: Cli,
        session_id: &'a str,
    },
    Nothing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    Resume(String),
    Fresh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentPlan {
    pub(crate) agent: Agent,
    pub(crate) profile_id: String,
    pub(crate) profile: Profile,
    pub(crate) start: Start,
    pub(crate) home: Home,
    /// Why a new session starts where the conversation could not continue.
    pub(crate) note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    Agent(Box<AgentPlan>),
    Shell { remembered: Option<(Cli, String)> },
}

pub(crate) fn same_folder(a: &Path, b: &Path) -> bool {
    let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    a == b || real(a) == real(b)
}

pub(crate) fn is_inside(dir: &Path, parent: &Path) -> bool {
    let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    dir.starts_with(parent) || real(dir).starts_with(real(parent))
}

/// The tab follows its session: where the session runs is where the tab lands, in the group of
/// the repo that folder belongs to. Only a worktree Tidewatch made and no other tab shows is
/// owned by the tab; any other folder is left alone when the tab closes.
pub(crate) fn landing(cwd: &Path, subject: &Subject, folder: &Folder) -> Landing {
    let stay = Landing {
        dir: cwd.to_path_buf(),
        worktree: subject.worktree.clone(),
        repo: subject.repo.clone(),
    };
    if is_inside(cwd, &subject.dir) {
        return stay;
    }
    let loose = Landing { worktree: None, ..stay };
    let Some(main) = folder.main.as_ref().filter(|_| !subject.pinned) else { return loose };
    let repo =
        subject.window.repos.iter().find(|open| same_folder(open, main)).unwrap_or(main).clone();
    let shown = subject.window.shown.iter().any(|dir| same_folder(dir, cwd));
    let worktree = (folder.made_by_tidewatch && !shown)
        .then(|| {
            folder.branch.clone().map(|branch| Worktree {
                repo: repo.clone(),
                path: cwd.to_path_buf(),
                branch,
            })
        })
        .flatten();
    Landing { dir: cwd.to_path_buf(), worktree, repo }
}

/// Where the converted tab goes. A session in the tab's own agent worktree, or in a worktree
/// Tidewatch made, keeps its folder. Any other folder inside a repo, and a plain folder with a repo
/// the session worked in, gets a fresh agent worktree of that repo. Only a folder with no repo at
/// all stays a folder tab.
pub(crate) fn home(cwd: &Path, subject: &Subject, folder: &Folder) -> Home {
    let stay = landing(cwd, subject, folder);
    let in_own_worktree = subject.worktree.is_some() && is_inside(cwd, &subject.dir);
    if in_own_worktree || (folder.made_by_tidewatch && folder.main.is_some()) {
        return Home::Stay(stay);
    }
    let Some(main) = folder.main.as_ref().or(folder.worked_in.as_ref()) else {
        return Home::Stay(stay);
    };
    let repo =
        subject.window.repos.iter().find(|open| same_folder(open, main)).unwrap_or(main).clone();
    Home::Fresh { repo, from: cwd.to_path_buf() }
}

/// The level that runs: the one asked for, or, when the CLI has no flags for it, the nearest it
/// has, preferring a lower level to a higher one.
pub(crate) fn fit_level(level: Level, unsupported: &[Level]) -> Level {
    let open = |candidate: &Level| !unsupported.contains(candidate);
    if open(&level) {
        return level;
    }
    Level::ALL
        .into_iter()
        .rev()
        .filter(open)
        .find(|candidate| *candidate < level)
        .or_else(|| Level::ALL.into_iter().filter(open).find(|candidate| *candidate > level))
        .unwrap_or(level)
}

pub(crate) fn unsupported_levels(agent: &Agent) -> Vec<Level> {
    Level::ALL.into_iter().filter(|level| agent.level_args(*level).is_none()).collect()
}

/// What to start for `target`, or what to wait for. `interrupt` lets a turn in progress be cut
/// short. A session in another CLI, one that cannot resume or one with no readable id starts a new
/// session in the same folder, and says why in `note`.
pub(crate) fn plan(
    target: &Target,
    source: Source,
    subject: &Subject,
    folder: &Folder,
    (profiles, agents, installed): (&Profiles, &AgentTable, &[String]),
    interrupt: bool,
) -> Result<Plan, Failure> {
    if subject.settling {
        return Err(Failure::Settling);
    }
    let known = subject.known_session_id.clone().filter(|id| is_session_id(id));
    let (cli, session_id, seen, cwd) = match source {
        Source::Running(session) => {
            let working = session.running().is_some_and(|r| r.activity == Activity::Working);
            if working && !interrupt {
                return Err(Failure::MidTurn);
            }
            let id = session.session_id.clone().filter(|id| is_session_id(id)).or(known);
            (Some(session.cli), id, session.flags.level, session.cwd.clone())
        }
        Source::Stored { cli, session_id } => {
            (Some(cli), Some(session_id.to_owned()), None, subject.dir.clone())
        }
        Source::Nothing => (None, None, None, subject.dir.clone()),
    };
    let Target::Agent { profile: profile_id, level } = target else {
        return Ok(Plan::Shell { remembered: cli.zip(session_id) });
    };
    let (profile_id, base) = match profiles.profile.get(profile_id) {
        Some(profile) if profile_id != Profile::DEFAULTS_ID => {
            (profile_id.clone(), profile.clone())
        }
        _ => (
            Profile::DEFAULTS_ID.to_owned(),
            Profile::defaults_for(cli.map_or("claude", Cli::id), Level::Edits),
        ),
    };
    let agent = agents
        .get(&base.agent)
        .filter(|agent| installed.contains(&agent.id))
        .ok_or_else(|| Failure::AgentMissing(base.agent.clone()))?;
    let home = home(&cwd, subject, folder);
    let moved = matches!(&home, Home::Fresh { .. });
    let (resume, note) =
        resumable(cli, session_id, agent, matches!(source, Source::Running(_)), moved);
    let wanted = match (level, resume.is_some()) {
        (Some(level), _) => *level,
        (None, true) => seen.unwrap_or(base.level),
        (None, false) => base.level,
    };
    let level = fit_level(wanted, &unsupported_levels(agent));
    Ok(Plan::Agent(Box::new(AgentPlan {
        agent: agent.clone(),
        profile_id,
        profile: Profile { level, ..base },
        start: resume.map_or(Start::Fresh, Start::Resume),
        home,
        note,
    })))
}

fn resumable(
    cli: Option<Cli>,
    session_id: Option<String>,
    agent: &Agent,
    running: bool,
    moved: bool,
) -> (Option<String>, Option<String>) {
    match (cli, session_id) {
        (Some(cli), _)
            if moved
                && cli.id() == agent.id
                && matches!(agent.resume, Some(ResumeForm::FlagLatest(_))) =>
        {
            (
                None,
                Some(format!(
                    "{} resumes only the newest conversation of the folder it ran in, so a new session started in the new worktree. The old one stays on disk.",
                    agent.name
                )),
            )
        }
        (Some(_), None) if matches!(agent.resume, Some(ResumeForm::FlagLatest(_))) => {
            (Some("latest".to_owned()), None)
        }
        (Some(cli), _) if cli.id() != agent.id => (
            None,
            Some(format!(
                "{} cannot continue a {} conversation, so a new session started. The old one stays on disk.",
                agent.name,
                cli.id()
            )),
        ),
        (Some(_), None) if running => (
            None,
            Some(
                "The conversation's id could not be read, so a new session started in its folder. The old one stays on disk."
                    .to_owned(),
            ),
        ),
        (_, id) => (id, None),
    }
}

#[cfg(test)]
mod tests;
