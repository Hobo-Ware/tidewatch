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

use serde::{Deserialize, Serialize};
use stdusk_core::handoff::{Fallback, Layout, PaneMeta, capture_pane, fallback};
use stdusk_core::terminal::PtyTerm;
use tidewatch_core::listener::Credential;
use tidewatch_core::meeseeks::{Meeseeks, Phase};
use tidewatch_core::meeseeks_log::Poofed;
use tidewatch_core::service_supervisor::Carry;
use tidewatch_core::status::{AgentStatus, Reason};
use tidewatch_core::worktree::Worktree;

use crate::agent_run::AgentRun;
use crate::pane::{AgentTab, Body, FirstPrompt, Launched, Pane};
use crate::park::Park;
use crate::pty;
use crate::reader::Reader;
use crate::remote_session::RemoteSession;
use crate::repo::Workspace;
use crate::tabs::{Action, Tab, Tabs};

pub(crate) type Incoming = stdusk_core::handoff::Incoming<Handover>;

const FORWARDED_ENV: [&str; 2] = ["TIDEWATCH_HOME", "TIDEWATCH_BACKGROUND"];
const NOT_INHERITED_ENV: [&str; 3] =
    ["TIDEWATCH_DEV_RESTART", "TIDEWATCH_DEV_AGENT", "TIDEWATCH_DEV_WORKTREE"];

/// Why a window cannot hand its tabs over right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blocker {
    TabInTransition,
    MeeseeksBusy,
}

impl Blocker {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::TabInTransition => "a tab is still being made or removed",
            Self::MeeseeksBusy => "a Meeseeks is being checked",
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
        let checking = pane
            .agent
            .as_ref()
            .and_then(|agent| agent.meeseeks.as_ref())
            .is_some_and(|meeseeks| meeseeks.phase != Phase::Working);
        checking.then_some(Blocker::MeeseeksBusy)
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
    /// The repo services, after every tab: the ptys of the live ones travel behind the tabs' own.
    #[serde(default, rename = "service", skip_serializing_if = "Vec::is_empty")]
    pub(crate) services: Vec<ServiceEntry>,
}

/// One repo service as the old window left it: where it stood, and whether a pty travels with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ServiceEntry {
    pub(crate) repo: PathBuf,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) live: bool,
    pub(crate) carry: Carry,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) park: Option<Park>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) keep: bool,
    /// A poofed Meeseeks' summary: nothing travels with it but the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<Box<Poofed>>,
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
    /// Sent by a window from before workflows were retired. Read so the tab comes back as its agent
    /// alone, with a note; never written.
    #[serde(default, skip_serializing)]
    pub(crate) workflow: Option<RetiredWorkflow>,
    /// The name of the remote session the running agent has, when remote control is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) remote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) meeseeks: Option<Meeseeks>,
    /// The old window saw the agent done or waiting at its prompt: no hook will say so again until
    /// its next turn.
    #[serde(default)]
    pub(crate) at_prompt: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct RetiredWorkflow {}

impl Handover {
    fn live(&self) -> impl Iterator<Item = &TabEntry> {
        self.groups.iter().flat_map(|group| group.tabs.iter()).filter(|tab| tab.live)
    }
}

impl Layout for Handover {
    fn pane_count(&self) -> usize {
        self.live().count() + self.services.iter().filter(|service| service.live).count()
    }
}

fn keeps<T>(tab: &Tab<Pane<T>>) -> bool {
    match &tab.pane.body {
        Body::Shell(_) | Body::Poofed(_) => true,
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
        workflow: None,
        remote: tab.remote.as_ref().map(|session| session.name.clone()),
        meeseeks: tab.meeseeks.clone(),
        at_prompt: matches!(
            tab.run.status(),
            AgentStatus::Done | AgentStatus::NeedsYou(Reason::Input | Reason::Attention)
        ),
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
                        park: tab.park,
                        keep: tab.keep,
                        summary: match &tab.pane.body {
                            Body::Poofed(record) => Some(record.clone()),
                            _ => None,
                        },
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
        services: Vec::new(),
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
    /// Each repo service with its pty, `None` when none travelled or it could not be taken.
    pub(crate) services: Vec<ResumedService<T>>,
}

pub(crate) struct ResumedService<T> {
    pub(crate) entry: ServiceEntry,
    pub(crate) term: Option<T>,
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
            let pane = match (entry.summary.as_ref(), received.map(|pane| adopt(pane, dir))) {
                (Some(record), _) => {
                    Pane { body: Body::Poofed(record.clone()), worktree: None, agent: None }
                }
                (None, Some(Ok(term))) => Pane {
                    body: Body::Shell(term),
                    worktree: entry.worktree.clone(),
                    agent: entry.agent.as_ref().map(|agent| agent_tab(agent, dir, now, &mut notes)),
                },
                (None, Some(Err(_))) => {
                    notes.push(LOST_TERMINAL.to_owned());
                    Pane {
                        body: Body::Shell(fresh(dir, Some(LOST_TERMINAL))),
                        worktree: entry.worktree.clone(),
                        agent: None,
                    }
                }
                (None, None) => Pane {
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
            if let Some(tab) = tabs.as_mut().and_then(|tabs| tabs.get_mut(id)) {
                tab.park = entry.park.filter(|_| !tab.pinned);
                tab.keep = entry.keep && !tab.pinned;
            }
            group_ids.push(id);
        }
        ids.push(group_ids);
    }
    let mut tabs = tabs.ok_or("the layout has no tabs")?;
    let services = resume_services(handover, &mut panes, adopt, &mut notes);
    for (group, group_ids) in handover.groups.iter().zip(&ids) {
        if let Some(id) = group_ids.get(group.selected).or(group_ids.first()) {
            tabs.select_id(*id);
        }
    }
    let front = handover.active_group.min(handover.groups.len() - 1);
    tabs.apply(Action::ShowGroup(front), || unreachable!("showing a group opens no tab"));
    Ok(Rebuilt { tabs, credentials, notes, services })
}

fn resume_services<P, T>(
    handover: &Handover,
    panes: &mut impl Iterator<Item = P>,
    adopt: &mut dyn FnMut(P, &Path) -> std::io::Result<T>,
    notes: &mut Vec<String>,
) -> Vec<ResumedService<T>> {
    handover
        .services
        .iter()
        .map(|entry| {
            let term = entry.live.then(|| panes.next()).flatten().and_then(|pane| {
                adopt(pane, &entry.repo)
                    .inspect_err(|_| {
                        notes.push(format!("The service {} was started again.", entry.name));
                    })
                    .ok()
            });
            ResumedService { entry: entry.clone(), term }
        })
        .collect()
}

fn agent_tab(entry: &AgentEntry, dir: &Path, now: f64, notes: &mut Vec<String>) -> AgentTab {
    let mut run = AgentRun::setting_up(entry.agent_id.clone(), now);
    run.started(entry.notes_file.clone(), now);
    if entry.credential.is_some() {
        if entry.at_prompt {
            run.resumed(now);
        } else {
            run.await_report();
        }
    }
    let session_id = entry.launched.as_ref().and_then(|launched| launched.session_id.clone());
    if entry.workflow.is_some() {
        notes.push("Workflows were retired, so a workflow tab shows its agent alone.".to_owned());
    }
    AgentTab {
        profile_name: entry.profile_name.clone(),
        run,
        reader: Reader::for_agent(&entry.agent_id, session_id, dir.to_path_buf()),
        launched: entry.launched.clone(),
        first_prompt: entry.first_prompt,
        resume_note: None,
        resume_line: None,
        resume_sent_at: None,
        base_note: None,
        remote: entry.remote.clone().map(RemoteSession::new),
        meeseeks: entry.meeseeks.clone(),
    }
}

const HOOKS_LOST: &str = "Agents that kept running could not reconnect to Tidewatch, so their status comes from the terminal and their progress, questions, findings and Meeseeks reports are lost.";

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
pub(crate) fn confirm(
    incoming: &Incoming,
    tabs: &mut Tabs<Pane<PtyTerm>>,
    services: &mut [ResumedService<PtyTerm>],
) -> std::io::Result<()> {
    let tab_terms = tabs.items_mut().iter_mut().filter_map(|tab| match &mut tab.pane.body {
        Body::Shell(term) => Some(term),
        _ => None,
    });
    confirm_terms(incoming, tab_terms.chain(services.iter_mut().filter_map(|s| s.term.as_mut())))
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
