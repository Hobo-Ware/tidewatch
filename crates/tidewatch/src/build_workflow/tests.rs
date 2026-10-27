use std::collections::BTreeMap;
use std::time::Duration;

use tidewatch_core::events::TabEvent;
use tidewatch_core::runner::{RunContext, Runner};
use tidewatch_core::status::{HookEvent, HookKind, Reason};
use tidewatch_core::workflows::Workflows;

use super::*;
use crate::agent_run::AgentRun;
use crate::pane::{AgentTab, FirstPrompt};
use crate::workflow_run::TabRun;

fn hook(kind: HookKind) -> TabEvent {
    TabEvent::Hook(HookEvent { kind, session_id: None, transcript_path: None, message: None })
}

fn agent_pane(events: &[HookKind], first: FirstPrompt) -> Pane<u32> {
    let mut run = AgentRun::setting_up("claude".into(), 0.0);
    run.started(None, 0.0);
    let mut at = 0.0;
    for kind in events {
        at += 1.0;
        run.on_event(hook(kind.clone()), at);
    }
    run.refresh(at);
    let mut agent = AgentTab::new("Builder".into(), run);
    agent.first_prompt = first;
    Pane { body: Body::Shell(1), worktree: None, agent: Some(agent) }
}

fn idle() -> Pane<u32> {
    agent_pane(&[HookKind::TurnStarted, HookKind::TurnDone], FirstPrompt::NotNeeded)
}

fn row(seq: u64, kind: Kind, text: &str) -> Row {
    Row { seq, kind, text: text.to_owned(), pictures: Vec::new() }
}

#[test]
fn the_command_is_for_an_idle_agent_tab_and_each_other_state_says_why_not() {
    assert_eq!(availability(&idle(), false), Ok(()));
    let working = agent_pane(&[HookKind::TurnStarted], FirstPrompt::NotNeeded);
    assert_eq!(availability(&working, false), Err(Blocked::Working));
    let asking =
        agent_pane(&[HookKind::TurnStarted, HookKind::PermissionNeeded], FirstPrompt::NotNeeded);
    assert_eq!(availability(&asking, false), Err(Blocked::WaitingOnYou));
    let mut starting = AgentRun::setting_up("claude".into(), 0.0);
    starting.refresh(0.0);
    let pane = Pane {
        body: Body::Shell(1),
        worktree: None,
        agent: Some(AgentTab::new("Builder".into(), starting)),
    };
    assert_eq!(availability(&pane, false), Err(Blocked::Starting));
    let mut failed = idle();
    failed.agent.as_mut().unwrap().run.fail("exited".into());
    failed.agent.as_mut().unwrap().run.refresh(11.0);
    assert_eq!(availability(&failed, false), Err(Blocked::NotRunning));
    assert_eq!(availability(&Pane::shell(1), false), Err(Blocked::NoAgent));
    assert_eq!(availability(&Pane::<u32>::creating(), false), Err(Blocked::NoAgent));
}

#[test]
fn a_ready_tab_counts_as_idle_and_a_request_already_out_blocks_a_second() {
    let ready = agent_pane(&[], FirstPrompt::Awaiting);
    assert_eq!(availability(&ready, false), Ok(()));
    assert_eq!(availability(&idle(), true), Err(Blocked::Asked));
}

#[test]
fn a_workflow_tab_is_refused_even_when_its_agent_is_idle() {
    let ship = Workflows::defaults().workflow.remove("ship").unwrap();
    let context = RunContext {
        task: "t".into(),
        base: "main".into(),
        branch: "agent/x".into(),
        test_command: None,
    };
    let runner = Runner::new(&ship, &BTreeMap::new(), context).unwrap();
    let mut pane = idle();
    pane.agent.as_mut().unwrap().workflow =
        Some(Box::new(TabRun::new(runner, None, Duration::from_secs(600))));
    assert_eq!(availability(&pane, false), Err(Blocked::InWorkflow));
}

#[test]
fn every_reason_fits_a_palette_row() {
    let all = [
        Blocked::NoAgent,
        Blocked::InWorkflow,
        Blocked::NotRunning,
        Blocked::Starting,
        Blocked::Working,
        Blocked::WaitingOnYou,
        Blocked::Asked,
    ];
    for blocked in all {
        let reason = blocked.reason();
        assert!(reason.len() <= 26 && reason.is_ascii(), "{reason}");
    }
}

#[test]
fn the_wait_ends_when_the_turn_does_and_scans_the_rows_a_little_later() {
    let mut asked = Asked::new(4, Some(10), 100.0);
    assert_eq!(asked.step(&AgentStatus::Done, 100.5), Step::Wait(Some(115.0)));
    assert_eq!(asked.step(&AgentStatus::Working, 101.0), Step::Wait(None));
    assert_eq!(asked.step(&AgentStatus::NeedsYou(Reason::Permission), 102.0), Step::Wait(None));
    assert_eq!(asked.step(&AgentStatus::Done, 110.0), Step::Wait(Some(111.5)));
    assert_eq!(asked.step(&AgentStatus::Done, 111.0), Step::Wait(Some(111.5)));
    assert_eq!(asked.step(&AgentStatus::Done, 111.5), Step::Scan);
    assert_eq!(asked.scanned_empty(), Step::Wait(Some(114.0)));
    assert_eq!(asked.step(&AgentStatus::Done, 114.0), Step::Scan);
    assert_eq!(asked.scanned_empty(), Step::Wait(Some(118.0)));
    assert_eq!(asked.step(&AgentStatus::Done, 118.0), Step::Scan);
    assert_eq!(asked.scanned_empty(), Step::Dropped(Dropped::NoWorkflow));
}

#[test]
fn a_new_turn_while_settling_goes_back_to_waiting_for_its_end() {
    let mut asked = Asked::new(4, None, 0.0);
    asked.step(&AgentStatus::Working, 1.0);
    asked.step(&AgentStatus::Done, 5.0);
    assert_eq!(asked.step(&AgentStatus::Working, 5.5), Step::Wait(None));
    assert_eq!(asked.step(&AgentStatus::Done, 9.0), Step::Wait(Some(10.5)));
}

#[test]
fn an_agent_that_never_starts_or_dies_drops_the_wait() {
    let mut idle = Asked::new(4, None, 0.0);
    assert_eq!(idle.step(&AgentStatus::Done, 14.9), Step::Wait(Some(15.0)));
    assert_eq!(idle.step(&AgentStatus::Done, 15.0), Step::Dropped(Dropped::NeverStarted));
    let mut dead = Asked::new(4, None, 0.0);
    dead.step(&AgentStatus::Working, 1.0);
    assert_eq!(dead.step(&AgentStatus::Failed("x".into()), 2.0), Step::Dropped(Dropped::Failed));
    assert_eq!(dead.step(&AgentStatus::Shell, 3.0), Step::Dropped(Dropped::Failed));
}

#[test]
fn the_fallback_reads_only_the_agents_messages_after_the_prompt() {
    let example = "```toml\nname = \"in the prompt\"\n```";
    let rows = [
        row(1, Kind::Assistant, "```toml\nname = \"before the prompt\"\n```"),
        row(2, Kind::User, example),
        row(3, Kind::Assistant, "```toml\nname = \"first\"\n```"),
        row(4, Kind::Assistant, "no block here"),
        row(5, Kind::Assistant, "```toml\nname = \"last\"\n```\nthanks"),
        row(6, Kind::User, example),
    ];
    assert_eq!(toml_after(rows.iter(), Some(2)).as_deref(), Some("name = \"last\""));
    assert_eq!(toml_after(rows.iter(), Some(5)), None);
    assert_eq!(toml_after(rows[..4].iter(), Some(2)).as_deref(), Some("name = \"first\""));
    assert_eq!(toml_after(rows[..2].iter(), Some(2)), None);
    assert_eq!(toml_after(rows[..2].iter(), None).as_deref(), Some("name = \"before the prompt\""));
}

#[test]
fn a_toml_reply_becomes_a_workflow_on_the_tabs_agent() {
    let text = "name = \"Reply\"\n[[stage]]\naction = \"work\"\nprompt = \"{task}\"\n[[stage]]\naction = \"review\"\non_fail = 1\n";
    let workflow = workflow_from_toml(text, "gemini").unwrap();
    assert_eq!(workflow.name, "Reply");
    assert!(workflow.stages.iter().all(|s| s.run.agent.as_deref() == Some("gemini")));
}

#[test]
fn a_toml_reply_that_breaks_the_rules_gives_its_first_problem() {
    let bad = "name = \"\"\n[[stage]]\naction = \"work\"\nmax_loops = 0\n";
    assert_eq!(workflow_from_toml(bad, "claude").unwrap_err(), "the workflow needs a name");
    assert_eq!(
        workflow_from_toml("[[stage]]\naction = 5\n", "claude").unwrap_err().split(':').next(),
        Some("the toml block is not valid")
    );
    assert_eq!(first(Vec::new()), "the workflow is not valid");
}

#[test]
fn the_dropped_wait_names_the_tab() {
    assert_eq!(
        Dropped::NoWorkflow.text("calm-comet"),
        "calm-comet finished without sending a workflow"
    );
    assert_eq!(Dropped::NeverStarted.text("calm-comet"), "calm-comet did not start on the request");
    assert_eq!(Dropped::Failed.text("calm-comet"), "calm-comet stopped before it sent a workflow");
}
