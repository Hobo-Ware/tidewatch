use super::*;
use tidewatch_core::pr_status::Seen;

fn job(name: &str, passed: bool) -> Job {
    Job {
        name: name.into(),
        passed,
        step: (!passed).then(|| "Run cargo clippy --workspace -- -D warnings".to_owned()),
        secs: Some(if passed { 131 } else { 108 }),
        url: format!("https://github.com/acme/web/actions/runs/7/job/{}", name.len()),
        run_id: Some(7),
        job_id: Some(8),
        log: if passed { Vec::new() } else { vec!["error: unused import".into()] },
    }
}

fn failed() -> Body {
    Body::ChecksFailed {
        head: "h".into(),
        jobs: vec![job("check (macos-14)", true), job("check (ubuntu-latest)", false)],
    }
}

fn cards_of(bodies: Vec<Body>) -> Cards {
    let mut cards = Cards::default();
    cards.follow(42);
    for (n, body) in bodies.into_iter().enumerate() {
        cards.add(n as i64 * 10, body);
    }
    cards
}

static UNCHECKED: Cleanup = Cleanup::Unchecked;

fn context(cards: &Cards, mode: PrMode) -> Context<'_> {
    Context {
        cleanup: &UNCHECKED,
        cards,
        profile: "Builder",
        mode,
        url: "https://github.com/acme/web/pull/42",
        checks_green: true,
    }
}

fn card(bodies: Vec<Body>, mode: PrMode) -> Model {
    let last = bodies.len() - 1;
    let cards = cards_of(bodies);
    match look(last, context(&cards, mode)).unwrap() {
        Look::Card(model) => model,
        Look::Bubble(bubble) => panic!("a bubble: {bubble:?}"),
    }
}

fn labels(model: &Model) -> Vec<(&str, bool)> {
    model.buttons.iter().map(|b| (b.label.as_str(), b.primary)).collect()
}

#[test]
fn a_failed_check_lists_its_jobs_the_log_and_three_buttons() {
    let model = card(vec![failed()], PrMode::Ask);
    assert_eq!((model.icon, model.ink, model.red_line), (Icon::Failed, Ink::Red, true));
    assert_eq!((model.title.as_str(), model.sub.as_str()), ("Checks failed", "1 of 2 failed"));
    assert_eq!(
        model.body,
        [
            Block::Jobs(vec![
                JobLine {
                    passed: true,
                    name: "check (macos-14)".into(),
                    at: None,
                    took: Some("2m 11s".into())
                },
                JobLine {
                    passed: false,
                    name: "check (ubuntu-latest)".into(),
                    at: Some("clippy".into()),
                    took: Some("1m 48s".into()),
                },
            ]),
            Block::Log(vec!["error: unused import".into()]),
        ]
    );
    assert_eq!(
        labels(&model),
        [("Ask Builder to fix", true), ("Full log", false), ("Re-run", false)]
    );
    assert_eq!(model.buttons[2].action, CardAction::Rerun(7));
    assert!(
        matches!(&model.buttons[1].action, CardAction::Open(url) if url.contains("/actions/runs/7/"))
    );
    assert_eq!(model.note.as_deref(), Some("Auto-fix is off for this tab"));
}

#[test]
fn the_quiet_line_follows_the_tabs_mode() {
    for (mode, want) in [
        (PrMode::Ask, "Auto-fix is off for this tab"),
        (PrMode::Off, "Auto-fix is off for this tab"),
        (PrMode::Auto, "Auto-fix is on for this tab"),
    ] {
        assert_eq!(card(vec![failed()], mode).note.as_deref(), Some(want), "{mode:?}");
    }
}

#[test]
fn a_failure_the_story_moved_past_has_no_buttons() {
    let cards =
        cards_of(vec![failed(), Body::ChecksRunning { head: "h2".into(), done: 0, total: 2 }]);
    let Some(Look::Card(old)) = look(0, context(&cards, PrMode::Ask)) else { panic!() };
    assert_eq!(old.buttons, []);
    assert_eq!(old.note, None);
    assert_ne!(old.body, [], "the card itself stays");
}

#[test]
fn running_checks_are_live_with_a_bar_each() {
    let model =
        card(vec![Body::ChecksRunning { head: "h".into(), done: 1, total: 20 }], PrMode::Ask);
    assert!(model.live);
    assert_eq!(model.sub, "1 of 20 done");
    assert_eq!(model.body, [Block::Bars { done: 1, total: MOST_BARS }]);
    assert_eq!(model.buttons, []);
}

#[test]
fn changes_requested_list_each_comment_and_offer_the_notes() {
    let body = Body::ChangesRequested {
        author: "reviewer".into(),
        body: String::new(),
        comments: vec![
            ReviewComment {
                path: "crates/core/src/runner.rs".into(),
                line: Some(212),
                text: "Move it?".into(),
            },
            ReviewComment { path: "lib.rs".into(), line: None, text: "Whole file.".into() },
        ],
    };
    let model = card(vec![body], PrMode::Ask);
    assert_eq!(model.sub, "by @reviewer \u{b7} 2 comments");
    assert_eq!(
        model.body,
        [Block::Comments(vec![
            ("runner.rs:212".into(), "Move it?".into()),
            ("lib.rs".into(), "Whole file.".into())
        ])]
    );
    assert_eq!(labels(&model), [("Ask Builder to address", true), ("Open as notes", false)]);
    assert_eq!(model.buttons[1].action, CardAction::OpenNotes);
}

#[test]
fn a_comment_can_be_answered_by_the_agent_or_in_the_browser() {
    let own = Body::Commented {
        author: "rev".into(),
        text: " Does it work? ".into(),
        url: "https://github.com/acme/web/pull/42#c1".into(),
    };
    let model = card(vec![own], PrMode::Ask);
    assert_eq!(model.icon, Icon::Comment('R'));
    assert_eq!(model.title, "@rev commented");
    assert_eq!(model.body, [Block::Text("Does it work?".into())]);
    assert_eq!(labels(&model), [("Ask Builder to answer", true), ("Reply myself", false)]);
    assert_eq!(
        model.buttons[1].action,
        CardAction::Open("https://github.com/acme/web/pull/42#c1".into())
    );
    let bare = Body::Commented { author: "rev".into(), text: "x".into(), url: String::new() };
    let model = card(vec![bare], PrMode::Ask);
    assert_eq!(
        model.buttons[1].action,
        CardAction::Open("https://github.com/acme/web/pull/42".into())
    );
}

#[test]
fn a_conflict_offers_one_button_while_it_lasts() {
    let mut cards = cards_of(vec![Body::Conflict { base: "main".into() }]);
    let Some(Look::Card(gone)) = look(0, context(&cards, PrMode::Ask)) else { panic!() };
    assert_eq!(gone.buttons, [], "nothing says it still conflicts");
    cards.seen = Some(Seen { conflicting: true, ..Seen::default() });
    let Some(Look::Card(now)) = look(0, context(&cards, PrMode::Ask)) else { panic!() };
    assert_eq!(labels(&now), [("Ask Builder to rebase", true)]);
    assert_eq!(now.title, "Conflicts with main");
}

#[test]
fn approval_only_opens_the_browser_and_says_merging_stays_yours() {
    let model = card(vec![Body::Approved { author: "rev".into() }], PrMode::Ask);
    assert_eq!(model.sub, "by @rev \u{b7} checks green");
    assert_eq!(labels(&model), [("Merge on GitHub", false)]);
    assert_eq!(
        model.buttons[0].action,
        CardAction::Open("https://github.com/acme/web/pull/42".into())
    );
    assert_eq!(model.note.as_deref(), Some("Merging stays yours"));
}

fn merged_with(cleanup: &Cleanup) -> (Cards, Model) {
    let cards = cards_of(vec![Body::Merged { number: 42, base: "main".into() }]);
    let context = Context { cleanup, ..context(&cards, PrMode::Ask) };
    let Some(Look::Card(model)) = look(0, context) else { panic!() };
    (cards, model)
}

fn report(ignored: &[&str]) -> tidewatch_core::worktree_cleanup::Report {
    tidewatch_core::worktree_cleanup::Report {
        local_commits: Some(0),
        ignored: ignored.iter().map(ToString::to_string).collect(),
        ..Default::default()
    }
}

#[test]
fn a_merged_card_waits_for_the_check_with_only_keep_it() {
    for cleanup in [Cleanup::Unchecked, Cleanup::Checking] {
        let (_, model) = merged_with(&cleanup);
        assert_eq!(model.title, "Merged into main");
        assert_eq!(labels(&model), [("Keep it", false)]);
    }
}

#[test]
fn a_safe_merged_worktree_offers_to_remove_it_and_lists_what_ignored_files_would_go() {
    let (_, plain) = merged_with(&Cleanup::Checked(report(&[])));
    assert_eq!(labels(&plain), [("Remove worktree and branch", true), ("Keep it", false)]);
    assert_eq!(plain.buttons[0].action, CardAction::Cleanup);
    assert_eq!(plain.body, [Block::Text("PR merged. Remove worktree and branch?".into())]);
    let (_, lost) = merged_with(&Cleanup::Checked(report(&["scratch.log", ".env.local"])));
    assert_eq!(lost.body[2], Block::Log(vec!["scratch.log".to_owned(), ".env.local".to_owned()]));
}

#[test]
fn a_merged_worktree_with_unsaved_work_is_not_offered_for_removal() {
    let blocked = tidewatch_core::worktree_cleanup::Report {
        untracked: vec!["idea.md".into()],
        local_commits: Some(0),
        ..Default::default()
    };
    let (_, model) = merged_with(&Cleanup::Checked(blocked));
    assert_eq!(labels(&model), [("Close tab and clean up", false), ("Keep it", false)]);
    assert_eq!(model.buttons[0].action, CardAction::CloseTab);
    let Block::Log(lines) = &model.body[1] else { panic!("{:?}", model.body) };
    assert_eq!(lines, &["1 untracked file", "  untracked  idea.md"]);
    let (_, failed) = merged_with(&Cleanup::Failed("no such folder".into()));
    assert_eq!(labels(&failed), [("Close tab and clean up", false), ("Keep it", false)]);
}

#[test]
fn keep_it_hides_the_merged_buttons() {
    let (mut cards, model) = merged_with(&Cleanup::Checked(report(&[])));
    let CardAction::KeepIt(id) = model.buttons[1].action else { panic!() };
    cards.handle(id);
    let context =
        Context { cleanup: &Cleanup::Checked(report(&[])), ..context(&cards, PrMode::Ask) };
    let Some(Look::Card(kept)) = look(0, context) else { panic!() };
    assert_eq!(kept.buttons, []);
}

#[test]
fn closed_can_only_be_reopened_on_github() {
    let model = card(vec![Body::Closed { number: 42 }], PrMode::Ask);
    assert_eq!(labels(&model), [("Reopen on GitHub", false)]);
}

#[test]
fn a_push_fix_offer_names_the_commits_and_pushes_through_the_delivery_flow() {
    let model = card(vec![Body::PushFix { commits: 2, head: "abc".into() }], PrMode::Ask);
    assert_eq!(model.title, "Builder committed 2 new commits");
    assert_eq!(labels(&model), [("Push fix", true), ("Not now", false)]);
    let one = card(vec![Body::PushFix { commits: 1, head: "abc".into() }], PrMode::Ask);
    assert_eq!(one.title, "Builder committed 1 new commit");
}

#[test]
fn a_sent_message_is_a_bubble_labelled_by_who_started_it() {
    let sent = |via, tag: &str, extra| Body::Sent {
        via,
        tag: tag.into(),
        headline: "check failed at clippy on PR #42.".into(),
        extra_lines: extra,
        text: "t".into(),
    };
    let cards = cards_of(vec![sent(Via::You, "CI failure", 12), sent(Via::Auto, "Review", 0)]);
    let Some(Look::Bubble(you)) = look(0, context(&cards, PrMode::Ask)) else { panic!() };
    assert_eq!((you.who, you.tag.as_str()), ("You, via Tidewatch", "CI failure"));
    assert_eq!(you.more.as_deref(), Some("+ 12 log lines"));
    let Some(Look::Bubble(auto)) = look(1, context(&cards, PrMode::Ask)) else { panic!() };
    assert_eq!(auto.who, "Tidewatch, auto-fix");
    assert_eq!(auto.more, None);
    assert_eq!(look(9, context(&cards, PrMode::Ask)), None);
}

#[test]
fn opened_names_the_branches_and_the_commits() {
    let body = Body::Opened {
        number: 42,
        title: "t".into(),
        base: "main".into(),
        branch: "agent/lively-lantern".into(),
        commits: Some(3),
    };
    let model = card(vec![body], PrMode::Ask);
    assert_eq!(model.title, "Pull request #42 opened");
    assert_eq!(model.sub, "by Tidewatch \u{b7} 3 commits pushed");
    assert_eq!(model.body, [Block::Mono("agent/lively-lantern \u{2192} main".into())]);
}

#[test]
fn durations_and_ages_read_short() {
    let durations = [
        (0, "0s"),
        (48, "48s"),
        (59, "59s"),
        (60, "1m 00s"),
        (182, "3m 02s"),
        (3_780, "1h 03m"),
        (-5, "0s"),
    ];
    for (secs, want) in durations {
        assert_eq!(duration(secs), want, "{secs}");
    }
    let ages = [
        (0, "just now"),
        (44_000, "just now"),
        (46_000, "1m ago"),
        (240_000, "4m ago"),
        (7_200_000, "2h ago"),
        (200_000_000, "2d ago"),
        (-5_000, "just now"),
    ];
    for (ms, want) in ages {
        assert_eq!(age(ms, 0), want, "{ms}");
    }
}

#[test]
fn only_web_addresses_open() {
    for (url, want) in [
        ("https://github.com/acme/web/pull/42", true),
        ("http://example.test/x", true),
        ("file:///etc/passwd", false),
        ("javascript:alert(1)", false),
        ("https://", false),
        ("https://a b", false),
        ("", false),
    ] {
        assert_eq!(is_web_url(url), want, "{url:?}");
    }
}

#[test]
fn the_waiting_and_paused_rows_use_the_profile_and_the_reason() {
    let message = tidewatch_core::pr_prompt::message(&failed(), 42).unwrap();
    let pending = Pending { id: 5, message };
    assert_eq!(
        waiting_text("Builder", &pending),
        "Will ask Builder to fix CI when it finishes this turn"
    );
    let [send, cancel] = waiting_buttons(&pending);
    assert_eq!((send.action, cancel.action), (CardAction::SendNow(5), CardAction::Cancel(5)));
    assert_eq!(
        paused_text(&Pause::SignedOut),
        ("PR status paused: gh is not signed in".to_owned(), Some("gh auth login"))
    );
}
