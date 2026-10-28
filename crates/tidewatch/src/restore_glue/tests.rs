use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::agents::Level;
use tidewatch_core::prefs::ResumeAgents;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::resume_mode::Why;
use tidewatch_core::session::{SavedAgentTab, SavedWindow, Session};
use tidewatch_core::transcript_locate::claude_project_dir_name;
use tidewatch_core::worktree;
use tidewatch_core::worktree::Worktree;

use super::*;
use crate::agent_run::TermSnapshot;
use crate::tabs::Action;
use tidewatch_core::status::AgentStatus;

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
        let env = Env { paths: &self.paths, catalog: &self.catalog, listener: None, trust: None };
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
        let report = self.host.tick(&self.ctx, &mut self.tabs, &env, &spawn, 1.0);
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
                Body::Poofed(record) => format!("poofed: {}", record.name),
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
        folder_repo: None,
    })
}

fn shell(worktree: &Worktree) -> SavedTab {
    SavedTab::Shell { dir: worktree.path.clone(), branch: Some(worktree.branch.clone()) }
}

#[test]
fn a_poofed_summary_tab_comes_back_as_a_summary_with_its_park_mark_and_saves_again_unchanged() {
    use crate::session_save::{WindowView, saved_window};
    use tidewatch_core::civil_date::Date;
    use tidewatch_core::meeseeks_log::Poofed;
    use tidewatch_core::session::SavedPark;
    let mut f = Fixture::new(false);
    let calm = worktree::add(&f.paths, &f.repo).unwrap();
    let mut record = Poofed::bare(
        "promo-test",
        "Added the promo test.",
        Some("https://github.com/acme/web/pull/213"),
        360,
        Date::new(2026, 10, 25).unwrap(),
    );
    record.task = "Add a promo test".into();
    record.branch = "agent/promo-test".into();
    record.repo = f.repo.clone();
    record.ended_ms = 1_790_000_000_000;
    let poofed = SavedTab::Poofed(Box::new(record));
    let window = SavedWindow {
        dir: f.repo.clone(),
        other: false,
        active: 0,
        focus_mode: false,
        tabs: vec![SavedTab::Main, shell(&calm), poofed.clone()],
        parked: vec![SavedPark {
            tab: 2,
            why: Some(crate::park::Why::Idle),
            since_ms: 9,
            keep: false,
        }],
    };
    Session { windows: vec![window] }.save(&f.paths).unwrap();
    f.begin();
    f.until_settled();
    assert_eq!(f.shells()[2], "poofed: promo-test");
    assert_eq!(f.titles()[2], "promo-test");
    let tab = &f.tabs.items()[2];
    assert!(tab.pane.agent.is_none() && tab.pane.worktree.is_none());
    assert_eq!(tab.park, Some(Park { why: crate::park::Why::Idle, since_ms: 9 }));
    let view = WindowView { group: 0, dir: &f.repo, other: false, focus: false };
    assert_eq!(saved_window(&view, &f.tabs).tabs[2], poofed);
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
fn an_agent_comes_back_at_the_mode_it_was_last_in_and_the_tab_says_so() {
    let mut f = Fixture::new(false);
    let brave = f.worktree_with_transcript(SESSION_A);
    let folder = f
        .home
        .join(".claude")
        .join("projects")
        .join(claude_project_dir_name(&brave.path))
        .join(format!("{SESSION_A}.jsonl"));
    std::fs::write(folder, "{\"type\":\"permission-mode\",\"permissionMode\":\"auto\"}\n").unwrap();
    f.save(1, false, vec![SavedTab::Main, agent(&brave, SESSION_A, Level::Plan)]);
    f.begin();
    f.until_settled();
    let resumed = f.tabs.items()[1].pane.agent.as_ref().expect("an agent tab");
    assert_eq!(resumed.launched.as_ref().map(|l| l.level), Some(Level::Auto));
    let shell = &f.shells()[1];
    assert!(shell.contains("--permission-mode auto"), "{shell}");
    assert!(!shell.contains("dontAsk"), "{shell}");
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
fn a_session_file_from_the_workflow_days_reopens_its_tab_as_a_shell_in_the_worktree() {
    let mut f = Fixture::new(false);
    let kept = worktree::add(&f.paths, &f.repo).unwrap();
    let gone = worktree::add(&f.paths, &f.repo).unwrap();
    worktree::remove(&f.paths, &gone, worktree::BranchChoice::Delete, worktree::RemoveMode::Safe)
        .unwrap();
    let tab = |wt: &Worktree| {
        format!(
            "[[window.tab]]\nkind = \"agent\"\ndir = {:?}\nbranch = {:?}\nagent = \"claude\"\n\
             profile = \"Builder\"\nlevel = \"edits\"\nsession_id = \"{SESSION_A}\"\n\
             stages = {:?}\n",
            wt.path,
            wt.branch,
            f.paths.worktree_dir(&f.repo, "old").join("stages.json")
        )
    };
    let text = format!(
        "[[window]]\ndir = {:?}\nactive = 1\n[[window.tab]]\nkind = \"main\"\n{}{}",
        f.repo,
        tab(&kept),
        tab(&gone)
    );
    std::fs::create_dir_all(f.paths.session_file().parent().unwrap()).unwrap();
    std::fs::write(f.paths.session_file(), text).unwrap();
    f.begin();
    f.until_settled();
    assert_eq!(f.tabs.items().len(), 2, "the tab whose worktree is gone has no tab");
    assert!(f.shells()[1].starts_with("shell@"), "a plain shell, no agent: {}", f.shells()[1]);
    assert!(f.shells()[1].contains("workflows were retired"), "{}", f.shells()[1]);
    assert!(f.tabs.items()[1].pane.agent.is_none());
    assert_eq!(f.tabs.items()[1].pane.worktree.as_ref(), Some(&kept));
    assert!(kept.path.is_dir(), "the worktree is left alone");
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
    crash_loop: bool,
    agent_crashed: bool,
) -> (ReopenedPane, Worktree) {
    let mut f = Fixture::new(false);
    Prefs { resume_agents: mode, ..Prefs::default() }.save(&f.paths).unwrap();
    f.host.crash_loop = crash_loop;
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
        (Auto, true, false, Some(Why::CrashLoop)),
        (Auto, false, true, Some(Why::AgentCrashed)),
        (Auto, true, true, Some(Why::CrashLoop)),
        (Prefill, false, false, Some(Why::Chosen)),
        (Prefill, true, false, Some(Why::CrashLoop)),
        (Prefill, false, true, Some(Why::AgentCrashed)),
    ];
    for (mode, app, agent, typed) in cases {
        let (got, brave) = reopen_one(mode, Level::Edits, app, agent);
        let label = format!("{mode:?} app {app} agent {agent}");
        let prefix = if typed.is_some() { "typed@" } else { "agent@" };
        assert!(got.pane.starts_with(prefix), "{label}: {}", got.pane);
        assert!(got.pane.contains(&format!("claude --resume {SESSION_A}")), "{label}");
        assert!(got.pane.contains(&brave.path.display().to_string()), "{label}");
        assert_eq!(got.note, typed.map(|why| why.note(true)), "{label}");
        assert!(got.started, "{label}: it stays an agent tab, so the next save keeps its session");
        assert_eq!(got.notice, None, "{label}");
    }
}

fn refreshed_status(f: &mut Fixture) -> AgentStatus {
    let agent = f.tabs.items_mut()[1].pane.agent.as_mut().expect("an agent tab");
    agent.run.feed(TermSnapshot::default(), 2.0);
    agent.run.refresh(2.0);
    agent.run.status().clone()
}

#[test]
fn a_waiting_resume_command_keeps_the_tab_stopped_and_a_run_one_does_not() {
    for (mode, waits) in [(ResumeAgents::Auto, false), (ResumeAgents::Prefill, true)] {
        let mut f = Fixture::new(false);
        Prefs { resume_agents: mode, ..Prefs::default() }.save(&f.paths).unwrap();
        let brave = f.worktree_with_transcript(SESSION_A);
        f.save(1, false, vec![SavedTab::Main, agent(&brave, SESSION_A, Level::Edits)]);
        f.begin();
        f.until_settled();
        let tab = f.tabs.items()[1].pane.agent.as_ref().unwrap();
        assert_eq!(tab.resume_line.is_some(), waits, "{mode:?}");
        let line = tab.resume_line.clone().unwrap_or_default();
        assert!(line.is_empty() || line.starts_with(&format!("claude --resume {SESSION_A}")));
        let want = if waits { AgentStatus::Stopped } else { AgentStatus::Done };
        assert_eq!(refreshed_status(&mut f), want, "{mode:?}");
    }
}

#[test]
fn a_command_too_long_to_type_is_kept_for_the_resume_button_with_a_note_that_says_so() {
    let mut f = Fixture::new(false);
    let mut profiles = Profiles::seeded(&["claude".to_owned()]);
    for profile in profiles.profile.values_mut() {
        profile.extra_args = vec!["--add-dir".to_owned(), "d".repeat(1000)].into();
    }
    f.catalog.set_profiles(profiles);
    Prefs { resume_agents: ResumeAgents::Prefill, ..Prefs::default() }.save(&f.paths).unwrap();
    let brave = f.worktree_with_transcript(SESSION_A);
    f.save(1, false, vec![SavedTab::Main, agent(&brave, SESSION_A, Level::Edits)]);
    f.begin();
    f.until_settled();
    let tab = f.tabs.items()[1].pane.agent.as_ref().unwrap();
    assert_eq!(tab.resume_note, Some(Why::Chosen.note(false)));
    assert!(tab.resume_line.as_deref().is_some_and(|line| line.contains(&"d".repeat(1000))));
    assert_eq!(refreshed_status(&mut f), AgentStatus::Stopped);
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
            assert_eq!(got.note, Some(Why::Yolo.note(true)), "{label}");
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

#[test]
fn a_reload_takes_the_restore_switches_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = Host::new(false, false, RealDisk::new(dir.path()));
    assert!(host.restore_enabled());
    host.reload(false, ResumeAgents::Off);
    assert!(!host.restore_enabled());
    assert_eq!(host.resume_agents(), ResumeAgents::Off);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
