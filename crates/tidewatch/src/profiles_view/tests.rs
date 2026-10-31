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
const SMALL_WINDOWS: [Vec2; 6] = [
    Vec2::new(690.0, 578.0),
    Vec2::new(716.0, 578.0),
    Vec2::new(740.0, 578.0),
    Vec2::new(690.0, 700.0),
    Vec2::new(716.0, 700.0),
    Vec2::new(740.0, 700.0),
];

fn input(area: Vec2, events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), area)),
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

struct Painted {
    text: String,
    rect: Rect,
    full: Rect,
    color: Color32,
}

fn painted(out: &egui::FullOutput) -> Vec<Painted> {
    out.shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => {
                let rect = text.visual_bounding_rect().intersect(clipped.clip_rect);
                let color = text.galley.job.sections.first()?.format.color;
                (rect.width() > 0.0 && rect.height() > 0.0).then(|| Painted {
                    text: text.galley.text().to_owned(),
                    rect,
                    full: text.visual_bounding_rect(),
                    color,
                })
            }
            _ => None,
        })
        .collect()
}

struct Harness {
    ctx: egui::Context,
    editor: Editor,
    area: Vec2,
    seen: Vec<Painted>,
}

impl Harness {
    fn new(editor: Editor) -> Self {
        Self::sized(editor, AREA)
    }

    fn sized(editor: Editor, area: Vec2) -> Self {
        let ctx = crate::fonts::test_context();
        crate::brand::tint_visuals(&ctx);
        Self { ctx, editor, area, seen: Vec::new() }
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Action> {
        let mut answer = None;
        let editor = &mut self.editor;
        let out = self.ctx.run_ui(input(self.area, events), |ui| {
            let area = ui.max_rect();
            answer = page(ui, area, editor, &probes(), "profiles.toml");
        });
        self.seen = painted(&out);
        answer
    }

    fn card(&self) -> Rect {
        let inner = page_area(Rect::from_min_size(pos2(0.0, 0.0), self.area));
        columns(inner, inner.top() + crate::settings_ui::HEADER_H).1
    }

    fn on_card(&self, text: &str) -> Option<&Painted> {
        let card = self.card();
        self.seen.iter().find(|painted| painted.text == text && card.contains_rect(painted.rect))
    }

    fn click_text(&mut self, text: &str) {
        self.frame(vec![]);
        let rect = self.on_card(text).unwrap_or_else(|| panic!("{text:?} is on the card")).rect;
        self.click(rect.center());
    }

    fn open_more_menu(&mut self) {
        let card = self.card();
        let header = Rect::from_min_size(card.min, vec2(card.width(), 76.0));
        self.frame(vec![]);
        for y in (header.top() as i32 + 8..header.bottom() as i32 - 8).step_by(6) {
            for x in (header.right() as i32 - 80..header.right() as i32).step_by(6) {
                self.click(pos2(x as f32, y as f32));
                if egui::Popup::is_any_open(&self.ctx) {
                    self.frame(vec![]);
                    return;
                }
            }
        }
        panic!("the header has a button that opens a menu");
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
    let found = harness.scan(114..394, 100..700, |h| h.editor.selected == last);
    assert!(found, "a row of the list selects its profile");
    assert!(!harness.editor.dirty, "choosing a profile is not an edit");
}

fn claude_draft() -> Editor {
    let mut editor = editor();
    editor.drafts[0].agent = "claude".into();
    editor.drafts[0].model = "opus".into();
    editor.drafts[0].effort = "medium".into();
    editor.drafts[0].level = Level::Edits;
    editor
}

#[test]
fn agent_effort_and_permission_are_segmented_controls_with_every_option() {
    let mut harness = Harness::new(claude_draft());
    harness.frame(vec![]);
    let agents: Vec<String> = agents().into_iter().map(|(_, name)| name).collect();
    let efforts = ["Default (medium)", "Low", "Medium", "High", "Extra high", "Max"];
    let levels = Level::ALL.map(Level::label);
    for label in agents.iter().map(String::as_str).chain(efforts).chain(levels) {
        assert!(harness.on_card(label).is_some(), "{label} is a segment on the card");
    }
    for (field, first) in [("Agent", "Claude Code"), ("Effort", "Default (medium)")] {
        let label = harness.on_card(field).unwrap().rect;
        let segment = harness.on_card(first).unwrap().rect;
        let edge = segment.left() - label.left();
        assert!((12.0..=14.0).contains(&edge), "{field}: {first} starts {edge} px in");
    }
    let label = harness.on_card("Permission level").unwrap().rect;
    let segment = harness.on_card("Read only").unwrap().rect;
    assert!((12.0..=14.0).contains(&(segment.left() - label.left())));
}

type Holds = fn(&Draft) -> bool;

#[test]
fn clicking_a_segment_sets_the_agent_the_effort_or_the_level_and_marks_the_page_unsaved() {
    let cases: [(&str, Holds); 5] = [
        ("Codex", |d| d.agent == "codex"),
        ("High", |d| d.effort == "high"),
        ("Extra high", |d| d.effort == "xhigh"),
        ("Default (medium)", |d| d.effort.is_empty()),
        ("Read only", |d| d.level == Level::Plan),
    ];
    for (label, holds) in cases {
        let mut harness = Harness::new(claude_draft());
        harness.click_text(label);
        assert!(holds(&harness.editor.drafts[0]), "{label}");
        assert!(harness.editor.dirty, "{label} is an edit");
    }
    let mut harness = Harness::new(claude_draft());
    harness.click_text("YOLO");
    assert_eq!(harness.editor.drafts[0].level, Level::Yolo);
}

#[test]
fn yolo_keeps_its_red_ink_in_the_segment() {
    let mut harness = Harness::new(claude_draft());
    harness.frame(vec![]);
    assert_eq!(harness.on_card("YOLO").unwrap().color, crate::brand::RED);
    assert_ne!(harness.on_card("Auto").unwrap().color, crate::brand::RED);
    harness.editor.drafts[0].level = Level::Yolo;
    harness.frame(vec![]);
    assert_eq!(harness.on_card("YOLO").unwrap().color, crate::brand::RED);
}

#[test]
fn a_description_line_sits_directly_under_effort_and_under_permission() {
    let mut harness = Harness::new(claude_draft());
    harness.frame(vec![]);
    let pairs = [
        ("Medium", effort_line("medium", Some("medium"))),
        ("Auto edit", level_note(Level::Edits).to_owned()),
    ];
    for (segment, line) in pairs {
        let control = harness.on_card(segment).unwrap().rect;
        let line = harness.on_card(&line).unwrap_or_else(|| panic!("{line:?} is on the card")).rect;
        let gap = line.top() - control.bottom();
        assert!((0.0..=24.0).contains(&gap), "{segment}: the line is {gap} px under the control");
        assert!(line.height() < 20.0, "{segment}: one line, not {}", line.height());
        assert_eq!(line.left(), harness.on_card("Effort").unwrap().rect.left());
    }
}

#[test]
fn spare_height_goes_below_the_last_field_and_never_between_fields() {
    let places = |height: f32| {
        let mut harness = Harness::sized(claude_draft(), vec2(AREA.x, height));
        harness.frame(vec![]);
        [
            "Agent",
            "Effort",
            "Permission level",
            "Advanced: setup command, extra arguments, push guard, house style",
        ]
        .map(|text| harness.on_card(text).unwrap_or_else(|| panic!("{text}")).rect.top())
    };
    assert_eq!(places(752.0), places(1000.0));
}

#[test]
fn new_profile_adds_one_and_selects_it() {
    let mut harness = Harness::new(editor());
    let count = harness.editor.drafts.len();
    let found = harness.scan(820..940, 40..110, |h| h.editor.drafts.len() > count);
    assert!(found, "the button at the top right adds a profile");
    assert_eq!(harness.editor.selected, count);
    assert!(harness.editor.dirty);
}

#[test]
fn the_save_bar_saves_and_reverts_only_once_there_are_edits() {
    let mut harness = Harness::new(editor());
    harness.frame(vec![]);
    let mut saves = None;
    for x in (786..936).step_by(6) {
        saves = saves.or(harness.click(pos2(x as f32, 698.0)));
    }
    assert!(saves.is_none(), "no bar, no Save");

    harness.editor.drafts[0].name = "Changed".into();
    harness.editor.touched();
    harness.frame(vec![]);
    let mut saves = None;
    for x in (878..936).step_by(6) {
        saves = saves.or(harness.click(pos2(x as f32, 698.0)));
    }
    assert!(matches!(saves, Some(Action::Save)));
    assert!(
        harness.editor.dirty,
        "saving is the app's job, the editor stays dirty until it answers"
    );

    for x in (786..866).step_by(6) {
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
fn the_summary_lists_model_effort_and_level_and_skips_what_is_empty() {
    let mut draft = editor().drafts[0].clone();
    draft.model = " opus ".into();
    draft.effort = "medium".into();
    draft.level = Level::Edits;
    assert_eq!(summary(&draft), "opus \u{b7} medium \u{b7} Auto edit");
    draft.model.clear();
    draft.effort = "  ".into();
    assert_eq!(summary(&draft), "Auto edit");
}

#[test]
fn every_level_has_its_note_and_effort_keeps_a_custom_value_visible() {
    for level in Level::ALL {
        let note = level_note(level);
        assert!(note.len() > 8, "{level:?}");
        assert!(note.starts_with(char::is_uppercase), "{note}");
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
fn every_effort_has_its_own_one_line_description_and_a_custom_one_is_passed_through() {
    let lines: Vec<String> =
        ["", "low", "medium", "high", "xhigh", "max", "ultra"].map(|e| effort_line(e, None)).into();
    let mut distinct = lines.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), lines.len(), "{lines:?}");
    assert!(lines.iter().all(|line| line.len() < 60 && !line.contains('\n')), "{lines:?}");
    assert_eq!(effort_line(" high ", None), effort_line("high", None));
    assert!(effort_line("", Some("high")).contains("high"));
    assert!(!effort_line("", None).contains('('));
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

fn draw_in(width: f32, add: impl FnMut(&mut Ui)) -> Vec<(String, Rect)> {
    let ctx = crate::fonts::test_context();
    let mut add = add;
    let raw = || RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(width, 600.0))),
        ..RawInput::default()
    };
    let _ = ctx.run_ui(raw(), |ui| add(ui));
    let out = ctx.run_ui(raw(), |ui| add(ui));
    crate::screen_kit::painted_texts(&out)
}

#[test]
fn the_card_header_keeps_its_subtitle_clear_of_the_more_button() {
    let line = "opus \u{b7} medium \u{b7} Auto edit and a long tail that has to be cut short";
    let texts = draw_in(320.0, |ui| {
        let header = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(315.0, 96.0));
        crate::settings_ui::editor::card_header(ui, header, None, "Builder", line);
    });
    let more_left = 315.0 - 20.0 - crate::screen_kit::Size::Compact.height();
    let subtitle = texts.iter().find(|(text, _)| text.starts_with("opus")).unwrap().1;
    assert!(subtitle.right() < more_left, "{subtitle:?} {more_left}");
    assert_eq!(texts.iter().filter(|(text, _)| text == "Delete").count(), 0);
}

#[test]
fn the_header_shows_no_delete_until_its_more_button_is_opened() {
    let mut harness = Harness::new(claude_draft());
    harness.frame(vec![]);
    assert!(harness.on_card("Delete").is_none());
    assert!(harness.on_card("Builder").is_some());
    assert!(harness.on_card("opus \u{b7} medium \u{b7} Auto edit").is_some());
    harness.open_more_menu();
    assert!(harness.seen.iter().any(|painted| painted.text == "Delete"));
}

#[test]
fn delete_in_the_more_menu_is_in_danger_ink_and_removes_the_profile() {
    let mut harness = Harness::new(claude_draft());
    let count = harness.editor.drafts.len();
    harness.open_more_menu();
    let delete = harness.seen.iter().find(|painted| painted.text == "Delete").unwrap();
    assert_eq!(delete.color, crate::brand::RED);
    let at = delete.rect.center();
    harness.click(at);
    assert_eq!(harness.editor.drafts.len(), count - 1);
    assert!(harness.editor.dirty);
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.shrink(0.5).intersects(b.shrink(0.5))
}

#[test]
fn the_editor_keeps_every_text_apart_and_inside_the_card_at_900x640_and_the_default_size() {
    for area in SMALL_WINDOWS.into_iter().chain([AREA]) {
        for state in 0..4 {
            let mut harness = Harness::sized(claude_draft(), area);
            match state {
                1 => harness.editor.drafts[0].level = Level::Yolo,
                2 => {
                    harness.editor.drafts[0].effort = "ultra".into();
                    harness.editor.drafts[0].agent = "ghost".into();
                    harness.editor.drafts[0].name = "A profile named with many words in it".into();
                }
                3 => {
                    harness.editor.drafts[0].name = "Changed".into();
                    harness.editor.touched();
                    harness.editor.error = Some("Could not save: disk full".into());
                }
                _ => {}
            }
            harness.frame(vec![]);
            harness.frame(vec![]);
            let card = harness.card();
            let inside: Vec<&Painted> =
                harness.seen.iter().filter(|p| card.contains(p.full.left_center())).collect();
            assert!(inside.len() > 12, "{area:?} {state}: the card has its texts");
            let typed = {
                let draft = &harness.editor.drafts[0];
                [draft.name.clone(), draft.model.clone()]
            };
            for painted in inside.iter().filter(|painted| !typed.contains(&painted.text)) {
                assert!(
                    painted.full.left() >= card.left() && painted.full.right() <= card.right(),
                    "{area:?} {state}: {:?} leaves the card {card:?}",
                    painted.text
                );
            }
            for (index, a) in inside.iter().enumerate() {
                for b in &inside[index + 1..] {
                    assert!(
                        !overlaps(a.rect, b.rect),
                        "{area:?} {state}: {:?} {:?} and {:?} {:?} overlap",
                        a.text,
                        a.rect,
                        b.text,
                        b.rect
                    );
                }
            }
        }
    }
}
