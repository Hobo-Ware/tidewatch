//! Agents and profiles page: rendering, selecting, editing, the save bar and the helpers.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Vec2, pos2};
use tidewatch_core::profiles::Profiles;

use super::*;

fn agents() -> Vec<(String, String)> {
    vec![
        ("claude".into(), "Claude Code".into()),
        ("codex".into(), "Codex".into()),
        ("gemini".into(), "Gemini CLI".into()),
    ]
}

fn editor() -> Editor {
    Editor::new(&Profiles::seeded(&["claude".to_owned(), "codex".to_owned()]), agents())
}

fn probes() -> Probes {
    Probes::from([
        ("claude".to_owned(), Availability::Found(Some("2.1.299".into()))),
        ("codex".to_owned(), Availability::Probing),
        ("gemini".to_owned(), Availability::Missing),
    ])
}

const AREA: Vec2 = Vec2::new(1048.0, 752.0);

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), AREA)),
        focused: true,
        events,
        ..RawInput::default()
    }
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
}

impl Harness {
    fn new(editor: Editor) -> Self {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        Self { ctx, editor }
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Action> {
        let mut answer = None;
        let editor = &mut self.editor;
        let _ = self.ctx.run_ui(input(events), |ui| {
            let area = ui.max_rect();
            answer = page(ui, area, editor, &probes(), "profiles.toml");
        });
        answer
    }

    fn click(&mut self, at: Pos2) -> Option<Action> {
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![press(at, true)]);
        self.frame(vec![press(at, false)])
    }

    fn scan(
        &mut self,
        xs: std::ops::Range<i32>,
        ys: std::ops::Range<i32>,
        until: impl Fn(&Self) -> bool,
    ) -> bool {
        self.frame(vec![]);
        for y in ys.step_by(6) {
            for x in xs.clone().step_by(6) {
                self.click(pos2(x as f32, y as f32));
                if until(self) {
                    return true;
                }
            }
        }
        false
    }
}

#[test]
fn the_page_renders_every_state_and_asks_for_nothing_until_clicked() {
    let mut harness = Harness::new(editor());
    for index in 0..harness.editor.drafts.len() {
        harness.editor.select(index);
        for _ in 0..2 {
            assert!(harness.frame(vec![]).is_none());
        }
    }
    harness.editor.drafts[0].level = Level::Yolo;
    harness.editor.drafts[0].effort = "ultra".into();
    harness.editor.drafts[1].agent = "ghost".into();
    harness.editor.touched();
    harness.editor.error = Some("Could not save: disk full".into());
    for _ in 0..2 {
        assert!(harness.frame(vec![]).is_none());
    }
    while !harness.editor.drafts.is_empty() {
        harness.editor.remove_selected();
    }
    assert!(harness.frame(vec![]).is_none());
}

#[test]
fn clicking_a_row_of_the_list_selects_that_profile() {
    let mut harness = Harness::new(editor());
    let last = harness.editor.drafts.len() - 1;
    let found = harness.scan(40..320, 100..700, |h| h.editor.selected == last);
    assert!(found, "a row of the list selects its profile");
    assert!(!harness.editor.dirty, "choosing a profile is not an edit");
}

#[test]
fn clicking_a_permission_card_sets_the_level_and_marks_the_page_unsaved() {
    let mut harness = Harness::new(editor());
    let before = harness.editor.drafts[0].level;
    let found = harness.scan(360..1010, 300..520, |h| h.editor.drafts[0].level != before);
    assert!(found, "one of the five cards changes the level");
    assert!(harness.editor.dirty);
}

#[test]
fn new_profile_adds_one_and_selects_it() {
    let mut harness = Harness::new(editor());
    let count = harness.editor.drafts.len();
    let found = harness.scan(900..1020, 40..110, |h| h.editor.drafts.len() > count);
    assert!(found, "the button at the top right adds a profile");
    assert_eq!(harness.editor.selected, count);
    assert!(harness.editor.dirty);
}

#[test]
fn the_save_bar_saves_and_reverts_only_once_there_are_edits() {
    let mut harness = Harness::new(editor());
    harness.frame(vec![]);
    let mut saves = None;
    for x in (860..1010).step_by(6) {
        saves = saves.or(harness.click(pos2(x as f32, 698.0)));
    }
    assert!(saves.is_none(), "no bar, no Save");

    harness.editor.drafts[0].name = "Changed".into();
    harness.editor.touched();
    harness.frame(vec![]);
    let mut saves = None;
    for x in (952..1010).step_by(6) {
        saves = saves.or(harness.click(pos2(x as f32, 698.0)));
    }
    assert!(matches!(saves, Some(Action::Save)));
    assert!(
        harness.editor.dirty,
        "saving is the app's job, the editor stays dirty until it answers"
    );

    for x in (860..940).step_by(6) {
        harness.click(pos2(x as f32, 698.0));
        if !harness.editor.dirty {
            break;
        }
    }
    assert!(!harness.editor.dirty, "Revert throws the edit away");
    assert_eq!(harness.editor.drafts[0].name, "Builder");
}

#[test]
fn group_headers_say_what_the_probe_found() {
    let found = Availability::Found(Some("2.1.299".into()));
    assert_eq!(probe_label(Some(&found)).0, "2.1.299 found");
    assert_eq!(probe_label(Some(&Availability::Found(None))).0, "found");
    assert_eq!(probe_label(Some(&Availability::Missing)), ("not installed".to_owned(), ORANGE));
    assert_eq!(probe_label(Some(&Availability::Probing)).0, "checking...");
    assert_eq!(probe_label(None).0, "checking...");
}

#[test]
fn the_summary_lists_agent_model_effort_and_level_and_skips_what_is_empty() {
    let mut draft = editor().drafts[0].clone();
    draft.model = " opus ".into();
    draft.effort = "high".into();
    draft.level = Level::Edits;
    assert_eq!(
        summary(&draft, "Claude Code"),
        "Claude Code \u{b7} opus \u{b7} high effort \u{b7} Auto edit"
    );
    draft.model.clear();
    draft.effort = "  ".into();
    assert_eq!(summary(&draft, "Claude Code"), "Claude Code \u{b7} Auto edit");
}

#[test]
fn every_level_has_its_note_and_effort_keeps_a_custom_value_visible() {
    for level in Level::ALL {
        assert!(level_note(level).len() > 8, "{level:?}");
    }
    let usual: Vec<String> =
        effort_choices("high", None).into_iter().map(|(value, _)| value).collect();
    assert_eq!(usual, ["", "low", "medium", "high", "xhigh", "max"]);
    let custom = effort_choices(" ultra ", None);
    assert_eq!(custom.last(), Some(&("ultra".to_owned(), "ultra".to_owned())));
    assert_eq!(custom.len(), 7);
    assert_eq!(effort_choices("", None).len(), 6);
    let labelled = effort_choices("", Some("high"));
    assert_eq!(labelled[0], (String::new(), "Default (high)".to_owned()));
    assert_eq!(effort_choices("", None)[0].1, "Default");
}

#[test]
fn each_known_agent_has_its_own_badge_and_an_unknown_one_a_neutral_letter() {
    let letters: Vec<String> =
        ["claude", "codex", "gemini", "copilot"].iter().map(|id| tint(id).0).collect();
    assert_eq!(letters, ["C", "X", "G", "P"]);
    let (letter, colors) = tint("aider");
    assert_eq!(letter, "A");
    assert_ne!(colors, tint("claude").1);
    assert_eq!(tint("").0, "");
}
