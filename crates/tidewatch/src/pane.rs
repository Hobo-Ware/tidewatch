//! What a tab holds: a shell, or the transient states of a worktree tab while git works.
//! Generic over the shell type so the transitions are testable without a pty.

use std::path::{Path, PathBuf};

use tidewatch_core::prepare::Prepared;
use tidewatch_core::worktree::Worktree;

use crate::agent_run::AgentRun;
use crate::reader::Reader;
use crate::tabs::Tabs;

pub(crate) const CREATING_TITLE: &str = "new worktree";
pub(crate) const STARTING_TITLE: &str = "starting";
pub(crate) const FAILED_TITLE: &str = "failed";

pub(crate) enum Body<T> {
    Shell(T),
    Creating,
    Removing,
    Failed(String),
}

/// The profile an agent tab was started from and what the agent is doing. Plain shells have none.
pub(crate) struct AgentTab {
    pub(crate) profile_name: String,
    pub(crate) run: AgentRun,
    pub(crate) reader: Reader,
}

impl AgentTab {
    pub(crate) fn new(profile_name: String, run: AgentRun) -> Self {
        let reader = Reader::for_agent(&run.agent_id, None, PathBuf::new());
        Self { profile_name, run, reader }
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
}

pub(crate) struct Started {
    pub(crate) profile_name: String,
    pub(crate) agent_id: String,
    pub(crate) notes_file: Option<PathBuf>,
    /// The id given to the agent's `--session-id`, when its CLI takes one.
    pub(crate) session_id: Option<String>,
    pub(crate) prepared: Prepared,
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
            other @ (Body::Creating | Body::Removing) => {
                self.body = other;
                None
            }
        }
    }

    pub(crate) fn remove_failed(&mut self, message: String) {
        self.body = Body::Failed(message);
    }
}

/// Titles of the tabs that run an agent straight in the shared folder (the Other group), for the
/// same-folder warning. A worktree tab has a folder of its own.
pub(crate) fn agent_tabs_in_folder<T>(tabs: &Tabs<Pane<T>>) -> Vec<String> {
    tabs.items()
        .iter()
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
                    run.started(started.notes_file, now);
                    let reader = Reader::for_agent(
                        &started.agent_id,
                        started.session_id,
                        opened.dir.clone(),
                    );
                    AgentTab { profile_name: started.profile_name, run, reader }
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
        Opened { worktree: Some(worktree()), dir: worktree().path, started }
    }

    fn started(notes: &[&str]) -> Box<Started> {
        Box::new(Started {
            profile_name: "Builder".into(),
            agent_id: "claude".into(),
            notes_file: None,
            session_id: None,
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
            },
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
        let in_other =
            Opened { worktree: None, dir: PathBuf::from("/notes"), started: Some(started(&[])) };
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
}
