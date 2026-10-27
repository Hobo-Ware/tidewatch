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
        shell_pid: 10,
        repo: PathBuf::from("/work/web"),
        worktree: in_worktree.then(worktree),
        dir: PathBuf::from("/work/web"),
        cli: None,
        known_session_id: None,
        foreground_busy: false,
        remote: false,
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
) -> Result<Plan, Refusal> {
    plan(target, source, subject, (&world.profiles, &world.agents, &world.installed), interrupt)
}

fn builder(level: Option<Level>) -> Target {
    Target::Agent { profile: "builder".to_owned(), level }
}

fn agent_plan(planned: Result<Plan, Refusal>) -> AgentPlan {
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
}

#[test]
fn yolo_is_kept_in_a_worktree_tab_and_lowered_outside_one() {
    let w = world();
    let found = session(Cli::Claude, Some(Level::Yolo), Activity::Idle, Some(ID));
    let kept = agent_plan(run(&builder(None), Source::Running(&found), &subject(true), &w, false));
    assert_eq!(kept.profile.level, Level::Yolo);
    let lowered =
        agent_plan(run(&builder(None), Source::Running(&found), &subject(false), &w, false));
    assert_eq!(Some(lowered.profile.level), lowered.agent.highest_level_below_yolo());
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
fn what_cannot_go_ahead_is_refused_with_its_reason() {
    let w = world();
    let idle = session(Cli::Claude, Some(Level::Edits), Activity::Idle, Some(ID));
    let working = session(Cli::Claude, Some(Level::Edits), Activity::Working, Some(ID));
    let no_id = session(Cli::Claude, Some(Level::Edits), Activity::Idle, None);
    let codex = session(Cli::Codex, None, Activity::Idle, Some(ID));
    let mut elsewhere = idle.clone();
    elsewhere.cwd = PathBuf::from("/work/other");
    let yolo = builder(Some(Level::Yolo));
    let cases: Vec<(&str, Result<Plan, Refusal>, Refusal)> = vec![
        (
            "mid-turn",
            run(&builder(None), Source::Running(&working), &subject(true), &w, false),
            Refusal::MidTurn,
        ),
        (
            "session id unknown",
            run(&builder(None), Source::Running(&no_id), &subject(true), &w, false),
            Refusal::NoSessionId,
        ),
        (
            "yolo outside a worktree",
            run(&yolo, Source::Running(&idle), &subject(false), &w, false),
            Refusal::YoloNeedsWorktree,
        ),
        (
            "another folder",
            run(&builder(None), Source::Running(&elsewhere), &subject(true), &w, false),
            Refusal::OtherFolder { session: PathBuf::from("/work/other") },
        ),
        (
            "unknown profile",
            run(
                &Target::Agent { profile: "nope".into(), level: None },
                Source::Running(&idle),
                &subject(true),
                &w,
                false,
            ),
            Refusal::NoProfile("nope".into()),
        ),
        (
            "profile of another cli",
            run(&builder(None), Source::Running(&codex), &subject(true), &w, false),
            Refusal::OtherCli {
                session: Cli::Codex,
                profile: w.profiles.profile["builder"].name.clone(),
                agent: "claude".into(),
            },
        ),
        (
            "nothing to end",
            run(&Target::Shell, Source::Nothing, &subject(true), &w, false),
            Refusal::NothingRunning,
        ),
        (
            "a command has the shell",
            run(
                &builder(None),
                Source::Nothing,
                &Subject { foreground_busy: true, ..subject(true) },
                &w,
                false,
            ),
            Refusal::ShellBusy,
        ),
    ];
    for (what, got, want) in cases {
        assert_eq!(got.err(), Some(want), "{what}");
    }
}

#[test]
fn a_cli_that_is_not_installed_is_refused() {
    let w = World { installed: Vec::new(), ..world() };
    let found = session(Cli::Claude, None, Activity::Idle, Some(ID));
    let got = run(&builder(None), Source::Running(&found), &subject(true), &w, false);
    assert_eq!(got.err(), Some(Refusal::AgentMissing("claude".into())));
}

#[test]
fn a_turn_in_progress_is_cut_short_only_when_asked() {
    let w = world();
    let working = session(Cli::Claude, Some(Level::Edits), Activity::Working, Some(ID));
    let plan = agent_plan(run(&builder(None), Source::Running(&working), &subject(true), &w, true));
    assert_eq!(plan.start, Start::Resume(ID.to_owned()));
}

#[test]
fn a_shell_target_ends_the_agent_and_keeps_the_session_id() {
    let w = world();
    let found = session(Cli::Claude, None, Activity::Idle, Some(ID));
    let got = run(&Target::Shell, Source::Running(&found), &subject(true), &w, false);
    assert_eq!(got, Ok(Plan::Shell { cli: Cli::Claude, session_id: ID.to_owned() }));
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
    assert_eq!(plan.cwd, PathBuf::from("/work/web"));
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
}

#[test]
fn every_refusal_has_a_sentence_for_the_toast() {
    let refusals = [
        Refusal::NothingRunning,
        Refusal::ShellBusy,
        Refusal::MidTurn,
        Refusal::NoSessionId,
        Refusal::YoloNeedsWorktree,
        Refusal::OtherFolder { session: "/x".into() },
        Refusal::NoProfile("p".into()),
        Refusal::OtherCli {
            session: Cli::Codex,
            profile: "Builder".into(),
            agent: "claude".into(),
        },
        Refusal::AgentMissing("claude".into()),
        Refusal::NotResumable("mine".into()),
        Refusal::Gone,
        Refusal::WillNotEnd { secs: 5 },
        Refusal::SignalFailed,
        Refusal::ScanFailed,
    ];
    for refusal in refusals {
        let text = refusal.text();
        assert!(!text.is_empty() && !text.ends_with('.'), "{refusal:?}: {text}");
    }
}
