use std::collections::BTreeMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use stdusk_core::handoff::{recv_session, send_session};
use stdusk_core::repaint::Repaint;
use stdusk_core::shell::ShellProfile;
use stdusk_core::terminal::SpawnOpts;
use tidewatch_core::agents::Level;
use tidewatch_core::runner::RunContext;
use tidewatch_core::workflows::Workflows;

use super::*;
use crate::park::Why;
use crate::repo::Repo;

const REPO: &str = "/work/web";

fn repo(dir: &str) -> Workspace {
    Workspace::Repo(Repo { root: PathBuf::from(dir), label: dir.into() })
}

fn other(dir: &str) -> Workspace {
    Workspace::Other { dir: PathBuf::from(dir), label: dir.into() }
}

fn worktree(name: &str) -> Worktree {
    Worktree {
        repo: PathBuf::from(REPO),
        path: PathBuf::from(format!("/home/.tidewatch/web-abc123/{name}")),
        branch: format!("agent/{name}"),
    }
}

fn credential(token: &str) -> Credential {
    Credential {
        token: token.into(),
        generation: 2,
        agent: "claude".into(),
        notes_file: PathBuf::from("/home/.tidewatch/.state/web-abc123/otter/notes.json"),
    }
}

fn launched(session: &str) -> Launched {
    Launched {
        profile_id: "builder".into(),
        level: Level::Edits,
        session_id: Some(session.into()),
        remote: false,
        dir: None,
    }
}

fn agent_pane(term: u32, name: &str, session: &str) -> Pane<u32> {
    let mut tab = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
    tab.run.started(Some(PathBuf::from(format!("/notes/{name}.json"))), 1.0);
    tab.launched = Some(launched(session));
    tab.first_prompt = FirstPrompt::Sent;
    Pane { body: Body::Shell(term), worktree: Some(worktree(name)), agent: Some(tab) }
}

fn facts(workspaces: &[Workspace], hook_port: Option<u16>) -> Facts<'_> {
    Facts { workspaces, focus: true, welcome: false, hook_port }
}

fn no_credentials(_: u64) -> Option<Credential> {
    None
}

struct Window {
    tabs: Tabs<Pane<u32>>,
    workspaces: Vec<Workspace>,
    agent_tab: u64,
}

fn window() -> Window {
    let mut tabs = Tabs::new(Pane::shell(100));
    let agent_tab = tabs.push_in(0, "otter".into(), agent_pane(101, "otter", "sess-1"));
    tabs.push_in(0, "shell".into(), Pane::shell(102));
    let second = tabs.add_group(Pane::shell(200));
    tabs.push_in(second, "api work".into(), Pane::shell(201));
    Window { tabs, workspaces: vec![repo(REPO), other("/scratch")], agent_tab }
}

#[test]
fn the_layout_lists_every_tab_in_order_and_names_the_ptys_that_go_with_it() {
    let mut w = window();
    let second = w.tabs.in_group(1).nth(1).map(|(_, tab)| tab.id).unwrap();
    w.tabs.select_id(second);
    w.tabs.select_id(w.agent_tab);
    let agent_tab = w.agent_tab;
    let credentials = move |id: u64| (id == agent_tab).then(|| credential("tok-a"));
    let (handover, order) = plan(&w.tabs, &facts(&w.workspaces, Some(4711)), &credentials);

    let titles: Vec<Vec<&str>> = handover
        .groups
        .iter()
        .map(|group| group.tabs.iter().map(|tab| tab.title.as_str()).collect())
        .collect();
    assert_eq!(titles, [vec!["main", "otter", "shell"], vec!["main", "api work"]]);
    assert_eq!(order, [0, 1, 2, 3, 4]);
    assert_eq!(handover.pane_count(), 5);
    assert_eq!(handover.active_group, 0);
    assert_eq!((handover.focus, handover.welcome, handover.hook_port), (true, false, Some(4711)));
    assert_eq!(
        handover.groups.iter().map(|g| (g.dir.clone(), g.other)).collect::<Vec<_>>(),
        [(PathBuf::from(REPO), false), (PathBuf::from("/scratch"), true)]
    );
    assert_eq!((handover.groups[0].selected, handover.groups[1].selected), (1, 1));
}

#[test]
fn an_agent_tab_carries_what_it_needs_to_keep_running_and_a_shell_carries_nothing_extra() {
    let w = window();
    let agent_tab = w.agent_tab;
    let credentials = move |id: u64| (id == agent_tab).then(|| credential("tok-a"));
    let (handover, _) = plan(&w.tabs, &facts(&w.workspaces, None), &credentials);

    let agent = handover.groups[0].tabs[1].agent.as_ref().unwrap();
    assert_eq!(agent.agent_id, "claude");
    assert_eq!(agent.profile_name, "Builder");
    assert_eq!(agent.launched, Some(launched("sess-1")));
    assert_eq!(agent.first_prompt, FirstPrompt::Sent);
    assert_eq!(agent.notes_file, Some(PathBuf::from("/notes/otter.json")));
    assert_eq!(agent.credential, Some(credential("tok-a")));
    assert_eq!(agent.workflow, None);
    assert_eq!(handover.groups[0].tabs[1].worktree, Some(worktree("otter")));
    assert!(handover.groups[0].tabs[2].agent.is_none());
    assert!(handover.groups[0].tabs[0].pinned);
}

#[test]
fn a_tab_that_failed_in_a_worktree_stays_without_a_pty_and_one_without_a_worktree_goes() {
    let mut w = window();
    w.tabs.push_in(
        0,
        "broken".into(),
        Pane { body: Body::Failed("boom".into()), worktree: Some(worktree("broken")), agent: None },
    );
    w.tabs.push_in(
        0,
        "failed".into(),
        Pane { body: Body::Failed("boom".into()), worktree: None, agent: None },
    );
    let (handover, order) = plan(&w.tabs, &facts(&w.workspaces, None), &no_credentials);

    let group = &handover.groups[0];
    let kept: Vec<(&str, bool)> = group.tabs.iter().map(|t| (t.title.as_str(), t.live)).collect();
    assert_eq!(kept, [("main", true), ("otter", true), ("shell", true), ("broken", false)]);
    assert_eq!(handover.pane_count(), 5, "the failed tab sends no pty");
    assert_eq!(order.len(), 5);
    let shells: Vec<usize> = w
        .tabs
        .items()
        .iter()
        .enumerate()
        .filter(|(_, tab)| matches!(tab.pane.body, Body::Shell(_)))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(order, shells);
}

#[test]
fn a_poofed_summary_travels_as_its_record_with_no_pty_and_comes_back_a_summary() {
    use tidewatch_core::civil_date::Date;
    use tidewatch_core::meeseeks_log::Poofed;
    let record = Poofed::bare("promo-test", "Done.", None, 60, Date::new(2026, 10, 25).unwrap());
    let mut w = window();
    let poofed =
        Pane::<u32> { body: Body::Poofed(Box::new(record.clone())), worktree: None, agent: None };
    w.tabs.push_in(0, "promo-test".into(), poofed);
    let (handover, order) = plan(&w.tabs, &facts(&w.workspaces, None), &no_credentials);
    let entry = handover.groups[0].tabs.last().unwrap();
    assert_eq!((entry.title.as_str(), entry.live), ("promo-test", false));
    assert_eq!(handover.pane_count(), 5, "the summary sends no pty");
    assert_eq!(order.len(), 5);
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap();
    let back = rebuilt.tabs.in_group(0).last().map(|(_, tab)| tab).unwrap();
    assert!(matches!(&back.pane.body, Body::Poofed(shown) if **shown == record));
    assert!(back.pane.agent.is_none() && back.pane.worktree.is_none());
}

#[test]
fn the_selected_tab_of_a_group_follows_the_tabs_that_are_kept() {
    let mut w = window();
    let failed = w.tabs.push_in(
        0,
        "failed".into(),
        Pane { body: Body::Failed("boom".into()), worktree: None, agent: None },
    );
    w.tabs.select_id(failed);
    let (handover, _) = plan(&w.tabs, &facts(&w.workspaces, None), &no_credentials);
    assert_eq!(handover.groups[0].selected, 2, "the tab before the one that is left out");
}

#[test]
fn the_group_on_show_is_the_active_one() {
    let mut w = window();
    w.tabs.apply(Action::ShowGroup(1), || unreachable!());
    let (handover, _) = plan(&w.tabs, &facts(&w.workspaces, None), &no_credentials);
    assert_eq!(handover.active_group, 1);
}

fn workflow_run(file: &Path) -> TabRun {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let context = RunContext {
        task: "add a retry".into(),
        base: "main".into(),
        branch: "agent/otter".into(),
        test_command: None,
    };
    let runner = Runner::new(&ship, &BTreeMap::new(), context).unwrap();
    TabRun::new(runner, Some(file.to_path_buf()), Duration::from_secs(90))
        .with_blurb("Ship it".into())
        .with_generation(3)
}

#[test]
fn a_window_with_a_tab_in_transition_or_a_stage_starting_cannot_hand_over() {
    let mut w = window();
    assert_eq!(blocker(&w.tabs), None);

    let making = w.tabs.push_in(0, "new".into(), Pane::creating());
    assert_eq!(blocker(&w.tabs), Some(Blocker::TabInTransition));
    w.tabs.remove_id(making);

    let removing = w.tabs.push_in(0, "old".into(), agent_pane(300, "old", "s"));
    w.tabs.get_mut(removing).unwrap().pane.begin_remove();
    assert_eq!(blocker(&w.tabs), Some(Blocker::TabInTransition));
    w.tabs.remove_id(removing);

    let dir = tempfile::tempdir().unwrap();
    let mut run = workflow_run(&dir.path().join("stages.json"));
    assert!(!run.busy());
    let execs = run.start();
    assert!(matches!(execs.as_slice(), [crate::workflow_run::Exec::Launch(_)]));
    assert!(run.busy(), "a launch is on its way");
    let mut pane = agent_pane(301, "flow", "s");
    pane.agent.as_mut().unwrap().workflow = Some(Box::new(run));
    w.tabs.push_in(0, "flow".into(), pane);
    assert_eq!(blocker(&w.tabs), Some(Blocker::WorkflowBusy));
}

#[test]
fn a_draft_neither_blocks_a_handover_nor_travels_with_it() {
    let mut w = window();
    let before = plan(&w.tabs, &facts(&w.workspaces, None), &no_credentials);
    let form = crate::new_tab::NewTab::open(
        crate::new_tab::Group::Repo,
        "web".into(),
        std::path::PathBuf::from("/work/web"),
        Vec::new(),
        None,
        1,
    );
    w.tabs.push_in(0, "New agent".into(), Pane::draft(form));
    assert_eq!(blocker(&w.tabs), None);
    assert_eq!(plan(&w.tabs, &facts(&w.workspaces, None), &no_credentials), before);
}

#[test]
fn a_failed_tab_alone_does_not_block() {
    let mut w = window();
    w.tabs.push_in(
        0,
        "failed".into(),
        Pane { body: Body::Failed("boom".into()), worktree: None, agent: None },
    );
    assert_eq!(blocker(&w.tabs), None);
}

fn handover_of(window: &Window) -> (Handover, Vec<usize>) {
    let agent_tab = window.agent_tab;
    let credentials = move |id: u64| (id == agent_tab).then(|| credential("tok-a"));
    plan(&window.tabs, &facts(&window.workspaces, Some(4711)), &credentials)
}

fn terms(tabs: &Tabs<Pane<u32>>) -> Vec<(usize, String, u32)> {
    tabs.items()
        .iter()
        .map(|tab| match tab.pane.body {
            Body::Shell(term) => (tab.group, tab.title.clone(), term),
            _ => panic!("every tab has a shell"),
        })
        .collect()
}

fn fresh(_: &Path, _: Option<&str>) -> u32 {
    999
}

#[test]
fn each_received_terminal_lands_in_the_tab_at_its_position() {
    let w = window();
    let (handover, order) = handover_of(&w);
    assert_eq!(order.len(), 5);
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap();

    assert_eq!(
        terms(&rebuilt.tabs),
        [
            (0, "main".into(), 100),
            (0, "otter".into(), 101),
            (0, "shell".into(), 102),
            (1, "main".into(), 200),
            (1, "api work".into(), 201),
        ]
    );
    assert_eq!(rebuilt.notes, Vec::<String>::new());
}

#[test]
fn the_active_group_the_selected_tabs_and_the_pinned_mains_come_back() {
    let mut w = window();
    let shell = w.tabs.in_group(0).nth(2).map(|(_, tab)| tab.id).unwrap();
    w.tabs.select_id(shell);
    w.tabs.apply(Action::ShowGroup(1), || unreachable!());
    let (handover, _) = handover_of(&w);
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let mut rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap()
    .tabs;

    assert_eq!(rebuilt.active_group(), 1);
    let front = rebuilt.items()[rebuilt.active()].title.clone();
    assert_eq!(front, "main", "the second group's own selected tab");
    let first_group_selected = rebuilt.selected_in(0);
    assert_eq!(rebuilt.items()[first_group_selected].title, "shell");
    assert!(rebuilt.main_id(0).is_some() && rebuilt.main_id(1).is_some());
    rebuilt.apply(Action::ShowGroup(0), || unreachable!());
    assert_eq!(rebuilt.items()[rebuilt.active()].title, "shell");
}

#[test]
fn a_layout_with_a_group_per_linked_worktree_comes_back_as_one_group_of_the_main_repo() {
    let mut tabs = Tabs::new(Pane::shell(100));
    let fix = tabs.add_group(Pane::shell(200));
    let nn = tabs.add_group(Pane::shell(300));
    let other_main = tabs.add_group(Pane::shell(400));
    let agent = tabs.push_in(fix, "otter".into(), agent_pane(201, "otter", "sess-1"));
    tabs.push_in(0, "shell".into(), Pane::shell(101));
    tabs.push_in(nn, "nn work".into(), Pane::shell(301));
    tabs.push_in(other_main, "api work".into(), Pane::shell(401));
    let sent = Window {
        tabs,
        workspaces: vec![repo("/w/web"), repo("/w/web-fix"), repo("/w/web-nn"), repo("/w/api")],
        agent_tab: agent,
    };
    let (handover, _) = handover_of(&sent);
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 200, 201, 300, 301, 400, 401],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap();
    assert_eq!(rebuilt.tabs.active_group(), 0);

    let mut tabs = rebuilt.tabs;
    let mut workspaces: Vec<Workspace> =
        handover.groups.iter().map(|group| repo(&group.dir.to_string_lossy())).collect();
    let main_of = |dir: &Path| {
        let root = if dir.starts_with("/w/api") { "/w/api" } else { "/w/web" };
        crate::repo::Repo { root: PathBuf::from(root), label: root.into() }
    };
    let mains = workspaces
        .iter()
        .map(|workspace| {
            let repo = main_of(workspace.dir());
            Some(crate::merge_groups::Main {
                at_home: repo.root == workspace.dir(),
                key: repo.root.clone(),
                repo,
            })
        })
        .collect();
    let facts = crate::merge_groups::facts(mains, &tabs, |_| false);
    let merges = crate::merge_groups::plan(&facts);
    crate::merge_groups::apply(&mut tabs, &mut workspaces, &merges, |_| unreachable!());

    assert_eq!(
        workspaces.iter().map(Workspace::dir).collect::<Vec<_>>(),
        [Path::new("/w/web"), Path::new("/w/api")]
    );
    assert_eq!(
        terms(&tabs),
        [
            (0, "main".into(), 100),
            (0, "shell".into(), 101),
            (0, "otter".into(), 201),
            (0, "nn work".into(), 301),
            (1, "main".into(), 400),
            (1, "api work".into(), 401),
        ]
    );
    let otter = tabs.get_mut(tabs.items()[2].id).unwrap();
    assert_eq!(otter.pane.worktree, Some(worktree("otter")));
    assert!(otter.pane.agent.is_some(), "the agent runs on in the merged group");
}

#[test]
fn parking_and_never_park_cross_the_handover_and_an_old_layout_has_neither() {
    let mut w = window();
    let (otter, shell) = (w.agent_tab, w.tabs.in_group(0).nth(2).map(|(_, tab)| tab.id).unwrap());
    w.tabs.set_park(otter, Park { why: Why::Job, since_ms: 5 });
    w.tabs.get_mut(shell).unwrap().keep = true;
    let (handover, _) = handover_of(&w);
    let text = serde_json::to_string(&handover).unwrap();
    let handover: Handover = serde_json::from_str(&text).unwrap();
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap()
    .tabs;
    let state: Vec<_> = rebuilt.items().iter().map(|tab| (tab.park, tab.keep)).collect();
    assert_eq!(
        state,
        [
            (None, false),
            (Some(Park { why: Why::Job, since_ms: 5 }), false),
            (None, true),
            (None, false),
            (None, false)
        ]
    );
    assert_eq!((text.matches("\"park\"").count(), text.matches("\"keep\"").count()), (1, 1));

    let plain = serde_json::to_string(&handover_of(&window()).0).unwrap();
    assert!(!plain.contains("park") && !plain.contains("keep"), "{plain}");
    let old: Handover = serde_json::from_str(&plain).unwrap();
    assert!(old.groups.iter().flat_map(|group| &group.tabs).all(|t| t.park.is_none() && !t.keep));
}

#[test]
fn an_agent_tab_comes_back_running_with_its_reader_session_and_credential_under_a_new_id() {
    let w = window();
    let (handover, _) = handover_of(&w);
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 200, 201],
        5.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap();

    let tab = rebuilt.tabs.items().iter().find(|tab| tab.title == "otter").unwrap();
    let agent = tab.pane.agent.as_ref().unwrap();
    assert_eq!(agent.profile_name, "Builder");
    assert_eq!(agent.run.agent_id, "claude");
    assert_eq!(agent.run.notes_file, Some(PathBuf::from("/notes/otter.json")));
    assert_eq!(agent.launched, Some(launched("sess-1")));
    assert_eq!(agent.first_prompt, FirstPrompt::Sent);
    assert!(agent.workflow.is_none());
    assert_eq!(tab.pane.worktree, Some(worktree("otter")));
    assert_eq!(rebuilt.credentials, [(tab.id, credential("tok-a"))]);
}

fn rebuilt_run(w: &Window, at: f64) -> AgentRun {
    let (handover, _) = handover_of(w);
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let ptys = Ptys { adopt: &mut adopt, fresh: &fresh };
    let mut rebuilt = rebuild(&handover, vec![100, 101, 102, 200, 201], 5.0, ptys).unwrap();
    let id = rebuilt.tabs.items().iter().find(|tab| tab.title == "otter").unwrap().id;
    let mut agent = rebuilt.tabs.get_mut(id).unwrap().pane.agent.take().unwrap();
    agent.run.refresh(at);
    agent.run
}

#[test]
fn an_agent_that_was_done_comes_back_done_and_one_that_was_busy_waits_for_its_first_word() {
    let mut w = window();
    for (held, done, unreported) in [
        (AgentStatus::Done, true, false),
        (AgentStatus::NeedsYou(Reason::Input), true, false),
        (AgentStatus::Working, false, true),
        (AgentStatus::NeedsYou(Reason::Permission), false, true),
        (AgentStatus::NeedsYou(Reason::Quiet), false, true),
    ] {
        let agent = w.tabs.get_mut(w.agent_tab).unwrap().pane.agent.as_mut().unwrap();
        agent.run.hold(Some(held.clone()), None);
        agent.run.refresh(2.0);
        let run = rebuilt_run(&w, 600.0);
        assert_eq!(*run.status() == AgentStatus::Done, done, "{held:?}");
        assert_eq!(run.unreported(), unreported, "{held:?}");
    }
}

#[test]
fn an_agent_that_does_not_report_by_hooks_keeps_reading_its_terminal_after_a_handover() {
    let mut w = window();
    let agent = w.tabs.get_mut(w.agent_tab).unwrap().pane.agent.as_mut().unwrap();
    agent.run.hold(Some(AgentStatus::Done), None);
    agent.run.refresh(2.0);
    let (handover, _) = plan(&w.tabs, &facts(&w.workspaces, None), &no_credentials);
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let ptys = Ptys { adopt: &mut adopt, fresh: &fresh };
    let mut rebuilt = rebuild(&handover, vec![100, 101, 102, 200, 201], 5.0, ptys).unwrap();
    let id = rebuilt.tabs.items().iter().find(|tab| tab.title == "otter").unwrap().id;
    let run = &mut rebuilt.tabs.get_mut(id).unwrap().pane.agent.as_mut().unwrap().run;
    run.refresh(600.0);
    assert_eq!((*run.status() == AgentStatus::Done, run.unreported()), (false, false));
}

#[test]
fn an_agent_with_remote_control_comes_back_with_its_session_and_the_saved_setting() {
    let mut w = window();
    for saved in [true, false] {
        let agent = w.tabs.get_mut(w.agent_tab).unwrap().pane.agent.as_mut().unwrap();
        agent.remote = Some(RemoteSession::new("web \u{b7} otter".into()));
        agent.launched.as_mut().unwrap().remote = saved;
        let (handover, _) = handover_of(&w);
        let entry = handover.groups[0].tabs[1].agent.as_ref().unwrap();
        assert_eq!(entry.remote.as_deref(), Some("web \u{b7} otter"));
        let mut adopt = |term: u32, _: &Path| Ok(term);
        let ptys = Ptys { adopt: &mut adopt, fresh: &fresh };
        let rebuilt = rebuild(&handover, vec![100, 101, 102, 200, 201], 5.0, ptys).unwrap();
        let tab = rebuilt.tabs.items().iter().find(|tab| tab.title == "otter").unwrap();
        let agent = tab.pane.agent.as_ref().unwrap();
        assert_eq!(agent.remote.as_ref().map(|s| s.name.as_str()), Some("web \u{b7} otter"));
        assert_eq!(agent.launched.as_ref().map(|l| l.remote), Some(saved));
    }
}

#[test]
fn an_agent_without_remote_control_hands_over_without_a_session_name() {
    let w = window();
    let (handover, _) = handover_of(&w);
    assert_eq!(handover.groups[0].tabs[1].agent.as_ref().unwrap().remote, None);
}

#[test]
fn a_workflow_tab_comes_back_with_its_saved_run_and_stage_generation() {
    let dir = tempfile::tempdir().unwrap();
    let stages = dir.path().join("stages.json");
    let mut run = workflow_run(&stages);
    run.persist();
    let mut w = window();
    let mut pane = agent_pane(301, "flow", "s2");
    pane.agent.as_mut().unwrap().workflow = Some(Box::new(run));
    w.tabs.push_in(0, "flow".into(), pane);
    let (handover, _) = handover_of(&w);
    let entry = handover.groups[0].tabs.last().unwrap().agent.as_ref().unwrap();
    assert_eq!(
        entry.workflow,
        Some(WorkflowEntry {
            stages: stages.clone(),
            test_timeout_secs: 90,
            generation: 3,
            blurb: "Ship it".into(),
        })
    );

    let mut adopt = |term: u32, _: &Path| Ok(term);
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 301, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap();
    let tab = rebuilt.tabs.items().iter().find(|tab| tab.title == "flow").unwrap();
    let run = tab.pane.agent.as_ref().unwrap().workflow.as_deref().unwrap();
    assert_eq!((run.generation(), run.blurb()), (3, "Ship it"));
    assert_eq!(run.test_timeout(), Duration::from_secs(90));
    assert_eq!(run.stages_file(), Some(stages.as_path()));
    assert_eq!(rebuilt.notes, Vec::<String>::new());
}

#[test]
fn a_workflow_whose_file_is_gone_leaves_the_agent_tab_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = window();
    let mut pane = agent_pane(301, "flow", "s2");
    pane.agent.as_mut().unwrap().workflow =
        Some(Box::new(workflow_run(&dir.path().join("stages.json"))));
    w.tabs.push_in(0, "flow".into(), pane);
    let (handover, _) = handover_of(&w);

    let mut adopt = |term: u32, _: &Path| Ok(term);
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 301, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap();
    let tab = rebuilt.tabs.items().iter().find(|tab| tab.title == "flow").unwrap();
    assert!(tab.pane.agent.as_ref().unwrap().workflow.is_none());
    assert_eq!(rebuilt.notes.len(), 1);
}

#[test]
fn a_terminal_that_cannot_be_adopted_becomes_a_fresh_shell_without_the_agent() {
    let w = window();
    let (handover, _) = handover_of(&w);
    let mut adopt = |term: u32, _: &Path| {
        if term == 101 { Err(std::io::Error::other("no pty")) } else { Ok(term) }
    };
    let asked = std::cell::RefCell::new(Vec::new());
    let fresh = |dir: &Path, note: Option<&str>| {
        asked.borrow_mut().push((dir.to_path_buf(), note.map(str::to_owned)));
        999
    };
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap();

    let tab = rebuilt.tabs.items().iter().find(|tab| tab.title == "otter").unwrap();
    assert!(matches!(tab.pane.body, Body::Shell(999)));
    assert!(tab.pane.agent.is_none());
    assert_eq!(tab.pane.worktree, Some(worktree("otter")));
    assert!(rebuilt.credentials.is_empty(), "the lost agent's token is not adopted");
    assert_eq!(rebuilt.notes, [LOST_TERMINAL]);
    let asked = asked.into_inner();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, worktree("otter").path);
    assert_eq!(asked[0].1.as_deref(), Some(LOST_TERMINAL));
}

#[test]
fn a_failed_worktree_tab_gets_a_shell_in_its_worktree() {
    let mut w = window();
    w.tabs.push_in(
        0,
        "broken".into(),
        Pane { body: Body::Failed("boom".into()), worktree: Some(worktree("broken")), agent: None },
    );
    let (handover, _) = handover_of(&w);
    let asked = std::cell::RefCell::new(Vec::new());
    let fresh = |dir: &Path, note: Option<&str>| {
        asked.borrow_mut().push((dir.to_path_buf(), note.map(str::to_owned)));
        999
    };
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap();
    assert_eq!(asked.into_inner(), [(worktree("broken").path, None)]);
    let tab = rebuilt.tabs.items().iter().find(|tab| tab.title == "broken").unwrap();
    assert_eq!(tab.pane.worktree, Some(worktree("broken")));
}

#[test]
fn a_layout_that_does_not_line_up_is_refused() {
    let w = window();
    let (handover, _) = handover_of(&w);
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let short = rebuild(&handover, vec![1, 2], 0.0, Ptys { adopt: &mut adopt, fresh: &fresh });
    assert!(short.is_err());

    let mut headless = handover.clone();
    headless.groups[1].tabs[0].pinned = false;
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let result =
        rebuild(&headless, vec![1, 2, 3, 4, 5], 0.0, Ptys { adopt: &mut adopt, fresh: &fresh });
    assert!(result.is_err(), "a group needs its main tab first");

    let empty = Handover { groups: Vec::new(), ..handover };
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let none: Result<Rebuilt<u32>, String> =
        rebuild(&empty, Vec::new(), 0.0, Ptys { adopt: &mut adopt, fresh: &fresh });
    assert!(none.is_err());
}

#[test]
fn the_layout_crosses_the_wire_with_its_ptys_in_leaf_order() {
    let mut w = window();
    let flow_dir = tempfile::tempdir().unwrap();
    let mut pane = agent_pane(301, "flow", "s2");
    let mut run = workflow_run(&flow_dir.path().join("stages.json"));
    run.persist();
    pane.agent.as_mut().unwrap().workflow = Some(Box::new(run));
    w.tabs.push_in(0, "flow".into(), pane);
    let (handover, order) = handover_of(&w);
    assert_eq!(order.len(), 6);

    let pipes: Vec<_> = (0..order.len()).map(|_| std::io::pipe().unwrap()).collect();
    for (n, (_, tx)) in pipes.iter().enumerate() {
        let mut tx = tx;
        std::io::Write::write_all(&mut tx, format!("pane-{n}").as_bytes()).unwrap();
    }
    let (a, b) = UnixStream::pair().unwrap();
    let sent = handover.clone();
    let sender = std::thread::spawn(move || {
        let panes: Vec<_> = pipes.iter().map(|(rx, _)| (PaneMeta::default(), rx.as_fd())).collect();
        send_session(&a, &sent, &panes)
    });
    let (got, fds) = recv_session::<Handover>(&b).unwrap();
    assert_eq!(got, handover, "the layout survives the wire");
    assert_eq!(fds.len(), 6);
    let mut adopt = |(_, fd): (PaneMeta, std::os::fd::OwnedFd), _dir: &Path| {
        let mut text = String::new();
        std::fs::File::from(fd).take(6).read_to_string(&mut text).map(|_| text)
    };
    let fresh = |_: &Path, _: Option<&str>| String::new();
    let rebuilt = rebuild(&got, fds, 0.0, Ptys { adopt: &mut adopt, fresh: &fresh }).unwrap();
    let read: Vec<String> = rebuilt
        .tabs
        .items()
        .iter()
        .map(|tab| match &tab.pane.body {
            Body::Shell(text) => text.clone(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(read, ["pane-0", "pane-1", "pane-2", "pane-3", "pane-4", "pane-5"]);
    drop(b);
    assert!(sender.join().unwrap().is_err(), "no acknowledgement, so the sender is not released");
}

#[test]
fn a_launch_adopts_when_it_received_and_starts_normally_when_it_was_not_asked() {
    assert_eq!(boot::<u8>(None, || unreachable!("no handover, nothing to ask")), Boot::Normal);
    assert_eq!(boot(Some(Ok(7u8)), || unreachable!("it worked")), Boot::Adopt(7));
}

#[test]
fn a_failed_handover_gives_way_to_a_window_that_still_runs_and_opens_one_when_none_does() {
    let failed = || Some(Err::<u8, _>(std::io::Error::other("version mismatch")));
    assert_eq!(boot(failed(), || true), Boot::GiveWay);
    assert_eq!(boot(failed(), || false), Boot::Normal);
}

#[test]
fn the_successor_starts_through_open_from_a_bundle_and_directly_otherwise() {
    let socket = Path::new("/state/handoff-9.sock");
    let env = |name: &str| match name {
        "TIDEWATCH_HOME" => Some(OsString::from("/tmp/twh")),
        _ => None,
    };
    let from_bundle = successor_command(
        Path::new("/x/Tidewatch"),
        Some(Path::new("/Apps/Tidewatch.app")),
        socket,
        env,
    );
    assert_eq!(from_bundle.get_program(), "open");
    let args: Vec<_> = from_bundle.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(
        args,
        [
            "-n",
            "-a",
            "/Apps/Tidewatch.app",
            "--env",
            "TIDEWATCH_HOME=/tmp/twh",
            "--args",
            "--handoff",
            "/state/handoff-9.sock"
        ]
    );

    let direct = successor_command(Path::new("/x/tidewatch"), None, socket, env);
    assert_eq!(direct.get_program(), "/x/tidewatch");
    let args: Vec<_> = direct.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(args, ["--handoff", "/state/handoff-9.sock"]);
}

#[test]
fn the_successor_does_not_inherit_the_dev_switches_that_open_or_restart_tabs() {
    let command =
        successor_command(Path::new("/x/tidewatch"), None, Path::new("/s.sock"), |_| None);
    let mut removed: Vec<_> = command
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    removed.sort();
    assert_eq!(
        removed,
        [
            "TIDEWATCH_DEV_AGENT",
            "TIDEWATCH_DEV_RESTART",
            "TIDEWATCH_DEV_RESUME",
            "TIDEWATCH_DEV_WORKFLOW",
            "TIDEWATCH_DEV_WORKTREE"
        ]
    );
}

fn quiet_opts(script: &str) -> SpawnOpts {
    SpawnOpts {
        detect_progress: false,
        shell_integration: false,
        autosuggestions: false,
        scrollback_lines: 200,
        word_separators: " ".into(),
        bold_bright: false,
        agent_tracking: None,
        cwd: None,
        profile: Some(ShellProfile {
            shell: Some("/bin/sh".into()),
            args: vec!["-c".into(), script.into()],
            cwd: None,
            env: BTreeMap::new(),
        }),
    }
}

fn real_pane(script: &str) -> Pane<PtyTerm> {
    Pane::shell(PtyTerm::spawn(80, 24, Repaint::none(), &quiet_opts(script)))
}

#[test]
fn live_ptys_cross_a_real_socket_and_come_back_in_their_tabs_still_running() {
    let mut tabs = Tabs::new(real_pane("sleep 60"));
    tabs.push_in(0, "work".into(), real_pane("sleep 61"));
    tabs.add_group(real_pane("sleep 62"));
    let workspaces = [repo(REPO), other("/scratch")];
    let (handover, order) = plan(&tabs, &facts(&workspaces, None), &no_credentials);
    let sent_pids: Vec<Option<u32>> = tabs
        .items()
        .iter()
        .map(|tab| match &tab.pane.body {
            Body::Shell(term) => term.shell_pid(),
            _ => None,
        })
        .collect();
    assert!(sent_pids.iter().all(Option::is_some));

    let dir = tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("h.sock");
    let listener = stdusk_core::handoff::listen(&socket).unwrap();
    let successor = {
        let socket = socket.clone();
        std::thread::spawn(move || {
            let mut incoming = Incoming::receive(&socket).unwrap();
            let panes = incoming.take_panes();
            let handover = incoming.session().clone();
            let mut adopt = |(meta, fd): (PaneMeta, std::os::fd::OwnedFd), _: &Path| {
                stdusk_core::handoff::adopt_pane(Repaint::none(), &meta, fd, &quiet_opts("unused"))
            };
            let fresh = |_: &Path, _: Option<&str>| unreachable!("every pane is adopted");
            let rebuilt =
                rebuild(&handover, panes, 0.0, Ptys { adopt: &mut adopt, fresh: &fresh }).unwrap();
            let pids: Vec<Option<u32>> = rebuilt
                .tabs
                .items()
                .iter()
                .map(|tab| match &tab.pane.body {
                    Body::Shell(term) => term.shell_pid(),
                    _ => None,
                })
                .collect();
            incoming.ack().unwrap();
            (pids, rebuilt.tabs)
        })
    };
    let stream = stdusk_core::handoff::accept_within(&listener, Duration::from_secs(10)).unwrap();
    let panes = capture(&tabs, &order).unwrap();
    send_session(&stream, &handover, &panes).expect("the successor confirmed");
    drop(panes);
    release(&mut tabs);
    let (got_pids, adopted) = successor.join().unwrap();

    assert_eq!(got_pids, sent_pids, "the same shells, in the same tabs");
    assert_eq!(adopted.items().iter().map(|tab| tab.group).collect::<Vec<_>>(), [0, 0, 1]);
    for pid in sent_pids.into_iter().flatten() {
        assert!(process_alive(pid), "pid {pid} outlives the window that handed it over");
    }
    drop(tabs);
    for tab in adopted.items() {
        if let Body::Shell(term) = &tab.pane.body {
            assert!(term.shell_pid().is_some_and(process_alive), "the adopted shell runs on");
        }
    }
    let mut adopted = adopted;
    for tab in adopted.items_mut() {
        if let Body::Shell(term) = &mut tab.pane.body {
            term.arm_teardown();
            term.kill();
        }
    }
}

fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn the_new_window_speaks_up_only_about_what_did_not_come_back_as_it_was() {
    assert_eq!(notice(&[], 0, false), None);
    assert_eq!(notice(&[], 3, true), None);
    assert_eq!(notice(&[], 0, true), None);
    assert_eq!(notice(&[], 2, false), Some(HOOKS_LOST.to_owned()));
    let lost = vec![LOST_TERMINAL.to_owned()];
    assert_eq!(notice(&lost, 0, true), Some(LOST_TERMINAL.to_owned()));
    assert_eq!(notice(&lost, 1, false), Some(format!("{LOST_TERMINAL}\n{HOOKS_LOST}")));
}

#[test]
fn the_lost_hooks_notice_names_everything_that_stops_reaching_tidewatch() {
    for word in ["progress", "questions", "findings", "Meeseeks"] {
        assert!(HOOKS_LOST.contains(word), "{word}");
    }
}

fn a_meeseeks() -> tidewatch_core::meeseeks::Meeseeks {
    let mut meeseeks = tidewatch_core::meeseeks::Meeseeks::summon(
        "rename the helper",
        tidewatch_core::meeseeks::Criteria { tests: false, committed: true, pr: true },
        1_760_000_000_000,
    );
    meeseeks.turns = 9;
    meeseeks.failures = 2;
    meeseeks
}

#[test]
fn a_meeseeks_crosses_the_handover_with_its_counts_and_comes_back_as_one() {
    let mut w = window();
    w.tabs.get_mut(w.agent_tab).unwrap().pane.agent.as_mut().unwrap().meeseeks = Some(a_meeseeks());
    let (handover, _) = handover_of(&w);
    let text = serde_json::to_string(&handover).unwrap();
    let handover: Handover = serde_json::from_str(&text).unwrap();
    let mut adopt = |term: u32, _: &Path| Ok(term);
    let mut rebuilt = rebuild(
        &handover,
        vec![100, 101, 102, 200, 201],
        0.0,
        Ptys { adopt: &mut adopt, fresh: &fresh },
    )
    .unwrap()
    .tabs;
    let otter = rebuilt.items_mut().iter().find(|tab| tab.title == "otter").unwrap();
    assert_eq!(otter.pane.agent.as_ref().unwrap().meeseeks, Some(a_meeseeks()));
    let others = rebuilt.items().iter().filter(|tab| tab.title != "otter");
    assert!(others.filter_map(|tab| tab.pane.agent.as_ref()).all(|agent| agent.meeseeks.is_none()));
}

#[test]
fn a_meeseeks_that_is_being_checked_blocks_a_handover_and_one_that_works_does_not() {
    let mut w = window();
    let mut meeseeks = a_meeseeks();
    w.tabs.get_mut(w.agent_tab).unwrap().pane.agent.as_mut().unwrap().meeseeks =
        Some(meeseeks.clone());
    assert_eq!(blocker(&w.tabs), None);
    for phase in [
        tidewatch_core::meeseeks::Phase::Checking(tidewatch_core::meeseeks::Step::Tests),
        tidewatch_core::meeseeks::Phase::Poofing,
    ] {
        meeseeks.phase = phase;
        w.tabs.get_mut(w.agent_tab).unwrap().pane.agent.as_mut().unwrap().meeseeks =
            Some(meeseeks.clone());
        assert_eq!(blocker(&w.tabs), Some(Blocker::MeeseeksBusy), "{phase:?}");
    }
    assert_eq!(Blocker::MeeseeksBusy.text(), "a Meeseeks is being checked");
}
