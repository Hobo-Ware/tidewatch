use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::agents::Level;
use tidewatch_core::prefs::ResumeAgents;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::session::{SavedAgentTab, SavedWindow, Session};
use tidewatch_core::tools::Tools;
use tidewatch_core::transcript_locate::claude_project_dir_name;
use tidewatch_core::worktree;

use super::*;
use crate::tabs::Action;

const SESSION_A: &str = "0c2cbc96-1111-4222-8333-444455556666";
const SESSION_B: &str = "1d3dcd07-2222-4333-8444-555566667777";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn web(repo: &Path) -> Workspace {
    Workspace::Repo(crate::repo::Repo { root: repo.to_path_buf(), label: "web".into() })
}

struct Fixture {
    _dir: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
    catalog: Catalog,
    host: Host,
    runs: workflow_glue::Host,
    tabs: Tabs<Pane<String>>,
    ctx: egui::Context,
    toasts: Vec<String>,
    report: Report,
}

impl Fixture {
    fn new(skip: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = home.join("work").join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let claude = bin.join("claude");
        std::fs::write(&claude, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut catalog = Catalog::new(OsString::from(bin));
        catalog.set_profiles(Profiles::seeded(&["claude".to_owned()]));
        let mut host = Host::new(skip, false, RealDisk::new(&home));
        host.enqueue(0, &web(&repo));
        Self {
            paths: TidewatchPaths::new(&home),
            host,
            runs: workflow_glue::Host::new(),
            tabs: Tabs::new(Pane::shell("main".to_owned())),
            ctx: egui::Context::default(),
            toasts: Vec::new(),
            report: Report::default(),
            _dir: dir,
            home,
            repo,
            catalog,
        }
    }

    fn worktree_with_transcript(&self, session: &str) -> Worktree {
        let worktree = worktree::add(&self.paths, &self.repo).unwrap();
        let folder = self
            .home
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(&worktree.path));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{session}.jsonl")), "{}\n").unwrap();
        worktree
    }

    fn save(&self, active: usize, focus: bool, tabs: Vec<SavedTab>) {
        let window = SavedWindow {
            dir: self.repo.clone(),
            other: false,
            active,
            focus_mode: focus,
            tabs,
            parked: Vec::new(),
        };
        Session { windows: vec![window] }.save(&self.paths).unwrap();
    }

    fn tick(&mut self) {
        let env = Env {
            paths: &self.paths,
            catalog: &self.catalog,
            listener: None,
            tools: Tools::with_path(std::env::var_os("PATH").unwrap_or_default()),
            trust: None,
        };
        let agent = |dir: &Path, prepared: &Prepared| {
            format!("agent@{}: {}", dir.display(), prepared.launch.shell_line())
        };
        let shell = |dir: &Path, note: Option<&str>| {
            format!("shell@{}|{}", dir.display(), note.unwrap_or("-"))
        };
        let typed = |dir: &Path, prepared: &Prepared| {
            format!("typed@{}: {}", dir.display(), prepared.launch.shell_line())
        };
        let spawn = Spawn { agent: &agent, typed: &typed, shell: &shell };
        let report = self.host.tick(&self.ctx, &mut self.tabs, &env, &spawn, &mut self.runs, 1.0);
        self.toasts.extend(report.toasts.clone());
        if report.leftovers.is_some() {
            self.report.leftovers = report.leftovers;
        }
        self.report.notice = report.notice.or_else(|| self.report.notice.take());
        self.report.focus = report.focus.or(self.report.focus);
    }

    fn begin(&mut self) {
        self.host.begin(&self.ctx, &self.paths, &self.catalog);
    }

    fn until_settled(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.host.settled() {
            self.tick();
            assert!(Instant::now() < deadline, "the restore never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn shells(&self) -> Vec<String> {
        self.tabs
            .items()
            .iter()
            .map(|t| match &t.pane.body {
                Body::Shell(s) => s.clone(),
                Body::Creating => "creating".into(),
                Body::Removing => "removing".into(),
                Body::Draft(_) => "draft".into(),
                Body::Failed(m) => format!("failed: {m}"),
            })
            .collect()
    }

    fn titles(&self) -> Vec<&str> {
        self.tabs.items().iter().map(|t| t.title.as_str()).collect()
    }
}

fn agent(worktree: &Worktree, session: &str, level: Level) -> SavedTab {
    SavedTab::Agent(SavedAgentTab {
        dir: worktree.path.clone(),
        branch: Some(worktree.branch.clone()),
        agent: "claude".into(),
        profile: "builder".into(),
        level,
        session_id: Some(session.into()),
        stages: None,
        crashed: false,
        remote: false,
        meeseeks: None,
    })
}

fn shell(worktree: &Worktree) -> SavedTab {
    SavedTab::Shell { dir: worktree.path.clone(), branch: Some(worktree.branch.clone()) }
}

#[test]
fn parked_and_never_park_marks_come_back_on_the_tabs_they_were_saved_with() {
    use tidewatch_core::session::SavedPark;
    let mut f = Fixture::new(false);
    let calm = worktree::add(&f.paths, &f.repo).unwrap();
    let brave = worktree::add(&f.paths, &f.repo).unwrap();
    let window = SavedWindow {
        dir: f.repo.clone(),
        other: false,
        active: 0,
        focus_mode: false,
        tabs: vec![SavedTab::Main, shell(&calm), shell(&brave)],
        parked: vec![
            SavedPark { tab: 1, why: Some(crate::park::Why::Idle), since_ms: 9, keep: false },
            SavedPark { tab: 2, why: None, since_ms: 0, keep: true },
        ],
    };
    Session { windows: vec![window] }.save(&f.paths).unwrap();
    f.begin();
    f.until_settled();
    let marks: Vec<_> = f.tabs.items().iter().map(|tab| (tab.park, tab.keep)).collect();
    assert_eq!(
        marks,
        [
            (None, false),
            (Some(Park { why: crate::park::Why::Idle, since_ms: 9 }), false),
            (None, true)
        ]
    );
}

#[test]
fn nothing_is_settled_before_the_startup_job_is_back_and_the_saver_must_wait() {
    let mut f = Fixture::new(false);
    assert!(!f.host.settled());
    f.begin();
    assert!(!f.host.settled(), "planning is under way");
    f.until_settled();
    assert!(f.host.settled());
}

#[test]
fn begin_waits_for_the_profiles_and_runs_once() {
    let mut f = Fixture::new(false);
    f.catalog = Catalog::new(OsString::from("/usr/bin"));
    f.begin();
    assert_eq!(f.host.phase, Phase::Waiting);
    f.catalog.set_profiles(Profiles::seeded(&["claude".to_owned()]));
    f.begin();
    assert_eq!(f.host.phase, Phase::Planning);
    f.begin();
    f.until_settled();
}

#[test]
fn saved_tabs_come_back_in_order_a_shell_in_its_worktree_an_agent_resumed_the_active_tab_and_focus()
{
    let mut f = Fixture::new(false);
    let calm = worktree::add(&f.paths, &f.repo).unwrap();
    let brave = f.worktree_with_transcript(SESSION_A);
    f.save(2, true, vec![SavedTab::Main, shell(&calm), agent(&brave, SESSION_A, Level::Edits)]);
    f.begin();
    f.until_settled();
    assert_eq!(f.titles(), ["main", calm.name(), brave.name()]);
    let shells = f.shells();
    assert_eq!(shells[0], "main");
    assert_eq!(shells[1], format!("shell@{}|-", calm.path.display()));
    assert!(
        shells[2]
            .starts_with(&format!("agent@{}: claude --resume {SESSION_A} ", brave.path.display())),
        "{}",
        shells[2]
    );
    assert_eq!(f.tabs.active(), 2);
    assert_eq!(f.report.focus, Some(true));
    assert_eq!(f.report.notice, None);
    assert_eq!(f.tabs.items()[1].pane.worktree, Some(calm));
    let resumed = f.tabs.items()[2].pane.agent.as_ref().expect("an agent tab");
    assert_eq!(resumed.launched.as_ref().map(|l| l.session_id.as_deref()), Some(Some(SESSION_A)));
    assert_eq!(resumed.launched.as_ref().map(|l| l.profile_id.as_str()), Some("builder"));
    assert_eq!(resumed.run.agent_id, "claude");
}

#[test]
fn the_active_tab_is_the_saved_one_not_the_last() {
    let mut f = Fixture::new(false);
    let a = worktree::add(&f.paths, &f.repo).unwrap();
    let b = worktree::add(&f.paths, &f.repo).unwrap();
    f.save(1, false, vec![SavedTab::Main, shell(&a), shell(&b)]);
    f.begin();
    f.until_settled();
    assert_eq!(f.tabs.active(), 1);
    assert_eq!(f.report.focus, Some(false));
}

#[test]
fn agent_tabs_keep_their_saved_order_whichever_job_finishes_first() {
    let mut f = Fixture::new(false);
    let one = f.worktree_with_transcript(SESSION_A);
    let two = f.worktree_with_transcript(SESSION_B);
    f.save(
        0,
        false,
        vec![
            SavedTab::Main,
            agent(&one, SESSION_A, Level::Edits),
            agent(&two, SESSION_B, Level::Plan),
        ],
    );
    f.begin();
    f.until_settled();
    assert_eq!(f.titles(), ["main", one.name(), two.name()]);
    assert!(f.shells()[1].contains(SESSION_A) && f.shells()[2].contains(SESSION_B));
}

#[test]
fn a_tab_whose_agent_cannot_resume_is_a_shell_with_the_reason_and_a_gone_worktree_is_left_out() {
    let mut f = Fixture::new(false);
    let lost = worktree::add(&f.paths, &f.repo).unwrap();
    let gone = worktree::add(&f.paths, &f.repo).unwrap();
    worktree::remove(&f.paths, &gone, worktree::BranchChoice::Delete, worktree::RemoveMode::Safe)
        .unwrap();
    f.save(1, false, vec![SavedTab::Main, agent(&lost, SESSION_A, Level::Edits), shell(&gone)]);
    f.begin();
    f.until_settled();
    assert_eq!(f.tabs.items().len(), 2, "the gone worktree has no tab");
    assert!(f.shells()[1].contains("its session file is gone"), "{}", f.shells()[1]);
    assert!(f.shells()[1].starts_with("shell@"), "a plain shell, no agent: {}", f.shells()[1]);
    assert!(f.tabs.items()[1].pane.agent.is_none());
    let notice = f.report.notice.clone().expect("the plan says what did not come back as it was");
    assert!(notice.contains(gone.name()) && notice.contains("worktree folder gone"), "{notice}");
    assert!(notice.contains(lost.name()), "{notice}");
}

#[test]
fn restoring_off_opens_nothing_and_still_settles() {
    let mut f = Fixture::new(true);
    let a = worktree::add(&f.paths, &f.repo).unwrap();
    f.save(1, true, vec![SavedTab::Main, shell(&a)]);
    f.begin();
    f.until_settled();
    assert_eq!(f.tabs.items().len(), 1);
    assert_eq!(f.report.focus, None);
    assert_eq!(
        f.report.leftovers.as_ref().map(Vec::len),
        Some(1),
        "the unowned worktree is a leftover"
    );
}

#[test]
fn the_leftovers_exclude_what_the_saved_tabs_own() {
    let mut f = Fixture::new(false);
    let owned = worktree::add(&f.paths, &f.repo).unwrap();
    let stray = worktree::add(&f.paths, &f.repo).unwrap();
    f.save(0, false, vec![SavedTab::Main, shell(&owned)]);
    f.begin();
    f.until_settled();
    let left: Vec<&Path> =
        f.report.leftovers.as_ref().unwrap().iter().map(|l| l.path.as_path()).collect();
    assert_eq!(left, [stray.path.as_path()]);
}

#[test]
fn tabs_the_user_opened_meanwhile_keep_the_selection_and_focus_they_have() {
    let mut f = Fixture::new(false);
    let a = worktree::add(&f.paths, &f.repo).unwrap();
    f.save(1, true, vec![SavedTab::Main, shell(&a)]);
    f.tabs.push("mine".into(), Pane::shell("mine".to_owned()));
    f.begin();
    f.until_settled();
    assert_eq!(f.titles(), ["main", "mine", a.name()]);
    assert_eq!(f.tabs.active(), 1, "still the user's own tab");
    assert_eq!(f.report.focus, None);
}

#[test]
fn a_plan_stays_unsettled_while_an_agent_is_still_being_made() {
    let mut f = Fixture::new(false);
    let brave = f.worktree_with_transcript(SESSION_A);
    f.save(0, false, vec![SavedTab::Main, agent(&brave, SESSION_A, Level::Edits)]);
    f.begin();
    let deadline = Instant::now() + Duration::from_secs(30);
    while f.host.phase == Phase::Planning {
        f.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    if f.host.phase == Phase::Settling {
        assert!(!f.host.settled());
        assert_eq!(f.shells()[1], "creating");
    }
    f.until_settled();
    assert!(f.shells()[1].starts_with("agent@"));
}

#[test]
fn a_workflow_tab_with_unreadable_state_reopens_as_a_shell_with_a_toast() {
    let mut f = Fixture::new(false);
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    let plan = WindowPlan {
        dir: f.repo.clone(),
        other: false,
        focus_mode: false,
        active: 0,
        tabs: vec![TabPlan {
            saved: SavedTab::Agent(SavedAgentTab {
                dir: wt.path.clone(),
                branch: Some(wt.branch.clone()),
                agent: "claude".into(),
                profile: "builder".into(),
                level: Level::Edits,
                session_id: None,
                stages: Some(f.home.join("missing-stages.json")),
                crashed: false,
                remote: false,
                meeseeks: None,
            }),
            dir: wt.path.clone(),
            worktree: Some(wt.clone()),
            action: TabAction::ResumeWorkflow { stages: f.home.join("missing-stages.json") },
            park: None,
        }],
        skipped: Vec::new(),
    };
    let env = Env {
        paths: &f.paths,
        catalog: &f.catalog,
        listener: None,
        tools: Tools::with_path(OsString::new()),
        trust: None,
    };
    let agent = |_: &Path, _: &Prepared| "agent".to_owned();
    let typed = |_: &Path, _: &Prepared| "typed".to_owned();
    let shell = |_: &Path, note: Option<&str>| format!("shell|{}", note.unwrap_or("-"));
    let spawn = Spawn { agent: &agent, typed: &typed, shell: &shell };
    let report = f.host.apply(&plan, 0, &f.ctx, &mut f.tabs, &env, &spawn, &mut f.runs, 0.0);
    assert_eq!(report.toasts.len(), 1);
    assert!(f.shells()[1].starts_with("shell|The workflow was not resumed"), "{}", f.shells()[1]);
}

#[test]
fn a_saved_workflow_run_goes_back_through_the_workflow_host_in_its_place() {
    use std::collections::BTreeMap;
    use tidewatch_core::runner::{RunContext, Runner};
    use tidewatch_core::workflows::Workflows;
    let mut f = Fixture::new(false);
    let first = worktree::add(&f.paths, &f.repo).unwrap();
    let wt = worktree::add(&f.paths, &f.repo).unwrap();
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let context = RunContext {
        task: "t".into(),
        base: "main".into(),
        branch: wt.branch.clone(),
        test_command: None,
    };
    let mut runner = Runner::new(&ship, &BTreeMap::new(), context).unwrap();
    runner.start();
    let stages = f.paths.stages_file(&f.repo, wt.name());
    runner.save(&stages).unwrap();
    let workflow_tab = SavedTab::Agent(SavedAgentTab {
        dir: wt.path.clone(),
        branch: Some(wt.branch.clone()),
        agent: "claude".into(),
        profile: "Builder".into(),
        level: Level::Edits,
        session_id: None,
        stages: Some(stages),
        crashed: false,
        remote: false,
        meeseeks: None,
    });
    f.save(2, false, vec![SavedTab::Main, shell(&first), workflow_tab]);
    f.begin();
    let deadline = Instant::now() + Duration::from_secs(30);
    while f.host.phase != Phase::Settling {
        f.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(f.tabs.items().len(), 3);
    assert_eq!(f.tabs.items()[1].title, first.name());
    assert!(
        f.tabs.items()[2].pane.agent.as_ref().is_some_and(|a| a.profile_name == "Ship a feature")
    );
    assert_eq!(f.tabs.active(), 2);
    assert!(!f.host.settled(), "the workflow tab is still being opened");
}

#[test]
fn the_dev_resume_plan_is_one_workflow_tab_in_the_named_worktree() {
    let f = Fixture::new(false);
    let plan = dev_resume_plan(&f.paths, &f.repo, "brave-otter");
    assert_eq!(plan.tabs.len(), 1);
    let tab = &plan.tabs[0];
    assert_eq!(tab.worktree.as_ref().map(|w| w.branch.as_str()), Some("agent/brave-otter"));
    assert_eq!(tab.dir, f.paths.worktree_dir(&f.repo, "brave-otter"));
    assert_eq!(
        tab.action,
        TabAction::ResumeWorkflow { stages: f.paths.stages_file(&f.repo, "brave-otter") }
    );
    assert_eq!(plan.skipped, []);
}

#[test]
fn turning_restoring_off_in_settings_is_written_to_the_settings_file() {
    let mut f = Fixture::new(false);
    assert!(f.host.restore_enabled());
    f.host.set_restore_enabled(&f.paths, false);
    assert!(!f.host.restore_enabled());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Prefs::load(&f.paths).restore_session {
        assert!(Instant::now() < deadline, "the setting was never written");
        std::thread::sleep(Duration::from_millis(5));
    }
    f.host.set_restore_enabled(&f.paths, true);
    assert!(f.host.restore_enabled());
}

#[test]
fn choosing_a_resume_mode_in_settings_is_written_to_the_settings_file_and_keeps_the_rest() {
    let mut f = Fixture::new(false);
    assert_eq!(f.host.resume_agents(), ResumeAgents::Auto);
    f.host.set_resume_agents(&f.paths, ResumeAgents::Prefill);
    assert_eq!(f.host.resume_agents(), ResumeAgents::Prefill);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Prefs::load(&f.paths).resume_agents != ResumeAgents::Prefill {
        assert!(Instant::now() < deadline, "the setting was never written");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(Prefs::load(&f.paths).restore_session);
}

#[test]
fn the_flag_is_reported_apart_from_the_setting() {
    let f = Fixture::new(true);
    assert!(f.host.skipped_by_flag());
    assert!(f.host.restore_enabled(), "the setting itself is still on");
}

fn second_repo(f: &Fixture) -> PathBuf {
    let repo = f.home.join("work").join("api");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    repo
}

fn saved_window(dir: &Path, focus_mode: bool, tabs: Vec<SavedTab>) -> SavedWindow {
    SavedWindow {
        dir: dir.to_path_buf(),
        other: false,
        active: 1,
        focus_mode,
        tabs,
        parked: Vec::new(),
    }
}

fn api(repo: &Path) -> Workspace {
    Workspace::Repo(crate::repo::Repo { root: repo.to_path_buf(), label: "api".into() })
}

#[test]
fn a_second_repo_comes_back_in_its_own_group_and_never_takes_the_window_from_the_first() {
    let mut f = Fixture::new(false);
    let calm = worktree::add(&f.paths, &f.repo).unwrap();
    let api_repo = second_repo(&f);
    let busy = worktree::add(&f.paths, &api_repo).unwrap();
    Session {
        windows: vec![
            saved_window(&f.repo, true, vec![SavedTab::Main, shell(&calm)]),
            saved_window(&api_repo, false, vec![SavedTab::Main, shell(&busy)]),
        ],
    }
    .save(&f.paths)
    .unwrap();
    let group = f.tabs.add_group(Pane::shell("api main".to_owned()));
    f.host.enqueue(group, &api(&api_repo));
    f.begin();
    f.until_settled();
    let in_group = |f: &Fixture, group: usize| -> Vec<String> {
        f.tabs.in_group(group).map(|(_, tab)| tab.title.clone()).collect()
    };
    assert_eq!(in_group(&f, 0), ["main", calm.name()]);
    assert_eq!(in_group(&f, group), ["main", busy.name()]);
    assert_eq!(f.tabs.active_group(), 0, "the first repo stays in front");
    assert_eq!(f.tabs.items()[f.tabs.active()].title, calm.name());
    assert_eq!(f.report.focus, Some(true), "the first repo's focus mode, not the second's");
    assert_eq!(f.report.leftovers.as_ref().map(Vec::len), Some(0));
    f.tabs.apply(Action::NextGroup, || unreachable!());
    assert_eq!(f.tabs.items()[f.tabs.active()].title, busy.name(), "its saved active tab");
}

#[test]
fn a_repo_added_later_makes_the_window_unsettled_until_its_tabs_are_back() {
    let mut f = Fixture::new(false);
    f.begin();
    f.until_settled();
    let api_repo = second_repo(&f);
    let busy = worktree::add(&f.paths, &api_repo).unwrap();
    Session { windows: vec![saved_window(&api_repo, true, vec![SavedTab::Main, shell(&busy)])] }
        .save(&f.paths)
        .unwrap();
    let group = f.tabs.add_group(Pane::shell("api main".to_owned()));
    f.host.enqueue(group, &api(&api_repo));
    assert!(!f.host.settled(), "the saver must not overwrite the second repo's saved tabs");
    f.begin();
    f.until_settled();
    assert_eq!(f.tabs.in_group(group).count(), 2);
    assert_eq!(f.report.focus, None, "only the first repo sets the window's focus mode");
    assert_eq!(f.tabs.active_group(), 0);
}

fn crashed_agent(worktree: &Worktree, session: &str, level: Level, crashed: bool) -> SavedTab {
    let SavedTab::Agent(tab) = agent(worktree, session, level) else { unreachable!() };
    SavedTab::Agent(SavedAgentTab { crashed, ..tab })
}

struct ReopenedPane {
    pane: String,
    note: Option<&'static str>,
    started: bool,
    notice: Option<String>,
}

fn reopen_one(
    mode: ResumeAgents,
    level: Level,
    app_crashed: bool,
    agent_crashed: bool,
) -> (ReopenedPane, Worktree) {
    let mut f = Fixture::new(false);
    Prefs { resume_agents: mode, ..Prefs::default() }.save(&f.paths).unwrap();
    f.host.app_crashed = app_crashed;
    let brave = f.worktree_with_transcript(SESSION_A);
    f.save(1, false, vec![SavedTab::Main, crashed_agent(&brave, SESSION_A, level, agent_crashed)]);
    f.begin();
    f.until_settled();
    let tab = &f.tabs.items()[1].pane;
    let reopened = ReopenedPane {
        pane: f.shells()[1].clone(),
        note: tab.agent.as_ref().and_then(|agent| agent.resume_note),
        started: tab.agent.is_some(),
        notice: f.report.notice.clone(),
    };
    (reopened, brave)
}

#[test]
fn a_resumed_agent_runs_or_waits_typed_for_enter_by_setting_and_how_it_ended() {
    use ResumeAgents::{Auto, Prefill};
    let cases = [
        (Auto, false, false, None),
        (Auto, true, false, Some(Why::AppCrashed)),
        (Auto, false, true, Some(Why::AgentCrashed)),
        (Auto, true, true, Some(Why::AppCrashed)),
        (Prefill, false, false, Some(Why::Chosen)),
        (Prefill, true, false, Some(Why::AppCrashed)),
        (Prefill, false, true, Some(Why::AgentCrashed)),
    ];
    for (mode, app, agent, typed) in cases {
        let (got, brave) = reopen_one(mode, Level::Edits, app, agent);
        let label = format!("{mode:?} app {app} agent {agent}");
        let prefix = if typed.is_some() { "typed@" } else { "agent@" };
        assert!(got.pane.starts_with(prefix), "{label}: {}", got.pane);
        assert!(got.pane.contains(&format!("claude --resume {SESSION_A}")), "{label}");
        assert!(got.pane.contains(&brave.path.display().to_string()), "{label}");
        assert_eq!(got.note, typed.map(Why::note), "{label}");
        assert!(got.started, "{label}: it stays an agent tab, so the next save keeps its session");
        assert_eq!(got.notice, None, "{label}");
    }
}

#[test]
fn off_reopens_a_plain_shell_with_the_reason_and_says_nothing_else() {
    for (app, agent) in [(false, false), (true, false), (false, true)] {
        let (got, _) = reopen_one(ResumeAgents::Off, Level::Edits, app, agent);
        assert!(got.pane.starts_with("shell@"), "{}", got.pane);
        assert!(got.pane.contains("resuming agents is turned off in Settings"), "{}", got.pane);
        assert!(!got.started);
        assert_eq!(got.notice, None, "a choice the user made is no news");
    }
}

#[test]
fn yolo_is_typed_with_its_yolo_flags_in_every_mode_but_off_even_after_a_crash() {
    for mode in [ResumeAgents::Auto, ResumeAgents::Prefill] {
        for (app, agent) in [(false, false), (true, false), (false, true)] {
            let (got, brave) = reopen_one(mode, Level::Yolo, app, agent);
            let label = format!("{mode:?} app {app} agent {agent}");
            assert!(got.pane.starts_with("typed@"), "{label}: {}", got.pane);
            assert!(got.pane.contains(&format!("claude --resume {SESSION_A}")), "{label}");
            assert!(got.pane.contains("--dangerously-skip-permissions"), "{label}: {}", got.pane);
            assert!(got.pane.contains(&brave.path.display().to_string()), "{label}");
            assert_eq!(got.note, Some(Why::Yolo.note()), "{label}");
            assert!(got.started, "{label}");
        }
    }
}

#[test]
fn yolo_with_resuming_off_is_a_plain_shell_like_every_agent() {
    for (app, agent) in [(false, false), (true, false), (false, true)] {
        let (got, _) = reopen_one(ResumeAgents::Off, Level::Yolo, app, agent);
        assert!(got.pane.starts_with("shell@"), "{}", got.pane);
        assert!(got.pane.contains("resuming agents is turned off in Settings"), "{}", got.pane);
        assert!(!got.started);
    }
}
