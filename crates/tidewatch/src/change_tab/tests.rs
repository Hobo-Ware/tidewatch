use tidewatch_core::meeseeks::{Criteria, Meeseeks};

use super::*;
use crate::agent_run::AgentRun;
use crate::pane::{AgentTab, Launched};

fn card(id: &str, name: &str, agent: &str, level: Level) -> Card {
    Card {
        id: id.to_owned(),
        name: name.to_owned(),
        detail: "opus \u{b7} high".to_owned(),
        agent_id: agent.to_owned(),
        cli_name: if agent == "claude" { "Claude" } else { "Codex" }.to_owned(),
        unsupported: Vec::new(),
        below_yolo: Some(Level::Auto),
        level,
    }
}

fn facts(running: Running) -> Facts {
    Facts {
        tab: 4,
        in_worktree: true,
        running,
        stored: None,
        current: None,
        seen_level: None,
        cards: vec![
            card("builder", "Builder", "claude", Level::Edits),
            card("reviewer", "Reviewer", "claude", Level::Plan),
            card("cdx", "Codex builder", "codex", Level::Edits),
        ],
    }
}

fn claude(working: bool) -> Running {
    Running::Agent { agent_id: "claude".into(), cli_name: "Claude".into(), working }
}

fn builder_of(cli: &str) -> Option<String> {
    (cli == "claude").then(|| "builder".to_owned())
}

fn pick(kind: Kind, profile: &str, level: Level) -> Pick {
    Pick { kind, profile: profile.to_owned(), level }
}

#[test]
fn an_agent_tab_opens_on_its_own_profile_and_level() {
    let f = Facts { current: Some(("reviewer".into(), Level::Auto)), ..facts(claude(false)) };
    assert_eq!(Pick::start(&f, &builder_of), pick(Kind::Agent, "reviewer", Level::Auto));
}

#[test]
fn a_shell_with_a_found_session_opens_on_builder_at_the_level_it_runs_at() {
    let f = Facts { seen_level: Some(Level::Auto), ..facts(claude(false)) };
    assert_eq!(Pick::start(&f, &builder_of), pick(Kind::Agent, "builder", Level::Auto));
}

#[test]
fn a_session_at_yolo_outside_a_worktree_opens_one_level_below() {
    let f = Facts { seen_level: Some(Level::Yolo), in_worktree: false, ..facts(claude(false)) };
    assert_eq!(Pick::start(&f, &builder_of), pick(Kind::Agent, "builder", Level::Auto));
    let inside = Facts { seen_level: Some(Level::Yolo), ..facts(claude(false)) };
    assert_eq!(Pick::start(&inside, &builder_of).level, Level::Yolo);
}

#[test]
fn a_plain_shell_opens_as_a_shell_and_one_that_ended_a_session_opens_on_agent() {
    let plain = facts(Running::No);
    assert_eq!(Pick::start(&plain, &builder_of).kind, Kind::Shell);
    let stored = Facts { stored: Some(("claude".into(), "Claude".into())), ..facts(Running::No) };
    let start = Pick::start(&stored, &builder_of);
    assert_eq!((start.kind, start.profile.as_str()), (Kind::Agent, "builder"));
}

#[test]
fn profiles_of_another_cli_are_refused_for_a_conversation_and_open_for_a_new_session() {
    let f = facts(claude(false));
    let other = &f.cards[2];
    assert!(card_refusal(&f, other).unwrap().contains("another CLI"));
    assert_eq!(card_refusal(&f, &f.cards[0]), None);
    let mut p = pick(Kind::Agent, "builder", Level::Edits);
    p.pick_profile(&f, "cdx");
    assert_eq!(p.profile, "builder", "a refused card is not picked");
    let fresh = facts(Running::No);
    assert_eq!(card_refusal(&fresh, &fresh.cards[2]), None);
}

#[test]
fn yolo_says_why_it_is_refused_outside_a_worktree() {
    let f = Facts { in_worktree: false, ..facts(claude(false)) };
    let reason = level_refusal(&f, &f.cards[0], Level::Yolo).unwrap();
    assert_eq!(reason, "YOLO only runs in a worktree tab.");
    assert_eq!(level_refusal(&f, &f.cards[0], Level::Auto), None);
    let mut p = pick(Kind::Agent, "builder", Level::Edits);
    p.pick_level(&f, Level::Yolo);
    assert_eq!(p.level, Level::Edits);
    let inside = facts(claude(false));
    p.pick_level(&inside, Level::Yolo);
    assert_eq!(p.level, Level::Yolo);
}

#[test]
fn a_level_the_cli_has_no_flags_for_is_refused() {
    let mut f = facts(claude(false));
    f.cards[0].unsupported = vec![Level::Plan];
    assert!(level_refusal(&f, &f.cards[0], Level::Plan).unwrap().contains("no flags"));
}

#[test]
fn picking_a_profile_that_cannot_run_the_current_level_moves_to_one_it_can() {
    let mut f = Facts { in_worktree: false, ..facts(Running::No) };
    f.cards[1].unsupported = vec![Level::Ask];
    let mut p = pick(Kind::Agent, "builder", Level::Ask);
    p.pick_profile(&f, "reviewer");
    assert_eq!((p.profile.as_str(), p.level), ("reviewer", Level::Plan));
}

#[test]
fn the_sentence_says_what_will_happen_in_each_case() {
    let keeps = "Worktree, branch and history stay as they are.";
    let who = "Builder \u{b7} YOLO";
    let p = pick(Kind::Agent, "builder", Level::Yolo);
    let cases = [
        (
            facts(claude(true)),
            format!(
                "Claude finishes this turn, then the same conversation resumes as {who}. {keeps}"
            ),
        ),
        (
            facts(claude(false)),
            format!("Claude stops, then the same conversation resumes as {who}. {keeps}"),
        ),
        (
            Facts { stored: Some(("claude".into(), "Claude".into())), ..facts(Running::No) },
            format!("The conversation Claude ended in this tab resumes as {who}. {keeps}"),
        ),
        (facts(Running::No), format!("A new Claude session starts in this tab as {who}. {keeps}")),
        (
            Facts { in_worktree: false, ..facts(Running::No) },
            format!(
                "A new Claude session starts in this tab as {who}. The folder and history stay as they are."
            ),
        ),
    ];
    for (f, want) in cases {
        assert_eq!(sentence(&f, &p), want);
    }
    let shell = pick(Kind::Shell, "builder", Level::Edits);
    assert!(
        sentence(&facts(claude(false)), &shell)
            .starts_with("Claude stops and this tab keeps its shell.")
    );
}

#[test]
fn change_is_blocked_for_what_is_already_so_and_for_a_shell_with_nothing_to_stop() {
    let f = Facts { current: Some(("builder".into(), Level::Edits)), ..facts(claude(false)) };
    assert_eq!(
        blocked(&f, &pick(Kind::Agent, "builder", Level::Edits)).as_deref(),
        Some("Already running as Builder \u{b7} Auto edit.")
    );
    assert_eq!(blocked(&f, &pick(Kind::Agent, "builder", Level::Auto)), None);
    assert_eq!(blocked(&f, &pick(Kind::Agent, "reviewer", Level::Edits)), None);
    assert_eq!(blocked(&f, &pick(Kind::Shell, "builder", Level::Edits)), None);
    let plain = facts(Running::No);
    assert_eq!(
        blocked(&plain, &pick(Kind::Shell, "builder", Level::Edits)).as_deref(),
        Some("This tab is already a shell.")
    );
    assert_eq!(kind_refusal(&plain, Kind::Shell), Some("No agent runs in this tab."));
    assert_eq!(kind_refusal(&f, Kind::Shell), None);
}

#[test]
fn stop_now_is_offered_only_while_the_agent_works_and_change_can_go_ahead() {
    let p = pick(Kind::Agent, "reviewer", Level::Plan);
    assert!(offers_stop_now(&facts(claude(true)), &p));
    assert!(!offers_stop_now(&facts(claude(false)), &p));
    let same = Facts { current: Some(("reviewer".into(), Level::Plan)), ..facts(claude(true)) };
    assert!(!offers_stop_now(&same, &p));
}

#[test]
fn change_hands_the_picked_target_and_whether_to_interrupt() {
    let f = facts(claude(true));
    let p = pick(Kind::Agent, "reviewer", Level::Plan);
    let want = Target::Agent { profile: "reviewer".into(), level: Some(Level::Plan) };
    assert_eq!(
        change(&f, &p, false),
        Some(Change { tab: 4, target: want.clone(), interrupt: false })
    );
    assert_eq!(change(&f, &p, true), Some(Change { tab: 4, target: want, interrupt: true }));
    let shell = pick(Kind::Shell, "reviewer", Level::Plan);
    assert_eq!(change(&f, &shell, false).map(|c| c.target), Some(Target::Shell));
    assert_eq!(change(&facts(Running::No), &shell, false), None);
}

fn agent_pane(special: Option<&str>) -> Pane<u32> {
    let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
    agent.launched = Some(Launched {
        profile_id: "builder".into(),
        level: Level::Edits,
        session_id: None,
        remote: false,
        dir: None,
    });
    match special {
        Some("meeseeks") => {
            agent.meeseeks = Some(Meeseeks::summon(
                "task",
                Criteria { tests: true, committed: true, pr: false },
                0,
            ));
        }
        Some("starting") => agent.launched = None,
        _ => {}
    }
    Pane { body: Body::Shell(0), worktree: None, agent: Some(agent) }
}

#[test]
fn which_tabs_can_change_and_why_the_others_cannot() {
    let creating: Pane<u32> = Pane { body: Body::Creating, worktree: None, agent: None };
    let cases = [
        (tab_kind(&Pane::shell(0u32), false), TabKind::Shell, true),
        (tab_kind(&Pane::shell(0u32), true), TabKind::Main, false),
        (tab_kind(&agent_pane(None), false), TabKind::Agent, true),
        (tab_kind(&agent_pane(Some("meeseeks")), false), TabKind::Meeseeks, false),
        (tab_kind(&agent_pane(Some("starting")), false), TabKind::Other, false),
        (tab_kind(&creating, false), TabKind::Other, false),
    ];
    for (got, want, ok) in cases {
        assert_eq!(got, want);
        assert_eq!(availability(got).is_ok(), ok, "{want:?}");
        assert_eq!(availability(got).err().map(str::is_empty), (!ok).then_some(false));
    }
}

fn named(id: &str, name: &str, agent: &str) -> Card {
    Card { id: id.to_owned(), name: name.to_owned(), ..card(id, name, agent, Level::Edits) }
}

fn ids(cards: &[Card]) -> Vec<&str> {
    cards.iter().map(|card| card.id.as_str()).collect()
}

#[test]
fn the_grid_lists_the_tabs_own_cli_first_then_the_others_in_settings_order() {
    let settings_order = vec![
        named("builder", "Builder", "claude"),
        named("codex-builder", "Codex Builder", "codex"),
        named("codex-reviewer", "Codex Reviewer", "codex"),
        named("copilot-builder", "Copilot CLI Builder", "copilot"),
        named("gemini-builder", "Gemini CLI Builder", "gemini"),
        named("mine", "Mine", "claude"),
        named("reviewer", "Reviewer", "claude"),
    ];
    let cases: [(Option<&str>, [&str; 7]); 4] = [
        (
            Some("claude"),
            [
                "builder",
                "reviewer",
                "mine",
                "codex-builder",
                "codex-reviewer",
                "copilot-builder",
                "gemini-builder",
            ],
        ),
        (
            Some("codex"),
            [
                "codex-builder",
                "codex-reviewer",
                "builder",
                "reviewer",
                "mine",
                "copilot-builder",
                "gemini-builder",
            ],
        ),
        (
            None,
            [
                "builder",
                "reviewer",
                "mine",
                "codex-builder",
                "codex-reviewer",
                "copilot-builder",
                "gemini-builder",
            ],
        ),
        (
            Some("gemini"),
            [
                "gemini-builder",
                "builder",
                "reviewer",
                "mine",
                "codex-builder",
                "codex-reviewer",
                "copilot-builder",
            ],
        ),
    ];
    for (own, want) in cases {
        let got = ordered_cards(&settings_order, own);
        assert_eq!(ids(&got), want, "{own:?}");
    }
}

#[test]
fn a_reviewer_the_user_made_for_a_cli_comes_before_that_clis_other_profiles() {
    let cards = vec![
        named("aaa", "Aaa", "claude"),
        named("zeta-reviewer", "Zeta Reviewer", "claude"),
        named("builder", "Builder", "claude"),
    ];
    assert_eq!(ids(&ordered_cards(&cards, Some("claude"))), ["builder", "zeta-reviewer", "aaa"]);
}
