use super::*;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/progress")
        .join(format!("{name}.md"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ledger(name: &str) -> Ledger {
    Ledger::parse(&fixture(name))
}

fn date(text: &str) -> Date {
    Date::parse(text).unwrap()
}

fn texts(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.text.as_str()).collect()
}

fn phase_states(ledger: &Ledger) -> Vec<PhaseState> {
    ledger.checklist.iter().map(Phase::state).collect()
}

#[test]
fn a_ledger_just_written_has_its_first_phase_under_way() {
    let l = ledger("start");
    assert!(l.goal.starts_with("Add a slugify(text) function to lib.py"));
    assert_eq!(l.status.phase, "1");
    assert_eq!(l.status.state, Some(State::Working));
    assert!(l.status.note.starts_with("Writing ledger"));
    assert_eq!(l.checklist.len(), 4);
    assert_eq!(
        l.checklist.iter().map(|p| (p.number, p.title.as_str())).collect::<Vec<_>>(),
        [
            (Some(1), "Understand and decide"),
            (Some(2), "Build"),
            (Some(3), "Gate"),
            (Some(4), "Close out")
        ]
    );
    assert_eq!(l.counts(), Counts { done: 0, total: 9, skipped: 0 });
    assert_eq!(
        phase_states(&l),
        [PhaseState::Doing, PhaseState::Todo, PhaseState::Todo, PhaseState::Todo]
    );
    assert_eq!(l.current_phase().map(|p| p.title.as_str()), Some("Understand and decide"));
    assert_eq!(texts(&l.decisions), ["(none yet)"]);
    assert_eq!(real(&l.decisions).count(), 0);
    assert_eq!(real(&l.next).count(), 1);
    assert!(!l.has_question());
    assert!(!l.is_blocked());
    assert_eq!(l.summary(), "saved: phase 1 Understand and decide, 0/9 done");
}

#[test]
fn a_ledger_in_the_middle_counts_what_is_done_and_keeps_every_decision_with_its_why() {
    let l = ledger("middle");
    assert_eq!(l.counts(), Counts { done: 4, total: 9, skipped: 0 });
    assert_eq!(
        phase_states(&l),
        [PhaseState::Done, PhaseState::Doing, PhaseState::Todo, PhaseState::Todo]
    );
    assert_eq!(l.current_phase().map(|p| p.title.as_str()), Some("Gate"));
    assert_eq!(l.decisions.len(), 4);
    let separator = &l.decisions[1];
    assert!(separator.text.starts_with("Separator = any run of non-letter"));
    assert!(separator.text.contains("`[\\W_]+`"));
    assert_eq!(separator.why.as_deref().map(|why| why.starts_with("simplest rule")), Some(true));
    assert!(!l.has_question(), "'Nothing. Open choice ...' is no question");
    assert_eq!(l.summary(), "saved: phase 3 Gate, 4/9 done");
}

#[test]
fn a_finished_ledger_skips_the_skipped_keeps_the_section_it_does_not_know_and_has_a_question() {
    let l = ledger("finished");
    assert_eq!(l.status.state, Some(State::Done));
    assert_eq!(l.counts(), Counts { done: 8, total: 8, skipped: 1 });
    assert_eq!(l.checklist[2].counts(), Counts { done: 1, total: 1, skipped: 1 });
    assert!(phase_states(&l).iter().all(|state| *state == PhaseState::Done));
    let skipped = &l.checklist[2].items[0];
    assert_eq!(skipped.mark, Mark::Skipped);
    assert_eq!(skipped.why.as_deref(), Some("no formatter or linter config in this repo"));
    assert_eq!(l.extra.len(), 1);
    assert_eq!(l.extra[0].name, "Done");
    assert_eq!(l.extra[0].lines.len(), 1);
    assert!(l.extra[0].lines[0].contains("ec84557"));
    assert_eq!(l.verified.len(), 4);
    assert_eq!(l.verified[0].text, "`PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v` in repo");
    assert_eq!(
        l.verified[0].why.as_deref(),
        Some("Ran 8 tests, OK (test_greet + 7 slugify tests)")
    );
    assert!(l.has_question());
    assert!(!l.is_blocked(), "a done ledger is not blocked");
    assert_eq!(l.summary(), "saved: phase 4 Close out, 8/8 done");
}

#[test]
fn a_scheduled_only_ledger_has_every_kind_of_when_and_its_sub_steps() {
    let l = ledger("scheduled");
    assert_eq!(l.checklist, Vec::<Phase>::new());
    assert_eq!(l.status.phase_number(), Some(1));
    assert_eq!(l.current_phase(), None);
    assert_eq!(
        l.scheduled.iter().map(|s| s.when.clone()).collect::<Vec<_>>(),
        [
            When::Date(date("2026-11-01")),
            When::On("Beta sync is close to shipping".into()),
            When::About(date("2026-11-07")),
            When::On("next SDK PR needs bot approval".into()),
            When::Parked("until collection writes move off the old backend".into()),
        ]
    );
    let first = &l.scheduled[0];
    assert_eq!(first.what, "Switch off the old webhook (#101)");
    assert_eq!(first.why.as_deref(), Some("the main remaining milestone"));
    assert_eq!(first.steps.len(), 4);
    assert_eq!(first.steps[0].mark, Mark::Doing);
    assert_eq!(first.step_counts(), Counts { done: 0, total: 4, skipped: 0 });
    assert_eq!(l.scheduled[2].what, "Count which apps still send available_on=alpha");
    assert_eq!(l.scheduled[2].steps, Vec::<Item>::new());
    assert!(!l.has_question());
    assert_eq!(l.summary(), "saved: phase 1 (switch off the old webhook), 5 scheduled");
}

#[test]
fn the_you_kind_blocked_items_and_skipped_phases_are_told_apart() {
    let l = ledger("kinds");
    assert_eq!(l.status.state, Some(State::Waiting));
    assert_eq!(l.scheduled.len(), 5);
    let last = l.scheduled.last().unwrap();
    assert_eq!(last.when, When::You);
    assert_eq!(last.what, "Choose whether the audit rows are kept for 30 or 90 days");
    assert_eq!(last.why.as_deref(), Some("only the owner can decide this"));
    assert_eq!(phase_states(&l), [PhaseState::Done, PhaseState::Blocked, PhaseState::Skipped]);
    assert_eq!(l.counts(), Counts { done: 3, total: 6, skipped: 2 });
    assert!(l.is_blocked(), "a blocked item blocks while the agent only waits");
    assert!(l.has_question());
    assert_eq!(l.current_phase().map(|p| p.title.as_str()), Some("Build"));
    assert_eq!(l.decisions[1].text, "Run `cargo test -- --nocapture` in CI");
    assert_eq!(l.decisions[1].why.as_deref(), Some("the log shows which case failed"));
    assert_eq!(l.summary(), "saved: phase 2 Build, 3/6 done, 5 scheduled");
}

#[test]
fn messy_input_is_read_as_far_as_it_makes_sense_and_the_rest_is_kept() {
    let l = ledger("messy");
    assert_eq!(l.goal, "Ship the search box.");
    assert_eq!(l.status.phase, "Build");
    assert_eq!(l.status.state, Some(State::Working));
    assert_eq!(l.status.note, "wiring the endpoint: second colon here");
    assert_eq!(l.status.other, ["extra: kept as it is"]);

    assert_eq!(l.checklist.len(), 3);
    assert_eq!((l.checklist[0].number, l.checklist[0].title.as_str()), (None, ""));
    assert_eq!(l.checklist[0].items.len(), 1);
    let build = &l.checklist[1];
    assert_eq!((build.number, build.title.as_str()), (None, "Build"));
    assert_eq!(
        build.items.iter().map(|item| item.mark).collect::<Vec<_>>(),
        [Mark::Done, Mark::Doing, Mark::Blocked]
    );
    assert_eq!(build.items[1].text, "Star bullet in progress");
    assert_eq!(build.items[1].why.as_deref(), Some("why now"));
    assert_eq!(
        build.text,
        ["- A plain bullet that is no item", "Some prose line under the heading"]
    );
    assert_eq!((l.checklist[2].number, l.checklist[2].title.as_str()), (Some(3), "Gate"));
    assert_eq!(l.current_phase_index(), Some(1), "the phase text names Build");

    assert_eq!(
        texts(&l.decisions),
        [
            "Use the existing index",
            "Continued text of a line that is not a bullet",
            "Second decision indented continuation of the second",
        ]
    );
    assert_eq!(l.decisions[0].why.as_deref(), Some("it is already warm"));

    assert_eq!(
        l.extra,
        [Extra {
            name: "Done".into(),
            lines: vec!["- shipped the first half".into(), "- and the rest".into()]
        }]
    );

    assert_eq!(l.scheduled.len(), 2);
    assert_eq!(l.scheduled[0].when, When::Unknown("2026-13-45".into()));
    assert_eq!(l.scheduled[0].why.as_deref(), Some("stays as written"));
    assert_eq!(l.scheduled[1].when, When::Date(date("2026-12-01")));
    assert_eq!(l.scheduled[1].what, "a bullet without a box");
    assert_eq!(l.scheduled_text, ["Nothing else is scheduled."]);
    assert!(!l.has_question());
}

#[test]
fn text_before_the_first_heading_is_kept_and_written_first() {
    let l = Ledger::parse("Some words first\n\n# Goal\nDo it\n");
    assert_eq!(l.goal, "Do it");
    assert_eq!(l.extra, [Extra { name: String::new(), lines: vec!["Some words first".into()] }]);
    assert!(l.to_markdown().starts_with("Some words first\n\n# Goal\nDo it\n"));
}

#[test]
fn heading_names_match_without_case_and_a_second_section_adds_to_the_first() {
    let l = Ledger::parse(
        "# GOAL:\nFirst\n\n## decisions\n- a\n\n## Decisions\n- b\n\n## NEXT STEPS\n- c\n",
    );
    assert_eq!(l.goal, "First");
    assert_eq!(texts(&l.decisions), ["a", "b"]);
    assert_eq!(texts(&l.next), ["c"]);
}

#[test]
fn every_fixture_writes_back_to_the_text_it_was_read_from() {
    for name in ["start", "middle", "kinds"] {
        let text = fixture(name);
        assert_eq!(ledger(name).to_markdown(), format!("{}\n", text.trim_end()), "{name}");
    }
}

#[test]
fn writing_then_reading_gives_the_same_ledger_for_every_fixture() {
    for name in ["start", "middle", "finished", "scheduled", "kinds", "messy"] {
        let once = ledger(name);
        let text = once.to_markdown();
        let twice = Ledger::parse(&text);
        assert_eq!(twice, once, "{name}");
        assert_eq!(twice.to_markdown(), text, "{name}");
    }
}

#[test]
fn dashes_of_any_kind_cut_a_why_but_not_inside_backticks() {
    let cases = [
        ("plain", ("plain", None)),
        ("a -- b", ("a", Some("b"))),
        ("a \u{2014} b", ("a", Some("b"))),
        ("a \u{2013} b", ("a", Some("b"))),
        ("a -- b -- c", ("a", Some("b -- c"))),
        ("`x -- y` -- z", ("`x -- y`", Some("z"))),
        ("`cargo test -- --nocapture`", ("`cargo test -- --nocapture`", None)),
        ("a--b", ("a--b", None)),
        ("a -- ", ("a", None)),
        ("a -- -- b", ("a", Some("-- b"))),
        ("", ("", None)),
    ];
    for (input, (text, why)) in cases {
        assert_eq!(split_why(input), (text.to_owned(), why.map(str::to_owned)), "{input:?}");
    }
}

#[test]
fn when_reads_each_kind_and_keeps_what_it_cannot_read() {
    let cases = [
        ("2026-11-01", When::Date(date("2026-11-01"))),
        (" 2026-11-01 ", When::Date(date("2026-11-01"))),
        ("~2026-11-07", When::About(date("2026-11-07"))),
        ("~ 2026-11-07", When::About(date("2026-11-07"))),
        ("on: the partner replies", When::On("the partner replies".into())),
        ("On: Capital", When::On("Capital".into())),
        ("on:", When::On(String::new())),
        ("parked: until the rewrite lands", When::Parked("until the rewrite lands".into())),
        ("you", When::You),
        ("You", When::You),
        ("~soon", When::Unknown("~soon".into())),
        ("2026-02-30", When::Unknown("2026-02-30".into())),
        ("next week", When::Unknown("next week".into())),
        ("you decide", When::Unknown("you decide".into())),
        ("\u{e9}\u{e9}\u{e9}\u{e9}", When::Unknown("\u{e9}\u{e9}\u{e9}\u{e9}".into())),
        ("", When::Unknown(String::new())),
    ];
    for (input, want) in cases {
        assert_eq!(When::parse(input), want, "{input:?}");
    }
}

#[test]
fn when_prints_the_way_it_is_read() {
    for text in ["2026-11-01", "~2026-11-07", "on: x happens", "parked: until y", "you", "someday"]
    {
        assert_eq!(When::parse(text).to_string(), text);
    }
}

#[test]
fn due_and_days_until_count_from_today() {
    let today = date("2026-10-17");
    let cases = [
        (When::parse("2026-10-16"), true, Some(-1)),
        (When::parse("2026-10-17"), true, Some(0)),
        (When::parse("2026-10-18"), false, Some(1)),
        (When::parse("2026-11-01"), false, Some(15)),
        (When::parse("~2026-10-10"), true, Some(-7)),
        (When::parse("~2026-11-07"), false, Some(21)),
        (When::parse("on: it ships"), false, None),
        (When::parse("parked: later"), false, None),
        (When::parse("you"), false, None),
        (When::parse("whenever"), false, None),
    ];
    for (when, due, days) in cases {
        assert_eq!(when.due_on(today), due, "{when}");
        assert_eq!(when.days_until(today), days, "{when}");
    }
}

#[test]
fn done_and_total_leave_out_skipped_items_in_the_whole_list_and_in_each_phase() {
    let l = Ledger::parse(
        "# Goal\ng\n\n## Checklist\n### 1. A\n- [x] a\n- [-] b\n- [ ] c\n### 2. B\n- [-] d\n- [-] e\n### 3. C\n- [~] f\n- [!] g\n- [x] h\n",
    );
    assert_eq!(l.checklist[0].counts(), Counts { done: 1, total: 2, skipped: 1 });
    assert_eq!(l.checklist[1].counts(), Counts { done: 0, total: 0, skipped: 2 });
    assert_eq!(l.checklist[2].counts(), Counts { done: 1, total: 3, skipped: 0 });
    assert_eq!(l.counts(), Counts { done: 2, total: 5, skipped: 3 });
}

#[test]
fn a_phase_is_done_blocked_skipped_under_way_or_not_started() {
    let cases = [
        ("", PhaseState::Todo),
        ("- [ ] a\n- [ ] b", PhaseState::Todo),
        ("- [~] a\n- [ ] b", PhaseState::Doing),
        ("- [x] a\n- [ ] b", PhaseState::Doing),
        ("- [x] a\n- [x] b", PhaseState::Done),
        ("- [x] a\n- [-] b", PhaseState::Done),
        ("- [-] a\n- [-] b", PhaseState::Skipped),
        ("- [x] a\n- [!] b", PhaseState::Blocked),
        ("- [!] a\n- [x] b\n- [-] c", PhaseState::Blocked),
    ];
    for (items, want) in cases {
        let phases = checklist_phases(&format!("### 1. P\n{items}\n"));
        assert_eq!(phases[0].state(), want, "{items:?}");
    }
}

#[test]
fn the_current_phase_follows_the_status_then_the_first_unfinished_one() {
    let body = "## Checklist\n### 1. Plan\n- [x] a\n### 2. Build\n- [x] b\n- [ ] c\n### 3. Ship\n- [ ] d\n";
    let cases = [
        ("phase: 3", Some(2)),
        ("phase: 1", Some(0)),
        ("phase: Phase 2", Some(1)),
        ("phase: 2 (Build, nearly)", Some(1)),
        ("phase: ship it", Some(2)),
        ("phase: 9", Some(1)),
        ("phase: elsewhere", Some(1)),
        ("", Some(1)),
    ];
    for (status, want) in cases {
        let l = Ledger::parse(&format!("# Goal\ng\n\n## Status\n{status}\n\n{body}"));
        assert_eq!(l.current_phase_index(), want, "{status:?}");
    }
    let finished =
        Ledger::parse("# Goal\ng\n\n## Checklist\n### 1. Plan\n- [x] a\n### 2. Ship\n- [x] b\n");
    assert_eq!(finished.current_phase_index(), Some(1));
    let none = Ledger::parse("# Goal\ng\n");
    assert_eq!(none.current_phase_index(), None);
    let unnumbered = Ledger::parse(
        "# Goal\ng\n\n## Status\nphase: 2\n\n## Checklist\n### Plan\n- [x] a\n### Ship\n- [ ] b\n",
    );
    assert_eq!(unnumbered.current_phase_index(), Some(1));
}

#[test]
fn a_question_is_anything_in_needs_you_that_is_not_a_placeholder() {
    let cases = [
        ("", false),
        ("- nothing", false),
        ("- Nothing", false),
        ("- Nothing needs you today.", false),
        ("- nothing so far", false),
        ("- (none yet)", false),
        ("- None", false),
        ("- n/a", false),
        ("- Nothing. Open choice is noted in the report.", false),
        ("- Pick 30 or 90 days", true),
        ("- None of the options fit, which one now?", true),
        ("- Nothing\n- But also this", true),
        ("- Approve the migration -- it drops a column", true),
    ];
    for (needs, want) in cases {
        let l = Ledger::parse(&format!("# Goal\ng\n\n## Needs you\n{needs}\n"));
        assert_eq!(l.has_question(), want, "{needs:?}");
    }
}

#[test]
fn the_owner_is_awaited_by_a_question_or_a_waiting_or_blocked_state_unless_the_work_is_done() {
    let cases = [
        ("state: waiting", "- Pick one", true),
        ("state: waiting", "- Nothing", true),
        ("state: blocked", "- Nothing", true),
        ("state: working", "- Pick one", true),
        ("", "- Pick one", true),
        ("state: working", "- Nothing", false),
        ("", "- Nothing", false),
        ("state: done", "- Pick one", false),
        ("state: done", "- Nothing", false),
    ];
    for (status, needs, want) in cases {
        let l =
            Ledger::parse(&format!("# Goal\ng\n\n## Status\n{status}\n\n## Needs you\n{needs}\n"));
        assert_eq!(l.awaits_owner(), want, "{status:?} {needs:?}");
    }
}

#[test]
fn blocked_means_the_agent_says_so_or_an_item_is_and_the_work_is_not_done() {
    let cases = [
        ("state: blocked", "- [ ] a", true),
        ("state: working", "- [!] a -- stuck", true),
        ("state: waiting", "- [!] a", true),
        ("state: done", "- [!] a", false),
        ("state: working", "- [~] a", false),
        ("", "- [!] a", true),
        ("", "- [ ] a", false),
    ];
    for (status, items, want) in cases {
        let l = Ledger::parse(&format!(
            "# Goal\ng\n\n## Status\n{status}\n\n## Checklist\n### 1. P\n{items}\n"
        ));
        assert_eq!(l.is_blocked(), want, "{status:?} {items:?}");
    }
}

#[test]
fn the_state_word_is_read_loosely() {
    let cases = [
        ("working", Some(State::Working)),
        ("Working", Some(State::Working)),
        ("working | waiting | blocked | done", Some(State::Working)),
        ("in progress", Some(State::Working)),
        ("waiting on you", Some(State::Waiting)),
        ("blocked", Some(State::Blocked)),
        ("done", Some(State::Done)),
        ("completed", Some(State::Done)),
        ("asleep", None),
        ("", None),
    ];
    for (text, want) in cases {
        assert_eq!(State::parse(text), want, "{text:?}");
    }
}

#[test]
fn the_summary_names_the_phase_the_counts_and_the_scheduled_items() {
    let cases = [
        ("# Goal\ng\n", "saved"),
        ("# Goal\ng\n\n## Status\nphase: phase 4 Wrap up\n", "saved: phase 4 Wrap up"),
        (
            "# Goal\ng\n\n## Checklist\n### 2. Build\n- [x] a\n- [ ] b\n- [-] c\n\n## Scheduled\n- [ ] you -- x\n",
            "saved: phase 2 Build, 1/2 done, 1 scheduled",
        ),
        (
            "# Goal\ng\n\n## Scheduled\n- [ ] you -- x\n- [ ] ~2026-11-07 -- y\n",
            "saved: 2 scheduled",
        ),
        ("# Goal\ng\n\n## Checklist\n### Build\n- [ ] a\n", "saved: phase Build, 0/1 done"),
    ];
    for (text, want) in cases {
        assert_eq!(Ledger::parse(text).summary(), want, "{text:?}");
    }
}

#[test]
fn only_a_missing_goal_is_a_problem() {
    let empty = [
        "",
        "   \n",
        "## Status\nphase: 1\n",
        "# Goal\n\n## Status\nstate: working\n",
        "# Goal\n   \n",
    ];
    for text in empty {
        assert_eq!(Ledger::parse(text).problems().len(), 1, "{text:?}");
    }
    let fine = ["# Goal\nx\n", "# Goal\nx\n\n## Checklist\n- [ ] only\n", "nonsense\n\n# Goal\nx"];
    for text in fine {
        assert_eq!(Ledger::parse(text).problems(), Vec::<String>::new(), "{text:?}");
    }
}

#[test]
fn a_ledger_quoted_in_a_fence_is_unwrapped_and_the_words_around_it_dropped() {
    let cases = [
        "```markdown\n# Goal\nShip it\n\n## Next\n- go\n```\n",
        "Here is my ledger:\n\n```md\n# Goal\nShip it\n\n## Next\n- go\n```\n\nLet me know.\n",
        "```\n# Goal\nShip it\n\n## Next\n- go\n```",
    ];
    for text in cases {
        let l = Ledger::parse(text);
        assert_eq!(l.goal, "Ship it", "{text:?}");
        assert_eq!(texts(&l.next), ["go"], "{text:?}");
        assert_eq!(l.extra, Vec::<Extra>::new(), "{text:?}");
    }
}

#[test]
fn empty_sections_are_not_written_and_the_goal_always_is() {
    let l = Ledger::parse("# Goal\nx\n\n## Decisions\n\n## Scheduled\n\n## Checklist\n");
    assert_eq!(l.to_markdown(), "# Goal\nx\n");
    assert_eq!(Ledger::default().to_markdown(), "# Goal\n");
}

fn noise(seed: u64, lines: usize) -> String {
    const PARTS: [&str; 36] = [
        "# Goal",
        "## Status",
        "## Checklist",
        "## Scheduled",
        "## Needs you",
        "## Decisions",
        "## Other",
        "### 1. Plan",
        "### Build",
        "###",
        "- [ ] a",
        "- [~] b -- why",
        "- [x] c",
        "- [-]",
        "- [!] d",
        "- [",
        "- [x",
        "  - [ ] step",
        "- 2026-11-01 -- x -- y",
        "- [ ] on: it -- z",
        "- [ ] you",
        "- [ ] ~2026-02-30 -- q",
        "phase: 2",
        "state: done",
        "note: x: y",
        "```",
        "~~~",
        "text",
        "1. numbered",
        "* star",
        "`",
        "--",
        " -- ",
        "\u{e9}\u{4e16}\u{1f600}",
        "\t",
        "",
    ];
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..lines)
        .map(|_| {
            let count = next() % 3 + 1;
            (0..count)
                .map(|_| PARTS[(next() % PARTS.len() as u64) as usize])
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn any_text_parses_and_what_is_written_reads_back_the_same() {
    for seed in 1..=300 {
        let text = noise(seed, 1 + (seed as usize % 24));
        let once = Ledger::parse(&text);
        let written = once.to_markdown();
        let twice = Ledger::parse(&written);
        assert_eq!(twice, once, "seed {seed}:\n{text}\n--- written:\n{written}");
        let _ = (
            once.summary(),
            once.counts(),
            once.current_phase_index(),
            once.has_question(),
            once.is_blocked(),
        );
    }
}

#[test]
fn odd_bytes_never_panic() {
    let emoji = "\u{1f600}".repeat(1000);
    let inputs = [
        "\u{0}",
        "\u{feff}# Goal\nx",
        "#",
        "# ",
        "## ",
        "###",
        "- [",
        "- []",
        "- [x]",
        "- [\u{e9}] x",
        "[x] x",
        "# Goal\r\nwindows\r\n\r\n## Next\r\n- a\r\n",
        &emoji,
        "## \u{e9}\n- \u{e9}",
        "- [ ] \u{e9}\u{e9} -- \u{e9}",
        "~~~\n# Goal\nx",
    ];
    for text in inputs {
        let l = Ledger::parse(text);
        let _ = l.to_markdown();
    }
    let windows = Ledger::parse("# Goal\r\nwindows\r\n\r\n## Next\r\n- a\r\n");
    assert_eq!(windows.goal, "windows");
    assert_eq!(texts(&windows.next), ["a"]);
}

#[test]
fn a_kind_line_in_status_reads_back_and_is_written_again() {
    let cases = [
        ("kind: read-only", Some(Kind::ReadOnly)),
        ("kind: Edit", Some(Kind::Edit)),
        ("- kind: read only", Some(Kind::ReadOnly)),
        ("kind: guess", None),
    ];
    for (line, want) in cases {
        let l = Ledger::parse(&format!("# Goal\nx\n\n## Status\nphase: 1\n{line}\n"));
        assert_eq!(l.status.kind, want, "{line}");
        assert_eq!(l.status.other.len(), usize::from(want.is_none()), "{line}");
        assert_eq!(Ledger::parse(&l.to_markdown()), l, "{line}");
    }
    let plain = Ledger::parse("# Goal\nx\n\n## Status\nphase: 1\n");
    assert_eq!(plain.status.kind, None);
    assert!(!plain.to_markdown().contains("kind"));
}
