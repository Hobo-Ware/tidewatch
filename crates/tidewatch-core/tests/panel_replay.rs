//! The panel store end to end: the recorded hook and transcript fixtures folded and replayed from
//! disk, the invariants that make a replay exact (seeded random streams), and a fault harness that
//! drops hooks. The backup facts the harness writes by hand (turn marks, process probes, output) are
//! what the app must feed for real in the next slice.

use std::path::Path;

use serde_json::Value;
use tidewatch_core::end_block::EndBlock;
use tidewatch_core::job;
use tidewatch_core::link_board::{Link, LinkState, TitleFrom};
use tidewatch_core::panel_event::{
    AskKind, ChecksWord, Event, Fact, GitFacts, PrFacts, PrWord, Source, StepBy,
};
use tidewatch_core::panel_log::{CAP_EVENTS, Log};
use tidewatch_core::panel_rules::ReportDigest;
use tidewatch_core::panel_state::PanelState;
use tidewatch_core::panel_store::{self, Writer};
use tidewatch_core::panel_view::{ReportMark, Tone};
use tidewatch_core::progress::Ledger;
use tidewatch_core::status::{HookEvent, HookKind, ProgressSignal, TerminalSignal, parse_hook};
use tidewatch_core::transcript::{self, Role, ToolPhase};

const CLAUDE_HOOKS: &str = include_str!("fixtures/claude_hooks.jsonl");
const CLAUDE_TRANSCRIPT: &str = include_str!("fixtures/claude.jsonl");
const CODEX_HOOKS: &str = include_str!("fixtures/codex_hooks.jsonl");
const CODEX_TRANSCRIPT: &str = include_str!("fixtures/codex_live.jsonl");
const COPILOT_HOOKS: &str = include_str!("fixtures/copilot_hooks.jsonl");
const COPILOT_TRANSCRIPT: &str = include_str!("fixtures/copilot.jsonl");
const GEMINI_HOOKS: &str = include_str!("fixtures/gemini_hooks.jsonl");
const GEMINI_TRANSCRIPT: &str = include_str!("fixtures/gemini.jsonl");

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn percent(&mut self, chance: u64) -> bool {
        self.below(100) < chance
    }
}

struct Tape {
    log: Log,
    state: PanelState,
}

impl Tape {
    fn new() -> Self {
        Self { log: Log::new(), state: PanelState::new() }
    }

    fn put(&mut self, source: Source, fact: Fact, at: i64) {
        let event = self.log.append(source, fact, at);
        self.state.apply(&event);
    }
}

fn hook_fact(kind: HookKind) -> Fact {
    Fact::Hook(HookEvent { kind, session_id: None, transcript_path: None, message: None })
}

fn hook_saying(kind: HookKind, message: &str) -> Fact {
    Fact::Hook(HookEvent {
        kind,
        session_id: None,
        transcript_path: None,
        message: Some(message.to_owned()),
    })
}

fn busy(on: bool) -> Fact {
    Fact::Terminal(TerminalSignal::Progress(if on {
        ProgressSignal::Working
    } else {
        ProgressSignal::Off
    }))
}

fn digest(state: &str, needs: &str) -> ReportDigest {
    ReportDigest::of(&Ledger::parse(&format!(
        "# Goal\nShip\n\n## Status\nstate: {state}\n\n## Needs you\n{needs}\n"
    )))
}

fn hooks_of(agent: &str, fixture: &str) -> Vec<HookEvent> {
    fixture
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter_map(|payload| parse_hook(agent, &payload))
        .collect()
}

/// What a transcript says in the terms of the store: its first user line opens the turn, the
/// assistant's lines are what it said, its tool calls open and close, and its end closes the turn.
fn transcript_facts(source: transcript::Source, text: &str) -> Vec<Fact> {
    let entries = transcript::parse_all(source, text);
    let mut facts = Vec::new();
    let mut opened = false;
    for entry in &entries {
        match (entry.role, &entry.tool) {
            (Role::User, _) if !opened => {
                opened = true;
                facts.push(Fact::Turn { started: true, ts_ms: None });
            }
            (Role::Assistant, _) => facts.push(Fact::Said { tail: entry.text.clone() }),
            (Role::Tool, Some(call)) => {
                let id = call.id.clone().unwrap_or_else(|| call.summary.clone());
                facts.push(match call.phase {
                    ToolPhase::Call => Fact::ToolOpened { id, name: call.name.clone() },
                    ToolPhase::Result { .. } => Fact::ToolClosed { id },
                });
            }
            _ => {}
        }
    }
    if opened {
        facts.push(Fact::Turn { started: false, ts_ms: None });
    }
    facts
}

/// The hooks and the transcript of one agent, interleaved by time over the same span.
fn session_of(
    hooks: &[HookEvent],
    transcript: &[Fact],
    t0: i64,
    span: i64,
) -> Vec<(Source, Fact, i64)> {
    let mut timed: Vec<(i64, u8, Source, Fact)> = Vec::new();
    for (i, hook) in hooks.iter().enumerate() {
        let at = t0 + span * i as i64 / hooks.len() as i64;
        timed.push((at, 0, Source::Hook, Fact::Hook(hook.clone())));
    }
    for (j, fact) in transcript.iter().enumerate() {
        let at = t0 + span * (2 * j as i64 + 1) / (2 * transcript.len() as i64);
        let fact = match fact {
            Fact::Turn { started, .. } => Fact::Turn { started: *started, ts_ms: Some(at) },
            other => other.clone(),
        };
        timed.push((at, 1, Source::Transcript, fact));
    }
    timed.sort_by_key(|(at, order, ..)| (*at, *order));
    timed.into_iter().map(|(at, _, source, fact)| (source, fact, at)).collect()
}

fn fold_session(session: Vec<(Source, Fact, i64)>) -> Tape {
    let mut tape = Tape::new();
    tape.put(Source::App, Fact::Started { hooked: true }, 1_000);
    tape.put(Source::Terminal, busy(true), 1_000);
    let mut last = 1_000;
    for (source, fact, at) in session {
        last = last.max(at);
        tape.put(source, fact, at);
    }
    tape.put(Source::Terminal, busy(false), last + 1);
    tape
}

fn through_the_disk(tape: &Tape, dir: &Path) -> PanelState {
    let file = dir.join("panel-log.jsonl");
    panel_store::save(&file, &tape.log).unwrap();
    panel_store::load(&file).replay()
}

#[test]
fn every_recorded_session_ends_done_and_replays_exactly_from_disk() {
    let sessions = [
        ("claude", CLAUDE_HOOKS, CLAUDE_TRANSCRIPT, transcript::Source::Claude),
        ("codex", CODEX_HOOKS, CODEX_TRANSCRIPT, transcript::Source::Codex),
        ("copilot", COPILOT_HOOKS, COPILOT_TRANSCRIPT, transcript::Source::Copilot),
        ("gemini", GEMINI_HOOKS, GEMINI_TRANSCRIPT, transcript::Source::Gemini),
    ];
    for (agent, hooks, transcript_text, source) in sessions {
        let hooks = hooks_of(agent, hooks);
        let facts = transcript_facts(source, transcript_text);
        assert!(hooks.len() >= 5, "{agent}: {} hooks", hooks.len());
        assert!(facts.len() >= 2, "{agent}: {} transcript facts", facts.len());
        let tape = fold_session(session_of(&hooks, &facts, 10_000, 60_000));
        let view = tape.state.view(120_000);
        assert_eq!(view.tone, Tone::Done, "{agent}");
        assert!(!view.turn.running, "{agent}");
        assert!(view.since >= 1, "{agent}: {} calls", view.since);
        assert_eq!(view.report, None, "{agent}");
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            through_the_disk(&tape, dir.path()),
            tape.state,
            "{agent}: replay(load(save(log)))"
        );
        assert_eq!(tape.log.replay(), tape.state, "{agent}: replay of the log");
    }
}

#[test]
fn the_claude_stop_message_becomes_the_end_block_the_card_reads() {
    let hooks = hooks_of("claude", CLAUDE_HOOKS);
    let kinds: Vec<&HookKind> = hooks.iter().map(|hook| &hook.kind).collect();
    assert!(matches!(kinds[0], HookKind::SessionStart));
    assert!(matches!(kinds[1], HookKind::TurnStarted));
    assert!(matches!(kinds[2], HookKind::ToolStarted { id: Some(id), .. } if id == "toolu_01"));
    assert!(matches!(kinds[3], HookKind::ToolPermission(_)));
    assert!(matches!(kinds[4], HookKind::PermissionNeeded));
    assert!(matches!(kinds[5], HookKind::ToolFinished { .. }));
    assert!(matches!(kinds[6], HookKind::TurnDone));
    assert!(matches!(kinds[7], HookKind::SessionEnd));
    let facts = transcript_facts(transcript::Source::Claude, CLAUDE_TRANSCRIPT);
    let tape = fold_session(session_of(&hooks, &facts, 10_000, 60_000));
    let want = EndBlock {
        done: vec!["made the file".into()],
        needs_you: vec!["nothing".into()],
        next: vec!["merge the branch".into()],
    };
    assert_eq!(tape.state.view(100_000).end_block, Some(want));
}

#[test]
fn a_permission_prompt_in_a_recorded_session_holds_the_tab_on_needs_you_until_its_call_ends() {
    for (agent, fixture) in [
        ("codex", CODEX_HOOKS),
        ("gemini", GEMINI_HOOKS),
        ("copilot", COPILOT_HOOKS),
        ("claude", CLAUDE_HOOKS),
    ] {
        let hooks = hooks_of(agent, fixture);
        let asked = hooks
            .iter()
            .position(|h| {
                matches!(h.kind, HookKind::PermissionNeeded | HookKind::ToolPermission(_))
            })
            .unwrap();
        let ended =
            hooks.iter().position(|h| matches!(h.kind, HookKind::ToolFinished { .. })).unwrap();
        let mut tape = Tape::new();
        tape.put(Source::App, Fact::Started { hooked: true }, 0);
        for (i, hook) in hooks[..=asked].iter().enumerate() {
            tape.put(Source::Hook, Fact::Hook(hook.clone()), 1_000 + 1_000 * i as i64);
        }
        for second in 10..70 {
            tape.put(Source::Terminal, Fact::Output, second * 1_000);
        }
        assert_eq!(tape.state.view(70_500).tone, Tone::Needs, "{agent}");
        for hook in &hooks[asked + 1..=ended] {
            tape.put(Source::Hook, Fact::Hook(hook.clone()), 71_000);
        }
        assert_eq!(tape.state.view(71_100).tone, Tone::Working, "{agent}");
    }
}

fn pick<T: Clone>(rng: &mut Rng, from: &[T]) -> T {
    from[rng.below(from.len() as u64) as usize].clone()
}

fn random_fact(rng: &mut Rng, n: u64) -> (Source, Fact) {
    let digests = [
        digest("working", "- Nothing"),
        digest("needs_you", "- Nothing"),
        digest("needs_you", "- Pick one"),
        digest("waiting", "- Nothing"),
        digest("blocked", "- Nothing"),
        digest("done", "- Nothing"),
    ];
    let end_block =
        "Fixed it.\n\n**Done**\n- the fix\n\n**Needs you**\n- nothing\n\n**Next**\n- merge";
    let kinds = [
        HookKind::TurnStarted,
        HookKind::TurnDone,
        HookKind::TurnFailed("boom".into()),
        HookKind::IdleWaiting,
        HookKind::PermissionNeeded,
        HookKind::QuestionAsked,
        HookKind::SessionStart,
        HookKind::SessionEnd,
        HookKind::ToolStarted {
            key: format!("Bash\nls {}", n % 5),
            id: Some(format!("c{}", n % 5)),
        },
        HookKind::ToolFinished {
            key: format!("Bash\nls {}", n % 5),
            id: Some(format!("c{}", n % 5)),
        },
        HookKind::ToolPermission(format!("Bash\nls {}", n % 5)),
    ];
    match rng.below(30) {
        0..=5 => {
            let kind = pick(rng, &kinds);
            let fact = if matches!(kind, HookKind::TurnDone) && rng.percent(50) {
                hook_saying(kind, end_block)
            } else {
                hook_fact(kind)
            };
            (Source::Hook, fact)
        }
        6 => (
            Source::Transcript,
            Fact::Turn { started: rng.percent(50), ts_ms: Some(rng.below(100_000) as i64) },
        ),
        7 => (Source::Transcript, Fact::Said { tail: format!("said {n}") }),
        8 => (
            Source::Transcript,
            Fact::ToolOpened { id: format!("c{}", n % 5), name: "Bash".into() },
        ),
        9 => (Source::Transcript, Fact::ToolClosed { id: format!("c{}", n % 5) }),
        10 => (Source::Transcript, Fact::AskOpened { summary: format!("q{n}"), n: 1 }),
        11 => (Source::Transcript, Fact::AskClosed),
        12 => (Source::Transcript, Fact::PermissionOpened { tool: "Bash: rm".into() }),
        13 => (Source::Transcript, Fact::PermissionClosed),
        14 => (Source::Terminal, busy(rng.percent(50))),
        15 | 16 => (Source::Terminal, Fact::Output),
        17 => (Source::Terminal, Fact::Terminal(TerminalSignal::Bell)),
        18 => (
            Source::Process,
            Fact::Process { alive: !rng.percent(10), exit_code: Some(rng.below(3) as i32) },
        ),
        19 => (
            Source::Report,
            Fact::ReportSaved { saved_ms: rng.below(100_000) as i64, digest: pick(rng, &digests) },
        ),
        20 => (
            Source::Job,
            Fact::Job(job::Report {
                done: Some((n % 40) as f64),
                total: Some(40.0),
                ..job::Report::named(["import", "deploy"][(n % 2) as usize])
            }),
        ),
        21 => (
            Source::Link,
            Fact::Link(Link {
                url: format!("https://github.com/o/r/pull/{}", n % 4),
                title: format!("PR {n}"),
                note: None,
                first_seen_ms: n as i64,
                title_from: TitleFrom::Seen,
                state: LinkState::Open,
            }),
        ),
        22 => (
            Source::Git,
            Fact::Git(GitFacts {
                changed: (n % 4) as u32,
                head: Some(format!("h{}", n % 3)),
                ..GitFacts::default()
            }),
        ),
        23 => (
            Source::Pr,
            Fact::Pr(PrFacts { number: 7, state: PrWord::Open, checks: Some(ChecksWord::Running) }),
        ),
        24 => (
            Source::Tidewatch,
            Fact::Check {
                name: ["tests", "fmt"][(n % 2) as usize].into(),
                passed: rng.percent(50),
            },
        ),
        25 => (
            Source::App,
            Fact::Answered {
                kind: if rng.percent(50) { AskKind::Question } else { AskKind::Permission },
            },
        ),
        26 => (
            Source::Tidewatch,
            Fact::StepMark { step: format!("step {}", n % 6), by: StepBy::Tidewatch },
        ),
        27 => (Source::App, Fact::Restart),
        28 => (Source::App, Fact::Started { hooked: rng.percent(70) }),
        _ => (Source::Terminal, Fact::Terminal(TerminalSignal::TitleChanged)),
    }
}

fn random_log(seed: u64, backward: bool) -> Log {
    let mut rng = Rng::new(seed);
    let len = 1 + rng.below(160);
    let mut log = Log::new();
    let mut now = 1_000_000i64;
    for n in 0..len {
        let (source, fact) = random_fact(&mut rng, n);
        let step = rng.below(4_000) as i64;
        now += if backward && rng.percent(25) { -(rng.below(500_000) as i64) } else { step };
        log.append(source, fact, now);
    }
    log
}

const SEEDS: u64 = 400;

#[test]
fn folding_the_first_part_and_then_the_rest_equals_folding_everything() {
    for seed in 0..SEEDS {
        let log = random_log(seed, seed % 2 == 0);
        let whole = log.replay();
        let mut rng = Rng::new(seed + 10_000);
        let cut = rng.below(log.len() as u64 + 1) as usize;
        let mut split = PanelState::replay(&log.events()[..cut]);
        for event in &log.events()[cut..] {
            split.apply(event);
        }
        assert_eq!(split, whole, "seed {seed} cut at {cut}");
        let now = 1_000_000 + rng.below(2_000_000) as i64;
        assert_eq!(split.view(now), whole.view(now), "seed {seed}: the view at {now}");
    }
}

#[test]
fn folding_an_event_again_never_changes_the_state() {
    for seed in 0..SEEDS {
        let log = random_log(seed, true);
        let whole = log.replay();
        let mut again = whole.clone();
        for event in log.events() {
            assert!(!again.apply(event).changed, "seed {seed} seq {}", event.seq);
        }
        assert_eq!(again, whole, "seed {seed}");
    }
}

#[test]
fn a_trimmed_log_with_its_checkpoint_replays_to_the_same_state() {
    for seed in 0..SEEDS {
        let mut log = random_log(seed, seed % 3 == 0);
        let whole = log.replay();
        let mut rng = Rng::new(seed + 20_000);
        for keep in [rng.below(40) as usize, rng.below(10) as usize, 0] {
            log.trim(keep);
            assert_eq!(log.replay(), whole, "seed {seed} keep {keep}");
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("panel-log.jsonl");
        panel_store::save(&file, &log).unwrap();
        assert_eq!(panel_store::load(&file).replay(), whole, "seed {seed}: through the disk");
    }
}

#[test]
fn a_wall_clock_that_goes_backward_never_reorders_anything() {
    for seed in 0..SEEDS {
        let raw = random_log(seed, true);
        let seqs: Vec<u64> = raw.events().iter().map(|e| e.seq).collect();
        assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]), "seed {seed}");
        let mut high = i64::MIN;
        let clamped: Vec<Event> = raw
            .events()
            .iter()
            .map(|event| {
                high = high.max(event.at_ms);
                Event { at_ms: high, ..event.clone() }
            })
            .collect();
        assert_eq!(
            raw.replay(),
            PanelState::replay(&clamped),
            "seed {seed}: a backward step reads as no time passing"
        );
        let state = raw.replay();
        assert_eq!(state.clock_ms(), high.max(0), "seed {seed}");
        assert_eq!(state.shown_seq(), *seqs.last().unwrap(), "seed {seed}");
    }
}

#[test]
fn a_log_cut_anywhere_loads_every_whole_line_before_the_cut() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    for seed in 0..60 {
        let log = random_log(seed, false);
        panel_store::save(&file, &log).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let mut rng = Rng::new(seed + 30_000);
        let cut = rng.below(text.len() as u64 + 1) as usize;
        let mut cut_at = cut;
        while !text.is_char_boundary(cut_at) {
            cut_at -= 1;
        }
        std::fs::write(&file, &text[..cut_at]).unwrap();
        let whole_lines = text[..cut_at].matches('\n').count();
        let back = panel_store::load(&file);
        assert_eq!(back.events(), &log.events()[..whole_lines], "seed {seed} cut at {cut_at}");
        assert_eq!(back.replay(), PanelState::replay(&log.events()[..whole_lines]), "seed {seed}");
    }
}

#[test]
fn the_event_cap_holds_for_a_long_random_run_and_the_state_survives_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel-log.jsonl");
    let mut rng = Rng::new(7);
    let mut live = Log::new();
    let mut now = 5_000_000;
    let writer = Writer::new(file.clone());
    for n in 0..(CAP_EVENTS as u64 * 3 + 17) {
        let (source, fact) = random_fact(&mut rng, n);
        now += rng.below(3_000) as i64;
        writer.append(live.append(source, fact, now));
    }
    drop(writer);
    let lines = std::fs::read_to_string(&file).unwrap().lines().count();
    assert!(lines <= CAP_EVENTS, "{lines} lines");
    let back = panel_store::load(&file);
    assert_eq!(back.next_seq(), live.next_seq());
    assert_eq!(back.replay(), live.replay());
}

struct Step {
    at: i64,
    source: Source,
    fact: Fact,
    hook: bool,
}

/// Two turns of an agent that reports by hooks, with the backups a healthy app also sees.
fn script() -> (Vec<Step>, Vec<i64>) {
    let mut steps = Vec::new();
    let mut midpoints = Vec::new();
    let mut add = |at: i64, source: Source, fact: Fact| {
        let hook = source == Source::Hook;
        steps.push(Step { at, source, fact, hook });
    };
    for turn in 0..2i64 {
        let t = 10_000 + turn * 60_000;
        add(t, Source::Terminal, busy(true));
        add(t, Source::Hook, hook_fact(HookKind::TurnStarted));
        add(t + 400, Source::Transcript, Fact::Turn { started: true, ts_ms: Some(t) });
        for call in 0..3i64 {
            let c = t + 1_000 + call * 4_000;
            let id = format!("c{turn}{call}");
            let key = format!("Bash\nls {turn}{call}");
            add(
                c,
                Source::Hook,
                hook_fact(HookKind::ToolStarted { key: key.clone(), id: Some(id.clone()) }),
            );
            add(
                c + 300,
                Source::Transcript,
                Fact::ToolOpened { id: id.clone(), name: "Bash".into() },
            );
            add(c + 1_000, Source::Terminal, Fact::Output);
            add(c + 2_000, Source::Terminal, Fact::Output);
            add(
                c + 2_500,
                Source::Hook,
                hook_fact(HookKind::ToolFinished { key, id: Some(id.clone()) }),
            );
            add(c + 2_800, Source::Transcript, Fact::ToolClosed { id });
            add(c + 2_000, Source::Process, Fact::Process { alive: true, exit_code: None });
        }
        midpoints.push(t + 9_000);
        let end = t + 16_000;
        add(end - 200, Source::Transcript, Fact::Said { tail: format!("turn {turn} done") });
        add(end, Source::Hook, hook_fact(HookKind::TurnDone));
        add(end, Source::Terminal, busy(false));
        add(end + 300, Source::Transcript, Fact::Turn { started: false, ts_ms: Some(end) });
        midpoints.push(end + 2_000);
        midpoints.push(end + 25_000);
    }
    steps.sort_by_key(|step| step.at);
    (steps, midpoints)
}

/// Folds `steps` in order and reads the tone at each probe time, after the steps up to it.
fn tones_at(steps: &[&Step], probes: &[i64], restart_at: Option<i64>) -> Vec<(Tone, bool)> {
    let mut tape = Tape::new();
    tape.put(Source::App, Fact::Started { hooked: true }, 1_000);
    let mut read = Vec::new();
    let mut restarted = restart_at.is_none();
    let mut next = 0;
    for step in steps {
        while next < probes.len() && probes[next] < step.at {
            let view = tape.state.view(probes[next]);
            read.push((view.tone, view.turn.running));
            next += 1;
        }
        if !restarted && restart_at.is_some_and(|at| at <= step.at) {
            restarted = true;
            tape.put(Source::App, Fact::Restart, step.at);
        }
        tape.put(step.source, step.fact.clone(), step.at);
    }
    for probe in &probes[next..] {
        let view = tape.state.view(*probe);
        read.push((view.tone, view.turn.running));
    }
    read
}

#[test]
fn with_a_third_of_the_hooks_lost_the_view_converges_to_the_same_tone() {
    let (steps, probes) = script();
    let all: Vec<&Step> = steps.iter().collect();
    let reference = tones_at(&all, &probes, None);
    let want = [
        (Tone::Working, true),
        (Tone::Done, false),
        (Tone::Done, false),
        (Tone::Working, true),
        (Tone::Done, false),
        (Tone::Done, false),
    ];
    assert_eq!(reference, want);
    let mut lost = 0;
    for seed in 0..200 {
        let mut rng = Rng::new(seed);
        let kept: Vec<&Step> = steps.iter().filter(|s| !s.hook || !rng.percent(30)).collect();
        lost += steps.len() - kept.len();
        assert_eq!(tones_at(&kept, &probes, None), reference, "seed {seed}");
    }
    assert!(lost > 200 * 3, "the harness really dropped hooks: {lost}");
}

#[test]
fn with_every_hook_lost_the_backup_facts_alone_carry_the_turns() {
    let (steps, probes) = script();
    let all: Vec<&Step> = steps.iter().collect();
    let reference = tones_at(&all, &probes, None);
    let silent: Vec<&Step> = steps.iter().filter(|s| !s.hook).collect();
    assert_eq!(tones_at(&silent, &probes, None), reference);
}

#[test]
fn a_listener_gap_with_a_restart_after_it_converges_too() {
    let (steps, probes) = script();
    let all: Vec<&Step> = steps.iter().collect();
    let reference = tones_at(&all, &probes, None);
    for (from, to) in [(10_000, 30_000), (20_000, 70_000), (50_000, 100_000), (14_000, 15_000)] {
        let gap: Vec<&Step> =
            steps.iter().filter(|s| !(s.hook && s.at >= from && s.at < to)).collect();
        assert_eq!(tones_at(&gap, &probes, Some(to)), reference, "gap {from}..{to}");
    }
}

#[test]
fn a_restart_in_the_middle_of_a_turn_converges_with_all_hooks_kept() {
    let (steps, probes) = script();
    let all: Vec<&Step> = steps.iter().collect();
    let reference = tones_at(&all, &probes, None);
    for restart_at in [11_000, 15_000, 19_000, 70_000, 75_000] {
        assert_eq!(tones_at(&all, &probes, Some(restart_at)), reference, "restart at {restart_at}");
    }
}

#[test]
fn a_dead_process_backs_up_a_stop_hook_that_never_came() {
    for (code, tone) in [(Some(0), Tone::Done), (Some(1), Tone::Failed), (None, Tone::Failed)] {
        let mut tape = Tape::new();
        tape.put(Source::App, Fact::Started { hooked: true }, 0);
        tape.put(Source::Terminal, busy(true), 1_000);
        tape.put(Source::Transcript, Fact::Turn { started: true, ts_ms: Some(1_000) }, 1_300);
        assert_eq!(tape.state.view(2_000).tone, Tone::Working);
        tape.put(Source::Process, Fact::Process { alive: false, exit_code: code }, 5_000);
        let view = tape.state.view(5_100);
        assert_eq!((view.tone, view.turn.running), (tone, false), "{code:?}");
    }
}

#[test]
fn the_report_is_weighed_against_the_replayed_facts_the_same_as_against_the_live_ones() {
    let mut tape = Tape::new();
    tape.put(Source::App, Fact::Started { hooked: true }, 0);
    tape.put(Source::Hook, hook_fact(HookKind::TurnStarted), 1_000);
    tape.put(
        Source::Report,
        Fact::ReportSaved { saved_ms: 1_500, digest: digest("working", "- Nothing") },
        1_500,
    );
    tape.put(Source::Hook, hook_fact(HookKind::TurnDone), 2_000);
    let dir = tempfile::tempdir().unwrap();
    let replayed = through_the_disk(&tape, dir.path());
    for now in [2_100, 700_000] {
        let live = tape.state.view(now);
        assert_eq!(live, replayed.view(now));
        assert_eq!(live.tone, Tone::Done);
        assert!(matches!(live.report, Some(ReportMark::OutOfDate(_))));
    }
}
