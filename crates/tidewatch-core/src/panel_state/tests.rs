use super::*;
use crate::link_board::{Link, LinkState, TitleFrom};
use crate::panel_event::{ChecksWord, PrFacts};
use crate::panel_log::Log;
use crate::progress::Ledger;
use crate::status::{ProgressSignal, TerminalSignal};

struct Tape {
    log: Log,
    state: PanelState,
}

impl Tape {
    fn new() -> Self {
        Self { log: Log::new(), state: PanelState::new() }
    }

    fn put(&mut self, source: Source, fact: Fact, at: i64) -> Applied {
        let event = self.log.append(source, fact, at);
        self.state.apply(&event)
    }

    fn hook(&mut self, kind: HookKind, at: i64) {
        self.hook_saying(kind, None, at);
    }

    fn hook_saying(&mut self, kind: HookKind, message: Option<&str>, at: i64) {
        let hook = HookEvent {
            kind,
            session_id: None,
            transcript_path: None,
            message: message.map(str::to_owned),
        };
        self.put(Source::Hook, Fact::Hook(hook), at);
    }

    fn started(&mut self) {
        self.put(Source::App, Fact::Started { hooked: true }, 0);
    }

    fn busy(&mut self, at: i64) {
        self.put(
            Source::Terminal,
            Fact::Terminal(TerminalSignal::Progress(ProgressSignal::Working)),
            at,
        );
    }

    fn saved(&mut self, saved_ms: i64, state: &str, needs: &str, at: i64) {
        let digest = digest(state, needs);
        self.put(Source::Report, Fact::ReportSaved { saved_ms, digest }, at);
    }
}

fn digest(state: &str, needs: &str) -> ReportDigest {
    ReportDigest::of(&Ledger::parse(&format!(
        "# Goal\nShip\n\n## Status\nstate: {state}\n\n## Needs you\n{needs}\n"
    )))
}

const NO_QUESTION: &str = "- Nothing";
const A_QUESTION: &str = "- Yes or changes on the board?";

struct Case {
    name: &'static str,
    state: &'static str,
    needs: &'static str,
    turn_ends: Option<i64>,
    now: i64,
    tone: Tone,
    mark: ReportMark,
}

#[test]
fn a_report_the_facts_contradict_is_overruled_and_marked_out_of_date() {
    let out_of_date = ReportMark::OutOfDate;
    let cases = [
        Case {
            name: "needs you with no question, the turn ended after it",
            state: "needs_you",
            needs: NO_QUESTION,
            turn_ends: Some(2_000),
            now: 2_500,
            tone: Tone::Done,
            mark: out_of_date(Why::NeedsYouNoQuestion),
        },
        Case {
            name: "needs you with no question while the turn still runs",
            state: "needs_you",
            needs: NO_QUESTION,
            turn_ends: None,
            now: 2_500,
            tone: Tone::Working,
            mark: out_of_date(Why::NeedsYouNoQuestion),
        },
        Case {
            name: "needs you with a question stands",
            state: "needs_you",
            needs: A_QUESTION,
            turn_ends: Some(2_000),
            now: 2_500,
            tone: Tone::Needs,
            mark: ReportMark::Fresh,
        },
        Case {
            name: "working, the turn ended ten minutes ago",
            state: "working",
            needs: NO_QUESTION,
            turn_ends: Some(2_000),
            now: 2_000 + 600_000,
            tone: Tone::Done,
            mark: out_of_date(Why::WorkingButTurnEnded),
        },
        Case {
            name: "waiting on nothing, the turn ended after it",
            state: "waiting",
            needs: NO_QUESTION,
            turn_ends: Some(2_000),
            now: 2_500,
            tone: Tone::Done,
            mark: out_of_date(Why::WaitingOnNothing),
        },
        Case {
            name: "working while the turn runs stands",
            state: "working",
            needs: NO_QUESTION,
            turn_ends: None,
            now: 2_500,
            tone: Tone::Working,
            mark: ReportMark::Fresh,
        },
    ];
    for case in cases {
        let mut tape = Tape::new();
        tape.started();
        tape.hook(HookKind::TurnStarted, 1_000);
        tape.busy(1_000);
        tape.saved(1_500, case.state, case.needs, 1_500);
        if let Some(at) = case.turn_ends {
            tape.hook(HookKind::TurnDone, at);
            tape.put(
                Source::Terminal,
                Fact::Terminal(TerminalSignal::Progress(ProgressSignal::Off)),
                at,
            );
        }
        let view = tape.state.view(case.now);
        assert_eq!(view.tone, case.tone, "{}", case.name);
        assert_eq!(view.report, Some(case.mark), "{}", case.name);
    }
}

#[test]
fn a_done_report_is_overruled_by_a_turn_that_started_after_it() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.saved(1_500, "done", NO_QUESTION, 1_500);
    tape.hook(HookKind::TurnDone, 2_000);
    assert_eq!(tape.state.view(2_100).tone, Tone::Done);
    tape.hook(HookKind::TurnStarted, 5_000);
    tape.busy(5_000);
    let view = tape.state.view(5_500);
    assert_eq!(view.tone, Tone::Working);
    assert_eq!(view.reported, Some(State::Working));
    assert_eq!(view.report, Some(ReportMark::OutOfDate(Why::DoneButTurnRunning)));
    tape.saved(5_200, "done", NO_QUESTION, 5_600);
    let saved_in_turn = tape.state.view(5_700);
    assert_eq!(saved_in_turn.report, Some(ReportMark::Fresh));
}

#[test]
fn a_question_saved_before_the_turn_that_followed_it_is_stale_and_so_is_one_the_owner_answered() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.saved(1_500, "needs_you", A_QUESTION, 1_500);
    tape.hook(HookKind::TurnDone, 2_000);
    assert_eq!(tape.state.view(2_100).tone, Tone::Needs);
    tape.put(Source::App, Fact::Answered { kind: AskKind::Question }, 2_500);
    let answered = tape.state.view(2_600);
    assert_eq!(answered.report, Some(ReportMark::OutOfDate(Why::QuestionAnswered)));
    assert_eq!(answered.reported, Some(State::Working));
    tape.saved(2_700, "needs_you", A_QUESTION, 2_700);
    assert_eq!(tape.state.view(2_800).report, Some(ReportMark::Fresh));
    tape.hook(HookKind::TurnStarted, 3_000);
    tape.busy(3_000);
    let older = tape.state.view(3_100);
    assert_eq!(older.report, Some(ReportMark::OutOfDate(Why::OlderThanTurn)));
    assert_eq!(older.tone, Tone::Working);
}

#[test]
fn no_report_means_no_mark() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    let view = tape.state.view(1_100);
    assert_eq!(view.report, None);
    assert_eq!(view.reported, None);
    assert_eq!(view.ages.report, None);
}

#[test]
fn events_are_ordered_by_number_and_a_clock_that_goes_backward_never_reorders() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 100_000);
    tape.hook(HookKind::TurnDone, 40_000);
    tape.hook(HookKind::TurnStarted, 30_000);
    let view = tape.state.view(10_000);
    assert_eq!(
        view.turn,
        Turn { running: true, started_ms: Some(100_000), ended_ms: Some(100_000) }
    );
    assert_eq!(tape.state.clock_ms(), 100_000);
    assert_eq!(view.ages.turn.map(|age| age.text), Some("just now".to_owned()));
    let seqs: Vec<u64> = tape.log.events().iter().map(|event| event.seq).collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
}

#[test]
fn an_event_folded_twice_changes_nothing() {
    let mut tape = Tape::new();
    tape.started();
    let event = tape.log.append(
        Source::Transcript,
        Fact::ToolOpened { id: "c1".into(), name: "Bash".into() },
        1_000,
    );
    assert!(tape.state.apply(&event).changed);
    let once = tape.state.clone();
    assert_eq!(tape.state.view(1_100).since, 1);
    assert_eq!(tape.state.apply(&event), Applied::default());
    assert_eq!(tape.state, once);
}

#[test]
fn the_transcript_stands_in_for_a_hook_that_never_came() {
    let mut tape = Tape::new();
    tape.started();
    tape.busy(900);
    tape.put(Source::Transcript, Fact::Turn { started: true, ts_ms: Some(1_000) }, 1_200);
    let working = tape.state.view(1_500);
    assert_eq!((working.tone, working.turn.running), (Tone::Working, true));
    tape.put(Source::Transcript, Fact::Said { tail: "All done.".into() }, 3_000);
    tape.put(Source::Transcript, Fact::Turn { started: false, ts_ms: Some(3_000) }, 3_100);
    tape.put(
        Source::Terminal,
        Fact::Terminal(TerminalSignal::Progress(ProgressSignal::Off)),
        3_100,
    );
    let done = tape.state.view(3_200);
    assert_eq!((done.tone, done.turn.running), (Tone::Done, false));
    assert_eq!(tape.state.last_message(), Some("All done."));
}

#[test]
fn a_transcript_turn_mark_for_another_turn_is_ignored() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 10_000);
    tape.hook(HookKind::TurnDone, 20_000);
    tape.put(Source::Transcript, Fact::Turn { started: true, ts_ms: Some(10_000) }, 21_000);
    assert!(!tape.state.view(21_100).turn.running, "the transcript is late with a turn that ended");
    tape.hook(HookKind::TurnStarted, 30_000);
    tape.put(Source::Transcript, Fact::Turn { started: false, ts_ms: Some(20_000) }, 31_000);
    assert!(tape.state.view(31_100).turn.running, "an old end does not end the new turn");
    tape.put(Source::Transcript, Fact::Turn { started: false, ts_ms: Some(40_000) }, 41_000);
    assert!(!tape.state.view(41_100).turn.running);
    tape.put(Source::Transcript, Fact::Turn { started: true, ts_ms: Some(50_000) }, 51_000);
    assert!(tape.state.view(51_100).turn.running, "a turn newer than the last end opens");
}

#[test]
fn a_question_open_in_the_transcript_beats_a_working_tracker_until_it_closes() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.busy(1_000);
    tape.put(Source::Transcript, Fact::AskOpened { summary: "Pick one".into(), n: 1 }, 2_000);
    let asked = tape.state.view(2_100);
    assert_eq!(asked.tone, Tone::Needs);
    assert_eq!(asked.status, AgentStatus::NeedsYou(Reason::Question));
    tape.put(Source::Transcript, Fact::AskClosed, 3_000);
    assert_eq!(tape.state.view(3_100).tone, Tone::Working);
    tape.put(Source::Transcript, Fact::PermissionOpened { tool: "Bash: rm x".into() }, 4_000);
    assert_eq!(tape.state.view(4_100).status, AgentStatus::NeedsYou(Reason::Permission));
    tape.put(Source::App, Fact::Answered { kind: AskKind::Permission }, 5_000);
    assert_eq!(tape.state.view(5_100).tone, Tone::Working);
}

#[test]
fn an_ask_does_not_outlive_its_turn() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.put(Source::Transcript, Fact::AskOpened { summary: "Pick one".into(), n: 1 }, 2_000);
    tape.hook(HookKind::TurnDone, 3_000);
    assert_eq!(tape.state.view(3_100).asking, None);
    tape.hook(HookKind::TurnStarted, 4_000);
    tape.busy(4_000);
    assert_eq!(tape.state.view(4_100).tone, Tone::Working);
}

#[test]
fn a_restart_drops_what_hooks_said_and_the_other_layers_decide() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.busy(1_000);
    tape.hook(HookKind::ToolStarted { key: "Bash\nrm x".into(), id: None }, 1_500);
    tape.hook(HookKind::ToolPermission("Bash\nrm x".into()), 2_000);
    tape.put(Source::Terminal, Fact::Output, 9_000);
    assert_eq!(tape.state.view(9_100).tone, Tone::Needs, "the prompt holds while hooks speak");
    tape.put(Source::App, Fact::Restart, 10_000);
    tape.put(Source::Terminal, Fact::Output, 10_500);
    assert_eq!(tape.state.view(10_600).tone, Tone::Working, "the prompt was a hook's word");
    tape.put(
        Source::Terminal,
        Fact::Terminal(TerminalSignal::Progress(ProgressSignal::Off)),
        13_000,
    );
    tape.put(Source::Transcript, Fact::Turn { started: false, ts_ms: Some(13_000) }, 13_100);
    let view = tape.state.view(13_200);
    assert_eq!((view.tone, view.turn.running), (Tone::Done, false));
}

#[test]
fn an_exited_process_ends_the_turn_and_decides_done_or_failed() {
    for (code, tone) in [(Some(0), Tone::Done), (Some(2), Tone::Failed), (None, Tone::Failed)] {
        let mut tape = Tape::new();
        tape.started();
        tape.hook(HookKind::TurnStarted, 1_000);
        tape.put(Source::Process, Fact::Process { alive: false, exit_code: code }, 2_000);
        let view = tape.state.view(2_100);
        assert_eq!(view.tone, tone, "{code:?}");
        assert!(!view.turn.running);
    }
}

#[test]
fn the_hook_message_of_a_finished_turn_becomes_the_end_block_and_the_next_turn_clears_it() {
    let message =
        "Wrapped up.\n\n**Done**\n- the fix\n\n**Needs you**\n- nothing\n\n**Next**\n- merge";
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.hook_saying(HookKind::TurnDone, Some(message), 2_000);
    let block = tape.state.view(2_100).end_block.expect("a complete block parses");
    assert_eq!(block.done, ["the fix"]);
    assert_eq!(block.next, ["merge"]);
    tape.hook(HookKind::TurnStarted, 3_000);
    assert_eq!(tape.state.view(3_100).end_block, None);
}

#[test]
fn what_happened_since_the_report_counts_calls_once_however_many_sources_saw_them() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.saved(1_100, "working", NO_QUESTION, 1_100);
    assert_eq!(tape.state.view(1_200).since, 0);
    let call = |tape: &mut Tape, n: u32| {
        tape.hook(
            HookKind::ToolStarted { key: format!("Bash\nls {n}"), id: Some(format!("c{n}")) },
            2_000,
        );
    };
    let opened = |tape: &mut Tape, n: u32| {
        tape.put(
            Source::Transcript,
            Fact::ToolOpened { id: format!("c{n}"), name: "Bash".into() },
            2_100,
        );
    };
    call(&mut tape, 1);
    opened(&mut tape, 1);
    call(&mut tape, 2);
    assert_eq!(tape.state.view(2_200).since, 2, "two calls, one seen by both sources");
    opened(&mut tape, 2);
    opened(&mut tape, 3);
    assert_eq!(tape.state.view(2_300).since, 3, "a call only the transcript saw still counts");
    tape.put(
        Source::Git,
        Fact::Git(GitFacts { head: Some("a".into()), ..GitFacts::default() }),
        2_400,
    );
    tape.put(
        Source::Git,
        Fact::Git(GitFacts { head: Some("b".into()), ..GitFacts::default() }),
        2_500,
    );
    tape.put(Source::Tidewatch, Fact::Check { name: "tests".into(), passed: true }, 2_600);
    assert_eq!(tape.state.view(2_700).since, 5, "a new commit and a check join the calls");
    tape.saved(2_800, "working", NO_QUESTION, 2_800);
    assert_eq!(tape.state.view(2_900).since, 0, "a new report starts the count again");
}

#[test]
fn a_source_quiet_past_its_limit_says_so_with_its_last_word() {
    let mut tape = Tape::new();
    tape.started();
    tape.put(Source::Transcript, Fact::Turn { started: true, ts_ms: Some(1_000) }, 1_000);
    let quiet = |tape: &Tape, now| {
        tape.state.view(now).quiet.iter().map(|q| (q.source, q.since_ms)).collect::<Vec<_>>()
    };
    assert_eq!(quiet(&tape, 2_500), Vec::<(Source, i64)>::new());
    assert_eq!(
        quiet(&tape, 3_500),
        [(Source::Hook, 1_000)],
        "no hook within 2 s of the turn start"
    );
    tape.hook(HookKind::ToolStarted { key: "k".into(), id: None }, 3_600);
    assert_eq!(quiet(&tape, 3_700), Vec::<(Source, i64)>::new());
    assert_eq!(quiet(&tape, 15_900), Vec::<(Source, i64)>::new(), "transcript spoke 14.9 s ago");
    assert_eq!(quiet(&tape, 16_100), [(Source::Transcript, 1_000)]);
}

#[test]
fn jobs_git_and_the_pull_request_go_quiet_on_their_own_clocks() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.put(
        Source::Job,
        Fact::Job(job::Report {
            done: Some(1.0),
            total: Some(10.0),
            ..job::Report::named("import")
        }),
        2_000,
    );
    tape.put(Source::Git, Fact::Git(GitFacts::default()), 2_000);
    tape.put(
        Source::Pr,
        Fact::Pr(PrFacts { number: 7, state: PrWord::Open, checks: Some(ChecksWord::Running) }),
        2_000,
    );
    tape.hook(HookKind::TurnDone, 5_000);
    let sources = |now| tape.state.view(now).quiet.iter().map(|q| q.source).collect::<Vec<_>>();
    assert_eq!(sources(6_000), Vec::<Source>::new());
    assert_eq!(sources(7_100), [Source::Git], "no look within 2 s of the turn end");
    assert_eq!(sources(62_100), [Source::Job, Source::Git]);
    assert_eq!(sources(122_100), [Source::Job, Source::Git, Source::Pr]);
}

#[test]
fn a_git_look_after_the_turn_end_clears_its_quiet_mark() {
    let mut tape = Tape::new();
    tape.started();
    tape.put(Source::Git, Fact::Git(GitFacts::default()), 1_000);
    tape.hook(HookKind::TurnStarted, 2_000);
    tape.hook(HookKind::TurnDone, 3_000);
    assert_eq!(tape.state.view(6_000).quiet.len(), 1);
    tape.put(Source::Git, Fact::Git(GitFacts { changed: 2, ..GitFacts::default() }), 5_000);
    assert_eq!(tape.state.view(6_000).quiet, Vec::new());
}

#[test]
fn an_event_that_alters_nothing_shown_is_not_a_change() {
    let mut tape = Tape::new();
    tape.started();
    assert!(tape.put(Source::Hook, hook_fact(HookKind::TurnStarted), 1_000).changed);
    assert!(!tape.put(Source::Terminal, Fact::Output, 1_100).changed);
    assert!(
        !tape
            .put(
                Source::Hook,
                hook_fact(HookKind::ToolFinished { key: "k".into(), id: None }),
                1_200
            )
            .changed
    );
    let link = Link {
        url: "https://github.com/o/r/pull/1".into(),
        title: "PR".into(),
        note: None,
        first_seen_ms: 1_300,
        title_from: TitleFrom::Seen,
        state: LinkState::Open,
    };
    assert!(tape.put(Source::Link, Fact::Link(link.clone()), 1_300).changed);
    assert!(!tape.put(Source::Link, Fact::Link(link), 1_400).changed, "the same link again");
    assert!(tape.put(Source::Hook, hook_fact(HookKind::TurnDone), 1_500).changed);
}

fn hook_fact(kind: HookKind) -> Fact {
    Fact::Hook(HookEvent { kind, session_id: None, transcript_path: None, message: None })
}

#[test]
fn a_checkpoint_stands_for_the_events_before_it() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.saved(1_100, "working", NO_QUESTION, 1_100);
    tape.put(Source::Job, Fact::Job(job::Report::named("import")), 1_200);
    let live = tape.state.clone();
    let checkpoint = Event {
        seq: live.shown_seq(),
        at_ms: live.clock_ms(),
        source: Source::App,
        fact: Fact::Checkpoint(Box::new(live.clone())),
    };
    assert_eq!(PanelState::replay([&checkpoint]), live);
}

#[test]
fn the_state_survives_a_json_round_trip_exactly() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_793_017_688_981);
    tape.busy(1_793_017_688_981);
    tape.put(Source::Terminal, Fact::Output, 1_793_017_690_117);
    tape.put(
        Source::Job,
        Fact::Job(job::Report {
            done: Some(0.1 + 0.2),
            total: Some(7.0),
            ..job::Report::named("import")
        }),
        1_793_017_691_000,
    );
    tape.saved(1_793_017_692_000, "working", NO_QUESTION, 1_793_017_692_000);
    let text = serde_json::to_string(&tape.state).unwrap();
    assert_eq!(serde_json::from_str::<PanelState>(&text).unwrap(), tape.state);
}

fn answer(tape: &mut Tape, question: &str, at: i64) {
    let fact = Fact::QuestionAnswered { question: question.into(), answer: "Yes".into() };
    tape.put(Source::Transcript, fact, at);
}

#[test]
fn each_answered_question_counts_once_since_the_report_whichever_source_told_of_it() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.saved(1_100, "working", NO_QUESTION, 1_100);
    assert_eq!(tape.state.view(1_200).answers, 0);
    answer(&mut tape, "Step 1: do the five rules hold?", 1_300);
    answer(&mut tape, "  step 1: Do the five rules hold? ", 1_350);
    answer(&mut tape, "Step 2: which wording?", 1_400);
    assert_eq!(tape.state.view(1_500).answers, 2, "the same question twice is one answer");
    tape.saved(1_600, "working", NO_QUESTION, 1_600);
    assert_eq!(tape.state.view(1_700).answers, 0, "a new report starts the count again");
    answer(&mut tape, "Step 1: do the five rules hold?", 1_800);
    assert_eq!(tape.state.view(1_900).answers, 1, "a question asked again after a report counts");
}

#[test]
fn the_answered_questions_are_listed_for_the_steps_they_tick_until_the_next_report() {
    let mut tape = Tape::new();
    tape.started();
    tape.saved(1_100, "working", NO_QUESTION, 1_100);
    answer(&mut tape, "Which base?", 1_300);
    let listed: Vec<&str> =
        tape.state.answered().iter().map(|seen| seen.question.as_str()).collect();
    assert_eq!(listed, ["Which base?"]);
    tape.saved(1_600, "working", NO_QUESTION, 1_600);
    assert_eq!(tape.state.answered().len(), 0);
}

#[test]
fn an_answered_question_ends_the_wait_the_report_held_for_it() {
    let mut tape = Tape::new();
    tape.started();
    tape.hook(HookKind::TurnStarted, 1_000);
    tape.saved(1_100, "needs_you", "- Which base?", 1_100);
    answer(&mut tape, "Which base?", 1_300);
    assert_eq!(
        tape.state.view(1_400).report,
        Some(ReportMark::OutOfDate(Why::QuestionAnswered)),
        "the report's question is answered, so the report is out of date"
    );
}
