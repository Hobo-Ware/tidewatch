use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use tidewatch_core::events::TabEvent;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::runner::{AskReason, Phase, StageState};
use tidewatch_core::status::{HookEvent, HookKind};
use tidewatch_core::workflows::{Gate, Stage, StageRun, Workflows};

use super::*;
use crate::workflow_ask::Choice;

fn saved_run(paths: &TidewatchPaths, worktree: &Worktree) -> Option<Box<Runner>> {
    let file = paths.stages_file(&worktree.repo, worktree.name());
    tidewatch_core::restore::restore_workflow(&file).map(|(runner, _)| Box::new(runner))
}

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

struct Fixture {
    _home: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    paths: TidewatchPaths,
    catalog: Catalog,
    host: Host,
    tabs: Tabs<Pane<u32>>,
    ctx: egui::Context,
    spawned: Cell<u32>,
    shells: Cell<u32>,
    played: HashSet<(u64, u64)>,
    attempts: HashMap<usize, u8>,
    asked: Vec<(u64, Vec<Exec>)>,
    toasts: Vec<String>,
    log: RefCell<Vec<String>>,
}

impl Fixture {
    fn new(test_script: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().canonicalize().unwrap();
        let repo = root.join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join(".tidewatch.toml"), format!("test = \"{test_script}\"\n"))
            .unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let mut catalog = Catalog::new(OsString::from("/usr/bin:/bin"));
        catalog.set_profiles(Profiles::seeded(&["claude".to_owned(), "gemini".to_owned()]));
        Self {
            paths: TidewatchPaths::new(&root),
            _home: home,
            root,
            repo,
            catalog,
            host: Host::new(),
            tabs: Tabs::new(Pane::shell(0)),
            ctx: egui::Context::default(),
            spawned: Cell::new(0),
            shells: Cell::new(0),
            played: HashSet::new(),
            attempts: HashMap::new(),
            asked: Vec::new(),
            toasts: Vec::new(),
            log: RefCell::new(Vec::new()),
        }
    }

    fn request(workflow: Workflow, task: &str) -> WorkflowRequest {
        let profiles = Profiles::seeded(&["claude".to_owned(), "gemini".to_owned()]).profile;
        let runner = Runner::new(
            &workflow,
            &profiles,
            RunContext {
                task: task.into(),
                base: "main".into(),
                branch: "b".into(),
                test_command: None,
            },
        )
        .unwrap();
        let first_agent = runner.profile(0).unwrap().agent.clone();
        WorkflowRequest { workflow, profiles, task: task.into(), first_agent }
    }

    fn ship(task: &str) -> WorkflowRequest {
        Self::request(Workflows::defaults().workflow.remove("ship").unwrap(), task)
    }

    fn open(&mut self, start: Start, existing: Option<Worktree>) -> u64 {
        let env = Env {
            paths: &self.paths,
            catalog: &self.catalog,
            listener: None,
            tools: Tools::with_path(std::env::var_os("PATH").unwrap_or_default()),
            trust: None,
        };
        self.host.open(&self.ctx, &mut self.tabs, 0, &env, &self.repo, existing, start, 0.0)
    }

    fn tick(&mut self) {
        let env = Env {
            paths: &self.paths,
            catalog: &self.catalog,
            listener: None,
            tools: Tools::with_path(std::env::var_os("PATH").unwrap_or_default()),
            trust: None,
        };
        let agent = |_: &Path, _: &Prepared| {
            self.spawned.set(self.spawned.get() + 1);
            self.spawned.get()
        };
        let shell = |_: &Path| {
            self.shells.set(self.shells.get() + 1);
            0
        };
        let spawn = Spawners { agent: &agent, shell: &shell };
        let asked = std::mem::take(&mut self.asked);
        let toasts = self.host.tick(&self.ctx, &mut self.tabs, &env, &spawn, asked, 1.0);
        self.toasts.extend(toasts);
    }

    fn run(&mut self, tab: u64) -> &mut TabRun {
        tab_run(&mut self.tabs, tab).expect("a workflow tab")
    }

    fn feed(&mut self, tab: u64, generation: u64, event: TabEvent) {
        let message = TabMessage { tab_id: tab, generation, event };
        if let Some(execs) = feed(&mut self.tabs, &message) {
            self.asked.push((tab, execs));
        }
    }

    fn hook(&mut self, tab: u64, generation: u64) {
        let event = TabEvent::Hook(HookEvent {
            kind: HookKind::TurnDone,
            session_id: None,
            transcript_path: None,
            message: None,
        });
        self.feed(tab, generation, event);
    }

    fn done(&mut self, tab: u64, generation: u64, summary: &str) {
        self.feed(tab, generation, TabEvent::StageDone { summary: summary.into() });
    }

    fn blocking(&mut self, tab: u64, generation: u64, text: &str) {
        let event = TabEvent::Finding {
            file: "src/auth.ts".into(),
            line: 12,
            blocking: true,
            text: text.into(),
        };
        self.feed(tab, generation, event);
    }

    /// Plays the agent of whatever stage just started in `tab`, from the script for `(stage, round)`.
    fn play_agent(&mut self, tab: u64) -> bool {
        let (generation, index, ready) = {
            let run = self.run(tab);
            (run.generation(), run.runner().position(), !run.launching())
        };
        let working = *self.run(tab).runner().phase() == Phase::Working;
        if !ready || !working || generation == 0 || !self.played.insert((tab, generation)) {
            return false;
        }
        let round = self.attempts.entry(index).or_insert(0);
        *round += 1;
        let round = *round;
        let late = generation - 1;
        self.hook(tab, late);
        self.done(tab, late, "a stale summary");
        match (index, round) {
            (0, 1) => self.done(tab, generation, "Added the refresh timer"),
            (1, 1) => {
                self.blocking(tab, generation, "the timer keeps running after sign-out");
                self.done(tab, generation, "One blocking finding");
            }
            (0, 2) => self.done(tab, generation, "Cleared the timer in signOut"),
            (1, 2) => self.done(tab, generation, "Nothing blocking"),
            (2, 1) => self.done(tab, generation, "Removed the duplicate in session.ts"),
            (3, 1) => self.done(tab, generation, "feat: retry the token refresh"),
            other => panic!("no scripted turn for {other:?}"),
        }
        self.hook(tab, generation);
        self.log.borrow_mut().push(format!("played {index}/{round} as {generation}"));
        true
    }

    /// Ticks, with the scripted agents answering, until the run waits for the user or ends.
    fn until_asked(&mut self, tab: u64) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            self.tick();
            if self.run_exists(tab) != Some(true) {
                assert!(Instant::now() < deadline, "the tab never got its run");
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            self.play_agent(tab);
            let run = self.run(tab);
            if !run.launching()
                && matches!(
                    run.runner().phase(),
                    Phase::Asking(_) | Phase::Finished | Phase::Stopped(_)
                )
            {
                self.tick();
                return;
            }
            assert!(Instant::now() < deadline, "the run never asked: {:?}", self.log.borrow());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn runs_of_the_tests(&self) -> usize {
        std::fs::read_to_string(self.root.join("runs.log")).map_or(0, |text| text.lines().count())
    }
}

fn counting_tests(root: &Path) -> String {
    format!("echo run >> {}/runs.log", root.display())
}

#[test]
fn ship_a_feature_through_the_glue_loops_once_and_waits_at_hand_off() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().canonicalize().unwrap();
    let mut f = Fixture::new(&counting_tests(&root));
    f.root = root;
    let request = Fixture::ship("add a retry");
    let tab = f.open(Start::New(Box::new(request)), None);
    assert_eq!(f.tabs.get_mut(tab).unwrap().title, CREATING_TITLE);

    f.until_asked(tab);

    let found = f.tabs.get_mut(tab).unwrap();
    let worktree = found.pane.worktree.clone().expect("the worktree was made");
    assert_eq!(found.title, worktree.name());
    assert!(matches!(found.pane.body, Body::Shell(6)), "the sixth agent runs in the tab");
    assert_eq!(f.spawned.get(), 6, "work, review, work, review, refactor, hand off");
    let run = f.run(tab);
    assert_eq!(run.runner().loops_used(1), 1);
    assert_eq!(
        run.runner().states(),
        [StageState::Done, StageState::Done, StageState::Done, StageState::NeedsYou]
    );
    assert_eq!(run.runner().summary(1), Some("Nothing blocking"), "stale summaries never landed");
    assert_eq!(run.last_summary(), Some("feat: retry the token refresh"));
    assert!(matches!(run.runner().phase(), Phase::Asking(AskReason::Approve)));
    assert_eq!(run.runner().context().branch, worktree.branch);
    assert_eq!(run.runner().context().base, "main");

    let agent = f.tabs.get_mut(tab).unwrap().pane.agent.as_mut().unwrap();
    agent.run.refresh(2.0);
    assert_eq!(agent.run.status(), &AgentStatus::NeedsYou(tidewatch_core::status::Reason::Input));
    let dividers: Vec<_> = agent.reader.rows.iter().map(|row| row.text.clone()).collect();
    assert_eq!(
        dividers,
        [
            "--- Stage 1: Work (claude opus, Auto edit) ---",
            "--- Stage 2: Review (gemini, Read only) ---",
            "--- Stage 1: Work (claude opus, Auto edit, round 2) ---",
            "--- Stage 2: Review (gemini, Read only, round 2) ---",
            "--- Stage 3: Refactor (claude sonnet, Auto edit) ---",
            "--- Stage 4: Hand off (claude haiku, Read only) ---",
        ]
    );
    assert_eq!(f.runs_of_the_tests(), 3, "work, work again, refactor");
    let file = f.paths.stages_file(&worktree.repo, worktree.name());
    assert_eq!(
        Runner::load(&file).as_ref(),
        Some(f.run(tab).runner()),
        "saved after the last event"
    );

    let execs = f.run(tab).choose(Choice::Approve);
    assert_eq!(execs, Vec::new());
    f.tick();
    let agent = f.tabs.get_mut(tab).unwrap().pane.agent.as_mut().unwrap();
    agent.run.refresh(3.0);
    assert_eq!(agent.run.status(), &AgentStatus::Done);
    assert!(f.run(tab).finished());
}

#[test]
fn a_saved_run_comes_back_in_a_new_tab_and_carries_on() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().canonicalize().unwrap();
    let mut f = Fixture::new(&counting_tests(&root));
    f.root = root;
    let request = Fixture::ship("add a retry");
    let tab = f.open(Start::New(Box::new(request)), None);
    f.until_asked(tab);
    let worktree = f.tabs.get_mut(tab).unwrap().pane.worktree.clone().unwrap();

    let mut again = Fixture::new("true");
    again.paths = f.paths.clone();
    again.repo = f.repo.clone();
    let runner = saved_run(&f.paths, &worktree).expect("the saved run loads");
    let resumed = again.open(Start::Resume(runner), Some(worktree.clone()));
    let deadline = Instant::now() + Duration::from_secs(30);
    while again.tabs.get_mut(resumed).unwrap().pane.worktree.is_none() {
        again.tick();
        assert!(Instant::now() < deadline, "the resumed tab never opened");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        again.shells.get(),
        1,
        "a run that waits for you gets a plain shell until it launches"
    );
    assert_eq!(again.spawned.get(), 0);
    let ask = again.run(resumed).ask().expect("it asks again");
    assert_eq!(ask.title, "Hand off 4/4 is waiting for your approval");
    assert_eq!(ask.detail.as_deref(), Some("feat: retry the token refresh"));

    again.run(resumed).choose(Choice::Approve);
    again.tick();
    assert!(again.run(resumed).finished());
}

#[test]
fn a_stage_that_runs_again_after_a_restart_launches_its_agent() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().canonicalize().unwrap();
    let mut f = Fixture::new(&counting_tests(&root));
    f.root = root;
    let request = Fixture::ship("add a retry");
    let tab = f.open(Start::New(Box::new(request)), None);
    let deadline = Instant::now() + Duration::from_secs(30);
    while f.run_exists(tab).is_none_or(|run| !run) {
        f.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    while f.spawned.get() == 0 {
        f.tick();
        assert!(Instant::now() < deadline, "the first agent never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    let worktree = f.tabs.get_mut(tab).unwrap().pane.worktree.clone().unwrap();

    let mut again = Fixture::new("true");
    again.paths = f.paths.clone();
    again.repo = f.repo.clone();
    let runner = saved_run(&f.paths, &worktree).unwrap();
    let resumed = again.open(Start::Resume(runner), Some(worktree));
    while again.spawned.get() == 0 {
        again.tick();
        assert!(Instant::now() < deadline, "the resumed stage never launched");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(again.shells.get(), 0);
    assert_eq!(again.run(resumed).runner().position(), 0);
}

impl Fixture {
    fn run_exists(&mut self, tab: u64) -> Option<bool> {
        let found = self.tabs.get_mut(tab)?;
        Some(found.pane.agent.as_ref().is_some_and(|agent| agent.workflow.is_some()))
    }
}

fn workflow_with_agent(agent: &str) -> Workflow {
    Workflow {
        id: "w".into(),
        name: "Lonely".into(),
        description: String::new(),
        stages: vec![Stage {
            gate: Gate::NoBlocking,
            run: StageRun { agent: Some(agent.into()), ..StageRun::default() },
            ..Stage::new(tidewatch_core::workflows::Action::Review)
        }],
    }
}

#[test]
fn a_stage_whose_agent_is_not_known_asks_instead_of_hanging() {
    let mut f = Fixture::new("true");
    let request = WorkflowRequest {
        workflow: workflow_with_agent("ghost"),
        profiles: BTreeMap::new(),
        task: "x".into(),
        first_agent: "ghost".into(),
    };
    let tab = f.open(Start::New(Box::new(request)), None);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        f.tick();
        if f.run_exists(tab) == Some(true) && f.run(tab).ask().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "no question came");
        std::thread::sleep(Duration::from_millis(5));
    }
    let ask = f.run(tab).ask().unwrap();
    assert_eq!(ask.title, "Review 1/1: the agent failed");
    assert_eq!(ask.detail.as_deref(), Some("unknown agent \"ghost\""));
}

#[test]
fn a_repo_that_cannot_make_a_worktree_shows_the_error_in_the_tab() {
    let mut f = Fixture::new("true");
    f.repo = f.root.join("nowhere");
    let request = Fixture::ship("x");
    let tab = f.open(Start::New(Box::new(request)), None);
    let deadline = Instant::now() + Duration::from_secs(30);
    while matches!(f.tabs.get_mut(tab).unwrap().pane.body, Body::Creating) {
        f.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(f.tabs.get_mut(tab).unwrap().pane.body, Body::Failed(_)));
}

#[test]
fn a_late_message_of_a_replaced_agent_is_dropped_before_the_tab_sees_it() {
    let mut f = Fixture::new("true");
    let request = Fixture::ship("x");
    let tab = f.open(Start::New(Box::new(request)), None);
    let deadline = Instant::now() + Duration::from_secs(30);
    while f.spawned.get() == 0 {
        f.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let generation = f.run(tab).generation();
    let stale = TabMessage {
        tab_id: tab,
        generation: generation - 1,
        event: TabEvent::StageDone { summary: "late".into() },
    };
    assert_eq!(feed(&mut f.tabs, &stale), None);
    let live = TabMessage { generation, ..stale.clone() };
    assert!(feed(&mut f.tabs, &live).is_some());
    let closed = TabMessage { tab_id: 99, ..stale };
    assert_eq!(feed(&mut f.tabs, &closed), Some(Vec::new()), "the router drops a closed tab's");
    let plain = TabMessage {
        tab_id: 0,
        generation: 5,
        event: TabEvent::StageDone { summary: String::new() },
    };
    assert_eq!(feed(&mut f.tabs, &plain), Some(Vec::new()), "a tab without a workflow takes any");
}

#[test]
fn a_process_that_failed_fails_the_stage_unless_the_launch_is_still_on_its_way() {
    let mut f = Fixture::new("true");
    let request = Fixture::ship("x");
    let tab = f.open(Start::New(Box::new(request)), None);
    let deadline = Instant::now() + Duration::from_secs(30);
    while f.spawned.get() == 0 {
        f.tick();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let change =
        Change { from: AgentStatus::Working, to: AgentStatus::Failed("exited with code 1".into()) };
    assert_eq!(failures(&mut f.tabs, &[(tab, change)]), Vec::new(), "it asks, it launches nothing");
    let ask = f.run(tab).ask().expect("the stage failed");
    assert_eq!(ask.title, "Work 1/4: the agent failed");
    assert_eq!(ask.detail.as_deref(), Some("exited with code 1"));
    let other = Change { from: AgentStatus::Working, to: AgentStatus::Done };
    assert_eq!(failures(&mut f.tabs, &[(tab, other)]), Vec::new());
}

#[test]
fn a_workflow_opened_with_no_task_waits_in_its_tab_and_starts_only_on_start_run() {
    let mut f = Fixture::new("true");
    let tab = f.open(Start::New(Box::new(Fixture::ship(""))), None);
    let deadline = Instant::now() + Duration::from_secs(30);
    while f.run_exists(tab) != Some(true) {
        f.tick();
        assert!(Instant::now() < deadline, "the tab never got its run");
        std::thread::sleep(Duration::from_millis(5));
    }
    f.tick();
    assert!(f.run(tab).awaiting_task());
    assert_eq!((f.spawned.get(), f.shells.get()), (0, 1), "a shell in the worktree, no agent");
    let worktree = f.tabs.get_mut(tab).unwrap().pane.worktree.clone().unwrap();
    let saved = saved_run(&f.paths, &worktree).expect("the waiting run is saved");
    assert_eq!(*saved.phase(), Phase::Idle);

    let execs = f.run(tab).start_with("add a retry");
    f.asked.push((tab, execs));
    f.until_asked(tab);
    assert!(!f.run(tab).awaiting_task());
    assert!(f.spawned.get() >= 1, "stage 1 launched its agent");
}
