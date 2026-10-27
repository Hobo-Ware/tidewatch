//! Changing what a tab runs, the pure half: the target (an agent under a profile and level, or a
//! plain shell), what blocks it as plain data (`Refusal`), and `plan`, which turns the session
//! found in the tab plus the target into what the job starts. Nothing here touches the machine.

use std::path::{Path, PathBuf};

use tidewatch_core::agents::{Agent, AgentTable, Level};
use tidewatch_core::elsewhere::{Activity, Cli, Session, is_session_id};
use tidewatch_core::profiles::{Profile, Profiles};
use tidewatch_core::restore::resumed_level;
use tidewatch_core::worktree::Worktree;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Resume the same session under `profile` (a `profiles.toml` key, or `Profile::DEFAULTS_ID`).
    /// `level: None` keeps the level the session last ran at, with the rules a restart uses.
    Agent { profile: String, level: Option<Level> },
    /// End the agent and keep a shell; the session id is kept to resume it later.
    Shell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    NothingRunning,
    ShellBusy,
    MidTurn,
    NoSessionId,
    YoloNeedsWorktree,
    OtherFolder { session: PathBuf },
    NoProfile(String),
    OtherCli { session: Cli, profile: String, agent: String },
    AgentMissing(String),
    NotResumable(String),
    Gone,
    WillNotEnd { secs: u64 },
    SignalFailed,
    ScanFailed,
}

impl Refusal {
    pub(crate) fn text(&self) -> String {
        match self {
            Self::NothingRunning => "no agent is running in this tab".to_owned(),
            Self::ShellBusy => "a command is running in this shell; let it finish first".to_owned(),
            Self::MidTurn => "the agent is working; try again when it is idle".to_owned(),
            Self::NoSessionId => "the session id could not be read".to_owned(),
            Self::YoloNeedsWorktree => "YOLO only runs in a worktree tab".to_owned(),
            Self::OtherFolder { session } => {
                format!("the session runs in {}, not in the folder of this tab", session.display())
            }
            Self::NoProfile(id) => format!("there is no profile {id}"),
            Self::OtherCli { session, profile, agent } => {
                format!("this is a {} session and {profile} runs {agent}", session.id())
            }
            Self::AgentMissing(agent) => format!("{agent} is not installed"),
            Self::NotResumable(agent) => format!("{agent} cannot resume a session"),
            Self::Gone => "it is not running as it was; nothing was ended".to_owned(),
            Self::WillNotEnd { secs } => {
                format!("it did not exit within {secs} s and was left running")
            }
            Self::SignalFailed => "the signal could not be sent; it was left running".to_owned(),
            Self::ScanFailed => "the process list could not be read; nothing was ended".to_owned(),
        }
    }
}

/// What the app knows of the tab it changes.
#[derive(Clone, Debug)]
pub(crate) struct Subject {
    pub(crate) tab: u64,
    /// The pid of the tab's pty process: the shell, or the agent when the pty runs it directly.
    pub(crate) shell_pid: u32,
    /// The repo root, or the folder of the Other group.
    pub(crate) repo: PathBuf,
    pub(crate) worktree: Option<Worktree>,
    /// The folder the tab runs in.
    pub(crate) dir: PathBuf,
    /// The CLI the tab's agent runs, when it is an agent tab.
    pub(crate) cli: Option<Cli>,
    /// The id the tab's agent was launched with, for a CLI whose scan cannot name it.
    pub(crate) known_session_id: Option<String>,
    /// A command other than the shell has the terminal.
    pub(crate) foreground_busy: bool,
    pub(crate) remote: bool,
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
    pub(crate) cwd: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    Agent(Box<AgentPlan>),
    Shell { cli: Cli, session_id: String },
}

pub(crate) fn same_folder(a: &Path, b: &Path) -> bool {
    let real = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    a == b || real(a) == real(b)
}

/// What to start for `target`, or why not. `interrupt` lets a turn in progress be cut short.
pub(crate) fn plan(
    target: &Target,
    source: Source,
    subject: &Subject,
    (profiles, agents, installed): (&Profiles, &AgentTable, &[String]),
    interrupt: bool,
) -> Result<Plan, Refusal> {
    let known = subject.known_session_id.clone().filter(|id| is_session_id(id));
    if subject.foreground_busy && !matches!(source, Source::Running(_)) {
        return Err(Refusal::ShellBusy);
    }
    let (cli, session_id, seen, cwd) = match source {
        Source::Running(session) => {
            let running = session.running().ok_or(Refusal::NothingRunning)?;
            if running.activity == Activity::Working && !interrupt {
                return Err(Refusal::MidTurn);
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
        return match (cli, session_id) {
            (Some(cli), Some(session_id)) if matches!(source, Source::Running(_)) => {
                Ok(Plan::Shell { cli, session_id })
            }
            (_, None) if matches!(source, Source::Running(_)) => Err(Refusal::NoSessionId),
            _ => Err(Refusal::NothingRunning),
        };
    };
    if matches!(source, Source::Running(_)) && session_id.is_none() {
        return Err(Refusal::NoSessionId);
    }
    if !same_folder(&cwd, &subject.dir) {
        return Err(Refusal::OtherFolder { session: cwd });
    }
    let base = if profile_id == Profile::DEFAULTS_ID {
        Profile::defaults_for(cli.map_or("claude", Cli::id), Level::Edits)
    } else {
        profiles
            .profile
            .get(profile_id)
            .cloned()
            .ok_or_else(|| Refusal::NoProfile(profile_id.clone()))?
    };
    if let Some(cli) = cli.filter(|cli| cli.id() != base.agent) {
        return Err(Refusal::OtherCli { session: cli, profile: base.name, agent: base.agent });
    }
    let agent = agents
        .get(&base.agent)
        .filter(|agent| installed.contains(&agent.id))
        .ok_or_else(|| Refusal::AgentMissing(base.agent.clone()))?;
    if session_id.is_some() && agent.resume.is_none() {
        return Err(Refusal::NotResumable(agent.id.clone()));
    }
    let in_worktree = subject.worktree.is_some();
    let level = match (level, session_id.is_some()) {
        (Some(Level::Yolo), _) if !in_worktree => return Err(Refusal::YoloNeedsWorktree),
        (Some(level), _) => *level,
        (None, true) => resumed_level(base.level, agent, in_worktree, seen),
        (None, false) => resumed_level(base.level, agent, in_worktree, Some(base.level)),
    };
    Ok(Plan::Agent(Box::new(AgentPlan {
        agent: agent.clone(),
        profile_id: profile_id.clone(),
        profile: Profile { level, ..base },
        start: session_id.map_or(Start::Fresh, Start::Resume),
        cwd,
    })))
}

#[cfg(test)]
mod tests;
