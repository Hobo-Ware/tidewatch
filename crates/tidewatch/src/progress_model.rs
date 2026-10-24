//! What the progress screens say about a ledger, as plain data: ring, bars, rows, tags (pure).

use tidewatch_core::progress::{Entry, Ledger, Mark, Phase, PhaseState, State, real};

use crate::progress_stale::fresh;

/// How many checklist rows show before "Show all".
pub(crate) const ROW_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Working,
    Waiting,
    Blocked,
    Done,
}

impl Tone {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Waiting => "Waiting",
            Self::Blocked => "Blocked",
            Self::Done => "Done",
        }
    }
}

pub(crate) fn tone(ledger: &Ledger) -> Tone {
    if ledger.is_blocked() {
        return Tone::Blocked;
    }
    let counts = ledger.counts();
    match ledger.status.state {
        Some(State::Done) => Tone::Done,
        Some(State::Waiting) => Tone::Waiting,
        None if counts.total > 0 && counts.done == counts.total => Tone::Done,
        Some(State::Working | State::Blocked) | None => Tone::Working,
    }
}

/// The ring of a ledger: how far the checklist is, and its middle text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ring {
    pub(crate) tone: Tone,
    pub(crate) fraction: f32,
    pub(crate) done: usize,
    pub(crate) total: usize,
}

/// Where the work is: the current phase's place among the phases and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) number: usize,
    pub(crate) of: usize,
    pub(crate) name: String,
}

pub(crate) fn step(ledger: &Ledger) -> Option<Step> {
    let index = ledger.current_phase_index()?;
    Some(Step {
        number: index + 1,
        of: ledger.checklist.len(),
        name: phase_label(ledger.checklist.get(index)?),
    })
}

pub(crate) fn ring(ledger: &Ledger) -> Ring {
    let counts = ledger.counts();
    let tone = tone(ledger);
    let fraction = if counts.total > 0 {
        (counts.done as f32 / counts.total as f32).clamp(0.0, 1.0)
    } else if tone == Tone::Done {
        1.0
    } else {
        0.0
    };
    Ring { tone, fraction, done: counts.done, total: counts.total }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarKind {
    Plain,
    /// Every item of the phase is skipped.
    Skipped,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Bar {
    pub(crate) label: String,
    pub(crate) fill: f32,
    pub(crate) kind: BarKind,
    pub(crate) current: bool,
    pub(crate) done: usize,
    pub(crate) total: usize,
}

fn phase_label(phase: &Phase) -> String {
    match (phase.title.trim(), phase.number) {
        ("", Some(number)) => format!("Phase {number}"),
        ("", None) => "Phase".to_owned(),
        (title, _) => title.to_owned(),
    }
}

pub(crate) fn bars(ledger: &Ledger) -> Vec<Bar> {
    let current = ledger.current_phase_index();
    ledger
        .checklist
        .iter()
        .enumerate()
        .map(|(index, phase)| {
            let counts = phase.counts();
            let kind = if phase.state() == PhaseState::Skipped {
                BarKind::Skipped
            } else {
                BarKind::Plain
            };
            let under_way = phase.items.iter().filter(|item| item.mark == Mark::Doing).count();
            let fill = if counts.total == 0 {
                0.0
            } else {
                ((counts.done as f32 + 0.5 * under_way as f32) / counts.total as f32)
                    .clamp(0.0, 1.0)
            };
            Bar {
                label: phase_label(phase),
                fill,
                kind,
                current: current == Some(index),
                done: counts.done,
                total: counts.total,
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ItemRow<'a> {
    pub(crate) mark: Mark,
    pub(crate) text: &'a str,
    pub(crate) why: Option<&'a str>,
}

pub(crate) fn item_rows(ledger: &Ledger) -> Vec<ItemRow<'_>> {
    ledger.current_phase().map_or_else(Vec::new, |phase| {
        phase
            .items
            .iter()
            .map(|item| ItemRow {
                mark: item.mark,
                text: item.text.as_str(),
                why: item.why.as_deref().map(str::trim).filter(|why| !why.is_empty()),
            })
            .collect()
    })
}

/// How many of `total` rows show, and how many hide behind "Show all".
pub(crate) fn rows_shown(total: usize, expanded: bool) -> (usize, usize) {
    if expanded || total <= ROW_LIMIT { (total, 0) } else { (ROW_LIMIT, total - ROW_LIMIT) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tag {
    Verified,
    NotVerified,
    Decisions,
}

pub(crate) const TAGS: [Tag; 3] = [Tag::Verified, Tag::NotVerified, Tag::Decisions];

impl Tag {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Verified => 0,
            Self::NotVerified => 1,
            Self::Decisions => 2,
        }
    }

    pub(crate) fn label(self, count: usize) -> String {
        match self {
            Self::Verified => format!("{count} verified"),
            Self::NotVerified => format!("{count} not verified"),
            Self::Decisions => {
                format!("{count} {}", if count == 1 { "decision" } else { "decisions" })
            }
        }
    }

    pub(crate) fn caption(self) -> &'static str {
        match self {
            Self::Verified => "Verified",
            Self::NotVerified => "Not verified",
            Self::Decisions => "Decisions",
        }
    }
}

pub(crate) fn entries(ledger: &Ledger, tag: Tag) -> Vec<&Entry> {
    let list = match tag {
        Tag::Verified => &ledger.verified,
        Tag::NotVerified => &ledger.not_verified,
        Tag::Decisions => &ledger.decisions,
    };
    real(list).collect()
}

/// The questions waiting for the user, placeholders left out.
pub(crate) fn questions(ledger: &Ledger) -> Vec<&Entry> {
    real(&ledger.needs_you).collect()
}

/// `just now`, `4 min ago`, `2 h ago`, `3 d ago`.
pub(crate) fn ago(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..45 => "just now".to_owned(),
        45..3600 => format!("{} min ago", (secs / 60).max(1)),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// The quiet line under the goal: the state word, then when the ledger changed. A stale ledger
/// says it is the progress that is old, not the agent.
pub(crate) fn updated(now_ms: i64, then_ms: Option<i64>, stale: bool) -> Option<String> {
    let what = if stale { "progress updated" } else { "updated" };
    then_ms.map(|then| format!("{what} {}", ago(now_ms, then)))
}

fn phase_name(ledger: &Ledger) -> String {
    ledger.current_phase().map_or_else(
        || {
            let said = ledger.status.phase.trim();
            said.strip_prefix("phase").map_or(said, str::trim).to_owned()
        },
        phase_label,
    )
}

/// The tab ring's tooltip: the phase and how many items are done.
pub(crate) fn tab_tip(ledger: &Ledger) -> String {
    let counts = ledger.counts();
    let phase = phase_name(ledger);
    let done = (counts.total > 0).then(|| format!("{}/{} done", counts.done, counts.total));
    match (phase.is_empty(), done) {
        (false, Some(done)) => format!("{phase} - {done}"),
        (false, None) => phase,
        (true, Some(done)) => done,
        (true, None) => ledger.status.state.map_or("Progress", |state| state.word()).to_owned(),
    }
}

/// What a tab in the strip shows of its ledger: the ring and the tooltip.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabRing {
    pub(crate) ring: Ring,
    pub(crate) tip: String,
}

pub(crate) fn tab_ring(ledger: &Ledger, stale: bool) -> TabRing {
    let ledger = fresh(ledger, stale);
    TabRing { ring: ring(&ledger), tip: tab_tip(&ledger) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardTag {
    Question,
    Blocked,
    Done,
    Waiting,
    Working,
}

impl CardTag {
    /// The tag's words: `1 question`, `2 questions`, `Working`.
    pub(crate) fn label(self, questions: usize) -> String {
        match self {
            Self::Question => {
                format!("{questions} question{}", if questions == 1 { "" } else { "s" })
            }
            Self::Blocked => "Blocked".to_owned(),
            Self::Done => "Done".to_owned(),
            Self::Waiting => "Waiting".to_owned(),
            Self::Working => "Working".to_owned(),
        }
    }
}

pub(crate) fn card_tag(ledger: &Ledger) -> CardTag {
    if ledger.has_question() {
        return CardTag::Question;
    }
    match tone(ledger) {
        Tone::Blocked => CardTag::Blocked,
        Tone::Done => CardTag::Done,
        Tone::Waiting => CardTag::Waiting,
        Tone::Working => CardTag::Working,
    }
}

/// The Overview card's one line: the status note, with the phase before it while work goes on.
pub(crate) fn card_line(ledger: &Ledger) -> String {
    let note = ledger.status.note.trim();
    if tone(ledger) == Tone::Done {
        return note.to_owned();
    }
    let phase = phase_name(ledger);
    match (phase.is_empty(), note.is_empty()) {
        (false, false) => format!("{phase} \u{b7} {note}"),
        (false, true) => phase,
        (true, _) => note.to_owned(),
    }
}

/// What an Overview card says about a ledger.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Look {
    pub(crate) ring: Ring,
    pub(crate) line: String,
    pub(crate) tag: CardTag,
    pub(crate) questions: usize,
}

pub(crate) fn look(ledger: &Ledger, stale: bool) -> Look {
    let ledger = &*fresh(ledger, stale);
    Look {
        ring: ring(ledger),
        line: card_line(ledger),
        tag: card_tag(ledger),
        questions: questions(ledger).len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(text: &str) -> Ledger {
        Ledger::parse(text)
    }

    const MIDDLE: &str = "# Goal\nShip it\n\n## Status\nphase: 2\nstate: working\nnote: writing the validator\n\n## Checklist\n### 1. Understand\n- [x] Read\n- [x] Decide\n### 2. Build\n- [x] Skeleton\n- [~] Validator -- half done\n- [ ] Tests\n- [-] Docs -- not needed\n### 3. Gate\n- [-] Lint -- no config\n### 4. Close out\n- [ ] Commit\n";

    #[test]
    fn the_tone_follows_the_state_and_blockers() {
        let cases = [
            ("state: working", Tone::Working),
            ("state: waiting", Tone::Waiting),
            ("state: done", Tone::Done),
            ("state: blocked", Tone::Blocked),
            ("note: nothing said", Tone::Working),
        ];
        for (status, want) in cases {
            let text = format!(
                "# Goal\nx\n\n## Status\n{status}\n\n## Checklist\n### 1. A\n- [x] a\n- [ ] b\n"
            );
            assert_eq!(tone(&ledger(&text)), want, "{status}");
        }
        let blocked_item =
            "# Goal\nx\n\n## Status\nstate: working\n\n## Checklist\n### 1. A\n- [!] a\n";
        assert_eq!(tone(&ledger(blocked_item)), Tone::Blocked);
        let done_with_blocked =
            "# Goal\nx\n\n## Status\nstate: done\n\n## Checklist\n### 1. A\n- [!] a\n";
        assert_eq!(tone(&ledger(done_with_blocked)), Tone::Done);
        let all_done = "# Goal\nx\n\n## Checklist\n### 1. A\n- [x] a\n";
        assert_eq!(tone(&ledger(all_done)), Tone::Done, "no state, nothing left");
    }

    #[test]
    fn the_ring_counts_done_over_total_with_skipped_items_out() {
        let ring = ring(&ledger(MIDDLE));
        assert_eq!((ring.done, ring.total), (3, 6));
        assert_eq!(ring.fraction, 0.5);
        assert_eq!(ring.tone, Tone::Working);
    }

    #[test]
    fn a_ring_with_no_items_is_empty_unless_the_work_is_done() {
        let cases = [
            ("# Goal\nx\n", 0.0),
            ("# Goal\nx\n\n## Status\nstate: done\n", 1.0),
            ("# Goal\nx\n\n## Checklist\n### 1. A\n- [x] a\n- [x] b\n", 1.0),
            ("# Goal\nx\n\n## Checklist\n### 1. A\n- [-] a -- no\n", 0.0),
            ("# Goal\nx\n\n## Checklist\n### 1. A\n- [~] a\n- [ ] b\n", 0.0),
        ];
        for (text, fraction) in cases {
            assert_eq!(ring(&ledger(text)).fraction, fraction, "{text:?}");
        }
    }

    #[test]
    fn the_step_is_the_current_phase_among_the_phases() {
        let first = "# Goal\nx\n\n## Status\nphase: 1 Understand\n\n## Checklist\n### 1. Understand\n- [~] a\n### 2. Build\n- [ ] b\n### 3. Gate\n- [ ] c\n### 4. Close out\n- [ ] d\n";
        let want = Step { number: 1, of: 4, name: "Understand".to_owned() };
        assert_eq!(step(&ledger(first)), Some(want));
        assert_eq!(
            step(&ledger(MIDDLE)).map(|step| (step.number, step.name)),
            Some((2, "Build".to_owned()))
        );
        assert_eq!(step(&ledger("# Goal\nx\n")), None);
    }

    #[test]
    fn a_bar_fills_by_its_done_share_and_the_current_phase_is_marked() {
        let bars = bars(&ledger(MIDDLE));
        let seen: Vec<_> =
            bars.iter().map(|bar| (bar.label.as_str(), bar.fill, bar.kind, bar.current)).collect();
        assert_eq!(
            seen,
            [
                ("Understand", 1.0, BarKind::Plain, false),
                ("Build", 0.5, BarKind::Plain, true),
                ("Gate", 0.0, BarKind::Skipped, false),
                ("Close out", 0.0, BarKind::Plain, false),
            ]
        );
    }

    #[test]
    fn an_item_under_way_fills_its_own_phase_and_no_other() {
        let bars = bars(&ledger(
            "# Goal\nx\n\n## Status\nphase: 1 Understand\n\n## Checklist\n### 1. Understand\n- [~] Read\n- [ ] Decide\n### 2. Build\n- [ ] Change\n",
        ));
        let fills: Vec<_> =
            bars.iter().map(|bar| (bar.label.as_str(), bar.fill, bar.current)).collect();
        assert_eq!(fills, [("Understand", 0.25, true), ("Build", 0.0, false)]);
    }

    #[test]
    fn a_phase_without_a_title_is_called_by_its_number() {
        let bars = bars(&ledger("# Goal\nx\n\n## Checklist\n### 3.\n- [ ] a\n"));
        assert_eq!(bars[0].label, "Phase 3");
    }

    #[test]
    fn the_rows_are_the_current_phases_items_with_their_reasons() {
        let ledger = ledger(MIDDLE);
        let rows = item_rows(&ledger);
        let seen: Vec<_> = rows.iter().map(|row| (row.mark, row.text, row.why)).collect();
        assert_eq!(
            seen,
            [
                (Mark::Done, "Skeleton", None),
                (Mark::Doing, "Validator", Some("half done")),
                (Mark::Todo, "Tests", None),
                (Mark::Skipped, "Docs", Some("not needed")),
            ]
        );
        assert_eq!(item_rows(&Ledger::parse("# Goal\nx\n")), []);
    }

    #[test]
    fn at_most_eight_rows_show_until_all_are_asked_for() {
        let cases = [
            (0, false, (0, 0)),
            (8, false, (8, 0)),
            (9, false, (8, 1)),
            (30, false, (8, 22)),
            (30, true, (30, 0)),
        ];
        for (total, expanded, want) in cases {
            assert_eq!(rows_shown(total, expanded), want, "{total} {expanded}");
        }
    }

    #[test]
    fn tags_count_real_entries_only_and_say_decision_once() {
        let text = "# Goal\nx\n\n## Decisions\n- one\n\n## Verified\n- a -- how\n- b\n\n## Not verified\n- nothing\n";
        let ledger = ledger(text);
        let counts: Vec<_> = TAGS.iter().map(|tag| entries(&ledger, *tag).len()).collect();
        assert_eq!(counts, [2, 0, 1]);
        assert_eq!(Tag::Verified.label(2), "2 verified");
        assert_eq!(Tag::NotVerified.label(2), "2 not verified");
        assert_eq!(Tag::Decisions.label(1), "1 decision");
        assert_eq!(Tag::Decisions.label(4), "4 decisions");
        assert_eq!(TAGS.map(Tag::index), [0, 1, 2]);
    }

    #[test]
    fn questions_skip_the_nothing_placeholder() {
        let none = ledger("# Goal\nx\n\n## Needs you\n- Nothing needs you today.\n");
        assert_eq!(questions(&none), Vec::<&Entry>::new());
        let some = ledger("# Goal\nx\n\n## Needs you\n- Pick one -- it blocks the merge\n");
        assert_eq!(questions(&some)[0].text, "Pick one");
    }

    #[test]
    fn ago_steps_from_now_to_days() {
        let cases = [
            (0, "just now"),
            (44_000, "just now"),
            (45_000, "1 min ago"),
            (240_000, "4 min ago"),
            (3_599_000, "59 min ago"),
            (3_600_000, "1 h ago"),
            (7_300_000, "2 h ago"),
            (86_400_000, "1 d ago"),
            (3 * 86_400_000, "3 d ago"),
            (-5_000, "just now"),
        ];
        for (diff, want) in cases {
            assert_eq!(ago(1_000_000_000 + diff, 1_000_000_000), want, "{diff}");
        }
        assert_eq!(updated(5_000, None, false), None);
        assert_eq!(updated(5_000, Some(0), false).as_deref(), Some("updated just now"));
        assert_eq!(updated(480_000, Some(0), true).as_deref(), Some("progress updated 8 min ago"));
    }

    #[test]
    fn a_stale_wait_gives_the_tab_ring_the_working_tone() {
        let waiting = ledger("# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Yes?\n");
        assert_eq!(tab_ring(&waiting, false).ring.tone, Tone::Waiting);
        assert_eq!(tab_ring(&waiting, true).ring.tone, Tone::Working);
    }

    #[test]
    fn the_tab_tip_names_the_phase_and_the_count() {
        assert_eq!(tab_tip(&ledger(MIDDLE)), "Build - 3/6 done");
        assert_eq!(tab_tip(&ledger("# Goal\nx\n\n## Status\nphase: 2 (wiring)\n")), "2 (wiring)");
        assert_eq!(tab_tip(&ledger("# Goal\nx\n\n## Status\nstate: working\n")), "working");
        assert_eq!(tab_tip(&ledger("# Goal\nx\n")), "Progress");
        let only_count = "# Goal\nx\n\n## Checklist\n- [x] a\n- [ ] b\n";
        assert!(tab_tip(&ledger(only_count)).ends_with("1/2 done"));
    }

    #[test]
    fn a_card_says_question_before_anything_else() {
        let cases = [
            ("state: done\n\n## Needs you\n- Decide", CardTag::Question),
            ("state: blocked", CardTag::Blocked),
            ("state: done", CardTag::Done),
            ("state: waiting", CardTag::Waiting),
            ("state: working", CardTag::Working),
        ];
        for (rest, want) in cases {
            let text = format!("# Goal\nx\n\n## Status\n{rest}\n");
            assert_eq!(card_tag(&ledger(&text)), want, "{rest}");
        }
        for rest in ["state: waiting\n\n## Needs you\n- Yes or changes?", "state: blocked"] {
            let text = format!("# Goal\nx\n\n## Status\n{rest}\n");
            let stale = look(&ledger(&text), true);
            assert_eq!(stale.tag, CardTag::Working, "{rest}");
            assert_eq!((stale.ring.tone, stale.questions), (Tone::Working, 0), "{rest}");
        }
        assert_eq!(CardTag::Question.label(1), "1 question");
        assert_eq!(CardTag::Question.label(3), "3 questions");
        assert_eq!(CardTag::Working.label(0), "Working");
    }

    #[test]
    fn a_cards_line_is_the_note_after_the_phase_and_only_the_note_when_done() {
        let working = ledger(
            "# Goal\nx\n\n## Status\nphase: 2 Build\nstate: working\nnote: writing the validator\n\n## Checklist\n### 1. Understand\n- [x] a\n### 2. Build\n- [~] b\n",
        );
        assert_eq!(card_line(&working), "Build \u{b7} writing the validator");
        let done = ledger(
            "# Goal\nx\n\n## Status\nphase: 4\nstate: done\nnote: Committed ec84557\n\n## Checklist\n### 4. Close out\n- [x] a\n",
        );
        assert_eq!(card_line(&done), "Committed ec84557");
        let no_note = ledger(
            "# Goal\nx\n\n## Status\nphase: 2\nstate: working\n\n## Checklist\n### 1. A\n- [x] a\n### 2. Build\n- [~] b\n",
        );
        assert_eq!(card_line(&no_note), "Build");
        assert_eq!(card_line(&ledger("# Goal\nx\n\n## Status\nnote: only this\n")), "only this");
    }

    #[test]
    fn a_look_gathers_the_ring_the_line_the_tag_and_the_question_count() {
        let text = "# Goal\nx\n\n## Status\nphase: 1\nstate: done\nnote: Committed\n\n## Checklist\n### 1. A\n- [x] a\n\n## Needs you\n- One\n- Two\n";
        let look = look(&ledger(text), false);
        assert_eq!(look.tag, CardTag::Question);
        assert_eq!((look.questions, look.line.as_str()), (2, "Committed"));
        assert_eq!((look.ring.tone, look.ring.fraction), (Tone::Done, 1.0));
    }
}
