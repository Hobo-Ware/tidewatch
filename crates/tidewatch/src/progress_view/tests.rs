//! The panel: pure choices, and headless frames for the clicks and for text that never overlaps.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Sense, Shape, pos2, vec2};
use tidewatch_core::civil_date::Date;
use tidewatch_core::done_check::{Kind, Line, Report, Status, Unverified};
use tidewatch_core::end_block::EndBlock;
use tidewatch_core::link_board::{Link, LinkBoard, LinkState, TitleFrom};
use tidewatch_core::panel_event::StepBy;
use tidewatch_core::panel_state::MarkSeen;
use tidewatch_core::panel_view::Why;
use tidewatch_core::progress::Ledger;

use crate::done_check_glue::{Checked, Look};
use crate::done_check_view;

use super::*;

const LONG: &str = "# Goal\nAdd a slugify(text) function to lib.py that turns spaces and punctuation into single hyphens, with unit tests, and commit it\n\n## Status\nphase: 2 Build\nstate: working\nnote: writing the validator\n\n## Checklist\n### 1. Understand\n- [x] Restate the goal in one line\n- [x] Read the code that owns the behavior\n### 2. Build\n- [x] Smallest change that does the job -- reused two helpers\n- [~] Pure logic in its own small function with a unit test\n- [ ] Remove anything the change left unused\n- [-] Format, type check and lint on the touched files -- no formatter or linter config in this repo at all\n- [!] Wait for the database\n- [ ] Item six\n- [ ] Item seven\n- [ ] Item eight\n- [ ] Item nine\n- [ ] Item ten\n### 3. PR\n- [ ] Open the PR\n### 4. Done\n- [ ] Report\n\n## Decisions\n- Follow the unittest style of the repo -- match existing code\n- Keep accented letters\n\n## Verified\n- Ran 8 tests, OK -- python3 -m unittest -v\n- Only two files changed\n\n## Not verified\n- No formatter run\n\n## Needs you\n- Decide if accented letters should become ASCII (cafe). Current code keeps them -- it changes the output\n";

fn known(text: &str) -> TabLedger {
    TabLedger { ledger: Ledger::parse(text), updated_ms: Some(0) }
}

struct Panel {
    ctx: egui::Context,
    ledger: Option<TabLedger>,
    jobs: Vec<Card>,
    last_turn: Option<LastTurn>,
    state: TabState,
    size: egui::Vec2,
    holds_keys: bool,
    shown: Option<tidewatch_core::progress::State>,
    links: Option<LinkBoard>,
    blocks: Blocks,
    marks: Vec<MarkSeen>,
    texts: Vec<(String, Rect)>,
    action: Option<Action>,
    opened: Vec<String>,
}

impl Panel {
    fn new(text: &str, width: f32) -> Self {
        let mut panel = Self {
            ctx: crate::fonts::test_context(),
            ledger: Some(known(text)),
            jobs: Vec::new(),
            last_turn: None,
            state: TabState::default(),
            size: vec2(width, 900.0),
            holds_keys: false,
            shown: None,
            links: None,
            blocks: Blocks::default(),
            marks: Vec::new(),
            texts: Vec::new(),
            action: None,
            opened: Vec::new(),
        };
        panel.frame(Vec::new());
        panel.frame(Vec::new());
        panel
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Action> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), self.size)),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut action = None;
        let (ledger, jobs, state, size) =
            (self.ledger.as_ref(), &self.jobs, &mut self.state, self.size);
        let last_turn = self.last_turn.clone();
        let holds_keys = self.holds_keys;
        let shown = self.shown;
        let links = self.links.as_ref();
        let (blocks, marks) = (&self.blocks, &self.marks);
        let output = self.ctx.run_ui(raw, |ui| {
            if holds_keys {
                let at = Rect::from_min_size(pos2(0.0, 880.0), vec2(10.0, 10.0));
                let other = ui.interact(at, egui::Id::new("terminal-grid"), Sense::click());
                other.request_focus();
            }
            let input = Input {
                ledger,
                stale: false,
                jobs,
                now_ms: 240_000,
                time: 1.0,
                animate: false,
                first: Side::Progress,
                meeseeks: None,
                last_turn: last_turn.clone(),
                shown,
                links,
                blocks: blocks.clone(),
                marks,
            };
            action = show(ui, Rect::from_min_size(pos2(0.0, 0.0), size), &input, state);
        });
        self.opened.extend(output.platform_output.commands.iter().filter_map(
            |command| match command {
                egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            },
        ));
        self.texts.clear();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut self.texts);
        }
        self.action.clone_from(&action);
        action
    }

    fn center_of(&self, text: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(drawn, _)| drawn == text)
            .unwrap_or_else(|| panic!("{text:?} not drawn in {:?}", self.texts))
            .1
            .center()
    }

    fn click(&mut self, at: Pos2) -> Option<Action> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        let action = self.frame(vec![button(false)]);
        self.frame(Vec::new());
        action
    }

    fn drawn(&self, text: &str) -> bool {
        self.texts.iter().any(|(drawn, _)| drawn == text)
    }
}

fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Text(text) => {
            let rect = text.galley.rect.translate(text.pos.to_vec2());
            if !text.galley.text().trim().is_empty() {
                out.push((text.galley.text().to_owned(), rect));
            }
        }
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
        _ => {}
    }
}

#[test]
fn history_shows_until_a_ledger_arrives_then_the_choice_or_progress() {
    let cases = [
        (None, false, Side::Progress, Side::History),
        (None, false, Side::History, Side::History),
        (Some(Side::Progress), false, Side::Progress, Side::History),
        (None, true, Side::Progress, Side::Progress),
        (None, true, Side::History, Side::History),
        (Some(Side::History), true, Side::Progress, Side::History),
        (Some(Side::Progress), true, Side::History, Side::Progress),
        (Some(Side::Changes), false, Side::Progress, Side::Changes),
        (Some(Side::Changes), true, Side::History, Side::Changes),
    ];
    for (chosen, has_ledger, first, want) in cases {
        assert_eq!(
            side_shown(chosen, has_ledger, first),
            want,
            "{chosen:?} {has_ledger} {first:?}"
        );
    }
    assert_eq!(Side::from(RightPanel::Progress), Side::Progress);
    assert_eq!(Side::from(RightPanel::History), Side::History);
}

#[test]
fn the_switch_offers_progress_only_with_a_ledger_and_always_changes_before_history() {
    assert_eq!(sides(true), [Side::Progress, Side::Changes, Side::History]);
    assert_eq!(sides(false), [Side::Changes, Side::History]);
    let labels: Vec<&str> = sides(true).iter().map(|side| side.label()).collect();
    assert_eq!(labels, ["Progress", "Changes", "History"]);
}

#[test]
fn a_tab_remembers_its_choices_and_forgets_closed_tabs() {
    let mut memory = Memory::default();
    memory.tab(1).chosen = Some(Side::History);
    memory.tab(1).toggle(Tag::Decisions);
    memory.tab(2);
    assert_eq!(memory.tab(1).chosen, Some(Side::History));
    assert!(memory.tab(1).is_open(Tag::Decisions) && !memory.tab(1).is_open(Tag::Verified));
    memory.tab(1).toggle(Tag::Decisions);
    assert!(!memory.tab(1).is_open(Tag::Decisions));
    memory.forget_except(|id| id == 2);
    assert_eq!(memory.tab(1).chosen, None, "a closed tab starts over");
}

#[test]
fn phase_bars_wrap_into_rows_that_keep_a_readable_label() {
    let cases = [(400.0, 4, 4), (400.0, 10, 7), (196.0, 10, 3), (50.0, 5, 1), (1000.0, 3, 3)];
    for (width, count, want) in cases {
        assert_eq!(bar_columns(width, count), want, "{width} {count}");
    }
}

#[test]
fn tags_flow_into_rows_and_a_row_break_never_leaves_the_edge() {
    let (places, height) = flow(&[80.0, 90.0, 70.0], 300.0, TAG_H, TAG_GAP);
    assert_eq!(places, [(0.0, 0.0), (88.0, 0.0), (186.0, 0.0)]);
    assert_eq!(height, TAG_H);
    let (places, height) = flow(&[120.0, 120.0, 120.0], 260.0, TAG_H, TAG_GAP);
    assert_eq!(places, [(0.0, 0.0), (128.0, 0.0), (0.0, TAG_H + TAG_GAP)]);
    assert_eq!(height, 2.0 * TAG_H + TAG_GAP);
    assert_eq!(flow(&[], 100.0, TAG_H, TAG_GAP), (Vec::new(), 0.0));
}

#[test]
fn no_question_means_no_card_and_no_button() {
    let text = LONG.replace("## Needs you\n- Decide if accented letters should become ASCII (cafe). Current code keeps them -- it changes the output\n", "## Needs you\n- Nothing needs you.\n");
    let panel = Panel::new(&text, 420.0);
    assert!(!panel.drawn("Something else") && !panel.drawn("?"));
}

#[test]
fn a_finished_stage_hides_behind_a_link_and_it_toggles() {
    let mut panel = Panel::new(LONG, 420.0);
    assert!(panel.drawn("Show 1 finished stage"), "{:?}", panel.texts);
    assert!(panel.drawn("Item ten") && panel.drawn("Open the PR"), "open stages show every step");
    assert!(!panel.drawn("Restate the goal in one line"));
    let at = panel.center_of("Show 1 finished stage");
    panel.click(at);
    assert!(panel.state.show_all_rows());
    assert!(panel.drawn("Restate the goal in one line") && panel.drawn("Show less"));
    let at = panel.center_of("Show less");
    panel.click(at);
    assert!(!panel.drawn("Restate the goal in one line"));
}

#[test]
fn a_tag_opens_its_list_in_place_and_a_second_click_closes_it() {
    let mut panel = Panel::new(LONG, 420.0);
    assert!(
        panel.drawn("2 verified") && panel.drawn("1 not verified") && panel.drawn("2 decisions")
    );
    assert!(!panel.texts.iter().any(|(text, _)| text.starts_with("Ran 8 tests")));
    let at = panel.center_of("2 verified");
    panel.click(at);
    assert!(panel.state.is_open(Tag::Verified));
    assert!(panel.texts.iter().any(|(text, _)| text.starts_with("Ran 8 tests")));
    assert!(!panel.state.is_open(Tag::Decisions));
    let at = panel.center_of("2 verified");
    panel.click(at);
    assert!(!panel.texts.iter().any(|(text, _)| text.starts_with("Ran 8 tests")));
}

#[test]
fn a_tag_with_nothing_behind_it_is_not_drawn() {
    let panel = Panel::new("# Goal\nx\n\n## Verified\n- ok\n\n## Not verified\n- nothing\n", 420.0);
    assert!(panel.drawn("1 verified"));
    assert!(!panel.texts.iter().any(|(text, _)| text.contains("not verified")));
}

#[test]
fn the_header_says_the_goal_never_a_ring_count_and_the_meta_line_says_the_reported_state() {
    let panel = Panel::new(LONG, 420.0);
    assert!(!panel.texts.iter().any(|(text, _)| text.starts_with("Step ")), "{:?}", panel.texts);
    assert!(!panel.texts.iter().any(|(text, _)| text == "3/13" || text.starts_with("0/")));
    assert!(
        panel.drawn("\u{25cf} Needs you"),
        "a question the agent holds makes it needs you, a blocked step does not make it blocked"
    );
    assert!(!panel.texts.iter().any(|(text, _)| text.contains("updated")), "{:?}", panel.texts);
}

#[test]
fn an_old_ten_phase_ledger_still_reads_as_four_stages() {
    let text = "# Goal\nShip it\n\n## Status\nphase: 7 Review\nstate: waiting\n\n## Checklist\n### 1. Understand and decide\n- [x] a\n### 2. Build\n- [x] b\n### 3. Gate\n- [x] c\n### 4. Validate against production\n- [x] d\n### 5. Preview\n- [x] e\n### 6. PR\n- [x] Open it\n### 7. Review\n- [~] Wait for CI\n### 8. Ship\n- [ ] Merge\n### 9. Verify live\n- [ ] Errors\n### 10. Close out\n- [ ] Report\n";
    let panel = Panel::new(text, 420.0);
    for bar in ["Understand", "Build", "PR", "Done"] {
        assert!(panel.drawn(bar), "{bar:?} in {:?}", panel.texts);
    }
    assert!(panel.drawn("Open it") && panel.drawn("Wait for CI") && panel.drawn("Merge"));
    assert!(panel.drawn("Show 2 finished stages"), "{:?}", panel.texts);
    assert!(!panel.drawn("Gate") && !panel.drawn("c"));
}

#[test]
fn no_text_overlaps_at_any_width_the_card_has() {
    for width in [200.0, 240.0, 300.0, 330.0, 420.0, 640.0] {
        let mut panel = Panel::new(LONG, width);
        for tag in TAGS {
            let at = panel
                .center_of(&tag.label(entries(&panel.ledger.as_ref().unwrap().ledger, tag).len()));
            panel.click(at);
        }
        let at = panel.center_of("Show 1 finished stage");
        panel.click(at);
        for (index, (a, a_rect)) in panel.texts.iter().enumerate() {
            assert!(a_rect.right() <= width + 0.5, "{a:?} runs past the card at {width}");
            for (b, b_rect) in &panel.texts[index + 1..] {
                assert!(
                    !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                    "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                );
            }
        }
    }
}

const LINKED: &str = "# Goal\nShip the preview at localhost:5174 today\n\n## Status\nphase: 2 Build\nstate: waiting\n\n## Checklist\n### 2. Build\n- [x] Check http://localhost:8787/jobs -- see [the PR](https://github.com/o/r/pull/12)\n- [ ] Plain item\n\n## Verified\n- Opened https://claude.ai/artifact/abc and it loaded\n\n## Needs you\n- Which base should the preview use? Notes at https://example.com/notes\n";

impl Panel {
    fn opened_by_clicking_along(&mut self, text: &str, step: usize) -> Vec<String> {
        let rect = self.texts.iter().find(|(drawn, _)| drawn.starts_with(text)).unwrap().1;
        let before = self.opened.len();
        let mut x = rect.left() + 1.0;
        while x < rect.right() {
            self.click(pos2(x, rect.center().y));
            x += step as f32;
        }
        self.opened[before..].to_vec()
    }
}

#[test]
fn a_link_in_the_goal_a_row_a_note_and_the_question_opens_and_plain_text_does_not() {
    let mut panel = Panel::new(LINKED, 640.0);
    let at = panel.center_of("1 verified");
    panel.click(at);
    let cases = [
        ("Ship the preview", "http://localhost:5174"),
        ("Check http://localhost:8787/jobs", "http://localhost:8787/jobs"),
        ("Opened https", "https://claude.ai/artifact/abc"),
    ];
    for (text, url) in cases {
        let opened = panel.opened_by_clicking_along(text, 3);
        assert!(opened.iter().any(|u| u == url), "{text:?} opened {opened:?}");
        assert!(opened.iter().all(|u| crate::links::is_web_url(u)), "{opened:?}");
    }
    let opened = panel.opened_by_clicking_along("Plain item", 3);
    assert_eq!(opened.len(), 0);
}

fn turn(check: Option<Look>, can_ask: Result<(), &'static str>) -> LastTurn {
    let own = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
    let block = EndBlock {
        done: own(&["Added `FilterBar` to the board", "Kept the filters in the URL"]),
        needs_you: own(&["Should the filters apply to the archived column?"]),
        next: own(&["Add keyboard shortcuts"]),
    };
    LastTurn::of(Some(&block), check, can_ask, today()).unwrap()
}

fn today() -> Date {
    Date::parse("2026-10-27").unwrap()
}

const STAMP: Checked = Checked { at: 1_793_142_060, same: false };

fn blocked_report() -> Report {
    let line = |kind, status, detail: &str| Line { kind, status, detail: detail.to_owned() };
    Report {
        head: String::new(),
        lines: vec![
            line(Kind::Pushed, Status::Fail, "2 commits not pushed"),
            line(Kind::Unverified, Status::Unknown, "1 item the agent did not verify"),
        ],
        unverified: vec![Unverified::new("the Safari layout")],
        ..Report::default()
    }
}

fn blocked() -> Look {
    Look::Report(blocked_report(), STAMP)
}

fn waiting_report() -> Report {
    let line = |kind, status, detail: &str| Line { kind, status, detail: detail.to_owned() };
    Report {
        head: String::new(),
        lines: vec![
            line(Kind::PullRequest, Status::Pass, "PR #2359 merged"),
            line(Kind::Unverified, Status::Waiting, "1 item waiting until 2026-10-28"),
        ],
        unverified: vec![Unverified::until(
            "Live list across a day boundary",
            Date::parse("2026-10-28").unwrap(),
        )],
        ..Report::default()
    }
}

const SMALL: &str = "# Goal\nx\n\n## Status\nphase: 2 Build\nstate: working\n";

fn tall(text: &str, width: f32, last_turn: LastTurn) -> Panel {
    let mut panel = Panel::new(text, width);
    panel.size = vec2(width, 2400.0);
    panel.last_turn = Some(last_turn);
    panel.frame(Vec::new());
    panel.frame(Vec::new());
    panel
}

#[test]
fn the_last_turn_lists_done_needs_you_and_next_with_the_check_as_rows() {
    let panel = tall(SMALL, 420.0, turn(Some(blocked()), Ok(())));
    for text in [
        "Last turn",
        "Done",
        "Needs you",
        "Next",
        "Checks",
        "Kept the filters in the URL",
        "Should the filters apply to the archived column?",
        "Add keyboard shortcuts",
        "Blocked",
        "Pushed: 2 commits not pushed",
        "Ask agent to fix",
        "Check again",
    ] {
        assert!(panel.drawn(text), "{text:?} not drawn in {:?}", panel.texts);
    }
    assert!(!panel.drawn("Answer"), "the Reader's card is the one place to answer");
    let top = |text: &str| panel.texts.iter().find(|(t, _)| t == text).unwrap().1.top();
    assert!(top("Last turn") < top("Done") && top("Done") < top("Needs you"));
    assert!(top("Needs you") < top("Next") && top("Next") < top("Checks"));
}

#[test]
fn a_tab_that_runs_on_its_own_gets_no_ask_agent_to_fix_button() {
    let own = LastTurn { owner_asks: false, ..turn(Some(blocked()), Ok(())) };
    let panel = tall(SMALL, 420.0, own);
    assert!(!panel.drawn("Ask agent to fix"), "{:?}", panel.texts);
    assert!(panel.drawn("Check again"));
}

#[test]
fn the_last_turn_sits_under_the_stages_and_above_the_checklist() {
    let text = format!("{SMALL}\n## Checklist\n### 2. Build\n- [~] Wire the list\n");
    let panel = tall(&text, 420.0, turn(None, Ok(())));
    let top = |text: &str| panel.texts.iter().rfind(|(t, _)| t == text).unwrap().1.top();
    assert!(top("Build") < top("Last turn") && top("Last turn") < top("Wire the list"));
}

#[test]
fn a_summary_that_asks_nothing_has_no_needs_you_and_no_answer() {
    let own = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
    let block =
        EndBlock { done: own(&["Did it"]), needs_you: own(&["nothing"]), next: own(&["Ship"]) };
    let panel = tall(SMALL, 420.0, LastTurn::of(Some(&block), None, Ok(()), today()).unwrap());
    assert!(panel.drawn("Done") && panel.drawn("Next"));
    assert!(!panel.drawn("Needs you") && !panel.drawn("Answer") && !panel.drawn("Checks"));
}

#[test]
fn the_check_buttons_of_the_last_turn_send_the_same_asks_the_panel_always_did() {
    let mut panel = tall(SMALL, 420.0, turn(Some(blocked()), Ok(())));
    let at = panel.center_of("Check again");
    assert_eq!(panel.click(at), Some(Action::DoneCheck(done_check_view::Action::CheckAgain)));
    let at = panel.center_of("Ask agent to fix");
    assert_eq!(panel.click(at), Some(Action::DoneCheck(done_check_view::Action::AskFix)));
}

#[test]
fn ask_agent_to_fix_does_nothing_while_the_agent_is_busy() {
    let mut panel = tall(SMALL, 420.0, turn(Some(blocked()), Err("The agent is working")));
    let at = panel.center_of("Ask agent to fix");
    assert_eq!(panel.click(at), None);
}

#[test]
fn a_ledger_question_draws_no_card_and_no_answer_button_in_the_panel() {
    let panel = tall(LONG, 420.0, turn(None, Ok(())));
    for text in ["NEEDS YOU", "Something else", "Answer", "Not now"] {
        assert!(!panel.drawn(text), "{text:?} drawn in {:?}", panel.texts);
    }
}

#[test]
fn a_tab_with_no_ledger_still_shows_its_last_turn() {
    let mut panel = Panel::new(SMALL, 420.0);
    panel.ledger = None;
    panel.last_turn = Some(turn(Some(Look::Checking(None)), Ok(())));
    panel.size = vec2(420.0, 1200.0);
    panel.frame(Vec::new());
    panel.frame(Vec::new());
    assert!(panel.drawn("Last turn") && panel.drawn("Kept the filters in the URL"));
    assert!(panel.drawn("Checking that the work landed..."));
    assert!(!panel.drawn("Check again"));
}

#[test]
fn showing_the_section_clears_the_new_items_and_a_changed_check_makes_them_new_again() {
    let mut panel = Panel::new(SMALL, 420.0);
    assert_eq!(panel.state, TabState::default());
    let first = turn(Some(blocked()), Ok(()));
    let attention = first.attention();
    assert!(panel.state.unseen(attention));
    panel.last_turn = Some(first);
    panel.frame(Vec::new());
    assert!(!panel.state.unseen(attention));
    let changed = turn(None, Ok(())).attention();
    assert!(panel.state.unseen(changed), "another question is new");
    panel.last_turn = None;
    panel.frame(Vec::new());
    assert!(panel.state.unseen(attention), "a gone turn forgets what was seen");
}

#[test]
fn no_last_turn_text_overlaps_or_leaves_the_card_at_any_width() {
    let looks = [
        blocked(),
        Look::Report(waiting_report(), Checked { same: true, ..STAMP }),
        Look::Checking(Some(blocked_report())),
    ];
    for (width, look) in [200.0, 240.0, 300.0, 420.0, 640.0]
        .into_iter()
        .flat_map(|w| looks.iter().map(move |look| (w, look.clone())))
    {
        let long = "Pick a default owner filter: me or everyone, and say whether the archived column follows it or keeps its own filters";
        let mut last = turn(Some(look), Ok(()));
        last.needs.push(long.to_owned());
        let panel = tall(SMALL, width, last);
        for (index, (a, a_rect)) in panel.texts.iter().enumerate() {
            assert!(a_rect.right() <= width + 0.5, "{a:?} runs past the card at {width}");
            for (b, b_rect) in &panel.texts[index + 1..] {
                assert!(
                    !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                    "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                );
            }
        }
    }
}

const ASKING: &str = "# Goal\nShip it\n\n## Status\nstate: waiting\n\n## Needs you\n- Post the Slack follow-up -- the owner wanted it out today -- options: Post it | Skip it* | Draft only | Ask Sam | Later\n";

fn recap_report(state: &str) -> Ledger {
    let done = state == "done";
    let stage = |name: &str, kind: &str, status: &str| serde_json::json!({ "name": name, "kind": kind, "status": if done { "done" } else { status } });
    let step = |stage: &str, text: &str, status: &str, extra: serde_json::Value| {
        let mut step = serde_json::json!({
            "stage": stage, "text": text, "status": if done { "done" } else { status }
        });
        step.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        step
    };
    let mut args = serde_json::json!({
        "goal": "Link each season recap line to the cast it names, and backfill old rows",
        "title": "Link each recap line to its cast, and backfill old rows",
        "state": state,
        "now": "Backfill step 2 is running",
        "stages": [
            stage("Build", "build", "done"),
            stage("Ship", "ship", "done"),
            stage("Backfill", "ship", "active"),
            stage("Verify", "verify", "todo")
        ],
        "steps": [
            step("Backfill", "Live season route serves the new line format", "done", serde_json::json!({ "evidence": "200, people empty" })),
            step("Backfill", "Check links against each season's cast", "active", serde_json::json!({})),
            step("Backfill", "Guarded backfill writes", "blocked", serde_json::json!({ "reason": "waits on your answer" })),
            step("Verify", "Check one linked row on the live route", "todo", serde_json::json!({}))
        ],
        "verified": [{ "text": "Unit tests" }],
        "not_verified": [{ "text": "Live row" }],
        "decisions": [{ "text": "Batches of 500" }]
    });
    if state == "needs_you" {
        args.as_object_mut().unwrap().extend(
            serde_json::json!({
                "needs_you": {
                    "question": "When should step 3 run?",
                    "options": ["Pause after step 2", "Let it all run now", "Pause right now"],
                    "recommended": 0,
                    "why": "step 2 keeps running until you answer"
                },
                "waiting_on": { "job": "recap-people-backfill" }
            })
            .as_object()
            .unwrap()
            .clone(),
        );
    }
    tidewatch_core::progress_report::ProgressReport::check(args.as_object().unwrap(), None)
        .unwrap_or_else(|problems| panic!("{problems:?}"))
        .to_ledger()
}

fn backfill_job() -> Card {
    Card::Bar(crate::job_model::BarCard {
        id: 3,
        name: "recap-people-backfill".to_owned(),
        right: Some("4%".to_owned()),
        fraction: 0.04,
        detail: "people \u{b7} 2,246 of 58,650 rows \u{b7} about 9h 46m left".to_owned(),
    })
}

fn recap_panel(state: &str, width: f32) -> Panel {
    let mut panel = Panel::new("# Goal\nx\n", width);
    panel.ledger = Some(TabLedger { ledger: recap_report(state), updated_ms: Some(0) });
    panel.jobs = vec![backfill_job()];
    panel.frame(Vec::new());
    panel.frame(Vec::new());
    panel
}

#[test]
fn the_needs_you_report_draws_the_board_top_to_bottom() {
    let panel = recap_panel("needs_you", 520.0);
    let at = |text: &str| {
        panel
            .texts
            .iter()
            .find(|(drawn, _)| drawn == text)
            .unwrap_or_else(|| panic!("{text:?} not drawn in {:?}", panel.texts))
            .1
            .top()
    };
    let order = [
        "Link each recap line to its cast, and backfill old rows",
        "\u{25cf} Needs you",
        "Now",
        "Backfill step 2 is running",
        "Build",
        "BACKFILL",
        "Check links against each season's cast",
        "VERIFY",
        "Check one linked row on the live route",
        "1 verified",
    ];
    let tops: Vec<f32> = order.iter().map(|text| at(text)).collect();
    assert!(tops.windows(2).all(|pair| pair[0] <= pair[1] + 0.5), "{order:?} {tops:?}");
    for text in ["waits on your answer", "200, people empty"] {
        assert!(panel.drawn(text), "{text:?} in {:?}", panel.texts);
    }
    for text in ["NEEDS YOU", "When should step 3 run?", "Let it all run now", "Something else"] {
        assert!(!panel.drawn(text), "{text:?} drawn in {:?}", panel.texts);
    }
    assert!(panel.drawn("4% \u{b7} about 9h 46m left"), "{:?}", panel.texts);
    assert!(!panel.drawn("recap-people-backfill"), "the Now line took the job, no second card");
    for stage in ["Build", "Ship", "Backfill", "Verify"] {
        assert!(panel.drawn(stage), "{stage:?} bar");
    }
    assert!(!panel.drawn("Step 3 of 4"));
}

#[test]
fn a_working_report_has_no_question_card_and_a_done_one_reads_as_a_summary_with_every_stage() {
    let working = recap_panel("working", 520.0);
    assert!(!working.drawn("NEEDS YOU") && working.drawn("Now"));
    assert!(working.drawn("\u{25cf} Working"));
    assert!(!working.texts.iter().any(|(text, _)| text.contains("updated")));
    let mut done = recap_panel("done", 520.0);
    done.jobs.clear();
    done.frame(Vec::new());
    assert!(done.drawn("Summary") && !done.drawn("Now"), "{:?}", done.texts);
    assert!(done.drawn("\u{25cf} Done"));
    assert!(done.drawn("BUILD") || done.drawn("BACKFILL"));
    assert!(!done.drawn("Show 1 finished stage"), "all stages are over, so all of them show");
}

#[test]
fn a_wait_with_no_job_shows_what_it_waits_on_at_the_right_of_the_now_line() {
    let mut panel = recap_panel("working", 520.0);
    panel.jobs.clear();
    let mut ledger = recap_report("working");
    ledger.status.state = Some(tidewatch_core::progress::State::Waiting);
    ledger.waiting_on = Some(tidewatch_core::progress_report::WaitingOn::Pr(2340));
    panel.ledger = Some(TabLedger { ledger, updated_ms: Some(0) });
    panel.frame(Vec::new());
    assert!(panel.drawn("PR #2340"), "{:?}", panel.texts);
    assert!(panel.drawn("\u{25cf} Waiting"));
}

#[test]
fn the_state_the_tab_shows_decides_the_word_even_when_the_report_says_otherwise() {
    let mut panel = recap_panel("done", 520.0);
    panel.shown = Some(tidewatch_core::progress::State::Working);
    panel.frame(Vec::new());
    assert!(panel.drawn("\u{25cf} Working"), "{:?}", panel.texts);
}

#[test]
fn no_board_text_overlaps_or_leaves_the_card_at_any_width() {
    for width in [200.0, 240.0, 300.0, 420.0, 640.0] {
        let panel = recap_panel("needs_you", width);
        for (index, (a, a_rect)) in panel.texts.iter().enumerate() {
            assert!(a_rect.right() <= width + 0.5, "{a:?} runs past the card at {width}");
            for (b, b_rect) in &panel.texts[index + 1..] {
                assert!(
                    !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                    "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                );
            }
        }
    }
}

const AT: i64 = 240_000;
const MIN: i64 = 60_000;
const QUESTION: &str = "Post the Slack follow-up";

fn shared(url: &str, title: &str, note: Option<&str>, ago_min: i64) -> Link {
    Link {
        url: url.to_owned(),
        title: title.to_owned(),
        note: note.map(str::to_owned),
        first_seen_ms: AT - ago_min * MIN,
        title_from: TitleFrom::Agent,
        state: LinkState::Unknown,
    }
}

fn five_links() -> LinkBoard {
    let mut board = LinkBoard::default();
    for link in [
        shared("https://example.com/notes", "Notes on bonus pools in other teams", None, 300),
        shared("https://github.com/o/r/issues/38", "#38 Spend page loads slowly", None, 180),
        shared("https://docs.google.com/document/d/1", "Mercury spend export, October", None, 60),
        shared("https://github.com/o/r/pull/42", "#42 feat: profit and spend dashboard", None, 40),
        shared("https://claude.ai/artifact/abc", "Split options compared", Some(QUESTION), 12),
    ] {
        board.absorb(&link);
    }
    board
}

fn with_links(text: &str, width: f32, board: LinkBoard) -> Panel {
    let mut panel = Panel::new(text, width);
    panel.links = Some(board);
    panel.frame(Vec::new());
    panel.frame(Vec::new());
    panel
}

fn top_of(panel: &Panel, text: &str) -> f32 {
    panel
        .texts
        .iter()
        .find(|(drawn, _)| drawn == text)
        .unwrap_or_else(|| panic!("{text:?} not drawn in {:?}", panel.texts))
        .1
        .top()
}

fn assert_clean(panel: &Panel, width: f32) {
    for (index, (a, a_rect)) in panel.texts.iter().enumerate() {
        assert!(a_rect.right() <= width + 0.5, "{a:?} runs past the card at {width}");
        for (b, b_rect) in &panel.texts[index + 1..] {
            assert!(
                !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
            );
        }
    }
}

#[test]
fn the_links_card_sits_under_the_title_and_the_question_is_not_in_the_panel() {
    let panel = with_links(ASKING, 420.0, five_links());
    assert!(top_of(&panel, "Ship it") < top_of(&panel, "LINKS"));
    assert!(!panel.drawn("NEEDS YOU"));
    assert!(panel.drawn("5"), "the count tag");
}

#[test]
fn without_links_there_is_no_card() {
    let none = Panel::new(ASKING, 420.0);
    assert!(!none.drawn("LINKS"));
    let empty = with_links(ASKING, 420.0, LinkBoard::default());
    assert!(!empty.drawn("LINKS"));
}

#[test]
fn the_card_lists_the_newest_three_with_kind_tags_and_meta_then_show_all_expands_in_place() {
    let mut panel = with_links(ASKING, 520.0, five_links());
    for shown in [
        "Split options compared",
        "#42 feat: profit and spend dashboard",
        "Mercury spend export, October",
    ] {
        assert!(panel.drawn(shown), "{shown}");
    }
    for hidden in ["#38 Spend page loads slowly", "Notes on bonus pools in other teams"] {
        assert!(!panel.drawn(hidden), "{hidden}");
    }
    for tag in ["Artifact", "PR", "Doc"] {
        assert!(panel.drawn(tag), "{tag}");
    }
    assert!(panel.drawn(&format!("claude.ai \u{b7} for: {QUESTION} \u{b7} 12m ago")));
    assert!(panel.drawn("github.com \u{b7} 40m ago"));
    let at = panel.center_of("Show all 5");
    panel.click(at);
    assert!(panel.state.all_links);
    for shown in ["#38 Spend page loads slowly", "Notes on bonus pools in other teams"] {
        assert!(panel.drawn(shown), "{shown}");
    }
    assert!(panel.drawn("Issue") && panel.drawn("Web"));
    assert!(!panel.drawn("Show all 5") && panel.drawn("Show less"));
    let at = panel.center_of("Show less");
    panel.click(at);
    assert!(!panel.state.all_links && !panel.drawn("#38 Spend page loads slowly"));
}

#[test]
fn three_links_or_fewer_have_no_fold() {
    let mut board = LinkBoard::default();
    for n in 0..3 {
        board.absorb(&shared(&format!("https://a.dev/{n}"), &format!("Link {n}"), None, n));
    }
    let panel = with_links(ASKING, 420.0, board);
    assert!(!panel.drawn("Show all 3") && !panel.drawn("Show less"));
    assert!(panel.drawn("Link 0") && panel.drawn("Link 2"));
}

#[test]
fn no_links_card_text_overlaps_or_leaves_the_card_at_any_width_collapsed_or_open() {
    for width in [200.0, 240.0, 300.0, 420.0, 640.0] {
        let mut panel = with_links(ASKING, width, five_links());
        assert_clean(&panel, width);
        let at = panel.center_of("Show all 5");
        panel.click(at);
        assert_clean(&panel, width);
    }
}

#[test]
fn no_links_card_text_overlaps_with_no_ledger_either() {
    for width in [200.0, 300.0, 520.0] {
        let mut panel = Panel::new(ASKING, width);
        panel.ledger = None;
        panel.links = Some(five_links());
        panel.frame(Vec::new());
        panel.frame(Vec::new());
        assert!(panel.drawn("LINKS") && panel.drawn("Split options compared"), "{width}");
        assert_clean(&panel, width);
    }
}

#[test]
fn a_tab_with_links_and_no_ledger_still_shows_the_card() {
    let mut panel = Panel::new(ASKING, 420.0);
    panel.ledger = None;
    panel.links = Some(five_links());
    panel.frame(Vec::new());
    panel.frame(Vec::new());
    assert!(panel.drawn("LINKS"));
    assert!(!panel.drawn("NEEDS YOU"));
}

#[test]
fn a_click_on_a_title_opens_the_link_and_the_copy_button_asks_for_the_address_to_be_copied() {
    let mut panel = with_links(ASKING, 520.0, five_links());
    let at = panel.center_of("Split options compared");
    assert_eq!(panel.click(at), None);
    assert_eq!(panel.opened, ["https://claude.ai/artifact/abc"]);
    let copy = panel.center_of(stdusk_egui::ui::icons::COPY);
    assert_eq!(
        panel.click(copy),
        Some(Action::CopyLink("https://claude.ai/artifact/abc".to_owned()))
    );
    assert_eq!(panel.opened.len(), 1, "copying does not open");
}

#[test]
fn a_links_card_with_a_long_title_and_meta_is_cut_not_overlapped() {
    let mut board = LinkBoard::default();
    board.absorb(&shared(
        "https://a-very-long-host-name.example.com/a/very/long/path",
        &"A title that goes on and on ".repeat(4),
        Some(&"a note that explains and explains ".repeat(4)),
        3,
    ));
    for width in [200.0, 300.0, 640.0] {
        let panel = with_links(ASKING, width, board.clone());
        assert_clean(&panel, width);
    }
}

fn stamp_drawn(panel: &Panel) -> bool {
    panel.texts.iter().any(|(text, _)| text.starts_with("Checked "))
}

#[test]
fn a_waiting_check_is_one_blue_line_with_a_stamp_and_no_check_again() {
    let look = Look::Report(waiting_report(), STAMP);
    let panel = tall(SMALL, 420.0, turn(Some(look), Ok(())));
    assert!(panel.drawn("Waiting"), "{:?}", panel.texts);
    assert!(
        panel.drawn("Waiting until Wed 28 Oct: Live list across a day boundary"),
        "{:?}",
        panel.texts
    );
    assert!(!panel.drawn("Check again") && !panel.drawn("Ask agent to fix"));
    assert!(stamp_drawn(&panel), "{:?}", panel.texts);
}

#[test]
fn a_waiting_check_asks_to_be_checked_again_once_its_day_comes() {
    let look = Look::Report(waiting_report(), STAMP);
    let due = LastTurn { today: Date::parse("2026-10-28").unwrap(), ..turn(Some(look), Ok(())) };
    let panel = tall(SMALL, 420.0, due);
    assert!(panel.drawn("Not verified"), "{:?}", panel.texts);
    assert!(panel.drawn("Check again"));
    assert!(!panel.drawn("Waiting until Wed 28 Oct: Live list across a day boundary"));
}

#[test]
fn a_check_that_runs_again_keeps_its_card_and_shows_a_button_that_does_nothing() {
    let look = Look::Checking(Some(blocked_report()));
    let mut panel = tall(SMALL, 420.0, turn(Some(look), Ok(())));
    assert!(panel.drawn("Pushed: 2 commits not pushed"), "{:?}", panel.texts);
    assert!(panel.drawn("Checking\u{2026}"), "{:?}", panel.texts);
    assert!(!panel.drawn("Check again") && !stamp_drawn(&panel));
    let at = panel.center_of("Ask agent to fix");
    assert_eq!(panel.click(at), None);
    let at = panel.center_of("Checking\u{2026}");
    assert_eq!(panel.click(at), None);
}

#[test]
fn an_answer_says_when_it_was_made_and_when_it_was_the_same() {
    let again = Checked { same: true, ..STAMP };
    for (checked, same) in [(STAMP, false), (again, true)] {
        let look = Look::Report(blocked_report(), checked);
        let panel = tall(SMALL, 420.0, turn(Some(look), Ok(())));
        assert!(stamp_drawn(&panel), "{:?}", panel.texts);
        let said = panel
            .texts
            .iter()
            .any(|(t, _)| t.starts_with("Checked ") && t.ends_with("same result"));
        assert_eq!(said, same, "{:?}", panel.texts);
        assert!(panel.drawn("Check again"));
    }
}
fn archived(url: &str, title: &str, ago_min: i64, state: LinkState) -> Link {
    Link { state, ..shared(url, title, None, ago_min) }
}

fn with_archive() -> LinkBoard {
    let mut board = five_links();
    board.absorb(&archived(
        "https://github.com/o/r/pull/9",
        "#9 fix: round totals",
        700,
        LinkState::Merged,
    ));
    board.absorb(&archived(
        "https://github.com/o/r/issues/7",
        "#7 Export fails",
        900,
        LinkState::Closed,
    ));
    board
}

#[test]
fn merged_and_closed_links_sit_in_a_folded_group_at_the_bottom_and_stay_out_of_the_count() {
    let panel = with_links(ASKING, 520.0, with_archive());
    assert!(panel.drawn("Merged and closed (2)"));
    for hidden in ["#9 fix: round totals", "#7 Export fails", "Merged", "Closed"] {
        assert!(!panel.drawn(hidden), "{hidden}");
    }
    assert!(panel.drawn("5"), "the header counts the main list only");
    assert!(!panel.drawn("7"));
    assert!(panel.drawn("Show all 5"));
    assert!(top_of(&panel, "Show all 5") < top_of(&panel, "Merged and closed (2)"));
}

#[test]
fn a_click_on_the_group_header_opens_it_and_a_second_click_folds_it_again() {
    let mut panel = with_links(ASKING, 520.0, with_archive());
    let at = panel.center_of("Merged and closed (2)");
    panel.click(at);
    assert!(panel.state.archive_links);
    for shown in ["#9 fix: round totals", "#7 Export fails", "Merged", "Closed"] {
        assert!(panel.drawn(shown), "{shown}");
    }
    assert!(top_of(&panel, "Merged and closed (2)") < top_of(&panel, "#9 fix: round totals"));
    assert_clean(&panel, 520.0);
    let at = panel.center_of("Merged and closed (2)");
    panel.click(at);
    assert!(!panel.state.archive_links && !panel.drawn("#9 fix: round totals"));
}

#[test]
fn an_archived_row_opens_and_copies_like_any_other_row() {
    let mut panel = with_links(ASKING, 520.0, with_archive());
    let at = panel.center_of("Merged and closed (2)");
    panel.click(at);
    let at = panel.center_of("#9 fix: round totals");
    assert_eq!(panel.click(at), None);
    assert_eq!(panel.opened, ["https://github.com/o/r/pull/9"]);
    let copies: Vec<Rect> = panel
        .texts
        .iter()
        .filter(|(text, _)| text == stdusk_egui::ui::icons::COPY)
        .map(|(_, rect)| *rect)
        .collect();
    assert_eq!(copies.len(), 5, "one copy button per drawn row: three main and two archived");
    let last = copies.iter().max_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
    assert_eq!(
        panel.click(last.center()),
        Some(Action::CopyLink("https://github.com/o/r/issues/7".to_owned()))
    );
}

#[test]
fn no_card_text_overlaps_or_leaves_the_card_with_the_group_folded_or_open_at_any_width() {
    for width in [200.0, 240.0, 300.0, 420.0, 640.0] {
        let mut panel = with_links(ASKING, width, with_archive());
        assert_clean(&panel, width);
        let at = panel.center_of("Merged and closed (2)");
        panel.click(at);
        assert_clean(&panel, width);
    }
}

#[test]
fn a_board_with_nothing_merged_or_closed_has_no_group() {
    let panel = with_links(ASKING, 520.0, five_links());
    assert!(!panel.texts.iter().any(|(text, _)| text.starts_with("Merged and closed")));
}

#[test]
fn a_board_of_only_merged_links_still_shows_the_card_with_the_group() {
    let mut board = LinkBoard::default();
    board.absorb(&archived(
        "https://github.com/o/r/pull/9",
        "#9 fix: round totals",
        70,
        LinkState::Merged,
    ));
    let panel = with_links(ASKING, 420.0, board);
    assert!(panel.drawn("LINKS") && panel.drawn("Merged and closed (1)"));
    assert!(!panel.drawn("#9 fix: round totals"));
}

fn aged(report: &str) -> Blocks {
    Blocks { report: Some(report.to_owned()), ..Blocks::default() }
}

fn rect_of(panel: &Panel, text: &str) -> Rect {
    panel
        .texts
        .iter()
        .find(|(drawn, _)| drawn == text)
        .unwrap_or_else(|| panic!("{text:?} not in {:?}", panel.texts))
        .1
}

#[test]
fn the_report_shows_its_age_at_the_right_edge_of_its_meta_row() {
    let mut panel = Panel::new(LONG, 420.0);
    panel.blocks = aged("2m ago");
    panel.frame(Vec::new());
    let meta = rect_of(&panel, "\u{25cf} Needs you");
    let age = rect_of(&panel, "2m ago");
    assert!((meta.center().y - age.center().y).abs() < 2.0, "{meta:?} {age:?}");
    assert!(age.left() > meta.right() + 20.0 && age.right() > 420.0 - 30.0, "{age:?}");
    assert!(
        !panel.drawn("out of date")
            && !panel.texts.iter().any(|(t, _)| t.contains("happened since"))
    );
}

#[test]
fn a_report_the_facts_contradict_is_tagged_right_after_its_title_with_the_count_under_it() {
    let mut panel = Panel::new(LONG, 420.0);
    panel.blocks = Blocks {
        out_of_date: Some(Why::WorkingButTurnEnded),
        since: Some("3 things happened since".to_owned()),
        ..aged("14m ago")
    };
    panel.frame(Vec::new());
    let title = panel.texts.iter().find(|(text, _)| text.starts_with("Add a slugify")).unwrap().1;
    let tag = rect_of(&panel, "out of date");
    let meta = rect_of(&panel, "\u{25cf} Needs you");
    let since = rect_of(&panel, "3 things happened since");
    assert!((title.center().y - tag.center().y).abs() < 6.0, "{title:?} {tag:?}");
    assert!(tag.left() >= title.right(), "{title:?} {tag:?}");
    assert!(
        since.top() >= meta.bottom() && since.left() <= meta.left() + 0.5,
        "{meta:?} {since:?}"
    );
    assert!(panel.drawn("14m ago"));
}

#[test]
fn the_age_the_tag_and_the_count_never_overlap_or_leave_the_card() {
    for width in [200.0, 240.0, 300.0, 420.0, 640.0] {
        let mut panel = Panel::new(LONG, width);
        panel.blocks = Blocks {
            out_of_date: Some(Why::NeedsYouNoQuestion),
            since: Some("12 things happened since".to_owned()),
            jobs: Some("no word since 14:02".to_owned()),
            turn: Some("no word since 14:02".to_owned()),
            ..aged("11h ago")
        };
        panel.jobs = vec![backfill_job()];
        panel.last_turn = Some(turn(None, Ok(())));
        panel.size = vec2(width, 2600.0);
        panel.frame(Vec::new());
        assert_clean(&panel, width);
    }
}

#[test]
fn a_step_tidewatch_ticked_reads_done_with_by_tidewatch_and_one_the_agent_finished_does_not() {
    let mut panel = Panel::new(LONG, 420.0);
    let mark = |step: &str| MarkSeen { step: step.to_owned(), by: StepBy::Tidewatch, at_ms: 0 };
    panel.marks = vec![
        mark("Pure logic in its own small function with a unit test"),
        mark("Smallest change that does the job"),
    ];
    panel.frame(Vec::new());
    let ticked = panel.texts.iter().filter(|(text, _)| text.contains("by Tidewatch")).count();
    assert_eq!(ticked, 1, "{:?}", panel.texts);
    assert!(panel.drawn("by Tidewatch"));
    let agent_done = panel.texts.iter().filter(|(text, _)| text == "reused two helpers").count();
    assert_eq!(agent_done, 1, "its own note stays and gets no label: {:?}", panel.texts);
}

#[test]
fn the_jobs_the_links_and_the_last_turn_each_carry_their_own_age() {
    let mut panel = Panel::new(LONG, 420.0);
    panel.size = vec2(420.0, 2600.0);
    panel.jobs = vec![
        backfill_job(),
        Card::Quiet(crate::job_model::QuietLine {
            id: 9,
            name: "deploy".to_owned(),
            elapsed: "3m".to_owned(),
        }),
    ];
    panel.links = Some(five_links());
    panel.last_turn = Some(turn(None, Ok(())));
    panel.blocks = Blocks {
        jobs: Some("1m ago".to_owned()),
        links: Some("5m ago".to_owned()),
        turn: Some("3m ago".to_owned()),
        ..aged("2m ago")
    };
    panel.frame(Vec::new());
    panel.frame(Vec::new());
    for (head, age) in [("JOBS", "1m ago"), ("LINKS", "5m ago"), ("Last turn", "3m ago")] {
        let head = rect_of(&panel, head);
        let age = rect_of(&panel, age);
        assert!((head.center().y - age.center().y).abs() < 4.0, "{head:?} {age:?}");
        assert!(age.left() > head.right(), "{head:?} {age:?}");
    }
}

#[test]
fn a_block_with_no_age_draws_none() {
    let mut panel = Panel::new(LONG, 420.0);
    panel.size = vec2(420.0, 2600.0);
    panel.jobs = vec![backfill_job()];
    panel.last_turn = Some(turn(None, Ok(())));
    panel.frame(Vec::new());
    assert!(!panel.drawn("JOBS"));
    assert!(
        !panel.texts.iter().any(|(text, _)| text.ends_with("ago") || text.starts_with("no word"))
    );
}

#[test]
fn a_quiet_turn_source_takes_the_age_slot_of_the_header_with_no_word_since() {
    let mut panel = Panel::new(LONG, 420.0);
    panel.blocks = Blocks { turn: Some("no word since 14:02".to_owned()), ..aged("8m ago") };
    panel.frame(Vec::new());
    let meta = rect_of(&panel, "\u{25cf} Needs you");
    let quiet = rect_of(&panel, "no word since 14:02");
    assert!((meta.center().y - quiet.center().y).abs() < 2.0, "{meta:?} {quiet:?}");
    assert!(quiet.right() > 420.0 - 30.0, "{quiet:?}");
    assert!(!panel.drawn("8m ago") && !panel.drawn("Last turn"), "{:?}", panel.texts);
    panel.blocks = Blocks { turn: Some("now".to_owned()), ..aged("8m ago") };
    panel.frame(Vec::new());
    assert!(panel.drawn("8m ago") && !panel.drawn("no word since 14:02"));
}
