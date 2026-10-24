//! The "Running elsewhere" card's data and rules (pure). A stdusk session is never ended here: its
//! own "Move tabs to Tidewatch" moves the live pty.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tidewatch_core::agents::{Agent, AgentTable, ArgList, Level, ResumeForm};
use tidewatch_core::elsewhere::{Activity, Cli, Host, Key, Session};
use tidewatch_core::launch::LaunchCtx;
use tidewatch_core::prefs::ResumeAgents;
use tidewatch_core::profiles::{Profile, Profiles};
use tidewatch_core::restore::{TabAction, TabPlan, WindowPlan, resume_command};
use tidewatch_core::resume_mode::{Ended, decide};
use tidewatch_core::session::{SavedAgentTab, SavedTab};
use tidewatch_core::worktree::Worktree;

use crate::screen_kit::cli_short_name;
use crate::takeover_group::Ident;
use crate::welcome::shown_path;

pub(crate) const SCAN_EVERY_S: f64 = 5.0;
pub(crate) const RECENT_PER_FOLDER: usize = 3;
pub(crate) const RECENT_ROWS: usize = 5;
pub(crate) const END_WAIT_S: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Idle,
    Working,
    Quiet,
    NotRunning,
}

impl Status {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Working => "Working",
            Self::Quiet => "Quiet",
            Self::NotRunning => "not running",
        }
    }
}

pub(crate) fn status_of(session: &Session) -> Status {
    match session.running().map(|running| running.activity) {
        Some(Activity::Idle) => Status::Idle,
        Some(Activity::Working) => Status::Working,
        Some(Activity::Quiet) => Status::Quiet,
        None => Status::NotRunning,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Offer {
    MoveHere,
    /// In stdusk: the button explains how, it does not end anything.
    MoveLive,
    MoveWhenIdle,
    /// Armed: moves by itself once idle; the button cancels.
    Waiting,
    Moving,
    ResumeHere,
    Blocked(String),
}

impl Offer {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::MoveLive => "Move live",
            Self::MoveWhenIdle => "Move when idle",
            Self::Waiting => "Cancel wait",
            Self::Moving => "Moving...",
            Self::ResumeHere => "Resume here",
            Self::MoveHere | Self::Blocked(_) => "Move here",
        }
    }
}

pub(crate) struct Env<'a> {
    pub(crate) agents: &'a AgentTable,
    pub(crate) profiles: Option<&'a Profiles>,
    pub(crate) installed: &'a dyn Fn(&str) -> bool,
    pub(crate) now_ms: i64,
    pub(crate) home: Option<&'a Path>,
}

fn resumable(env: &Env, session: &Session) -> Result<(), String> {
    let id = session.cli.id();
    let agent = env.agents.get(id).ok_or_else(|| format!("{id} is not configured"))?;
    if !(env.installed)(id) {
        return Err(format!("{} is not installed", agent.name));
    }
    match (agent.resume, &session.session_id) {
        (None, _) => Err(format!("{} cannot resume a session", agent.name)),
        (Some(ResumeForm::FlagLatest(_)), _) | (Some(_), Some(_)) => Ok(()),
        (Some(_), None) => Err("its session id could not be found".to_owned()),
    }
}

fn in_stdusk(session: &Session) -> bool {
    session.running().is_some_and(|running| running.host == Host::Stdusk)
}

pub(crate) fn is_yolo(session: &Session) -> bool {
    session.flags.level == Some(Level::Yolo)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Armed {
    pub(crate) session: Session,
    pub(crate) worktree: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MoveSpec {
    pub(crate) session: Session,
    pub(crate) worktree: bool,
    pub(crate) allow_quiet: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Confirm {
    pub(crate) sessions: Vec<Session>,
    pub(crate) worktree: bool,
    pub(crate) batch: bool,
}

pub(crate) struct Scanned {
    pub(crate) sessions: Vec<Session>,
    pub(crate) roots: HashMap<PathBuf, Option<PathBuf>>,
    pub(crate) idents: HashMap<PathBuf, Option<Ident>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CardAction {
    Press(Key),
    MoveAllIdle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    None,
    StdduskHint,
}

#[derive(Default)]
pub(crate) struct Takeover {
    sessions: Vec<Session>,
    roots: HashMap<PathBuf, Option<PathBuf>>,
    idents: HashMap<PathBuf, Option<Ident>>,
    last_scan: Option<f64>,
    in_flight: bool,
    was_shown: bool,
    confirm: Option<Confirm>,
    armed: Vec<Armed>,
    moving: Vec<Key>,
    reveal: bool,
}

impl Takeover {
    pub(crate) fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub(crate) fn dialog_open(&self) -> bool {
        self.confirm.is_some()
    }

    pub(crate) fn confirm_mut(&mut self) -> Option<&mut Confirm> {
        self.confirm.as_mut()
    }

    pub(crate) fn watching(&self) -> bool {
        !self.armed.is_empty()
    }

    pub(crate) fn ask_to_reveal(&mut self) {
        self.reveal = true;
        self.last_scan = None;
    }

    pub(crate) fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal)
    }

    /// Whether a scan should start now, and if so it counts as started: when the card shows (or a
    /// watch waits for idle), the first time it shows and then every few seconds, one at a time.
    pub(crate) fn begin_scan(&mut self, shown: bool, now: f64) -> bool {
        let opened = shown && !self.was_shown;
        self.was_shown = shown;
        if self.in_flight || !(shown || self.watching()) {
            return false;
        }
        let due = opened || self.last_scan.is_none_or(|at| now - at >= SCAN_EVERY_S);
        if due {
            self.in_flight = true;
            self.last_scan = Some(now);
        }
        due
    }

    /// Seconds until the next scan is due, while something wants one and none is under way (a
    /// finished scan wakes the window itself).
    pub(crate) fn next_scan_in(&self, shown: bool, now: f64) -> Option<f64> {
        (!self.in_flight && (shown || self.watching()))
            .then(|| self.last_scan.map_or(0.0, |at| (at + SCAN_EVERY_S - now).max(0.0)))
    }

    pub(crate) fn scan_failed(&mut self) {
        self.in_flight = false;
    }

    /// Takes a scan's result. Returns the watches whose session is idle now, which start moving.
    pub(crate) fn finish_scan(&mut self, scanned: Scanned) -> Vec<MoveSpec> {
        self.in_flight = false;
        self.roots.extend(scanned.roots);
        self.idents.extend(scanned.idents);
        self.sessions = scanned.sessions;
        self.moving.retain(|key| self.sessions.iter().any(|s| s.key().as_ref() == Some(key)));
        let mut ready = Vec::new();
        let sessions = &self.sessions;
        self.armed.retain(|armed| {
            let Some(key) = armed.session.key() else { return false };
            let now = sessions.iter().find(|s| s.key().as_ref() == Some(&key));
            match now.map(status_of) {
                Some(Status::Idle) => {
                    ready.push(MoveSpec {
                        session: now.cloned().unwrap_or_else(|| armed.session.clone()),
                        worktree: armed.worktree,
                        allow_quiet: false,
                    });
                    false
                }
                Some(Status::Working) => true,
                _ => false,
            }
        });
        self.moving.extend(ready.iter().filter_map(|spec| spec.session.key()));
        ready
    }

    pub(crate) fn known_roots(&self) -> HashMap<PathBuf, Option<PathBuf>> {
        self.roots.clone()
    }

    pub(crate) fn known_idents(&self) -> HashMap<PathBuf, Option<Ident>> {
        self.idents.clone()
    }

    pub(crate) fn ident_of(&self, dir: &Path) -> Option<&Ident> {
        self.idents.get(dir).and_then(Option::as_ref)
    }

    pub(crate) fn root_of(&self, session: &Session) -> Option<&Path> {
        self.roots.get(&session.cwd).and_then(Option::as_deref)
    }

    pub(crate) fn is_armed(&self, key: &Key) -> bool {
        self.armed.iter().any(|armed| armed.session.key().as_ref() == Some(key))
    }

    pub(crate) fn is_moving(&self, key: &Key) -> bool {
        self.moving.contains(key)
    }

    pub(crate) fn offer(&self, env: &Env, session: &Session) -> Offer {
        let Some(key) = session.key() else { return Offer::Blocked("no session id".to_owned()) };
        if self.is_moving(&key) {
            return Offer::Moving;
        }
        if self.is_armed(&key) {
            return Offer::Waiting;
        }
        if let Err(why) = resumable(env, session) {
            return Offer::Blocked(why);
        }
        match status_of(session) {
            Status::NotRunning => Offer::ResumeHere,
            _ if in_stdusk(session) => Offer::MoveLive,
            Status::Working => Offer::MoveWhenIdle,
            Status::Idle | Status::Quiet => Offer::MoveHere,
        }
    }

    pub(crate) fn idle_movable(&self, env: &Env) -> Vec<Session> {
        self.sessions
            .iter()
            .filter(|s| status_of(s) == Status::Idle && !is_yolo(s))
            .filter(|s| self.offer(env, s) == Offer::MoveHere)
            .cloned()
            .collect()
    }

    pub(crate) fn counts(&self) -> (usize, usize) {
        let running = self.sessions.iter().filter(|s| s.running().is_some()).count();
        let idle = self.sessions.iter().filter(|s| status_of(s) == Status::Idle).count();
        (running, idle)
    }

    pub(crate) fn press(&mut self, env: &Env, key: &Key) -> Effect {
        let Some(session) = self.sessions.iter().find(|s| s.key().as_ref() == Some(key)).cloned()
        else {
            return Effect::None;
        };
        match self.offer(env, &session) {
            Offer::MoveLive => Effect::StdduskHint,
            Offer::Waiting => {
                self.armed.retain(|armed| armed.session.key().as_ref() != Some(key));
                Effect::None
            }
            Offer::MoveHere | Offer::MoveWhenIdle | Offer::ResumeHere => {
                self.confirm =
                    Some(Confirm { sessions: vec![session], worktree: false, batch: false });
                Effect::None
            }
            Offer::Moving | Offer::Blocked(_) => Effect::None,
        }
    }

    pub(crate) fn open_batch(&mut self, env: &Env) -> bool {
        let sessions = self.idle_movable(env);
        let any = !sessions.is_empty();
        if any {
            self.confirm = Some(Confirm { sessions, worktree: false, batch: true });
        }
        any
    }

    pub(crate) fn cancel(&mut self) {
        self.confirm = None;
    }

    /// The person pressed Move: what to start now, and what to arm for later.
    pub(crate) fn accept(&mut self) -> Vec<MoveSpec> {
        let Some(confirm) = self.confirm.take() else { return Vec::new() };
        let mut start = Vec::new();
        for session in confirm.sessions {
            let Some(key) = session.key() else { continue };
            let worktree = confirm.worktree && !confirm.batch;
            if status_of(&session) == Status::Working {
                self.armed.push(Armed { session, worktree });
                continue;
            }
            let allow_quiet = status_of(&session) == Status::Quiet;
            self.moving.push(key);
            start.push(MoveSpec { session, worktree, allow_quiet });
        }
        start
    }

    pub(crate) fn moved(&mut self, key: &Key) {
        self.moving.retain(|moving| moving != key);
    }

    pub(crate) fn rows(&self, env: &Env) -> Vec<Row> {
        self.sessions
            .iter()
            .filter_map(|session| {
                let key = session.key()?;
                let level = session.flags.level.unwrap_or(Level::Ask);
                Some(Row {
                    key,
                    cli_id: session.cli.id().to_owned(),
                    name: session.display_name(env.now_ms),
                    folder: shown_path(&session.cwd, env.home),
                    host: session
                        .running()
                        .map_or_else(|| "not running".to_owned(), |r| r.host.label().to_owned()),
                    level: level.label().to_owned(),
                    yolo: level == Level::Yolo,
                    status: status_of(session),
                    offer: self.offer(env, session),
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) key: Key,
    pub(crate) cli_id: String,
    pub(crate) name: String,
    pub(crate) folder: String,
    pub(crate) host: String,
    pub(crate) level: String,
    pub(crate) yolo: bool,
    pub(crate) status: Status,
    pub(crate) offer: Offer,
}

pub(crate) fn order(sessions: &mut [Session]) {
    let rank = |session: &Session| match status_of(session) {
        Status::Idle => 0,
        Status::Working => 1,
        Status::Quiet => 2,
        Status::NotRunning => 3,
    };
    sessions.sort_by_key(|s| (rank(s), std::cmp::Reverse(s.since_ms)));
}

/// The profile a moved session resumes with: the session's own level, model, effort and other
/// flags on top of the first profile of its CLI (its name, the push guard and house style).
pub(crate) fn resolve_profile(
    profiles: Option<&Profiles>,
    agent: &Agent,
    session: &Session,
) -> (String, Profile) {
    let level = session.flags.level.unwrap_or(Level::Ask);
    let of_cli = || {
        profiles
            .into_iter()
            .flat_map(|profiles| profiles.profile.iter())
            .filter(|(_, profile)| profile.agent == agent.id)
    };
    let base = of_cli().find(|(_, profile)| profile.level == level).or_else(|| of_cli().next());
    let (id, name, never_push, house_style) = base.map_or_else(
        || (String::new(), agent.name.clone(), true, true),
        |(id, profile)| (id.clone(), profile.name.clone(), profile.never_push, profile.house_style),
    );
    let profile = Profile {
        name,
        agent: agent.id.clone(),
        model: session.flags.model.clone(),
        effort: session.flags.effort.clone(),
        level,
        setup: String::new(),
        never_push,
        extra_args: ArgList(session.flags.extra.clone()),
        house_style,
    };
    (id, profile)
}

/// The resume command as the dialog shows it: the CLI's resume arguments and the flags the session
/// had. Tidewatch adds its hooks and notes wiring when it runs it.
pub(crate) fn preview_command(env: &Env, session: &Session) -> Option<String> {
    let agent = env.agents.get(session.cli.id())?;
    let (_, profile) = resolve_profile(env.profiles, agent, session);
    let id = session.session_id.as_deref().unwrap_or("latest");
    let ctx =
        LaunchCtx { worktree: session.cwd.clone(), worktree_tab: true, ..LaunchCtx::default() };
    resume_command(agent, &profile, id, &ctx).ok().map(|launch| launch.shell_line())
}

pub(crate) fn move_blocked(
    session: &Session,
    worktree: bool,
    offer_worktree: bool,
) -> Option<&'static str> {
    if !is_yolo(session) || worktree {
        return None;
    }
    Some(if offer_worktree {
        "YOLO only runs in a worktree. Tick the box below to move it into a new one."
    } else {
        "YOLO only runs in a worktree, and this session cannot be moved into one from here. Nothing is ended."
    })
}

/// The box "Move it into a new worktree first" is offered for a Claude session in a git repo:
/// `claude --resume <id>` found the session from another folder (claude 2.1.301, print mode, and
/// 2.1.296 in PLAN.md), which is not known for the other CLIs.
pub(crate) fn can_use_worktree(takeover: &Takeover, session: &Session) -> bool {
    session.cli == Cli::Claude && takeover.root_of(session).is_some()
}

pub(crate) fn dialog_lines(session: &Session, worktree: bool, home: Option<&Path>) -> [String; 3] {
    let cli = cli_short_name(session.cli.id());
    let folder = shown_path(&session.cwd, home);
    let first = match session.running() {
        Some(running) => {
            format!("Ends {cli} in {}. The window stays open at a shell.", running.host.label())
        }
        None => "Nothing is ended: no process holds this conversation.".to_owned(),
    };
    let place = if worktree { format!("a new worktree of {folder}") } else { folder };
    [
        first,
        format!("Opens a tab in {place} and resumes the same conversation with the same flags."),
        "The Reader shows the whole history, and hooks, notes and PR cards work from here on."
            .to_owned(),
    ]
}

/// The one-tab plan that opens a moved session through the restore path, so it starts (or, for a
/// YOLO session, waits typed for Enter) exactly like a tab coming back after a restart. `None` when
/// the CLI is not in the table. The tab runs in the folder the session began in, or in the new
/// worktree.
pub(crate) fn window_plan(
    agents: &AgentTable,
    profiles: Option<&Profiles>,
    spec: &MoveSpec,
    workspace: (&Path, bool),
    worktree: Option<Worktree>,
) -> Option<WindowPlan> {
    let session = &spec.session;
    let agent = agents.get(session.cli.id())?;
    let (profile_id, profile) = resolve_profile(profiles, agent, session);
    let session_id = session.session_id.clone().unwrap_or_else(|| "latest".to_owned());
    let dir = worktree.as_ref().map_or_else(|| session.cwd.clone(), |made| made.path.clone());
    let how = decide(ResumeAgents::Auto, Ended::Clean, profile.level).ok()?;
    let saved = SavedTab::Agent(SavedAgentTab {
        dir: dir.clone(),
        branch: worktree.as_ref().map(|made| made.branch.clone()),
        agent: agent.id.clone(),
        profile: if profile_id.is_empty() { profile.name.clone() } else { profile_id },
        level: profile.level,
        session_id: Some(session_id.clone()),
        stages: None,
        crashed: false,
        remote: false,
        meeseeks: None,
    });
    let action = TabAction::ResumeAgent {
        agent: agent.id.clone(),
        profile: Box::new(profile),
        session_id,
        how,
    };
    Some(WindowPlan {
        dir: workspace.0.to_path_buf(),
        other: workspace.1,
        focus_mode: false,
        active: 0,
        tabs: vec![TabPlan { saved, dir, worktree, action, park: None }],
        skipped: Vec::new(),
    })
}

pub(crate) fn moved_text(session: &Session, now_ms: i64) -> String {
    format!("Moved {} into Tidewatch", session.display_name(now_ms))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DialogItem {
    pub(crate) name: String,
    pub(crate) folder: String,
    pub(crate) command: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DialogView {
    pub(crate) title: String,
    pub(crate) lines: [String; 3],
    pub(crate) items: Vec<DialogItem>,
    pub(crate) notes: Vec<String>,
    pub(crate) offer_worktree: bool,
    pub(crate) worktree: bool,
    pub(crate) blocked: Option<&'static str>,
    pub(crate) confirm: String,
}

impl Takeover {
    pub(crate) fn dialog_view(&self, env: &Env) -> Option<DialogView> {
        let confirm = self.confirm.as_ref()?;
        let first = confirm.sessions.first()?;
        let items: Vec<DialogItem> = confirm
            .sessions
            .iter()
            .map(|session| DialogItem {
                name: session.display_name(env.now_ms),
                folder: shown_path(&session.cwd, env.home),
                command: preview_command(env, session),
            })
            .collect();
        let status = status_of(first);
        let (title, confirm_label) = if confirm.batch {
            (format!("Move {} idle sessions to Tidewatch?", items.len()), "Move all".to_owned())
        } else {
            match status {
                Status::NotRunning => {
                    (format!("Resume {} in Tidewatch?", items[0].name), "Resume".to_owned())
                }
                Status::Working => (
                    format!("Move {} to Tidewatch when it is idle?", items[0].name),
                    "Move when idle".to_owned(),
                ),
                _ => (format!("Move {} to Tidewatch?", items[0].name), "Move".to_owned()),
            }
        };
        let offer_worktree = !confirm.batch && can_use_worktree(self, first);
        let worktree = confirm.worktree && offer_worktree;
        let mut lines = dialog_lines(first, worktree, env.home);
        if status == Status::Working {
            lines[0] = lines[0].replacen("Ends", "Waits until it is idle, then ends", 1);
        }
        let mut notes = Vec::new();
        if !confirm.batch && status == Status::Quiet {
            notes.push(
                "Its status is not known. Moving ends it, so make sure it is not in the middle of a turn."
                    .to_owned(),
            );
        }
        if confirm.batch {
            notes.push(
                "Each one is ended in the app it runs in. YOLO sessions are not included."
                    .to_owned(),
            );
        }
        let blocked =
            if confirm.batch { None } else { move_blocked(first, worktree, offer_worktree) };
        Some(DialogView {
            title,
            lines,
            items,
            notes,
            offer_worktree,
            worktree,
            blocked,
            confirm: confirm_label,
        })
    }
}

#[cfg(test)]
mod tests;
