//! The window as the next launch should find it: the tabs open now turned into a `SavedWindow`
//! (pure), and a cheap fingerprint of the same facts so the app can tell, without allocating, when
//! something worth saving changed. Both come from one borrowed view of each tab, so a field that
//! reaches the file always reaches the fingerprint.
//!
//! A tab that is still being made, or removed, is left out; so is one that failed without a
//! worktree. A tab that failed with a worktree is saved as a shell in it, so the work is not orphaned.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use tidewatch_core::agents::Level;
use tidewatch_core::session::{SavedAgentTab, SavedTab, SavedWindow};

use crate::pane::{AgentTab, Body, Pane};
use crate::repo::Workspace;
use crate::tabs::{Tab, Tabs};

/// What one workspace of the window says: its tab group, its folder, whether that is the Other
/// group, and focus mode.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowView<'a> {
    pub(crate) group: usize,
    pub(crate) dir: &'a Path,
    pub(crate) other: bool,
    pub(crate) focus: bool,
}

impl<'a> WindowView<'a> {
    pub(crate) fn of(group: usize, workspace: &'a Workspace, focus: bool) -> Self {
        Self {
            group,
            dir: workspace.dir(),
            other: matches!(workspace, Workspace::Other { .. }),
            focus,
        }
    }
}

#[derive(Hash)]
enum TabRef<'a> {
    Main,
    Shell { dir: &'a Path, branch: Option<&'a str> },
    Agent(AgentRef<'a>),
}

#[derive(Hash)]
struct AgentRef<'a> {
    dir: &'a Path,
    branch: Option<&'a str>,
    agent: &'a str,
    profile: &'a str,
    level: Level,
    session_id: Option<&'a str>,
    stages: Option<&'a Path>,
    crashed: bool,
}

impl TabRef<'_> {
    fn into_saved(self) -> SavedTab {
        match self {
            TabRef::Main => SavedTab::Main,
            TabRef::Shell { dir, branch } => {
                SavedTab::Shell { dir: dir.to_path_buf(), branch: branch.map(str::to_owned) }
            }
            TabRef::Agent(a) => SavedTab::Agent(SavedAgentTab {
                dir: a.dir.to_path_buf(),
                branch: a.branch.map(str::to_owned),
                agent: a.agent.to_owned(),
                profile: a.profile.to_owned(),
                level: a.level,
                session_id: a.session_id.map(str::to_owned),
                stages: a.stages.map(Path::to_path_buf),
                crashed: a.crashed,
            }),
        }
    }
}

fn agent_ref<'a>(
    agent: &'a AgentTab,
    dir: &'a Path,
    branch: Option<&'a str>,
) -> Option<TabRef<'a>> {
    let run = agent.workflow.as_deref();
    let (profile, level, session_id) = match (&agent.launched, run) {
        (Some(launched), _) => {
            (launched.profile_id.as_str(), launched.level, launched.session_id.as_deref())
        }
        (None, Some(run)) => {
            let profile = run.runner().profile(run.runner().position())?;
            (profile.name.as_str(), profile.level, None)
        }
        (None, None) => return None,
    };
    Some(TabRef::Agent(AgentRef {
        dir,
        branch,
        agent: &agent.run.agent_id,
        profile,
        level,
        session_id,
        stages: run.and_then(|run| run.stages_file()),
        crashed: agent.run.exited_abnormally(),
    }))
}

fn tab_ref<'a, T>(tab: &'a Tab<Pane<T>>, window_dir: &'a Path) -> Option<TabRef<'a>> {
    if tab.pinned {
        return Some(TabRef::Main);
    }
    let pane = &tab.pane;
    let (dir, branch) = match &pane.worktree {
        Some(worktree) => (worktree.path.as_path(), Some(worktree.branch.as_str())),
        None => (window_dir, None),
    };
    match &pane.body {
        Body::Creating | Body::Removing | Body::Draft(_) => None,
        Body::Failed(_) => pane.worktree.as_ref().map(|_| TabRef::Shell { dir, branch }),
        Body::Shell(_) => Some(
            pane.agent
                .as_ref()
                .and_then(|agent| agent_ref(agent, dir, branch))
                .unwrap_or(TabRef::Shell { dir, branch }),
        ),
    }
}

/// The tabs of the view's group to save, in order, each with whether it sits at or before the
/// group's selected tab.
fn kept<'a, T>(
    tabs: &'a Tabs<Pane<T>>,
    view: &WindowView<'a>,
) -> impl Iterator<Item = (bool, TabRef<'a>)> {
    let selected = tabs.selected_in(view.group);
    let dir = view.dir;
    tabs.in_group(view.group)
        .filter_map(move |(i, tab)| tab_ref(tab, dir).map(|tab| (i <= selected, tab)))
}

fn active_index(before_or_at_active: usize) -> usize {
    before_or_at_active.saturating_sub(1)
}

/// The window to write for one workspace. The active tab is the saved one at or just before the
/// group's selected tab, so it still points somewhere when that tab is one that is not saved.
pub(crate) fn saved_window<T>(view: &WindowView, tabs: &Tabs<Pane<T>>) -> SavedWindow {
    let mut active = 0;
    let saved: Vec<SavedTab> = kept(tabs, view)
        .map(|(upto_active, tab)| {
            active += usize::from(upto_active);
            tab.into_saved()
        })
        .collect();
    SavedWindow {
        dir: view.dir.to_path_buf(),
        other: view.other,
        active: active_index(active),
        focus_mode: view.focus,
        tabs: saved,
    }
}

/// Changes whenever `saved_window` would. Allocates nothing.
pub(crate) fn signature<T>(view: &WindowView, tabs: &Tabs<Pane<T>>) -> u64 {
    let mut hasher = DefaultHasher::new();
    view.dir.hash(&mut hasher);
    view.other.hash(&mut hasher);
    view.focus.hash(&mut hasher);
    let mut active = 0;
    for (upto_active, tab) in kept(tabs, view) {
        active += usize::from(upto_active);
        tab.hash(&mut hasher);
    }
    active_index(active).hash(&mut hasher);
    hasher.finish()
}

/// The folders of every tab a window saved that lives in a worktree: what a leftover scan must not
/// count as left over.
pub(crate) fn worktree_dirs(window: &SavedWindow) -> Vec<PathBuf> {
    window
        .tabs
        .iter()
        .filter(|tab| tab.branch().is_some())
        .map(|tab| tab.dir(&window.dir).to_path_buf())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::time::Duration;
    use tidewatch_core::runner::{RunContext, Runner};
    use tidewatch_core::workflows::Workflows;
    use tidewatch_core::worktree::Worktree;

    use crate::agent_run::{AgentRun, TermSnapshot};
    use crate::pane::Launched;
    use crate::workflow_run::TabRun;

    const DIR: &str = "/work/web";

    fn view(focus: bool) -> WindowView<'static> {
        WindowView { group: 0, dir: Path::new(DIR), other: false, focus }
    }

    fn worktree(name: &str) -> Worktree {
        Worktree {
            repo: PathBuf::from(DIR),
            path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
            branch: format!("agent/{name}"),
        }
    }

    fn launched(session: Option<&str>) -> Launched {
        Launched {
            profile_id: "builder".into(),
            level: Level::Edits,
            session_id: session.map(str::to_owned),
        }
    }

    fn agent(session: Option<&str>) -> AgentTab {
        let mut tab = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        tab.launched = Some(launched(session));
        tab
    }

    fn pane(body: Body<u32>, worktree: Option<Worktree>, agent: Option<AgentTab>) -> Pane<u32> {
        Pane { body, worktree, agent }
    }

    fn workflow_run() -> TabRun {
        let ship = Workflows::defaults().workflow.remove("ship").unwrap();
        let context = RunContext {
            task: "t".into(),
            base: "main".into(),
            branch: "agent/otter".into(),
            test_command: None,
        };
        TabRun::new(
            Runner::new(&ship, &BTreeMap::new(), context).unwrap(),
            Some(PathBuf::from("/home/.tidewatch/.state/web-abc123/otter/stages.json")),
            Duration::from_secs(60),
        )
    }

    fn tabs() -> Tabs<Pane<u32>> {
        Tabs::new(Pane::shell(0))
    }

    #[test]
    fn a_window_view_says_whether_the_folder_is_a_repo_or_the_other_group() {
        let repo =
            Workspace::Repo(crate::repo::Repo { root: PathBuf::from(DIR), label: "web".into() });
        let other = Workspace::Other { dir: PathBuf::from("/scratch"), label: "scratch".into() };
        let (repo, other) = (WindowView::of(0, &repo, true), WindowView::of(1, &other, false));
        assert_eq!((repo.dir, repo.other, repo.focus), (Path::new(DIR), false, true));
        assert_eq!((other.dir, other.other, other.focus), (Path::new("/scratch"), true, false));
        assert_eq!((repo.group, other.group), (0, 1));
    }

    #[test]
    fn a_window_with_only_the_main_tab_saves_just_that() {
        let saved = saved_window(&view(false), &tabs());
        assert_eq!(
            saved,
            SavedWindow {
                dir: PathBuf::from(DIR),
                other: false,
                active: 0,
                focus_mode: false,
                tabs: vec![SavedTab::Main],
            }
        );
    }

    #[test]
    fn tabs_are_saved_in_order_with_their_kind_worktree_and_agent_facts() {
        let mut tabs = tabs();
        tabs.push("calm-comet".into(), pane(Body::Shell(1), Some(worktree("calm-comet")), None));
        tabs.push(
            "brave-otter".into(),
            pane(
                Body::Shell(2),
                Some(worktree("brave-otter")),
                Some(agent(Some("0c2cbc96-1111-4222-8333-444455556666"))),
            ),
        );
        let saved = saved_window(&view(true), &tabs);
        assert!(saved.focus_mode);
        assert_eq!(saved.active, 2);
        assert_eq!(
            saved.tabs,
            vec![
                SavedTab::Main,
                SavedTab::Shell {
                    dir: worktree("calm-comet").path,
                    branch: Some("agent/calm-comet".into()),
                },
                SavedTab::Agent(SavedAgentTab {
                    dir: worktree("brave-otter").path,
                    branch: Some("agent/brave-otter".into()),
                    agent: "claude".into(),
                    profile: "builder".into(),
                    level: Level::Edits,
                    session_id: Some("0c2cbc96-1111-4222-8333-444455556666".into()),
                    stages: None,
                    crashed: false,
                }),
            ]
        );
    }

    #[test]
    fn the_active_tab_is_the_selected_one_not_the_last() {
        let mut tabs = tabs();
        tabs.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), None));
        tabs.push("b".into(), pane(Body::Shell(2), Some(worktree("b")), None));
        tabs.apply(crate::tabs::Action::Select(1), || unreachable!());
        assert_eq!(saved_window(&view(false), &tabs).active, 1);
    }

    #[test]
    fn a_tab_being_made_or_removed_is_left_out_and_the_active_index_follows_the_saved_list() {
        let mut tabs = tabs();
        tabs.push("making".into(), pane(Body::Creating, None, None));
        tabs.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), None));
        tabs.push("going".into(), pane(Body::Removing, Some(worktree("going")), None));
        tabs.push("b".into(), pane(Body::Shell(2), Some(worktree("b")), None));
        let saved = saved_window(&view(false), &tabs);
        assert_eq!(saved.tabs.len(), 3);
        assert_eq!(saved.active, 2, "b is the third saved tab");
        tabs.apply(crate::tabs::Action::Select(3), || unreachable!());
        assert_eq!(saved_window(&view(false), &tabs).active, 1, "the removed tab falls back to a");
        tabs.apply(crate::tabs::Action::Select(1), || unreachable!());
        assert_eq!(
            saved_window(&view(false), &tabs).active,
            0,
            "a tab being made falls back to main"
        );
    }

    #[test]
    fn a_failed_tab_keeps_its_worktree_as_a_shell_and_one_without_is_dropped() {
        let mut tabs = tabs();
        tabs.push(
            "kept".into(),
            pane(Body::Failed("no flags".into()), Some(worktree("kept")), Some(agent(None))),
        );
        tabs.push("nothing".into(), pane(Body::Failed("disk full".into()), None, None));
        let saved = saved_window(&view(false), &tabs);
        assert_eq!(saved.tabs.len(), 2);
        assert_eq!(
            saved.tabs[1],
            SavedTab::Shell { dir: worktree("kept").path, branch: Some("agent/kept".into()) }
        );
    }

    #[test]
    fn in_the_other_group_a_shell_and_an_agent_live_in_the_folder_itself() {
        let mut tabs = tabs();
        tabs.push("shell".into(), pane(Body::Shell(1), None, None));
        tabs.push("Builder".into(), pane(Body::Shell(2), None, Some(agent(Some("abc")))));
        let other = WindowView { group: 0, dir: Path::new("/scratch"), other: true, focus: false };
        let saved = saved_window(&other, &tabs);
        assert!(saved.other);
        assert_eq!(saved.tabs[1], SavedTab::Shell { dir: PathBuf::from("/scratch"), branch: None });
        let SavedTab::Agent(agent) = &saved.tabs[2] else { panic!("an agent tab") };
        assert_eq!((agent.dir.as_path(), agent.branch.as_deref()), (Path::new("/scratch"), None));
    }

    #[test]
    fn a_workflow_tab_saves_its_stages_file_and_the_profile_of_the_stage_it_is_at() {
        let mut tab =
            AgentTab::new("Ship a feature".into(), AgentRun::setting_up("claude".into(), 0.0));
        tab.workflow = Some(Box::new(workflow_run()));
        let mut tabs = tabs();
        tabs.push("otter".into(), pane(Body::Shell(1), Some(worktree("otter")), Some(tab)));
        let saved = saved_window(&view(false), &tabs);
        let SavedTab::Agent(agent) = &saved.tabs[1] else { panic!("an agent tab") };
        assert_eq!(
            agent.stages.as_deref(),
            Some(Path::new("/home/.tidewatch/.state/web-abc123/otter/stages.json"))
        );
        assert_eq!(agent.session_id, None);
        assert_ne!(agent.profile, "");
    }

    fn tab_with_exit(code: Option<i32>) -> AgentTab {
        let mut tab = agent(Some("0c2cbc96-1111-4222-8333-444455556666"));
        tab.run.started(None, 0.0);
        tab.run.feed(TermSnapshot { exit_code: code, ..Default::default() }, 1.0);
        tab
    }

    #[test]
    fn an_agent_that_ended_abnormally_is_saved_as_crashed_and_one_that_quit_is_not() {
        let saved_crashed = |code| {
            let mut tabs = tabs();
            tabs.push(
                "a".into(),
                pane(Body::Shell(1), Some(worktree("a")), Some(tab_with_exit(code))),
            );
            let SavedTab::Agent(agent) = saved_window(&view(false), &tabs).tabs.remove(1) else {
                panic!("an agent tab")
            };
            agent.crashed
        };
        let cases = [(None, false), (Some(0), false), (Some(130), false), (Some(1), true)];
        for (code, want) in cases {
            assert_eq!(saved_crashed(code), want, "exit {code:?}");
        }
    }

    #[test]
    fn an_agent_dying_abnormally_is_a_change_worth_saving() {
        let with = |code| {
            let mut tabs = tabs();
            tabs.push(
                "a".into(),
                pane(Body::Shell(1), Some(worktree("a")), Some(tab_with_exit(code))),
            );
            signature(&view(false), &tabs)
        };
        assert_ne!(with(None), with(Some(137)));
        assert_eq!(with(None), with(Some(0)), "a plain quit changes nothing that is saved");
    }

    #[test]
    fn an_agent_tab_that_never_started_is_saved_as_a_shell_in_its_worktree() {
        let mut tabs = tabs();
        let never = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        tabs.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), Some(never)));
        assert!(matches!(saved_window(&view(false), &tabs).tabs[1], SavedTab::Shell { .. }));
    }

    #[test]
    fn the_signature_moves_with_everything_that_is_saved_and_nothing_else() {
        let base = |session: Option<&str>| {
            let mut tabs = tabs();
            tabs.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), Some(agent(session))));
            tabs
        };
        let sig = |tabs: &Tabs<Pane<u32>>, focus| signature(&view(focus), tabs);
        let one = base(Some("s1"));
        assert_eq!(sig(&one, false), sig(&base(Some("s1")), false));
        assert_ne!(sig(&one, false), sig(&one, true), "focus mode");
        assert_ne!(sig(&one, false), sig(&base(Some("s2")), false), "the session id");
        assert_ne!(sig(&one, false), sig(&base(None), false));

        let mut more = base(Some("s1"));
        more.push("b".into(), pane(Body::Shell(2), Some(worktree("b")), None));
        assert_ne!(sig(&one, false), sig(&more, false), "a new tab");

        let mut moved = base(Some("s1"));
        moved.apply(crate::tabs::Action::Select(0), || unreachable!());
        assert_ne!(sig(&one, false), sig(&moved, false), "the active tab");

        let mut renamed = base(Some("s1"));
        renamed.items_mut()[1].title = "something else".into();
        assert_eq!(sig(&one, false), sig(&renamed, false), "a title is not saved");
    }

    #[test]
    fn moving_a_tab_changes_the_saved_order_the_active_index_and_the_signature() {
        let mut tabs = tabs();
        tabs.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), None));
        tabs.push("b".into(), pane(Body::Shell(2), Some(worktree("b")), None));
        let before = saved_window(&view(false), &tabs);
        let before_signature = signature(&view(false), &tabs);
        tabs.apply(crate::tabs::Action::MoveActive(crate::tabs::Step::Left), || unreachable!());
        let after = saved_window(&view(false), &tabs);
        let mut swapped = before.tabs.clone();
        swapped.swap(1, 2);
        assert_eq!(after.tabs, swapped);
        assert_eq!(after.active, 1, "the active tab is still b");
        assert_ne!(before_signature, signature(&view(false), &tabs));
        tabs.apply(crate::tabs::Action::MoveActive(crate::tabs::Step::Right), || unreachable!());
        assert_eq!(saved_window(&view(false), &tabs), before);
        assert_eq!(signature(&view(false), &tabs), before_signature);
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
    fn a_draft_is_never_saved_and_the_active_index_falls_back_past_it() {
        let mut tabs = tabs();
        tabs.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), None));
        tabs.push("New agent".into(), draft());
        let saved = saved_window(&view(false), &tabs);
        assert_eq!(saved.tabs.len(), 2);
        assert_eq!(saved.active, 1, "the draft is active, so the last saved tab before it is");
        assert_eq!(signature(&view(false), &tabs), {
            let mut without = self::tabs();
            without.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), None));
            without.apply(crate::tabs::Action::Select(1), || unreachable!());
            signature(&view(false), &without)
        });
    }

    #[test]
    fn a_tab_that_is_still_being_made_does_not_move_the_signature() {
        let one = tabs();
        let mut making = tabs();
        making.push("new worktree".into(), pane(Body::Creating, None, None));
        making.apply(crate::tabs::Action::Select(0), || unreachable!());
        assert_eq!(signature(&view(false), &one), signature(&view(false), &making));
    }

    #[test]
    fn the_worktree_dirs_of_a_saved_window_are_those_of_its_worktree_tabs() {
        let mut tabs = tabs();
        tabs.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), None));
        tabs.push("b".into(), pane(Body::Shell(2), None, None));
        let saved = saved_window(&view(false), &tabs);
        assert_eq!(worktree_dirs(&saved), vec![worktree("a").path]);
    }

    #[test]
    fn each_group_saves_its_own_tabs_and_its_own_active_tab() {
        let mut tabs = tabs();
        tabs.push("a".into(), pane(Body::Shell(1), Some(worktree("a")), None));
        let api = tabs.add_group(Pane::shell(10));
        let b = tabs.push_in(api, "b".into(), pane(Body::Shell(2), Some(worktree("b")), None));
        tabs.push_in(api, "c".into(), pane(Body::Shell(3), Some(worktree("c")), None));
        tabs.select_id(b);
        let web = saved_window(&view(false), &tabs);
        let other =
            WindowView { group: api, dir: Path::new("/work/api"), other: false, focus: false };
        let api = saved_window(&other, &tabs);
        assert_eq!((web.tabs.len(), web.active), (2, 1));
        assert_eq!((api.dir.as_path(), api.tabs.len(), api.active), (Path::new("/work/api"), 3, 1));
        assert_eq!(api.tabs[0], SavedTab::Main);
        assert_eq!(worktree_dirs(&api), vec![worktree("b").path, worktree("c").path]);
    }

    #[test]
    fn a_tab_in_another_group_moves_only_that_groups_signature() {
        let mut tabs = tabs();
        let api = tabs.add_group(Pane::shell(10));
        let other =
            WindowView { group: api, dir: Path::new("/work/api"), other: false, focus: false };
        let (web_before, api_before) = (signature(&view(false), &tabs), signature(&other, &tabs));
        tabs.push_in(api, "b".into(), pane(Body::Shell(2), Some(worktree("b")), None));
        assert_eq!(signature(&view(false), &tabs), web_before);
        assert_ne!(signature(&other, &tabs), api_before);
    }
}
