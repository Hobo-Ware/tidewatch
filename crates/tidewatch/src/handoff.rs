//! Restarting for an update without stopping what runs in the tabs. The window that restarts
//! launches its replacement and sends it the layout of the window and the live pty of every tab
//! (`stdusk_core::handoff` is the transport); the replacement builds the same tabs around those
//! ptys, so every shell and every agent keeps running where it was.
//!
//! Terminals travel in one fixed order: groups in order, tabs in order within a group, only the tabs
//! whose entry is `live`. Both sides derive that order from the [`Handover`] alone, and the receiver
//! pairs terminal N with the Nth live entry by position, so nothing else identifies a terminal.
//!
//! This module is the pure half: what the layout holds, which tabs can travel, how a layout and its
//! terminals become tabs again and how a launch decides between adopting, a normal start and giving
//! way. The window, the sockets and the launch of the replacement are in `app/restart.rs`.

use std::ffi::OsString;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use stdusk_core::handoff::{Fallback, Layout, PaneMeta, capture_pane, fallback};
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::listener::Credential;
use tidewatch_core::runner::Runner;
use tidewatch_core::worktree::Worktree;

use crate::agent_run::AgentRun;
use crate::pane::{AgentTab, Body, FirstPrompt, Launched, Pane};
use crate::pty;
use crate::reader::Reader;
use crate::remote_session::RemoteSession;
use crate::repo::Workspace;
use crate::tabs::{Action, Tab, Tabs};
use crate::workflow_run::TabRun;

pub(crate) type Incoming = stdusk_core::handoff::Incoming<Handover>;

const FORWARDED_ENV: [&str; 2] = ["TIDEWATCH_HOME", "TIDEWATCH_BACKGROUND"];
const NOT_INHERITED_ENV: [&str; 5] = [
    "TIDEWATCH_DEV_RESTART",
    "TIDEWATCH_DEV_AGENT",
    "TIDEWATCH_DEV_WORKTREE",
    "TIDEWATCH_DEV_WORKFLOW",
    "TIDEWATCH_DEV_RESUME",
];

/// Why a window cannot hand its tabs over right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blocker {
    TabInTransition,
    WorkflowBusy,
}

impl Blocker {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::TabInTransition => "a tab is still being made or removed",
            Self::WorkflowBusy => "a workflow stage is starting or its tests are running",
        }
    }
}

/// The first reason the window must stay as it is: work in this process that no other can finish.
pub(crate) fn blocker<T>(tabs: &Tabs<Pane<T>>) -> Option<Blocker> {
    tabs.items().iter().find_map(|tab| {
        let pane = &tab.pane;
        if matches!(pane.body, Body::Creating | Body::Removing) {
            return Some(Blocker::TabInTransition);
        }
        let busy = pane
            .agent
            .as_ref()
            .and_then(|agent| agent.workflow.as_deref())
            .is_some_and(TabRun::busy);
        busy.then_some(Blocker::WorkflowBusy)
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Handover {
    pub(crate) groups: Vec<Group>,
    pub(crate) active_group: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) focus: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) welcome: bool,
    /// The loopback port of the hook listener. The agents that keep running report to it, so the
    /// replacement serves the same one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) hook_port: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Group {
    pub(crate) dir: PathBuf,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) other: bool,
    #[serde(default)]
    pub(crate) selected: usize,
    #[serde(default, rename = "tab")]
    pub(crate) tabs: Vec<TabEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TabEntry {
    pub(crate) title: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) pinned: bool,
    /// A pty travels with this tab. A tab that failed in a worktree has none: it gets a fresh
    /// shell in the worktree, so the work is not orphaned.
    #[serde(default)]
    pub(crate) live: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) worktree: Option<Worktree>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<AgentEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AgentEntry {
    pub(crate) agent_id: String,
    pub(crate) profile_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) launched: Option<Launched>,
    #[serde(default)]
    pub(crate) first_prompt: FirstPrompt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notes_file: Option<PathBuf>,
    /// What the agent reports with. Absent when the old window had no listener.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) credential: Option<Credential>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) workflow: Option<WorkflowEntry>,
    /// The name of the remote session the running agent has, when remote control is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) remote: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkflowEntry {
    pub(crate) stages: PathBuf,
    pub(crate) test_timeout_secs: u64,
    pub(crate) generation: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) blurb: String,
}

impl Handover {
    fn live(&self) -> impl Iterator<Item = &TabEntry> {
        self.groups.iter().flat_map(|group| group.tabs.iter()).filter(|tab| tab.live)
    }
}

impl Layout for Handover {
    fn pane_count(&self) -> usize {
        self.live().count()
    }
}

fn keeps<T>(tab: &Tab<Pane<T>>) -> bool {
    match &tab.pane.body {
        Body::Shell(_) => true,
        Body::Failed(_) => tab.pane.worktree.is_some(),
        Body::Creating | Body::Removing | Body::Draft(_) => false,
    }
}

fn agent_entry(tab: &AgentTab, credential: Option<Credential>) -> AgentEntry {
    AgentEntry {
        agent_id: tab.run.agent_id.clone(),
        profile_name: tab.profile_name.clone(),
        launched: tab.launched.clone(),
        first_prompt: tab.first_prompt,
        notes_file: tab.run.notes_file.clone(),
        credential,
        workflow: tab.workflow.as_deref().and_then(|run| {
            Some(WorkflowEntry {
                stages: run.stages_file()?.to_path_buf(),
                test_timeout_secs: run.test_timeout().as_secs(),
                generation: run.generation(),
                blurb: run.blurb().to_owned(),
            })
        }),
        remote: tab.remote.as_ref().map(|session| session.name.clone()),
    }
}

pub(crate) struct Facts<'a> {
    pub(crate) workspaces: &'a [Workspace],
    pub(crate) focus: bool,
    pub(crate) welcome: bool,
    pub(crate) hook_port: Option<u16>,
}

/// The layout to send, and the positions in `tabs.items()` of the tabs whose pty goes with it, in
/// the order they must be sent.
pub(crate) fn plan<T>(
    tabs: &Tabs<Pane<T>>,
    facts: &Facts,
    credential: &dyn Fn(u64) -> Option<Credential>,
) -> (Handover, Vec<usize>) {
    let mut order = Vec::new();
    let groups = facts
        .workspaces
        .iter()
        .enumerate()
        .map(|(group, workspace)| {
            let kept: Vec<(usize, &Tab<Pane<T>>)> =
                tabs.in_group(group).filter(|(_, tab)| keeps(tab)).collect();
            let selected_at = tabs.selected_in(group);
            let selected = kept.iter().rposition(|(at, _)| *at <= selected_at).unwrap_or(0);
            let entries = kept
                .into_iter()
                .map(|(at, tab)| {
                    let live = matches!(tab.pane.body, Body::Shell(_));
                    if live {
                        order.push(at);
                    }
                    let agent = tab.pane.agent.as_ref().filter(|_| live);
                    TabEntry {
                        title: tab.title.clone(),
                        pinned: tab.pinned,
                        live,
                        worktree: tab.pane.worktree.clone(),
                        agent: agent.map(|agent| agent_entry(agent, credential(tab.id))),
                    }
                })
                .collect();
            Group {
                dir: workspace.dir().to_path_buf(),
                other: matches!(workspace, Workspace::Other { .. }),
                selected,
                tabs: entries,
            }
        })
        .collect();
    let handover = Handover {
        groups,
        active_group: tabs.active_group(),
        focus: facts.focus,
        welcome: facts.welcome,
        hook_port: facts.hook_port,
    };
    (handover, order)
}

/// The tabs a received layout describes, around the terminals that came with it.
pub(crate) struct Rebuilt<T> {
    pub(crate) tabs: Tabs<Pane<T>>,
    /// Each hooked agent's tab, with what it reports with.
    pub(crate) credentials: Vec<(u64, Credential)>,
    /// What did not come back as it was, for the window to say.
    pub(crate) notes: Vec<String>,
}

/// How a received terminal becomes a live one, and how a tab with none gets a shell.
pub(crate) struct Ptys<'a, P, T> {
    pub(crate) adopt: &'a mut dyn FnMut(P, &Path) -> std::io::Result<T>,
    pub(crate) fresh: &'a dyn Fn(&Path, Option<&str>) -> T,
}

const LOST_TERMINAL: &str =
    "This tab's terminal could not be handed over to the new version, so it is a fresh shell.";

/// Builds the tabs of `handover` around `panes`, in the order [`plan`] sent them. A layout that does
/// not line up with its terminals is refused, and so is one with a group that has no main tab: the
/// window that sent it stays as it was.
pub(crate) fn rebuild<P, T>(
    handover: &Handover,
    panes: Vec<P>,
    now: f64,
    ptys: Ptys<P, T>,
) -> Result<Rebuilt<T>, String> {
    if panes.len() != handover.pane_count() {
        return Err("the terminals do not match the layout".to_owned());
    }
    let well_formed =
        handover.groups.iter().all(|group| group.tabs.first().is_some_and(|tab| tab.pinned));
    if handover.groups.is_empty() || !well_formed {
        return Err("a group of the layout has no main tab".to_owned());
    }
    let Ptys { adopt, fresh } = ptys;
    let mut panes = panes.into_iter();
    let mut tabs: Option<Tabs<Pane<T>>> = None;
    let mut ids: Vec<Vec<u64>> = Vec::new();
    let mut credentials = Vec::new();
    let mut notes = Vec::new();
    for (group_at, group) in handover.groups.iter().enumerate() {
        let mut group_ids = Vec::new();
        for entry in &group.tabs {
            let dir = entry.worktree.as_ref().map_or(group.dir.as_path(), |w| w.path.as_path());
            let received = entry.live.then(|| panes.next()).flatten();
            let pane = match received.map(|pane| adopt(pane, dir)) {
                Some(Ok(term)) => Pane {
                    body: Body::Shell(term),
                    worktree: entry.worktree.clone(),
                    agent: entry.agent.as_ref().map(|agent| agent_tab(agent, dir, now, &mut notes)),
                },
                Some(Err(_)) => {
                    notes.push(LOST_TERMINAL.to_owned());
                    Pane {
                        body: Body::Shell(fresh(dir, Some(LOST_TERMINAL))),
                        worktree: entry.worktree.clone(),
                        agent: None,
                    }
                }
                None => Pane {
                    body: Body::Shell(fresh(dir, None)),
                    worktree: entry.worktree.clone(),
                    agent: None,
                },
            };
            let credential = pane
                .agent
                .as_ref()
                .and(entry.agent.as_ref())
                .and_then(|agent| agent.credential.clone());
            let id = match (tabs.as_mut(), group_ids.is_empty()) {
                (None, _) => {
                    let new = Tabs::new(pane);
                    let id = new.items()[0].id;
                    tabs = Some(new);
                    id
                }
                (Some(tabs), true) => {
                    let group = tabs.add_group(pane);
                    tabs.main_id(group).unwrap_or_default()
                }
                (Some(tabs), false) => tabs.push_in(group_at, entry.title.clone(), pane),
            };
            if let Some(credential) = credential {
                credentials.push((id, credential));
            }
            group_ids.push(id);
        }
        ids.push(group_ids);
    }
    let mut tabs = tabs.ok_or("the layout has no tabs")?;
    for (group, group_ids) in handover.groups.iter().zip(&ids) {
        if let Some(id) = group_ids.get(group.selected).or(group_ids.first()) {
            tabs.select_id(*id);
        }
    }
    let front = handover.active_group.min(handover.groups.len() - 1);
    tabs.apply(Action::ShowGroup(front), || unreachable!("showing a group opens no tab"));
    Ok(Rebuilt { tabs, credentials, notes })
}

fn agent_tab(entry: &AgentEntry, dir: &Path, now: f64, notes: &mut Vec<String>) -> AgentTab {
    let mut run = AgentRun::setting_up(entry.agent_id.clone(), now);
    run.started(entry.notes_file.clone(), now);
    let session_id = entry.launched.as_ref().and_then(|launched| launched.session_id.clone());
    let workflow = entry.workflow.as_ref().and_then(|saved| {
        let Some(runner) = Runner::load(&saved.stages) else {
            notes.push(
                "A workflow run could not be read back, so its tab shows the agent alone."
                    .to_owned(),
            );
            return None;
        };
        let run = TabRun::new(
            runner,
            Some(saved.stages.clone()),
            Duration::from_secs(saved.test_timeout_secs),
        );
        Some(Box::new(run.with_blurb(saved.blurb.clone()).with_generation(saved.generation)))
    });
    AgentTab {
        profile_name: entry.profile_name.clone(),
        run,
        reader: Reader::for_agent(&entry.agent_id, session_id, dir.to_path_buf()),
        launched: entry.launched.clone(),
        workflow,
        first_prompt: entry.first_prompt,
        resume_note: None,
        base_note: None,
        remote: entry.remote.clone().map(RemoteSession::new),
    }
}

const HOOKS_LOST: &str = "Agents that kept running could not reconnect to Tidewatch, so their status comes from the terminal and they cannot report findings.";

/// What the new window tells the user about the handover: what came back as something else, and
/// whether the agents that kept running still reach Tidewatch. `hooked` is how many of them reported
/// to the old window, `kept` whether this one serves the port they report to.
pub(crate) fn notice(notes: &[String], hooked: usize, kept: bool) -> Option<String> {
    let mut lines = notes.to_vec();
    if hooked > 0 && !kept {
        lines.push(HOOKS_LOST.to_owned());
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// What a launch does about `--handoff`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Boot<I> {
    /// Not a replacement: start as usual.
    Normal,
    /// Adopt what the old window sent.
    Adopt(I),
    /// The handover did not happen and the old window is still running with everything: leave it be.
    GiveWay,
}

/// `received` is what reading the old window's terminals gave, `None` for a launch that was not
/// asked to. `old_window_answers` is asked only when the handover failed.
pub(crate) fn boot<I>(
    received: Option<std::io::Result<I>>,
    old_window_answers: impl FnOnce() -> bool,
) -> Boot<I> {
    match received {
        None => Boot::Normal,
        Some(Ok(incoming)) => Boot::Adopt(incoming),
        Some(Err(_)) => match fallback(old_window_answers()) {
            Fallback::ExitQuietly => Boot::GiveWay,
            Fallback::FreshWindow => Boot::Normal,
        },
    }
}

/// The command that starts the replacement. From an app bundle it goes through `open`, which is
/// what makes launchd give the new process its window; the settings a throwaway run depends on
/// travel as `--env`, since `open` does not pass the environment on. Anywhere else the same
/// executable starts directly and inherits the environment, except for the dev switches that open
/// or restart tabs: the new window continues the old one's tabs and starts none of its own.
pub(crate) fn successor_command(
    exe: &Path,
    bundle: Option<&Path>,
    socket: &Path,
    env: impl Fn(&str) -> Option<OsString>,
) -> Command {
    let mut command = match bundle {
        Some(bundle) => {
            let mut open = Command::new("open");
            open.arg("-n").arg("-a").arg(bundle);
            for name in FORWARDED_ENV {
                if let Some(value) = env(name) {
                    let mut pair = OsString::from(format!("{name}="));
                    pair.push(value);
                    open.arg("--env").arg(pair);
                }
            }
            open.arg("--args");
            open
        }
        None => Command::new(exe),
    };
    command.arg("--handoff").arg(socket);
    for name in NOT_INHERITED_ENV {
        command.env_remove(name);
    }
    command
}

/// Every pty to send, with its metadata, in the order [`plan`] named. One that cannot be borrowed
/// aborts the whole handover: half a window is worse than none.
pub(crate) fn capture<'a>(
    tabs: &'a Tabs<Pane<PtyTerm>>,
    order: &[usize],
) -> Result<Vec<(PaneMeta, BorrowedFd<'a>)>, String> {
    order
        .iter()
        .map(|&at| match &tabs.items()[at].pane.body {
            Body::Shell(term) => capture_pane(term),
            _ => None,
        })
        .map(|captured| captured.ok_or_else(|| "a terminal cannot be handed over".to_owned()))
        .collect()
}

/// After the new window confirmed: its terminals are not this window's to stop any more.
pub(crate) fn release(tabs: &mut Tabs<Pane<PtyTerm>>) {
    for tab in tabs.items_mut() {
        if let Body::Shell(term) = &mut tab.pane.body {
            term.mark_handed_off();
        }
    }
}

/// The tabs of an old window, around its adopted terminals. Nothing is confirmed yet: until
/// [`confirm`] the old window still guards every shell.
pub(crate) fn adopt(
    ctx: &egui::Context,
    incoming: &mut Incoming,
    now: f64,
) -> Result<Rebuilt<PtyTerm>, String> {
    let handover = incoming.session().clone();
    let panes = incoming.take_panes();
    let mut adopt = |(meta, fd): (PaneMeta, OwnedFd), dir: &Path| pty::adopt(ctx, &meta, fd, dir);
    let fresh = |dir: &Path, note: Option<&str>| pty::spawn_shell_noted(ctx, dir, note);
    rebuild(&handover, panes, now, Ptys { adopt: &mut adopt, fresh: &fresh })
}

/// Tells the old window it may let go, and only then takes the terminals' shells as this window's
/// to stop. Everything must be live before this: nothing can be undone after it.
pub(crate) fn confirm(incoming: &Incoming, tabs: &mut Tabs<Pane<PtyTerm>>) -> std::io::Result<()> {
    let terms = tabs.items_mut().iter_mut().filter_map(|tab| match &mut tab.pane.body {
        Body::Shell(term) => Some(term),
        _ => None,
    });
    confirm_terms(incoming, terms)
}

/// [`confirm`] for terminals that are not in tabs yet, over any layout that was received.
pub(crate) fn confirm_terms<'a, S: Layout>(
    incoming: &stdusk_core::handoff::Incoming<S>,
    terms: impl IntoIterator<Item = &'a mut PtyTerm>,
) -> std::io::Result<()> {
    incoming.ack()?;
    for term in terms {
        term.arm_teardown();
    }
    Ok(())
}

#[cfg(test)]
mod tests;
