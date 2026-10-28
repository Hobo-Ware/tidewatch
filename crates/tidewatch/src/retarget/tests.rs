use tidewatch_core::elsewhere::{Host, Presence, Running};
use tidewatch_core::elsewhere_flags::Flags;

use super::*;

const ID: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";
const START: &str = "Fri Oct 16 13:23:31 2026";

fn session(cli: Cli, level: Option<Level>, activity: Activity, id: Option<&str>) -> Session {
    Session {
        cli,
        session_id: id.map(str::to_owned),
        cwd: PathBuf::from("/work/web"),
        name: None,
        flags: Flags { level, ..Flags::default() },
        presence: Presence::Running(Running {
            pid: 11,
            proc_start: START.to_owned(),
            host: Host::Other,
            activity,
        }),
        since_ms: None,
    }
}

fn worktree() -> Worktree {
    Worktree {
        repo: PathBuf::from("/work/web"),
        path: PathBuf::from("/work/web"),
        branch: "agent/calm-comet".to_owned(),
    }
}

fn subject(in_worktree: bool) -> Subject {
    Subject {
        tab: 3,
        shell_pid: Some(10),
        repo: PathBuf::from("/work/web"),
        worktree: in_worktree.then(worktree),
        dir: PathBuf::from("/work/web"),
        cli: None,
        known_session_id: None,
        remote: false,
        pinned: false,
        settling: false,
        window: Window {
            repos: vec![PathBuf::from("/work/web"), PathBuf::from("/work/api")],
            shown: vec![PathBuf::from("/work/web"), PathBuf::from("/tw/web/shared-otter")],
        },
    }
}

struct World {
    profiles: Profiles,
    agents: AgentTable,
    installed: Vec<String>,
}

fn world() -> World {
    let profiles = Profiles::seeded(&["claude".to_owned(), "codex".to_owned()]);
    let agents = profiles.agent_table();
    World { profiles, agents, installed: vec!["claude".to_owned(), "codex".to_owned()] }
}

fn run(
    target: &Target,
    source: Source,
    subject: &Subject,
    world: &World,
    interrupt: bool,
) -> Result<Plan, Failure> {
    plan(
        target,
        source,
        subject,
        &Folder::default(),
        (&world.profiles, &world.agents, &world.installed),
        interrupt,
    )
}

fn builder(level: Option<Level>) -> Target {
    Target::Agent { profile: "builder".to_owned(), level }
}

fn agent_plan(planned: Result<Plan, Failure>) -> AgentPlan {
    match planned {
        Ok(Plan::Agent(plan)) => *plan,
        other => panic!("expected an agent plan, got {other:?}"),
    }
}

#[test]
fn a_session_found_in_a_shell_tab_resumes_under_the_builder_profile_at_the_level_it_ran_at() {
    let w = world();
    let found = session(Cli::Claude, Some(Level::Auto), Activity::Idle, Some(ID));
    let plan = agent_plan(run(&builder(None), Source::Running(&found), &subject(true), &w, false));
    assert_eq!(plan.start, Start::Resume(ID.to_owned()));
    assert_eq!(plan.profile.level, Level::Auto);
    assert_eq!(plan.profile_id, "builder");
    assert_eq!(plan.agent.id, "claude");
    assert_eq!(plan.note, None);
}

#[test]
fn yolo_runs_in_any_folder_whether_asked_for_or_resumed() {
    let w = world();
    let found = session(Cli::Claude, Some(Level::Yolo), Activity::Idle, Some(ID));
    for in_worktree in [true, false] {
        let kept = agent_plan(run(
            &builder(None),
            Source::Running(&found),
            &subject(in_worktree),
            &w,
            false,
        ));
        assert_eq!(kept.profile.level, Level::Yolo, "resumed at the level it ran at");
        let asked = agent_plan(run(
            &builder(Some(Level::Yolo)),
            Source::Running(&found),
            &subject(in_worktree),
            &w,
            false,
        ));
        assert_eq!(asked.profile.level, Level::Yolo, "exactly the level picked");
        assert_eq!(asked.start, Start::Resume(ID.to_owned()));
    }
}

#[test]
fn a_level_asked_for_wins_over_the_one_the_session_ran_at() {
    let w = world();
    let found = session(Cli::Claude, Some(Level::Auto), Activity::Idle, Some(ID));
    let plan = agent_plan(run(
        &builder(Some(Level::Plan)),
        Source::Running(&found),
        &subject(true),
        &w,
        false,
    ));
    assert_eq!(plan.profile.level, Level::Plan);
}

#[test]
fn fit_level_keeps_what_runs_and_moves_what_has_no_flags_to_the_nearest_lower_level() {
    let cases = [
        (Level::Auto, vec![], Level::Auto),
        (Level::Yolo, vec![], Level::Yolo),
        (Level::Yolo, vec![Level::Yolo], Level::Auto),
        (Level::Yolo, vec![Level::Auto, Level::Yolo], Level::Edits),
        (Level::Plan, vec![Level::Plan], Level::Ask),
        (Level::Plan, vec![Level::Plan, Level::Ask, Level::Edits], Level::Auto),
        (Level::Edits, vec![Level::Edits, Level::Ask], Level::Plan),
        (Level::Yolo, Level::ALL.to_vec(), Level::Yolo),
    ];
    for (level, unsupported, want) in cases {
        assert_eq!(fit_level(level, &unsupported), want, "{level:?} {unsupported:?}");
    }
}

#[test]
fn what_used_to_be_refused_now_goes_ahead() {
    let w = world();
    let idle = session(Cli::Claude, Some(Level::Edits), Activity::Idle, Some(ID));
    let no_id = session(Cli::Claude, Some(Level::Edits), Activity::Idle, None);
    let codex = session(Cli::Codex, None, Activity::Idle, Some(ID));
    let mut elsewhere = idle.clone();
    elsewhere.cwd = PathBuf::from("/work/other");

    let other_folder =
        agent_plan(run(&builder(None), Source::Running(&elsewhere), &subject(true), &w, false));
    assert_eq!(other_folder.start, Start::Resume(ID.to_owned()));
    assert_eq!(other_folder.landing.dir, PathBuf::from("/work/other"));

    let unknown = Target::Agent { profile: "nope".into(), level: None };
    let fallback = agent_plan(run(&unknown, Source::Running(&idle), &subject(true), &w, false));
    assert_eq!(fallback.profile_id, Profile::DEFAULTS_ID);
    assert_eq!(fallback.agent.id, "claude");
    assert_eq!(fallback.start, Start::Resume(ID.to_owned()));

    let cross = agent_plan(run(&builder(None), Source::Running(&codex), &subject(true), &w, false));
    assert_eq!(cross.start, Start::Fresh, "a Codex conversation cannot continue under Claude");
    assert!(cross.note.as_deref().unwrap().contains("cannot continue a codex conversation"));
    let codex_key = w
        .profiles
        .profile
        .iter()
        .find(|(_, profile)| profile.agent == "codex")
        .map(|(key, _)| key.clone())
        .unwrap();
    let to_codex = Target::Agent { profile: codex_key, level: None };
    let across = agent_plan(run(&to_codex, Source::Running(&idle), &subject(true), &w, false));
    assert_eq!(across.start, Start::Fresh);
    assert!(across.note.as_deref().unwrap().contains("cannot continue a claude conversation"));

    let unreadable =
        agent_plan(run(&builder(None), Source::Running(&no_id), &subject(true), &w, false));
    assert_eq!(unreadable.start, Start::Fresh);
    assert!(unreadable.note.as_deref().unwrap().contains("id could not be read"));

    let plain = agent_plan(run(&builder(None), Source::Nothing, &subject(true), &w, false));
    assert_eq!(plain.start, Start::Fresh);
    assert_eq!(plain.note, None);
}

#[test]
fn only_a_wait_or_a_real_error_stops_a_change() {
    let w = world();
    let working = session(Cli::Claude, Some(Level::Edits), Activity::Working, Some(ID));
    let mid = run(&builder(None), Source::Running(&working), &subject(true), &w, false);
    assert_eq!(mid.err(), Some(Failure::MidTurn));
    let settling = Subject { settling: true, ..subject(true) };
    let wait = run(&builder(None), Source::Nothing, &settling, &w, false);
    assert_eq!(wait.err(), Some(Failure::Settling));
    let w = World { installed: Vec::new(), ..world() };
    let found = session(Cli::Claude, None, Activity::Idle, Some(ID));
    let got = run(&builder(None), Source::Running(&found), &subject(true), &w, false);
    assert_eq!(got.err(), Some(Failure::AgentMissing("claude".into())));
    for failure in [Failure::MidTurn, Failure::Settling] {
        assert!(failure.waits());
    }
    for failure in [Failure::ScanFailed, Failure::Changed, Failure::AgentMissing("x".into())] {
        assert!(!failure.waits());
    }
}

#[test]
fn a_busy_shell_with_a_command_changes_like_any_other() {
    let w = world();
    let plan = agent_plan(run(&builder(None), Source::Nothing, &subject(true), &w, false));
    assert_eq!(plan.start, Start::Fresh);
}

#[test]
fn a_turn_in_progress_is_cut_short_only_when_asked() {
    let w = world();
    let working = session(Cli::Claude, Some(Level::Edits), Activity::Working, Some(ID));
    let plan = agent_plan(run(&builder(None), Source::Running(&working), &subject(true), &w, true));
    assert_eq!(plan.start, Start::Resume(ID.to_owned()));
}

#[test]
fn a_shell_target_ends_the_agent_and_keeps_the_session_id_when_there_is_one() {
    let w = world();
    let found = session(Cli::Claude, None, Activity::Idle, Some(ID));
    let got = run(&Target::Shell, Source::Running(&found), &subject(true), &w, false);
    assert_eq!(got, Ok(Plan::Shell { remembered: Some((Cli::Claude, ID.to_owned())) }));
    let no_id = session(Cli::Claude, None, Activity::Idle, None);
    let got = run(&Target::Shell, Source::Running(&no_id), &subject(true), &w, false);
    assert_eq!(got, Ok(Plan::Shell { remembered: None }), "nothing to remember is not a refusal");
    let nothing = run(&Target::Shell, Source::Nothing, &subject(true), &w, false);
    assert_eq!(nothing, Ok(Plan::Shell { remembered: None }), "a plain shell restarts");
}

#[test]
fn an_agent_tab_whose_scan_cannot_name_the_session_uses_the_id_it_was_launched_with() {
    let w = world();
    let found = session(Cli::Claude, Some(Level::Edits), Activity::Idle, None);
    let launched = Subject { known_session_id: Some(ID.to_owned()), ..subject(true) };
    let plan = agent_plan(run(&builder(None), Source::Running(&found), &launched, &w, false));
    assert_eq!(plan.start, Start::Resume(ID.to_owned()));
}

#[test]
fn a_session_the_tab_ended_earlier_resumes_without_ending_anything() {
    let w = world();
    let source = Source::Stored { cli: Cli::Claude, session_id: ID };
    let plan = agent_plan(run(&builder(None), source, &subject(true), &w, false));
    assert_eq!(plan.start, Start::Resume(ID.to_owned()));
    assert_eq!(plan.landing.dir, PathBuf::from("/work/web"));
}

#[test]
fn a_plain_shell_starts_a_new_session_in_place_at_the_profiles_level() {
    let w = world();
    let plan = agent_plan(run(&builder(None), Source::Nothing, &subject(true), &w, false));
    assert_eq!(plan.start, Start::Fresh);
    assert_eq!(plan.profile.level, w.profiles.profile["builder"].level);
}

#[test]
fn the_folder_check_treats_two_spellings_of_one_folder_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().canonicalize().unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(same_folder(&real, &link));
    assert!(!same_folder(&real, Path::new("/nowhere/else")));
    std::fs::create_dir(real.join("sub")).unwrap();
    assert!(is_inside(&link.join("sub"), &real));
    assert!(!is_inside(Path::new("/work/webby"), Path::new("/work/web")));
}

fn folder(main: &str, branch: Option<&str>, made: bool) -> Folder {
    Folder {
        main: Some(PathBuf::from(main)),
        branch: branch.map(str::to_owned),
        made_by_tidewatch: made,
    }
}

fn tw_worktree(repo: &str, path: &str, branch: &str) -> Worktree {
    Worktree { repo: PathBuf::from(repo), path: PathBuf::from(path), branch: branch.to_owned() }
}

#[test]
fn the_tab_follows_the_folder_the_session_runs_in() {
    let tab = subject(true);
    let cases: Vec<(&str, &str, Folder, Subject, Landing)> = vec![
        (
            "same folder",
            "/work/web",
            Folder::default(),
            tab.clone(),
            Landing {
                dir: "/work/web".into(),
                worktree: Some(worktree()),
                repo: "/work/web".into(),
            },
        ),
        (
            "a subfolder of the tab's own folder",
            "/work/web/packages/ui",
            folder("/work/web", Some("agent/calm-comet"), false),
            tab.clone(),
            Landing {
                dir: "/work/web/packages/ui".into(),
                worktree: Some(worktree()),
                repo: "/work/web".into(),
            },
        ),
        (
            "another worktree of the same repo",
            "/tw/web/upbeat-lantern",
            folder("/work/web", Some("agent/upbeat-lantern"), true),
            tab.clone(),
            Landing {
                dir: "/tw/web/upbeat-lantern".into(),
                worktree: Some(tw_worktree(
                    "/work/web",
                    "/tw/web/upbeat-lantern",
                    "agent/upbeat-lantern",
                )),
                repo: "/work/web".into(),
            },
        ),
        (
            "a worktree of another open repo",
            "/tw/api/quick-fox",
            folder("/work/api", Some("agent/quick-fox"), true),
            tab.clone(),
            Landing {
                dir: "/tw/api/quick-fox".into(),
                worktree: Some(tw_worktree("/work/api", "/tw/api/quick-fox", "agent/quick-fox")),
                repo: "/work/api".into(),
            },
        ),
        (
            "a worktree of a repo that is not open",
            "/tw/lib/calm-owl",
            folder("/code/lib", Some("agent/calm-owl"), true),
            tab.clone(),
            Landing {
                dir: "/tw/lib/calm-owl".into(),
                worktree: Some(tw_worktree("/code/lib", "/tw/lib/calm-owl", "agent/calm-owl")),
                repo: "/code/lib".into(),
            },
        ),
        (
            "an unknown git repo",
            "/code/lib",
            folder("/code/lib", Some("main"), false),
            tab.clone(),
            Landing { dir: "/code/lib".into(), worktree: None, repo: "/code/lib".into() },
        ),
        (
            "a worktree that is not Tidewatch's",
            "/code/web-fix",
            folder("/work/web", Some("fix/login"), false),
            tab.clone(),
            Landing { dir: "/code/web-fix".into(), worktree: None, repo: "/work/web".into() },
        ),
        (
            "a plain folder",
            "/home/me/notes",
            Folder::default(),
            tab.clone(),
            Landing { dir: "/home/me/notes".into(), worktree: None, repo: "/work/web".into() },
        ),
        (
            "a folder another tab shows",
            "/tw/web/shared-otter",
            folder("/work/web", Some("agent/shared-otter"), true),
            tab.clone(),
            Landing {
                dir: "/tw/web/shared-otter".into(),
                worktree: None,
                repo: "/work/web".into(),
            },
        ),
        (
            "the pinned main tab stays in its group",
            "/tw/api/quick-fox",
            folder("/work/api", Some("agent/quick-fox"), true),
            Subject { pinned: true, ..tab.clone() },
            Landing { dir: "/tw/api/quick-fox".into(), worktree: None, repo: "/work/web".into() },
        ),
    ];
    for (what, cwd, folder, subject, want) in cases {
        assert_eq!(landing(Path::new(cwd), &subject, &folder), want, "{what}");
    }
}

#[test]
fn a_session_in_another_folder_resumes_there_with_the_folders_worktree() {
    let w = world();
    let mut found = session(Cli::Claude, Some(Level::Auto), Activity::Idle, Some(ID));
    found.cwd = PathBuf::from("/tw/web/upbeat-lantern");
    let there = folder("/work/web", Some("agent/upbeat-lantern"), true);
    let planned = plan(
        &builder(None),
        Source::Running(&found),
        &subject(false),
        &there,
        (&w.profiles, &w.agents, &w.installed),
        false,
    );
    let plan = agent_plan(planned);
    assert_eq!(plan.start, Start::Resume(ID.to_owned()));
    assert_eq!(plan.landing.dir, PathBuf::from("/tw/web/upbeat-lantern"));
    assert_eq!(plan.landing.worktree.as_ref().map(Worktree::name), Some("upbeat-lantern"));
    assert_eq!(plan.profile.level, Level::Auto);
    let yolo = session(Cli::Claude, Some(Level::Yolo), Activity::Idle, Some(ID));
    let yolo = Session { cwd: found.cwd.clone(), ..yolo };
    let planned = plan_for(&w, &yolo, &subject(false), &Folder::default());
    assert_eq!(planned.profile.level, Level::Yolo, "a plain folder keeps YOLO too");
}

fn plan_for(w: &World, found: &Session, subject: &Subject, there: &Folder) -> AgentPlan {
    agent_plan(plan(
        &builder(None),
        Source::Running(found),
        subject,
        there,
        (&w.profiles, &w.agents, &w.installed),
        false,
    ))
}

#[test]
fn every_failure_has_a_plain_sentence_and_none_shows_a_home_folder() {
    let failures = [
        Failure::MidTurn,
        Failure::Settling,
        Failure::ScanFailed,
        Failure::Changed,
        Failure::AgentMissing("claude".into()),
    ];
    for failure in failures {
        let text = failure.text();
        assert!(!text.is_empty() && text.ends_with('.'), "{failure:?}: {text}");
        assert!(!text.contains("/Users/") && !text.contains("/home/"), "{text}");
    }
}
