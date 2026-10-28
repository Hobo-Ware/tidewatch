use super::*;

fn card(id: &str, name: &str, agent: &str, level: Level) -> Card {
    Card {
        id: id.to_owned(),
        name: name.to_owned(),
        detail: "opus \u{b7} high".to_owned(),
        agent_id: agent.to_owned(),
        cli_name: if agent == "claude" { "Claude" } else { "Codex" }.to_owned(),
        unsupported: Vec::new(),
        installed: true,
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
        goes: Goes::Stays,
        busy: false,
        meeseeks: false,
        pinned: false,
        replaces: None,
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
fn a_session_at_yolo_opens_at_yolo_in_a_worktree_and_outside_one() {
    let f = Facts { seen_level: Some(Level::Yolo), in_worktree: false, ..facts(claude(false)) };
    assert_eq!(Pick::start(&f, &builder_of), pick(Kind::Agent, "builder", Level::Yolo));
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
fn every_profile_and_every_level_can_be_picked() {
    let f = Facts { in_worktree: false, ..facts(claude(false)) };
    let mut p = pick(Kind::Agent, "builder", Level::Edits);
    p.pick_profile(&f, "cdx");
    assert_eq!(p.profile, "cdx", "another CLI is allowed: it starts a new conversation");
    p.pick_level(Level::Yolo);
    assert_eq!(p.level, Level::Yolo, "YOLO outside a worktree is picked as it is");
    p.pick_kind(Kind::Shell);
    assert_eq!(p.kind, Kind::Shell);
    p.pick_profile(&f, "nope");
    assert_eq!(p.profile, "cdx");
}

#[test]
fn yolo_is_never_fitted_but_a_level_the_cli_has_no_flags_for_is_and_the_sentence_says_so() {
    let mut f = Facts { in_worktree: false, ..facts(Running::No) };
    let yolo = pick(Kind::Agent, "builder", Level::Yolo);
    assert!(!sentence(&f, &yolo).contains("runs as"), "{}", sentence(&f, &yolo));
    f.cards[0].unsupported = vec![Level::Plan];
    let plan = pick(Kind::Agent, "builder", Level::Plan);
    assert!(
        sentence(&f, &plan).contains("Claude has no flags for Read only, so it runs as Ask."),
        "{}",
        sentence(&f, &plan)
    );
}

#[test]
fn the_start_level_falls_back_only_when_the_profile_has_no_flags_for_it() {
    let mut g = Facts { in_worktree: false, ..facts(Running::No) };
    g.cards[0].level = Level::Plan;
    g.cards[0].unsupported = vec![Level::Plan];
    assert_eq!(Pick::start(&g, &builder_of).level, Level::Ask);
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
fn a_missing_cli_or_profile_is_a_fix_not_a_refusal() {
    let mut f = facts(Running::No);
    f.cards[2].installed = false;
    let p = pick(Kind::Agent, "cdx", Level::Edits);
    let missing = fix(&f, &p).unwrap();
    assert_eq!(missing, Fix::Install("Codex".into()));
    assert_eq!(missing.button(), "Set up Codex");
    assert_eq!(
        sentence(&f, &p),
        "Codex is not installed here. Set it up and this tab switches over."
    );
    assert_eq!(change(&f, &p, false), None, "nothing starts until it is set up");
    assert_eq!(fix(&f, &pick(Kind::Agent, "builder", Level::Edits)), None);
    assert_eq!(fix(&f, &pick(Kind::Shell, "cdx", Level::Edits)), None);
    let none = Facts { cards: Vec::new(), ..facts(Running::No) };
    let nothing = pick(Kind::Agent, "", Level::Edits);
    assert_eq!(fix(&none, &nothing), Some(Fix::AddProfile));
    assert_eq!(Pick::start(&none, &builder_of).profile, "");
}

#[test]
fn the_start_pick_prefers_a_profile_whose_cli_is_installed() {
    let mut f = facts(Running::No);
    f.cards[0].installed = false;
    f.cards[1].installed = false;
    assert_eq!(Pick::start(&f, &|_| None).profile, "cdx");
}

#[test]
fn a_change_that_is_already_so_runs_again_and_a_plain_shell_restarts() {
    let f = Facts { current: Some(("builder".into(), Level::Edits)), ..facts(claude(false)) };
    let same = pick(Kind::Agent, "builder", Level::Edits);
    assert!(change(&f, &same, false).is_some());
    let plain = facts(Running::No);
    let shell = pick(Kind::Shell, "builder", Level::Edits);
    assert!(change(&plain, &shell, false).is_some());
    assert!(sentence(&plain, &shell).starts_with("The shell restarts in the same folder."));
}

#[test]
fn another_cli_starts_a_new_conversation_and_says_so() {
    let p = pick(Kind::Agent, "cdx", Level::Edits);
    let working = sentence(&facts(claude(true)), &p);
    assert!(
        working.starts_with("Claude finishes this turn, then stops. Codex cannot continue its conversation, so it starts a new one as Codex builder \u{b7} Auto edit. The old one stays on disk."),
        "{working}"
    );
    let idle = sentence(&facts(claude(false)), &p);
    assert!(idle.starts_with("Claude stops. Codex cannot continue its conversation"), "{idle}");
    let stored = Facts { stored: Some(("claude".into(), "Claude".into())), ..facts(Running::No) };
    assert!(
        sentence(&stored, &p)
            .starts_with("Codex cannot continue the conversation Claude ended in this tab")
    );
}

#[test]
fn the_sentence_names_where_the_tab_goes_and_what_else_changes() {
    let p = pick(Kind::Agent, "builder", Level::Edits);
    let f = Facts {
        goes: Goes::Follows("~/.tidewatch/web/upbeat-lantern".into()),
        busy: true,
        meeseeks: true,
        pinned: true,
        replaces: Some("The form in this tab is discarded."),
        ..facts(claude(false))
    };
    let text = sentence(&f, &p);
    for part in [
        "The tab moves to ~/.tidewatch/web/upbeat-lantern, where the conversation runs.",
        "The command running in this shell stops.",
        "It stops being a Meeseeks and no longer ends by itself.",
        "The main tab stays pinned.",
        "The form in this tab is discarded.",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    assert!(!text.contains("stay as they are"), "{text}");
}

#[test]
fn stop_now_is_offered_only_while_the_agent_works_and_change_can_go_ahead() {
    let p = pick(Kind::Agent, "reviewer", Level::Plan);
    assert!(offers_stop_now(&facts(claude(true)), &p));
    assert!(!offers_stop_now(&facts(claude(false)), &p));
    let mut missing = facts(claude(true));
    missing.cards[1].installed = false;
    assert!(!offers_stop_now(&missing, &p), "a missing CLI is set up first");
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
    assert!(change(&facts(Running::No), &shell, false).is_some());
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

fn new_branch(left: Option<(usize, &str)>) -> Goes {
    Goes::NewBranch {
        repo: "tidewatch".into(),
        branch: Some("calm-comet".into()),
        left: left.map(|(count, folder)| (count, folder.to_owned())),
    }
}

#[test]
fn the_sentence_names_the_new_branch_and_says_what_stays_behind_only_when_something_does() {
    let p = pick(Kind::Agent, "builder", Level::Yolo);
    let on = "on a new branch agent/calm-comet in tidewatch.";
    let tab = |goes| Facts { in_worktree: false, goes, ..facts(claude(false)) };
    assert_eq!(
        sentence(&tab(new_branch(None)), &p),
        format!("Claude stops, then the same conversation resumes as Builder \u{b7} YOLO {on}")
    );
    let working = Facts { goes: new_branch(None), ..facts(claude(true)) };
    assert!(sentence(&working, &p).contains(&format!("YOLO {on}")));
    assert_eq!(
        sentence(&tab(new_branch(Some((3, "~/Git/ws/repo-b")))), &p),
        format!(
            "Claude stops, then the same conversation resumes as Builder \u{b7} YOLO {on} 3 uncommitted files stay in ~/Git/ws/repo-b."
        )
    );
    assert!(
        sentence(&tab(new_branch(Some((1, "~/ws")))), &p)
            .ends_with("1 uncommitted file stays in ~/ws.")
    );
    assert!(!sentence(&tab(new_branch(None)), &p).contains("uncommitted"));
    let stored = Facts {
        stored: Some(("claude".into(), "Claude".into())),
        goes: new_branch(None),
        ..facts(Running::No)
    };
    assert!(
        sentence(&stored, &p)
            .starts_with("The conversation Claude ended in this tab resumes as Builder")
    );
    assert!(sentence(&stored, &p).contains(on));
    let fresh = Facts { goes: new_branch(None), ..facts(Running::No) };
    assert!(sentence(&fresh, &p).contains(&format!("YOLO {on}")));
}

#[test]
fn the_sentence_says_nothing_of_a_branch_for_a_shell_or_an_unnamed_one_and_the_main_tab_keeps_its_shell()
 {
    let shell = pick(Kind::Shell, "builder", Level::Yolo);
    let f = Facts { goes: new_branch(None), ..facts(claude(false)) };
    assert!(!sentence(&f, &shell).contains("new branch"));
    let p = pick(Kind::Agent, "builder", Level::Yolo);
    let unnamed = Facts {
        goes: Goes::NewBranch { repo: "tidewatch".into(), branch: None, left: None },
        pinned: true,
        ..facts(claude(false))
    };
    let text = sentence(&unnamed, &p);
    assert!(text.contains("on a new branch agent/ in tidewatch."), "{text}");
    assert!(
        text.contains("The agent opens in a new tab and the main tab keeps its shell."),
        "{text}"
    );
    assert!(!text.contains("stays pinned"), "{text}");
}
