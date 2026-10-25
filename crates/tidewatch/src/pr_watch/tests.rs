use super::*;
use tidewatch_core::pr_cards::{MAX_AUTO_SENDS, MAX_AUTONOMOUS_SENDS};
use tidewatch_core::pr_status::{Decision, Event, Job, ReviewComment, State, SummaryChecks};
use tidewatch_core::status::Reason;

fn auto() -> GithubPrefs {
    GithubPrefs { mode: PrMode::Auto, ..GithubPrefs::default() }
}

fn ready(default_number: Option<u64>) -> Watch {
    let mut watch = Watch::new(PathBuf::from("/state/pr_cards.json"));
    assert!(watch.take_load());
    watch.loaded(Cards::default(), 0.0);
    if let Some(number) = default_number {
        watch.learn(number, "head-1", 0.0);
    }
    watch
}

fn summary(checks: SummaryChecks) -> Summary {
    Summary {
        number: 42,
        title: "Retry a stage".into(),
        url: "https://github.com/acme/web/pull/42".into(),
        state: State::Open,
        checks,
        comments: 0,
        decision: Decision::None,
        conflicting: false,
    }
}

fn job(name: &str, passed: bool) -> Job {
    Job {
        name: name.into(),
        passed,
        step: Some("Run cargo clippy --workspace".into()),
        secs: None,
        url: String::new(),
        run_id: Some(1),
        job_id: Some(2),
        log: vec!["error: unused import".into()],
    }
}

fn failed() -> Body {
    Body::ChecksFailed { head: "h".into(), jobs: vec![job("check (ubuntu-latest)", false)] }
}

fn polled(bodies: Vec<Body>, pending: bool, ended: bool) -> Polled {
    Polled {
        events: bodies
            .into_iter()
            .enumerate()
            .map(|(n, body)| Event { at_ms: 1_000 + n as i64, body })
            .collect(),
        seen: Seen { opened: true, ..Seen::default() },
        summary: summary(SummaryChecks::Failed),
        head: "head-1".into(),
        checks_pending: pending,
        ended,
    }
}

fn pasted(outs: &[Out]) -> Vec<&str> {
    outs.iter()
        .filter_map(|out| match out {
            Out::Paste(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn sent_count(watch: &Watch) -> usize {
    watch.cards.cards().iter().filter(|c| matches!(c.body, Body::Sent { .. })).count()
}

#[test]
fn a_look_is_due_only_for_a_known_open_watched_pull_request_that_is_not_being_read() {
    let default = GithubPrefs::default();
    let mut watch = Watch::new(PathBuf::from("/x"));
    assert_eq!(watch.should_look(default, 99.0), None, "not loaded yet");
    watch.take_load();
    assert_eq!(watch.should_look(default, 99.0), None, "still loading");
    watch.loaded(Cards::default(), 10.0);
    assert_eq!(watch.should_look(default, 99.0), None, "no pull request known");
    assert!(watch.learn(42, "h", 10.0));
    assert_eq!(watch.should_look(default, 9.0), None, "not due yet");
    assert_eq!(watch.should_look(default, 10.0), Some(Look { number: 42, prev: None }));
    watch.look_started();
    assert_eq!(watch.should_look(default, 99.0), None, "already being read");
    let off = GithubPrefs { mode: PrMode::Off, ..default };
    watch.set_policy(off, 10.0);
    watch.in_flight = false;
    assert_eq!(watch.should_look(default, 99.0), None, "switched off for this tab");
    assert_eq!(watch.next_look(default), None);
}

#[test]
fn looks_come_every_20_seconds_while_checks_run_and_every_2_minutes_otherwise() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    for (pending, gap) in [(true, FAST_SECS), (false, SLOW_SECS)] {
        watch.look_started();
        watch.polled(Ok(polled(vec![], pending, false)), default, "tab", 100.0, 0);
        assert_eq!(watch.next_look(default), Some(100.0 + gap));
        assert_eq!(watch.should_look(default, 100.0 + gap - 0.1), None);
        assert!(watch.should_look(default, 100.0 + gap).is_some());
    }
}

#[test]
fn a_merged_pull_request_is_never_looked_at_again() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.look_started();
    let merged = Body::Merged { number: 42, base: "main".into() };
    let mut poll = polled(vec![merged], false, true);
    poll.summary.state = State::Merged;
    watch.polled(Ok(poll), default, "tab", 5.0, 0);
    assert_eq!(watch.should_look(default, 1e9), None);
    assert_eq!(watch.next_look(default), None);
    let mut cards = Cards::default();
    cards.follow(42);
    cards.seen = Some(Seen { ended: true, ..Seen::default() });
    let mut restarted = Watch::new(PathBuf::from("/x"));
    restarted.take_load();
    restarted.loaded(cards, 0.0);
    assert!(!restarted.learn(42, "h", 0.0));
    assert_eq!(restarted.should_look(default, 1e9), None, "it ended before the restart");
}

fn closed_poll() -> Polled {
    let mut poll = polled(vec![Body::Closed { number: 42 }], false, true);
    poll.summary.state = State::Closed;
    poll
}

#[test]
fn the_strip_shows_for_open_and_merged_pull_requests_and_not_for_a_closed_one() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    assert!(watch.strip_summary().is_none(), "nothing read yet");
    for (state, shown) in [(State::Open, true), (State::Merged, true), (State::Closed, false)] {
        let mut poll = polled(vec![], false, state != State::Open);
        poll.summary.state = state;
        watch.polled(Ok(poll), default, "tab", 5.0, 0);
        assert_eq!(watch.strip_summary().is_some(), shown, "{state:?}");
    }
}

#[test]
fn a_closed_pull_request_is_looked_at_again_so_a_reopen_brings_the_strip_back() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.look_started();
    watch.polled(Ok(closed_poll()), default, "tab", 5.0, 0);
    assert_eq!(watch.strip_summary(), None);
    assert_eq!(watch.next_look(default), Some(5.0 + SLOW_SECS));
    assert!(watch.should_look(default, 5.0 + SLOW_SECS).is_some());
    watch.look_started();
    watch.polled(Ok(polled(vec![], false, false)), default, "tab", 130.0, 0);
    assert_eq!(watch.strip_summary().map(|summary| summary.state), Some(State::Open));
}

#[test]
fn auto_fix_does_nothing_for_a_closed_pull_request() {
    let default = GithubPrefs { mode: PrMode::Auto, ..GithubPrefs::default() };
    let mut watch = ready(Some(42));
    watch.autonomous = true;
    watch.cards.add(1_000, Body::PushFix { commits: 1, head: "h".into() });
    assert!(watch.push_fix_due(default).is_some(), "open: the left-over commits go out");
    watch.look_started();
    watch.polled(Ok(closed_poll()), default, "tab", 5.0, 0);
    assert_eq!(watch.push_fix_due(default), None);
    watch.look_started();
    let mut poll = closed_poll();
    poll.events.push(Event { at_ms: 2_000, body: failed() });
    let outs = watch.polled(Ok(poll), default, "tab", 6.0, 0);
    assert_eq!(pasted(&outs).len(), 0);
    assert_eq!(watch.pending.len(), 0);
}

#[test]
fn a_failed_look_pauses_the_status_and_the_next_good_one_resumes_it() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.look_started();
    let outs = watch.polled(Err(GhError::NotAuthenticated), default, "tab", 50.0, 0);
    assert_eq!(outs, []);
    assert_eq!(watch.pause, Some(Pause::SignedOut));
    assert_eq!(watch.next_look(default), Some(50.0 + SLOW_SECS));
    watch.look_started();
    watch.polled(Ok(polled(vec![], false, false)), default, "tab", 200.0, 0);
    assert_eq!(watch.pause, None);
}

#[test]
fn pauses_say_why_and_how_to_fix_it() {
    let cases = [
        (GhError::NotAuthenticated, "gh is not signed in", Some("gh auth login")),
        (GhError::Missing, "gh is not installed", Some("brew install gh")),
        (GhError::NoGithubRemote, "this repository has no GitHub remote", None),
        (GhError::Failed("boom\nmore".into()), "gh failed: boom", None),
    ];
    for (error, reason, fix) in cases {
        let pause = Pause::of(error);
        assert_eq!((pause.reason().as_str(), pause.fix()), (reason, fix));
    }
}

#[test]
fn ask_me_shows_the_card_and_notifies_but_sends_nothing() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.observe(&AgentStatus::Done);
    watch.look_started();
    let outs =
        watch.polled(Ok(polled(vec![failed()], false, false)), default, "lantern", 5.0, 9_000);
    assert_eq!(pasted(&outs), Vec::<&str>::new());
    assert!(outs.contains(&Out::Save));
    let notice = outs
        .iter()
        .find_map(|o| match o {
            Out::Notify(n) => Some(n),
            _ => None,
        })
        .unwrap();
    assert_eq!(notice.what, "lantern: checks failed on #42");
    assert_eq!(notice.detail, "check (ubuntu-latest) failed at clippy");
    assert_eq!(notice.tone, Tone::Failed);
    assert_eq!(watch.cards.cards().len(), 1);
    assert_eq!(watch.pending, []);
    assert_eq!(watch.pill().map(|(_, text)| text), Some("#42 checks failed".to_owned()));
}

#[test]
fn auto_fix_sends_at_once_when_the_agent_is_idle() {
    let mut watch = ready(Some(42));
    watch.set_policy(auto(), 0.0);
    watch.observe(&AgentStatus::Done);
    watch.look_started();
    let outs = watch.polled(Ok(polled(vec![failed()], false, false)), auto(), "t", 5.0, 9_000);
    let text = pasted(&outs);
    assert_eq!(text.len(), 1);
    assert!(
        text[0].starts_with("check (ubuntu-latest) failed at clippy on PR #42."),
        "{}",
        text[0]
    );
    assert_eq!(watch.cards.auto_sends, 1);
    let Body::Sent { via, tag, .. } = &watch.cards.cards().last().unwrap().body else {
        panic!("no message card")
    };
    assert_eq!((*via, tag.as_str()), (Via::Auto, "CI failure"));
    assert_eq!(watch.pending, []);
    assert!(watch.cards.fixing);
}

#[test]
fn auto_fix_waits_for_a_busy_agent_and_never_pastes_into_one_that_waits_on_you() {
    let mut watch = ready(Some(42));
    watch.set_policy(auto(), 0.0);
    watch.observe(&AgentStatus::Working);
    watch.look_started();
    let outs = watch.polled(Ok(polled(vec![failed()], false, false)), auto(), "t", 5.0, 9_000);
    assert_eq!(pasted(&outs), Vec::<&str>::new());
    assert_eq!(watch.pending.len(), 1);
    assert_eq!(watch.pending[0].message.ask.phrase, "fix CI");
    watch.observe(&AgentStatus::NeedsYou(Reason::Permission));
    assert_eq!(pasted(&watch.flush(auto(), 6.0, 9_100)), Vec::<&str>::new());
    watch.observe(&AgentStatus::Done);
    let outs = watch.flush(auto(), 7.0, 9_200);
    assert_eq!(pasted(&outs).len(), 1);
    assert_eq!(watch.pending, []);
    assert_eq!(watch.cards.auto_sends, 1);
}

#[test]
fn one_idle_moment_sends_one_message_and_the_hold_keeps_the_next_for_later() {
    let mut watch = ready(Some(42));
    watch.set_policy(auto(), 0.0);
    watch.observe(&AgentStatus::Working);
    let two = vec![failed(), Body::Conflict { base: "main".into() }];
    watch.look_started();
    watch.polled(Ok(polled(two, false, false)), auto(), "t", 5.0, 9_000);
    assert_eq!(watch.pending.len(), 2);
    watch.observe(&AgentStatus::Done);
    assert_eq!(pasted(&watch.flush(auto(), 10.0, 9_100)).len(), 1);
    assert_eq!(
        pasted(&watch.flush(auto(), 10.1, 9_200)),
        Vec::<&str>::new(),
        "held until the agent works again"
    );
    watch.observe(&AgentStatus::Working);
    watch.observe(&AgentStatus::Done);
    assert_eq!(pasted(&watch.flush(auto(), 30.0, 9_300)).len(), 1);
    assert_eq!(watch.pending, []);
}

#[test]
fn a_trigger_switched_off_is_a_card_and_not_a_message() {
    let policy = GithubPrefs { mode: PrMode::Auto, reviews: false, ..GithubPrefs::default() };
    let mut watch = ready(Some(42));
    watch.set_policy(policy, 0.0);
    watch.observe(&AgentStatus::Done);
    let comment = Body::Commented { author: "rev".into(), text: "why?".into(), url: String::new() };
    watch.look_started();
    let outs = watch.polled(Ok(polled(vec![comment, failed()], false, false)), policy, "t", 5.0, 0);
    assert_eq!(pasted(&outs).len(), 1, "only the failed check goes");
    assert_eq!(watch.cards.cards().len(), 3, "both cards and the message");
}

#[test]
fn after_three_automatic_sends_the_tab_asks_again() {
    let mut watch = ready(Some(42));
    watch.set_policy(auto(), 0.0);
    let mut toasts = Vec::new();
    for round in 0..3 {
        watch.observe(&AgentStatus::Done);
        watch.look_started();
        let policy = watch.policy(GithubPrefs::default());
        let outs = watch.polled(
            Ok(polled(vec![failed()], false, false)),
            policy,
            "t",
            f64::from(round) * 100.0,
            0,
        );
        assert_eq!(pasted(&outs).len(), 1, "round {round}");
        toasts.extend(outs.into_iter().filter(|o| matches!(o, Out::Toast(_))));
        watch.observe(&AgentStatus::Working);
    }
    assert_eq!(watch.cards.auto_sends, MAX_AUTO_SENDS);
    assert_eq!(watch.policy(GithubPrefs::default()).mode, PrMode::Ask);
    assert_eq!(
        toasts,
        [Out::Toast("Auto-fix sent 3 times on this pull request. Back to Ask me".into())]
    );
    watch.observe(&AgentStatus::Done);
    watch.look_started();
    let policy = watch.policy(GithubPrefs::default());
    let outs = watch.polled(Ok(polled(vec![failed()], false, false)), policy, "t", 900.0, 0);
    assert_eq!(pasted(&outs), Vec::<&str>::new(), "the fourth waits for a click");
}

#[test]
fn a_following_a_new_pull_request_gives_auto_fix_a_fresh_count() {
    let mut watch = ready(Some(42));
    watch.cards.auto_sends = 2;
    assert!(watch.learn(43, "h", 1.0));
    assert_eq!(watch.cards.auto_sends, 0);
}

#[test]
fn the_button_sends_to_a_working_or_idle_agent_and_refuses_one_that_waits_on_you() {
    let mut watch = ready(Some(42));
    let id = watch.cards.add(1_000, failed());
    let cases = [
        (AgentStatus::Working, true),
        (AgentStatus::Done, true),
        (AgentStatus::NeedsYou(Reason::Question), false),
        (AgentStatus::Shell, false),
    ];
    for (status, goes) in cases {
        watch.observe(&status);
        let got = watch.ask(id, 1.0, 2_000);
        assert_eq!(got.is_ok(), goes, "{status:?}");
    }
    assert_eq!(sent_count(&watch), 2);
    watch.observe(&AgentStatus::Done);
    let Err(reason) = watch.ask(999, 1.0, 0) else { panic!("no such card") };
    assert_eq!(reason, "There is nothing to ask the agent about");
    let Body::Sent { via, .. } = &watch.cards.cards().last().unwrap().body else { panic!() };
    assert_eq!(*via, Via::You);
    assert_eq!(watch.cards.auto_sends, 0, "a click is not an automatic send");
}

#[test]
fn send_now_and_cancel_act_on_the_waiting_row() {
    let mut watch = ready(Some(42));
    watch.set_policy(auto(), 0.0);
    watch.observe(&AgentStatus::Working);
    watch.look_started();
    watch.polled(Ok(polled(vec![failed()], false, false)), auto(), "t", 1.0, 0);
    let id = watch.pending[0].id;
    watch.cancel(id);
    assert_eq!(watch.pending, []);
    watch.look_started();
    watch.polled(Ok(polled(vec![failed()], false, false)), auto(), "t", 200.0, 0);
    let id = watch.pending[0].id;
    watch.observe(&AgentStatus::NeedsYou(Reason::Permission));
    assert_eq!(
        watch.send_now(id, 1.0, 5),
        Err("The agent is waiting on you. Answer it first".to_owned())
    );
    assert_eq!(watch.pending.len(), 1, "a refused send keeps the row");
    watch.observe(&AgentStatus::Working);
    let outs = watch.send_now(id, 1.0, 5).unwrap();
    assert_eq!(pasted(&outs).len(), 1);
    assert_eq!(watch.pending, []);
    assert_eq!(watch.send_now(id, 1.0, 5), Ok(Vec::new()), "gone is not an error");
}

#[test]
fn leaving_auto_clears_the_waiting_rows_and_leaving_off_looks_at_once() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.set_policy(auto(), 0.0);
    watch.observe(&AgentStatus::Working);
    watch.look_started();
    watch.polled(Ok(polled(vec![failed()], false, false)), auto(), "t", 1.0, 0);
    assert_eq!(watch.pending.len(), 1);
    watch.set_policy(GithubPrefs { mode: PrMode::Off, ..default }, 5.0);
    assert_eq!(watch.pending, []);
    assert_eq!(watch.should_look(default, 1e6), None);
    watch.set_policy(default, 500.0);
    assert!(watch.should_look(default, 500.0).is_some(), "switching it on looks now");
}

#[test]
fn what_the_agent_committed_after_a_message_becomes_an_offer_to_push() {
    let mut watch = ready(Some(42));
    watch.cards.seen = Some(Seen { opened: true, ..Seen::default() });
    watch.observe(&AgentStatus::Working);
    assert!(!watch.observe(&AgentStatus::Done), "nothing was sent yet");
    watch.observe(&AgentStatus::Working);
    watch.cards.fixing = true;
    assert!(watch.observe(&AgentStatus::Done), "it just finished after a message");
    assert!(!watch.observe(&AgentStatus::Done), "once");
    assert!(watch.committed(Some(("abc".into(), 2)), 7_000));
    assert!(!watch.cards.fixing);
    let Body::PushFix { commits, head } = &watch.cards.cards().last().unwrap().body else {
        panic!()
    };
    assert_eq!((*commits, head.as_str()), (2, "abc"));
    watch.cards.fixing = true;
    assert!(!watch.committed(Some(("abc".into(), 0)), 8_000), "no commits, no offer");
    assert!(!watch.committed(None, 8_000));
}

#[test]
fn a_pushed_fix_offer_goes_away_when_the_pull_request_has_it() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.committed(Some(("head-1".into(), 1)), 1);
    assert_eq!(watch.cards.cards().len(), 1);
    watch.look_started();
    watch.polled(Ok(polled(vec![], false, false)), default, "t", 1.0, 0);
    assert_eq!(watch.cards.cards().len(), 0);
}

#[test]
fn notifications_say_what_happened_and_the_quiet_cards_say_nothing() {
    let comment =
        ReviewComment { path: "runner.rs".into(), line: Some(1), text: "Move it.\nmore".into() };
    let cases = [
        (
            failed(),
            Some((
                Tone::Failed,
                "t: checks failed on #7",
                "check (ubuntu-latest) failed at clippy",
            )),
        ),
        (
            Body::ChangesRequested {
                author: "rev".into(),
                body: String::new(),
                comments: vec![comment],
            },
            Some((Tone::Needs, "t: changes requested on #7", "@rev: Move it.")),
        ),
        (
            Body::ChangesRequested { author: "rev".into(), body: String::new(), comments: vec![] },
            Some((Tone::Needs, "t: changes requested on #7", "by @rev")),
        ),
        (
            Body::Commented {
                author: "rev".into(),
                text: "Does it work?\nreally".into(),
                url: String::new(),
            },
            Some((Tone::Needs, "t: new comment on #7", "@rev: Does it work?")),
        ),
        (
            Body::Conflict { base: "main".into() },
            Some((Tone::Needs, "t: merge conflicts on #7", "with main")),
        ),
        (
            Body::Approved { author: "rev".into() },
            Some((Tone::Done, "t: approved on #7", "by @rev")),
        ),
        (
            Body::Merged { number: 7, base: "main".into() },
            Some((Tone::Done, "t: merged on #7", "into main")),
        ),
        (Body::Closed { number: 7 }, Some((Tone::Needs, "t: closed on #7", "without merging"))),
        (Body::ChecksPassed { total: 2, secs: None }, None),
        (Body::ChecksRunning { head: "h".into(), done: 0, total: 1 }, None),
        (Body::PushFix { commits: 1, head: "h".into() }, None),
    ];
    for (body, want) in cases {
        let got = notice(&body, 7, "t").map(|n| (n.tone, n.what, n.detail));
        let want = want.map(|(tone, what, detail)| (tone, what.to_owned(), detail.to_owned()));
        assert_eq!(got, want, "{body:?}");
    }
}

#[test]
fn a_lookup_that_follows_a_head_that_moved_looks_again_at_once() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.look_started();
    watch.polled(Ok(polled(vec![], false, false)), default, "t", 10.0, 0);
    assert_eq!(watch.should_look(default, 11.0), None);
    watch.learn(42, "head-2", 11.0);
    assert!(watch.should_look(default, 11.0).is_some(), "a push just went out");
}

#[test]
fn a_merge_card_is_offered_until_it_is_kept() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    assert!(!watch.merge_offered());
    watch.look_started();
    let merged = Body::Merged { number: 42, base: "main".into() };
    watch.polled(Ok(polled(vec![merged], false, true)), default, "tab", 5.0, 0);
    assert!(watch.merge_offered());
    let id = watch.cards.cards()[0].id;
    watch.cards.handle(id);
    assert!(!watch.merge_offered());
}

#[test]
fn only_a_clean_second_check_lets_the_removal_go() {
    use tidewatch_core::worktree_cleanup::Report;
    let clean = Report { local_commits: Some(0), ..Report::default() };
    let dirty = Report { changed: vec!["a.rs".into()], ..clean.clone() };
    let mut watch = ready(Some(42));
    assert_eq!(watch.checked(false, Ok(clean.clone())), []);
    assert_eq!(watch.cleanup, Cleanup::Checked(clean.clone()));
    assert_eq!(watch.checked(true, Ok(clean)), [Out::Remove]);
    let outs = watch.checked(true, Ok(dirty.clone()));
    assert!(matches!(outs.as_slice(), [Out::Toast(_)]), "{outs:?}");
    assert_eq!(watch.cleanup, Cleanup::Checked(dirty));
    assert_eq!(watch.checked(true, Err("gone".into())), []);
    assert_eq!(watch.cleanup, Cleanup::Failed("gone".into()));
}

fn on_its_own() -> Watch {
    let mut watch = ready(Some(42));
    watch.autonomous = true;
    watch
}

fn commented() -> Body {
    Body::Commented { author: "rev".into(), text: "why?".into(), url: String::new() }
}

fn changes_requested() -> Body {
    Body::ChangesRequested { author: "rev".into(), body: "rename it".into(), comments: Vec::new() }
}

#[test]
fn a_tab_that_runs_on_its_own_is_auto_fix_whatever_the_default_and_the_owner_can_still_change_it() {
    for mode in [PrMode::Ask, PrMode::Off, PrMode::Auto] {
        let default = GithubPrefs { mode, reviews: false, ..GithubPrefs::default() };
        let mut watch = on_its_own();
        let policy = watch.policy(default);
        assert_eq!(policy.mode, PrMode::Auto, "{mode:?}");
        assert!(!policy.reviews, "the default's tick boxes still apply");
        assert_eq!(ready(Some(42)).policy(default).mode, mode, "an ordinary tab keeps the default");
        watch.set_policy(GithubPrefs { mode: PrMode::Ask, ..default }, 0.0);
        assert_eq!(watch.policy(default).mode, PrMode::Ask, "the popover wins");
    }
}

#[test]
fn comments_review_changes_failed_checks_and_conflicts_go_to_an_idle_agent_without_a_click() {
    let default = GithubPrefs::default();
    let news = [
        (commented(), "Comment"),
        (changes_requested(), "Review"),
        (failed(), "CI failure"),
        (Body::Conflict { base: "main".into() }, "Conflict"),
    ];
    for (body, tag) in news {
        let mut watch = on_its_own();
        watch.observe(&AgentStatus::Done);
        watch.look_started();
        let outs = watch.polled(Ok(polled(vec![body], false, false)), default, "t", 5.0, 9_000);
        assert_eq!(pasted(&outs).len(), 1, "{tag}");
        assert_eq!(watch.cards.auto_sends, 1, "{tag}");
        let Body::Sent { via, tag: sent, .. } = &watch.cards.cards().last().unwrap().body else {
            panic!("{tag}: no record of the send")
        };
        assert_eq!((*via, sent.as_str()), (Via::Auto, tag));
    }
}

#[test]
fn a_busy_agent_gets_the_news_when_it_is_idle_and_not_before() {
    let default = GithubPrefs::default();
    let mut watch = on_its_own();
    watch.observe(&AgentStatus::Working);
    watch.look_started();
    let outs = watch.polled(Ok(polled(vec![commented()], false, false)), default, "t", 5.0, 9_000);
    assert_eq!(pasted(&outs), Vec::<&str>::new());
    assert_eq!(watch.pending.len(), 1);
    watch.observe(&AgentStatus::Done);
    assert_eq!(pasted(&watch.flush(default, 7.0, 9_100)).len(), 1);
}

#[test]
fn the_loop_guard_of_a_tab_on_its_own_is_ten_sends_and_then_it_pauses_without_asking() {
    let default = GithubPrefs::default();
    let mut watch = on_its_own();
    for round in 0..MAX_AUTONOMOUS_SENDS {
        watch.observe(&AgentStatus::Done);
        watch.look_started();
        let outs = watch.polled(
            Ok(polled(vec![failed()], false, false)),
            default,
            "t",
            f64::from(round) * 100.0,
            0,
        );
        assert_eq!(pasted(&outs).len(), 1, "round {round}");
        assert!(!outs.iter().any(|o| matches!(o, Out::Toast(_))), "no toast, round {round}");
        watch.observe(&AgentStatus::Working);
    }
    assert_eq!(watch.cards.auto_sends, MAX_AUTONOMOUS_SENDS);
    assert_eq!(watch.policy(default).mode, PrMode::Auto, "no fall back to Ask me");
    assert_eq!(watch.handling(default), Handling::Paused);
    watch.observe(&AgentStatus::Done);
    watch.look_started();
    let outs = watch.polled(Ok(polled(vec![failed()], false, false)), default, "t", 9_000.0, 0);
    assert_eq!(pasted(&outs), Vec::<&str>::new(), "paused");
    assert_eq!(watch.pending, [], "nothing waits for a button either");
    assert!(!outs.iter().any(|o| matches!(o, Out::Toast(_))));
    assert_eq!(paused_note(), "Auto-fix paused after 10 sends on this pull request");
}

#[test]
fn a_new_pull_request_lifts_the_pause() {
    let default = GithubPrefs::default();
    let mut watch = on_its_own();
    watch.cards.auto_sends = MAX_AUTONOMOUS_SENDS;
    assert_eq!(watch.handling(default), Handling::Paused);
    watch.learn(43, "h", 1.0);
    assert_eq!(watch.handling(default), Handling::Agent);
}

#[test]
fn who_handles_the_news_follows_the_tab_and_the_mode_the_owner_left_it_in() {
    let default = GithubPrefs::default();
    let cases = [
        (false, None, Handling::Owner),
        (false, Some(PrMode::Auto), Handling::Owner),
        (true, None, Handling::Agent),
        (true, Some(PrMode::Auto), Handling::Agent),
        (true, Some(PrMode::Ask), Handling::Owner),
        (true, Some(PrMode::Off), Handling::Owner),
    ];
    for (autonomous, mode, want) in cases {
        let mut watch = ready(Some(42));
        watch.autonomous = autonomous;
        if let Some(mode) = mode {
            watch.set_policy(GithubPrefs { mode, ..default }, 0.0);
        }
        assert_eq!(watch.handling(default), want, "{autonomous} {mode:?}");
    }
}

#[test]
fn commits_the_agent_made_after_a_message_are_pushed_by_a_tab_on_its_own_and_offered_to_others() {
    let default = GithubPrefs::default();
    let mut mine = on_its_own();
    let mut other = ready(Some(42));
    for watch in [&mut mine, &mut other] {
        watch.committed(Some(("abc".into(), 2)), 5_000);
    }
    let id = mine.cards.cards().last().unwrap().id;
    assert_eq!(mine.push_fix_due(default), Some(id));
    assert_eq!(other.push_fix_due(default), None);
    mine.cards.handle(id);
    assert_eq!(mine.push_fix_due(default), None, "once, then it is handled");
}

#[test]
fn a_review_bot_receipt_saved_by_an_older_version_is_not_loaded() {
    let mut cards = Cards::default();
    cards.add(
        10,
        Body::Commented {
            author: "chatgpt-codex-connector".into(),
            text: "<!-- codex-review-receipt:v1 -->".into(),
            url: String::new(),
        },
    );
    cards.add(20, commented());
    let mut watch = Watch::new(PathBuf::from("/state/pr_cards.json"));
    watch.take_load();
    watch.loaded(cards, 0.0);
    assert_eq!(watch.cards.cards().len(), 1);
}

fn merged_watch(autonomous: bool, left: usize) -> Watch {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.autonomous = autonomous;
    watch.look_started();
    let merged = Body::Merged { number: 42, base: "main".into() };
    watch.polled(Ok(polled(vec![merged], false, true)), default, "tab", 5.0, 0);
    watch.cards.after_merge = left;
    if let Some(seen) = &mut watch.cards.seen {
        seen.head = "head-1".into();
    }
    watch
}

#[test]
fn a_tab_on_its_own_whose_pull_request_merged_never_delivers_and_notes_what_was_left() {
    let cases = [
        (
            "merged, a commit left",
            merged_watch(true, 1),
            false,
            Some("1 commit after merge, not delivered"),
        ),
        (
            "merged, two left",
            merged_watch(true, 2),
            false,
            Some("2 commits after merge, not delivered"),
        ),
        ("merged, nothing left", merged_watch(true, 0), false, None),
        ("merged but the owner decides", merged_watch(false, 1), true, None),
        ("open", ready(Some(42)), true, None),
        ("no pull request", ready(None), true, None),
    ];
    for (what, mut watch, deliver, note) in cases {
        if what == "open" || what == "no pull request" {
            watch.autonomous = true;
            watch.cards.after_merge = 1;
        }
        let verdict = watch.verdict();
        assert_eq!(verdict.deliver, deliver, "{what}");
        assert_eq!(verdict.note.as_deref(), note, "{what}");
    }
}

#[test]
fn a_closed_pull_request_is_not_a_merged_one() {
    let default = GithubPrefs::default();
    let mut watch = ready(Some(42));
    watch.autonomous = true;
    watch.look_started();
    watch.polled(
        Ok(polled(vec![Body::Closed { number: 42 }], false, true)),
        default,
        "tab",
        5.0,
        0,
    );
    assert!(watch.verdict().deliver);
}

#[test]
fn a_new_pull_request_on_the_same_branch_delivers_again() {
    let mut watch = merged_watch(true, 1);
    assert!(!watch.verdict().deliver);
    assert!(watch.learn(43, "head-9", 10.0));
    let verdict = watch.verdict();
    assert!(verdict.deliver);
    assert_eq!(verdict.note, None);
}

#[test]
fn the_commits_after_the_merge_are_counted_once_per_idle_moment_for_a_tab_on_its_own() {
    let mut watch = merged_watch(true, 0);
    assert_eq!(watch.take_recount(), None, "not idle yet");
    watch.observe(&AgentStatus::Working);
    watch.observe(&AgentStatus::Done);
    assert_eq!(watch.take_recount().as_deref(), Some("head-1"));
    assert_eq!(watch.take_recount(), None, "once");
    assert!(watch.counted(Some(1)));
    assert!(!watch.counted(Some(1)), "nothing new to save");
    assert!(!watch.counted(None), "git could not say");
    assert_eq!(watch.cards.after_merge, 1);

    let mut owner = merged_watch(false, 0);
    owner.observe(&AgentStatus::Done);
    assert_eq!(owner.take_recount(), None);
    let mut open = ready(Some(42));
    open.autonomous = true;
    open.observe(&AgentStatus::Done);
    assert_eq!(open.take_recount(), None);
}

#[test]
fn a_tab_on_its_own_does_not_deliver_before_its_saved_cards_are_read() {
    let mut unread = Watch::new(PathBuf::from("/state/pr_cards.json"));
    unread.autonomous = true;
    assert!(delivery_shut(Some(&unread), true), "cards not read");
    assert!(delivery_shut(None, true), "no watch yet");
    assert!(!delivery_shut(None, false), "a tab that waits for the owner is never held");
    assert!(!delivery_shut(Some(&Watch::new(PathBuf::from("/x"))), false));
    unread.take_load();
    unread.loaded(Cards::default(), 0.0);
    assert!(!delivery_shut(Some(&unread), true), "no pull request");
    assert!(delivery_shut(Some(&merged_watch(true, 0)), true));
    assert!(!delivery_shut(Some(&merged_watch(false, 0)), false));
}
