//! What a tab holds: a shell, or the transient states of a worktree tab while git works.
//! Generic over the shell type so the transitions are testable without a pty.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tidewatch_core::agents::Level;
use tidewatch_core::meeseeks::Meeseeks;
use tidewatch_core::prepare::Prepared;
use tidewatch_core::worktree::Worktree;

use crate::agent_run::AgentRun;
use crate::new_tab::NewTab;
use crate::reader::Reader;
use crate::remote_session::RemoteSession;
use crate::tabs::Tabs;
use crate::workflow_run::TabRun;

pub(crate) const CREATING_TITLE: &str = "new worktree";
pub(crate) const STARTING_TITLE: &str = "starting";
pub(crate) const FAILED_TITLE: &str = "failed";

pub(crate) enum Body<T> {
    Shell(T),
    /// A new tab that is still the form: nothing exists yet, and nothing of it is saved.
    Draft(Box<NewTab>),
    Creating,
    Removing,
    Failed(String),
}

/// How an agent was started, which is what a restart needs to start it again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Launched {
    /// The `profiles.toml` key, or the profile's name for a workflow stage.
    pub(crate) profile_id: String,
    pub(crate) level: Level,
    /// The id given to the agent's `--session-id`, when its CLI takes one.
    pub(crate) session_id: Option<String>,
    /// The agent was started with remote control on, and a restart starts it that way again.
    #[serde(default)]
    pub(crate) remote: bool,
    /// The folder it runs in, which a tab without a worktree is saved with: a session taken over
    /// from a sibling checkout stays there while its group is the main repo.
    #[serde(default)]
    pub(crate) dir: Option<PathBuf>,
}

/// Whether an agent tab still waits for its first message, which is typed in the tab, not in the
/// new-tab dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FirstPrompt {
    /// The agent was given its task at launch, or the tab was resumed.
    #[default]
    NotNeeded,
    /// Started with no task: the tab shows its empty state and the composer.
    Awaiting,
    /// The first message went to the agent and its transcript has not shown up yet.
    Sent,
}

/// The profile an agent tab was started from and what the agent is doing. Plain shells have none.
pub(crate) struct AgentTab {
    pub(crate) profile_name: String,
    pub(crate) run: AgentRun,
    pub(crate) reader: Reader,
    /// Set once the agent has started.
    pub(crate) launched: Option<Launched>,
    /// The workflow this tab runs, when it was opened from one.
    pub(crate) workflow: Option<Box<TabRun>>,
    pub(crate) first_prompt: FirstPrompt,
    /// Why this tab's resume command waits typed at the prompt; shown until the user types.
    pub(crate) resume_note: Option<&'static str>,
    /// Why the worktree did not start from the fresh remote branch; shown until the user types.
    pub(crate) base_note: Option<String>,
    /// The agent process runs with remote control on, whatever the saved setting says now.
    pub(crate) remote: Option<RemoteSession>,
    pub(crate) meeseeks: Option<Meeseeks>,
}

impl AgentTab {
    pub(crate) fn new(profile_name: String, run: AgentRun) -> Self {
        let reader = Reader::for_agent(&run.agent_id, None, PathBuf::new());
        Self {
            profile_name,
            run,
            reader,
            launched: None,
            workflow: None,
            first_prompt: FirstPrompt::NotNeeded,
            resume_note: None,
            base_note: None,
            remote: None,
            meeseeks: None,
        }
    }
}

pub(crate) struct Pane<T> {
    pub(crate) body: Body<T>,
    pub(crate) worktree: Option<Worktree>,
    pub(crate) agent: Option<AgentTab>,
}

/// What a tab-opening job produced: the worktree (none in Other), the folder to run in and, for an
/// agent tab, what to start there.
pub(crate) struct Opened {
    pub(crate) worktree: Option<Worktree>,
    pub(crate) dir: PathBuf,
    pub(crate) started: Option<Box<Started>>,
    /// Set when the start branch could not be fetched, for the tab to say so.
    pub(crate) base_note: Option<String>,
}

pub(crate) struct Started {
    pub(crate) profile_name: String,
    pub(crate) profile_id: String,
    pub(crate) level: Level,
    pub(crate) agent_id: String,
    pub(crate) notes_file: Option<PathBuf>,
    /// The id given to the agent's `--session-id`, when its CLI takes one.
    pub(crate) session_id: Option<String>,
    pub(crate) awaiting_prompt: bool,
    /// The agent resumes a saved session and waits at its prompt.
    pub(crate) resumed: bool,
    pub(crate) prepared: Prepared,
    pub(crate) meeseeks: Option<Meeseeks>,
}

impl Started {
    fn launched(&self, dir: &Path) -> Launched {
        Launched {
            profile_id: self.profile_id.clone(),
            level: self.level,
            session_id: self.session_id.clone(),
            remote: self.prepared.remote.is_some(),
            dir: Some(dir.to_path_buf()),
        }
    }
}

pub(crate) enum OpenFailure {
    /// Nothing was left behind.
    Nothing(String),
    /// The worktree exists but the agent could not be set up; the tab keeps it so closing offers
    /// to remove it.
    Kept { worktree: Worktree, message: String },
}

/// What closing a tab does first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClosePolicy {
    Now,
    Confirm(Worktree),
    Wait,
}

impl<T> Pane<T> {
    pub(crate) fn shell(shell: T) -> Self {
        Self { body: Body::Shell(shell), worktree: None, agent: None }
    }

    pub(crate) fn draft(form: NewTab) -> Self {
        Self { body: Body::Draft(Box::new(form)), worktree: None, agent: None }
    }

    pub(crate) fn creating() -> Self {
        Self { body: Body::Creating, worktree: None, agent: None }
    }

    /// A tab whose worktree is being made for an agent: its status is "setting up" from the start.
    pub(crate) fn creating_agent(profile_name: String, agent_id: String, now: f64) -> Self {
        let run = AgentRun::setting_up(agent_id, now);
        Self { body: Body::Creating, worktree: None, agent: Some(AgentTab::new(profile_name, run)) }
    }

    pub(crate) fn close_policy(&self) -> ClosePolicy {
        match (&self.body, &self.worktree) {
            (Body::Creating | Body::Removing, _) => ClosePolicy::Wait,
            (_, Some(worktree)) => ClosePolicy::Confirm(worktree.clone()),
            (_, None) => ClosePolicy::Now,
        }
    }

    /// Hands back the shell, so the caller can stop it off the UI thread, and the worktree to
    /// remove. `None` while there is nothing to remove or a job is already running.
    pub(crate) fn begin_remove(&mut self) -> Option<(Option<T>, Worktree)> {
        let worktree = self.worktree.clone()?;
        match std::mem::replace(&mut self.body, Body::Removing) {
            Body::Shell(shell) => Some((Some(shell), worktree)),
            Body::Failed(_) => Some((None, worktree)),
            other @ (Body::Creating | Body::Removing | Body::Draft(_)) => {
                self.body = other;
                None
            }
        }
    }

    pub(crate) fn restore_shell(&mut self, shell: T) {
        if matches!(self.body, Body::Removing) {
            self.body = Body::Shell(shell);
        }
    }

    pub(crate) fn remove_failed(&mut self, message: String) {
        self.body = Body::Failed(message);
    }
}

/// Titles of the tabs of the visible repo that run an agent straight in the shared folder (the
/// Other group), for the same-folder warning. A worktree tab has a folder of its own.
pub(crate) fn agent_tabs_in_folder<T>(tabs: &Tabs<Pane<T>>) -> Vec<String> {
    tabs.in_group(tabs.active_group())
        .map(|(_, tab)| tab)
        .filter(|tab| {
            tab.pane.agent.is_some()
                && tab.pane.worktree.is_none()
                && matches!(tab.pane.body, Body::Shell(_))
        })
        .map(|tab| tab.title.clone())
        .collect()
}

/// Applies a finished tab opening: the shell or agent starts in its folder and the tab takes its
/// name, or the tab shows the error. `spawn` gets what to start (nothing means a plain shell).
/// Returns the notes the preparation left, for the UI to mention.
pub(crate) fn finish_open<T>(
    tabs: &mut Tabs<Pane<T>>,
    id: u64,
    result: Result<Opened, OpenFailure>,
    now: f64,
    spawn: impl FnOnce(&Path, Option<&Prepared>) -> T,
) -> Vec<String> {
    let Some(tab) = tabs.get_mut(id) else { return Vec::new() };
    if !matches!(tab.pane.body, Body::Creating) {
        return Vec::new();
    }
    match result {
        Ok(opened) => {
            let notes =
                opened.started.as_ref().map(|s| s.prepared.notes.clone()).unwrap_or_default();
            let title = match (&opened.worktree, &opened.started) {
                (Some(worktree), _) => worktree.name().to_owned(),
                (None, Some(started)) => started.profile_name.clone(),
                (None, None) => tab.title.clone(),
            };
            title.clone_into(&mut tab.title);
            let shell = spawn(&opened.dir, opened.started.as_ref().map(|s| &s.prepared));
            let previous = tab.pane.agent.take();
            tab.pane = Pane {
                body: Body::Shell(shell),
                worktree: opened.worktree,
                agent: opened.started.map(|started| {
                    let mut run = previous.map_or_else(
                        || AgentRun::setting_up(started.agent_id.clone(), now),
                        |agent| agent.run,
                    );
                    run.started(started.notes_file.clone(), now);
                    if started.resumed {
                        run.resumed(now);
                    }
                    let launched = started.launched(&opened.dir);
                    let remote = started.prepared.remote.clone().map(RemoteSession::new);
                    let reader = Reader::for_agent(
                        &started.agent_id,
                        started.session_id,
                        opened.dir.clone(),
                    );
                    AgentTab {
                        profile_name: started.profile_name,
                        run,
                        reader,
                        launched: Some(launched),
                        workflow: None,
                        first_prompt: if started.awaiting_prompt {
                            FirstPrompt::Awaiting
                        } else {
                            FirstPrompt::NotNeeded
                        },
                        resume_note: None,
                        base_note: None,
                        remote,
                        meeseeks: started.meeseeks,
                    }
                }),
            };
            notes
        }
        Err(OpenFailure::Nothing(message)) => {
            FAILED_TITLE.clone_into(&mut tab.title);
            if let Some(agent) = &mut tab.pane.agent {
                agent.run.fail(message.clone());
            }
            tab.pane.body = Body::Failed(message);
            Vec::new()
        }
        Err(OpenFailure::Kept { worktree, message }) => {
            worktree.name().clone_into(&mut tab.title);
            let mut agent = tab.pane.agent.take();
            if let Some(agent) = &mut agent {
                agent.run.fail(message.clone());
            }
            tab.pane = Pane { body: Body::Failed(message), worktree: Some(worktree), agent };
            Vec::new()
        }
    }
}

/// A workflow stage's agent starts in a tab that already has a worktree, in place of the previous
/// stage's: the old shell is dropped (which ends its agent), the tab gets a fresh status, and its
/// Reader keeps what was read and marks the new stage. Does nothing for a tab that is closing or has
/// no agent. Returns the notes the preparation left.
pub(crate) fn relaunch_stage<T>(
    tabs: &mut Tabs<Pane<T>>,
    id: u64,
    started: Started,
    dir: &Path,
    divider: String,
    now: f64,
    spawn: impl FnOnce(&Path, &Prepared) -> T,
) -> Vec<String> {
    let Some(tab) = tabs.get_mut(id) else { return Vec::new() };
    if !matches!(tab.pane.body, Body::Shell(_) | Body::Creating) {
        return Vec::new();
    }
    let Some(agent) = tab.pane.agent.as_mut() else { return Vec::new() };
    tab.pane.body = Body::Shell(spawn(dir, &started.prepared));
    agent.launched = Some(started.launched(dir));
    agent.remote = started.prepared.remote.clone().map(RemoteSession::new);
    agent.profile_name = started.profile_name;
    agent.run = AgentRun::setting_up(started.agent_id.clone(), now);
    agent.run.started(started.notes_file, now);
    agent.reader.relaunch(&started.agent_id, started.session_id, divider);
    if let Some(run) = agent.workflow.as_mut() {
        run.launched();
    }
    started.prepared.notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tidewatch_core::status::AgentStatus;

    fn shell_of(pane: &Pane<u32>) -> Option<u32> {
        match pane.body {
            Body::Shell(n) => Some(n),
            _ => None,
        }
    }

    fn worktree() -> Worktree {
        Worktree {
            repo: PathBuf::from("/work/web"),
            path: PathBuf::from("/home/.tidewatch/web-abc123/brave-otter"),
            branch: "agent/brave-otter".into(),
        }
    }

    fn tabs_with_creating() -> (Tabs<Pane<u32>>, u64) {
        let mut tabs = Tabs::new(Pane::shell(0));
        let id = tabs.push(CREATING_TITLE.into(), Pane::creating());
        (tabs, id)
    }

    fn opened(started: Option<Box<Started>>) -> Opened {
        Opened { worktree: Some(worktree()), dir: worktree().path, started, base_note: None }
    }

    fn started(notes: &[&str]) -> Box<Started> {
        Box::new(Started {
            profile_name: "Builder".into(),
            profile_id: "builder".into(),
            level: Level::Edits,
            agent_id: "claude".into(),
            notes_file: None,
            session_id: None,
            awaiting_prompt: false,
            resumed: false,
            prepared: Prepared {
                launch: tidewatch_core::launch::Launch {
                    program: "claude".into(),
                    args: Vec::new(),
                    env: Vec::new(),
                    experimental: false,
                },
                env: Vec::new(),
                setup: String::new(),
                notes: notes.iter().map(|n| (*n).to_owned()).collect(),
                remote: None,
            },
            meeseeks: None,
        })
    }

    #[test]
    fn opened_worktree_starts_the_shell_inside_it_and_names_the_tab() {
        let (mut tabs, id) = tabs_with_creating();
        let mut seen = None;
        let notes = finish_open(&mut tabs, id, Ok(opened(None)), 0.0, |path, prepared| {
            seen = Some((path.to_path_buf(), prepared.is_some()));
            5
        });
        assert_eq!(seen, Some((worktree().path, false)));
        assert_eq!(notes, Vec::<String>::new());
        let tab = tabs.get_mut(id).unwrap();
        assert_eq!(tab.title, "brave-otter");
        assert_eq!(shell_of(&tab.pane), Some(5));
        assert_eq!(tab.pane.worktree, Some(worktree()));
        assert!(tab.pane.agent.is_none());
    }

    #[test]
    fn an_agent_tab_gets_what_to_start_remembers_its_profile_and_returns_the_notes() {
        let (mut tabs, id) = tabs_with_creating();
        let mut got_prepared = false;
        let notes = finish_open(
            &mut tabs,
            id,
            Ok(opened(Some(started(&["house style: x"])))),
            0.0,
            |_, p| {
                got_prepared = p.is_some();
                6
            },
        );
        assert!(got_prepared);
        assert_eq!(notes, ["house style: x"]);
        let tab = tabs.get_mut(id).unwrap();
        assert_eq!(tab.title, "brave-otter");
        assert_eq!(tab.pane.agent.as_ref().map(|a| a.profile_name.as_str()), Some("Builder"));
        assert_eq!(tab.pane.agent.as_ref().unwrap().run.status(), &AgentStatus::SettingUp);
        assert_eq!(tab.pane.close_policy(), ClosePolicy::Confirm(worktree()));
    }

    #[test]
    fn an_agent_tab_in_other_has_no_worktree_and_is_named_after_its_profile() {
        let (mut tabs, id) = tabs_with_creating();
        let in_other = Opened {
            worktree: None,
            dir: PathBuf::from("/notes"),
            started: Some(started(&[])),
            base_note: None,
        };
        finish_open(&mut tabs, id, Ok(in_other), 0.0, |_, _| 7);
        let tab = tabs.get_mut(id).unwrap();
        assert_eq!(tab.title, "Builder");
        assert_eq!(tab.pane.worktree, None);
        assert_eq!(tab.pane.close_policy(), ClosePolicy::Now);
    }

    #[test]
    fn failed_opening_shows_the_error_and_keeps_no_worktree() {
        let (mut tabs, id) = tabs_with_creating();
        finish_open(
            &mut tabs,
            id,
            Err(OpenFailure::Nothing("disk full".into())),
            0.0,
            |_, _| unreachable!(),
        );
        let tab = tabs.get_mut(id).unwrap();
        assert_eq!(tab.title, FAILED_TITLE);
        assert!(matches!(&tab.pane.body, Body::Failed(m) if m == "disk full"));
        assert_eq!(tab.pane.close_policy(), ClosePolicy::Now);
    }

    #[test]
    fn an_agent_that_could_not_be_set_up_keeps_its_worktree_so_closing_can_remove_it() {
        let (mut tabs, id) = tabs_with_creating();
        let failure = OpenFailure::Kept { worktree: worktree(), message: "no flags".into() };
        finish_open(&mut tabs, id, Err(failure), 0.0, |_, _| unreachable!());
        let tab = tabs.get_mut(id).unwrap();
        assert_eq!(tab.title, "brave-otter");
        assert!(matches!(&tab.pane.body, Body::Failed(m) if m == "no flags"));
        assert_eq!(tab.pane.close_policy(), ClosePolicy::Confirm(worktree()));
    }

    #[test]
    fn a_result_for_a_closed_tab_or_a_settled_tab_is_ignored() {
        let (mut tabs, id) = tabs_with_creating();
        finish_open(&mut tabs, 99, Ok(opened(None)), 0.0, |_, _| unreachable!());
        finish_open(
            &mut tabs,
            id,
            Err(OpenFailure::Nothing("x".into())),
            0.0,
            |_, _| unreachable!(),
        );
        finish_open(&mut tabs, id, Ok(opened(None)), 0.0, |_, _| unreachable!());
        assert!(matches!(tabs.get_mut(id).unwrap().pane.body, Body::Failed(_)));
    }

    #[test]
    fn only_agents_running_in_the_shared_folder_count_for_the_warning() {
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        let agent =
            || Some(AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0)));
        tabs.push("a".into(), Pane { body: Body::Shell(1), worktree: None, agent: agent() });
        tabs.push("shell".into(), Pane::shell(2));
        tabs.push(
            "b".into(),
            Pane { body: Body::Shell(3), worktree: Some(worktree()), agent: agent() },
        );
        assert_eq!(agent_tabs_in_folder(&tabs), ["a"]);
    }

    #[test]
    fn close_policy_by_tab_kind() {
        let plain = Pane::shell(1);
        assert_eq!(plain.close_policy(), ClosePolicy::Now);
        assert_eq!(Pane::<u32>::creating().close_policy(), ClosePolicy::Wait);
        let agent = Pane { body: Body::Shell(1), worktree: Some(worktree()), agent: None };
        assert_eq!(agent.close_policy(), ClosePolicy::Confirm(worktree()));
        let removing =
            Pane::<u32> { body: Body::Removing, worktree: Some(worktree()), agent: None };
        assert_eq!(removing.close_policy(), ClosePolicy::Wait);
        let failed =
            Pane::<u32> { body: Body::Failed("x".into()), worktree: Some(worktree()), agent: None };
        assert_eq!(failed.close_policy(), ClosePolicy::Confirm(worktree()));
    }

    #[test]
    fn begin_remove_hands_over_the_shell_once() {
        let mut pane = Pane { body: Body::Shell(3), worktree: Some(worktree()), agent: None };
        assert_eq!(pane.begin_remove(), Some((Some(3), worktree())));
        assert!(matches!(pane.body, Body::Removing));
        assert_eq!(pane.begin_remove(), None);
        assert!(matches!(pane.body, Body::Removing));
    }

    #[test]
    fn a_plain_shell_has_nothing_to_remove() {
        let mut pane = Pane::shell(3);
        assert_eq!(pane.begin_remove(), None);
        assert_eq!(shell_of(&pane), Some(3));
    }

    fn draft() -> Pane<u32> {
        let form = crate::new_tab::NewTab::open(
            crate::new_tab::Group::Repo,
            "web".into(),
            PathBuf::from("/work/web"),
            Vec::new(),
            None,
            1,
        );
        Pane::draft(form)
    }

    #[test]
    fn a_draft_closes_at_once_and_has_nothing_to_remove_or_to_finish_opening() {
        let mut pane = draft();
        assert_eq!(pane.close_policy(), ClosePolicy::Now);
        assert_eq!(pane.begin_remove(), None);
        assert!(matches!(pane.body, Body::Draft(_)));
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        let id = tabs.push("New agent".into(), draft());
        let result = Err(OpenFailure::Nothing("late job".into()));
        assert_eq!(finish_open(&mut tabs, id, result, 0.0, |_, _| 1), Vec::<String>::new());
        assert!(matches!(tabs.get_mut(id).unwrap().pane.body, Body::Draft(_)));
    }

    #[test]
    fn a_failed_removal_can_be_retried() {
        let mut pane = Pane { body: Body::Shell(3), worktree: Some(worktree()), agent: None };
        pane.begin_remove();
        pane.remove_failed("locked".into());
        assert!(matches!(&pane.body, Body::Failed(m) if m == "locked"));
        assert_eq!(pane.begin_remove(), Some((None, worktree())));
    }

    #[test]
    fn a_tab_created_for_an_agent_is_setting_up_until_the_agent_starts() {
        let pane = Pane::<u32>::creating_agent("Builder".into(), "claude".into(), 1.0);
        let agent = pane.agent.as_ref().unwrap();
        assert_eq!(agent.profile_name, "Builder");
        assert_eq!(agent.run.status(), &AgentStatus::SettingUp);
        assert!(matches!(pane.body, Body::Creating));
    }

    #[test]
    fn an_opening_that_fails_marks_the_agent_failed() {
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        let id = tabs.push(
            CREATING_TITLE.into(),
            Pane::creating_agent("Builder".into(), "claude".into(), 0.0),
        );
        finish_open(
            &mut tabs,
            id,
            Err(OpenFailure::Nothing("disk full".into())),
            1.0,
            |_, _| unreachable!(),
        );
        let run = &mut tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().run;
        assert_eq!(run.refresh(1.0).unwrap().to, AgentStatus::Failed("disk full".into()));
    }

    #[test]
    fn an_agent_opened_with_no_task_waits_for_its_first_message_and_one_with_a_task_does_not() {
        for (awaiting, want) in [(true, FirstPrompt::Awaiting), (false, FirstPrompt::NotNeeded)] {
            let (mut tabs, id) = tabs_with_creating();
            let mut agent = started(&[]);
            agent.awaiting_prompt = awaiting;
            finish_open(&mut tabs, id, Ok(opened(Some(agent))), 0.0, |_, _| 1);
            let tab = tabs.get_mut(id).unwrap();
            assert_eq!(tab.pane.agent.as_ref().unwrap().first_prompt, want);
        }
    }

    #[test]
    fn an_agent_whose_setup_failed_keeps_its_worktree_and_shows_failed() {
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        let id = tabs.push(
            CREATING_TITLE.into(),
            Pane::creating_agent("Builder".into(), "claude".into(), 0.0),
        );
        let failure = OpenFailure::Kept { worktree: worktree(), message: "no flags".into() };
        finish_open(&mut tabs, id, Err(failure), 1.0, |_, _| unreachable!());
        let tab = tabs.get_mut(id).unwrap();
        assert_eq!(tab.pane.close_policy(), ClosePolicy::Confirm(worktree()));
        let run = &mut tab.pane.agent.as_mut().unwrap().run;
        assert_eq!(run.refresh(1.0).unwrap().to, AgentStatus::Failed("no flags".into()));
    }

    #[test]
    fn the_agent_starting_keeps_the_run_that_was_setting_up() {
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        let id = tabs.push(
            CREATING_TITLE.into(),
            Pane::creating_agent("Builder".into(), "claude".into(), 0.0),
        );
        finish_open(&mut tabs, id, Ok(opened(Some(started(&[])))), 5.0, |_, _| 9);
        let run = &mut tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().run;
        assert_eq!(run.refresh(5.5).unwrap().to, AgentStatus::Working);
        assert_eq!(run.elapsed(15.0), 10.0);
    }

    fn workflow_tab(tabs: &mut Tabs<Pane<u32>>, body: Body<u32>) -> u64 {
        use std::collections::BTreeMap;
        use std::time::Duration;
        use tidewatch_core::runner::{RunContext, Runner};
        use tidewatch_core::workflows::Workflows;
        let ship = Workflows::defaults().workflow.remove("ship").unwrap();
        let ctx = RunContext {
            task: "t".into(),
            base: "main".into(),
            branch: "agent/brave-otter".into(),
            test_command: None,
        };
        let runner = Runner::new(&ship, &BTreeMap::new(), ctx).unwrap();
        let mut run = TabRun::new(runner, None, Duration::from_secs(60));
        run.start();
        let mut pane = Pane::creating_agent("Ship a feature".into(), "claude".into(), 0.0);
        pane.agent.as_mut().unwrap().workflow = Some(Box::new(run));
        pane.body = body;
        tabs.push("brave-otter".into(), pane)
    }

    #[test]
    fn the_next_stage_replaces_the_shell_resets_the_status_and_marks_the_reader() {
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        let id = workflow_tab(&mut tabs, Body::Shell(1));
        let mut next = *started(&["note"]);
        next.profile_name = "Ship a feature - Review".into();
        next.agent_id = "gemini".into();
        let mut spawned = None;
        let notes = relaunch_stage(
            &mut tabs,
            id,
            next,
            &worktree().path,
            "--- Stage 2: Review (gemini, Read only) ---".into(),
            7.0,
            |dir, _| {
                spawned = Some(dir.to_path_buf());
                2
            },
        );
        assert_eq!(notes, ["note"]);
        assert_eq!(spawned, Some(worktree().path));
        let tab = tabs.get_mut(id).unwrap();
        assert!(matches!(tab.pane.body, Body::Shell(2)));
        let agent = tab.pane.agent.as_mut().unwrap();
        assert_eq!(agent.profile_name, "Ship a feature - Review");
        assert_eq!(agent.run.agent_id, "gemini");
        assert_eq!(agent.run.refresh(7.5).unwrap().to, AgentStatus::Working);
        assert_eq!(agent.run.elapsed(9.0), 2.0);
        let texts: Vec<_> = agent.reader.rows.iter().map(|row| row.text.clone()).collect();
        assert_eq!(texts, ["--- Stage 2: Review (gemini, Read only) ---"]);
        assert!(!agent.workflow.as_ref().unwrap().launching());
    }

    #[test]
    fn the_first_stage_takes_a_tab_that_is_still_being_made() {
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        let id = workflow_tab(&mut tabs, Body::Creating);
        relaunch_stage(&mut tabs, id, *started(&[]), &worktree().path, "---".into(), 1.0, |_, _| 5);
        assert!(matches!(tabs.get_mut(id).unwrap().pane.body, Body::Shell(5)));
    }

    #[test]
    fn a_tab_that_is_closing_or_has_no_agent_is_left_alone() {
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        let removing = workflow_tab(&mut tabs, Body::Removing);
        let failed = workflow_tab(&mut tabs, Body::Failed("x".into()));
        let plain = tabs.push("shell".into(), Pane::shell(3));
        for id in [removing, failed, plain, 99] {
            let notes = relaunch_stage(
                &mut tabs,
                id,
                *started(&["n"]),
                &worktree().path,
                "---".into(),
                1.0,
                |_, _| unreachable!(),
            );
            assert_eq!(notes, Vec::<String>::new(), "tab {id}");
        }
        assert!(matches!(tabs.get_mut(plain).unwrap().pane.body, Body::Shell(3)));
    }

    #[test]
    fn a_removal_refused_before_it_began_gives_the_shell_back_and_only_then() {
        let mut pane = Pane { body: Body::Shell(3), worktree: Some(worktree()), agent: None };
        let (shell, _) = pane.begin_remove().unwrap();
        assert!(matches!(pane.body, Body::Removing));
        pane.restore_shell(shell.unwrap());
        assert_eq!(shell_of(&pane), Some(3));
        pane.restore_shell(9);
        assert_eq!(shell_of(&pane), Some(3), "a tab that is not being removed keeps its shell");
        pane.remove_failed("gone".into());
        pane.restore_shell(9);
        assert!(matches!(pane.body, Body::Failed(_)));
    }
}
