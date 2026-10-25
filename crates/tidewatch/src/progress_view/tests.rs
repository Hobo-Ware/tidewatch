//! The panel: pure choices, and headless frames for the clicks and for text that never overlaps.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, pos2, vec2};
use tidewatch_core::done_check::{Kind, Line, Report, Status};
use tidewatch_core::end_block::EndBlock;
use tidewatch_core::progress::Ledger;

use crate::done_check_glue::Look;
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
        let output = self.ctx.run_ui(raw, |ui| {
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
fn the_answer_button_asks_to_answer_and_nothing_else_does() {
    let mut panel = Panel::new(LONG, 420.0);
    assert_eq!(panel.action, None);
    let at = panel.center_of("Answer");
    assert_eq!(panel.click(at), Some(Action::Answer));
}

#[test]
fn no_question_means_no_card_and_no_button() {
    let text = LONG.replace("## Needs you\n- Decide if accented letters should become ASCII (cafe). Current code keeps them -- it changes the output\n", "## Needs you\n- Nothing needs you.\n");
    let panel = Panel::new(&text, 420.0);
    assert!(!panel.drawn("Answer") && !panel.drawn("?"));
}

#[test]
fn more_than_eight_rows_hide_behind_show_all_and_it_toggles() {
    let mut panel = Panel::new(LONG, 420.0);
    assert!(panel.drawn("Show all 10"));
    assert!(!panel.drawn("Item nine") && !panel.drawn("Item ten"));
    let at = panel.center_of("Show all 10");
    panel.click(at);
    assert!(panel.state.show_all_rows());
    assert!(panel.drawn("Item ten") && panel.drawn("Show less"));
    let at = panel.center_of("Show less");
    panel.click(at);
    assert!(!panel.drawn("Item ten"));
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
fn the_header_names_the_step_never_a_ring_count_and_the_meta_line_says_state_and_age() {
    let panel = Panel::new(LONG, 420.0);
    assert!(panel.drawn("Step 2 of 4 \u{b7} Build"), "{:?}", panel.texts);
    assert!(!panel.texts.iter().any(|(text, _)| text == "3/13" || text.starts_with("0/")));
    assert!(
        panel.drawn("\u{25cf} Blocked \u{b7} updated 4 min ago"),
        "a blocked item makes it blocked"
    );
}

#[test]
fn an_old_ten_phase_ledger_still_reads_as_four_steps() {
    let text = "# Goal\nShip it\n\n## Status\nphase: 7 Review\nstate: waiting\n\n## Checklist\n### 1. Understand and decide\n- [x] a\n### 2. Build\n- [x] b\n### 3. Gate\n- [x] c\n### 4. Validate against production\n- [x] d\n### 5. Preview\n- [x] e\n### 6. PR\n- [x] Open it\n### 7. Review\n- [~] Wait for CI\n### 8. Ship\n- [ ] Merge\n### 9. Verify live\n- [ ] Errors\n### 10. Close out\n- [ ] Report\n";
    let panel = Panel::new(text, 420.0);
    assert!(panel.drawn("Step 3 of 4 \u{b7} PR"), "{:?}", panel.texts);
    assert!(panel.drawn("Open it") && panel.drawn("Wait for CI"));
    assert!(!panel.drawn("Merge") && !panel.drawn("Gate"));
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
        let at = panel.center_of("Show all 10");
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
        ("Which base should", "https://example.com/notes"),
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
    LastTurn::of(Some(&block), check, can_ask).unwrap()
}

fn blocked() -> Look {
    let line = |kind, status, detail: &str| Line { kind, status, detail: detail.to_owned() };
    Look::Report(Report {
        head: String::new(),
        lines: vec![
            line(Kind::Pushed, Status::Fail, "2 commits not pushed"),
            line(Kind::Unverified, Status::Unknown, "1 item the agent did not verify"),
        ],
        unverified: vec!["the Safari layout".to_owned()],
    })
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
        "Answer",
        "Ask agent to fix",
        "Check again",
    ] {
        assert!(panel.drawn(text), "{text:?} not drawn in {:?}", panel.texts);
    }
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
    let top = |text: &str| panel.texts.iter().find(|(t, _)| t == text).unwrap().1.top();
    assert!(top("Build") < top("Last turn") && top("Last turn") < top("Wire the list"));
}

#[test]
fn a_summary_that_asks_nothing_has_no_needs_you_and_no_answer() {
    let own = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
    let block =
        EndBlock { done: own(&["Did it"]), needs_you: own(&["nothing"]), next: own(&["Ship"]) };
    let panel = tall(SMALL, 420.0, LastTurn::of(Some(&block), None, Ok(())).unwrap());
    assert!(panel.drawn("Done") && panel.drawn("Next"));
    assert!(!panel.drawn("Needs you") && !panel.drawn("Answer") && !panel.drawn("Checks"));
}

#[test]
fn the_buttons_of_the_last_turn_send_the_same_asks_the_panel_always_did() {
    let mut panel = tall(SMALL, 420.0, turn(Some(blocked()), Ok(())));
    let at = panel.center_of("Answer");
    assert_eq!(panel.click(at), Some(Action::Answer));
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
fn a_ledger_question_keeps_the_one_answer_button_and_the_last_turn_adds_none() {
    let mut panel = tall(LONG, 420.0, turn(None, Ok(())));
    assert_eq!(panel.texts.iter().filter(|(text, _)| text == "Answer").count(), 1);
    let at = panel.center_of("Answer");
    assert_eq!(panel.click(at), Some(Action::Answer));
}

#[test]
fn a_tab_with_no_ledger_still_shows_its_last_turn() {
    let mut panel = Panel::new(SMALL, 420.0);
    panel.ledger = None;
    panel.last_turn = Some(turn(Some(Look::Checking), Ok(())));
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
    for width in [200.0, 240.0, 300.0, 420.0, 640.0] {
        let long = "Pick a default owner filter: me or everyone, and say whether the archived column follows it or keeps its own filters";
        let mut last = turn(Some(blocked()), Ok(()));
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
