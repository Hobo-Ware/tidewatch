use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, pos2};

use super::*;

struct Harness {
    ctx: egui::Context,
    prefs: NewTabPrefs,
    profiles: Vec<ProfileOption>,
    clock: f64,
}

fn profile(id: &str, name: &str, agent: &str) -> ProfileOption {
    ProfileOption { id: id.into(), name: name.into(), agent_id: agent.into() }
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

impl Harness {
    fn new(prefs: NewTabPrefs) -> Self {
        let profiles = vec![
            profile("builder", "Builder", "claude"),
            profile("reviewer", "Reviewer", "claude"),
            profile("copilot-builder", "Copilot Builder", "copilot"),
        ];
        let mut harness = Self { ctx: crate::fonts::test_context(), prefs, profiles, clock: 0.0 };
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<NewTabPrefs> {
        self.clock += 0.1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            events,
            focused: true,
            time: Some(self.clock),
            ..RawInput::default()
        };
        let mut changed = None;
        let _ = self.ctx.run_ui(raw, |ui| changed = card(ui, &self.prefs, &self.profiles));
        changed
    }

    fn placed(&self) -> Placed {
        self.ctx.data(|data| data.get_temp(placed_id())).expect("the card has drawn")
    }

    fn click(&mut self, at: Pos2) -> Option<NewTabPrefs> {
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![press(at, true)]);
        let changed = self.frame(vec![press(at, false)]);
        self.frame(Vec::new());
        changed
    }

    fn click_segment(
        &mut self,
        control: Rect,
        labels: &[&str],
        index: usize,
    ) -> Option<NewTabPrefs> {
        let mut centers = Vec::new();
        let _ = self.ctx.run_ui(RawInput::default(), |ui| {
            centers = segmented::centers(ui, labels, control.right(), control.center().y);
        });
        self.click(centers[index])
    }
}

fn levels() -> [&'static str; 5] {
    level_labels()
}

#[test]
fn the_card_draws_every_row_and_changes_nothing_by_itself() {
    let mut harness = Harness::new(NewTabPrefs::default());
    let placed = harness.placed();
    let rows =
        [placed.agent, placed.permission, placed.remote, placed.start_from, placed.right_panel];
    let tops: Vec<f32> = rows.iter().map(|rect| rect.expect("a row").center().y).collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "rows run top to bottom: {tops:?}");
    assert_eq!(harness.frame(Vec::new()), None);
}

#[test]
fn each_permission_segment_but_yolo_becomes_the_default() {
    for (index, level) in
        [Level::Plan, Level::Ask, Level::Edits, Level::Auto].into_iter().enumerate()
    {
        let mut harness = Harness::new(NewTabPrefs::default());
        let control = harness.placed().permission.unwrap();
        let changed = harness.click_segment(control, &levels(), index);
        assert_eq!(
            changed,
            Some(NewTabPrefs { permission: Some(level), ..NewTabPrefs::default() })
        );
    }
}

#[test]
fn yolo_is_shown_but_cannot_be_picked_as_a_default() {
    let mut harness =
        Harness::new(NewTabPrefs { permission: Some(Level::Edits), ..NewTabPrefs::default() });
    let control = harness.placed().permission.unwrap();
    assert_eq!(harness.click_segment(control, &levels(), 4), None);
    assert!(levels().contains(&"YOLO"));
}

#[test]
fn the_remote_switch_flips_the_default() {
    let mut harness = Harness::new(NewTabPrefs::default());
    let at = harness.placed().remote.unwrap().center();
    let changed = harness.click(at);
    assert_eq!(changed, Some(NewTabPrefs { remote: true, ..NewTabPrefs::default() }));
    let mut on = Harness::new(NewTabPrefs { remote: true, ..NewTabPrefs::default() });
    let at = on.placed().remote.unwrap().center();
    assert_eq!(on.click(at), Some(NewTabPrefs::default()));
}

#[test]
fn start_from_and_right_panel_pick_their_other_option() {
    let mut harness = Harness::new(NewTabPrefs::default());
    let control = harness.placed().start_from.unwrap();
    let labels = START_FROM.map(|(_, label)| label);
    assert_eq!(harness.click_segment(control, &labels, 0), None, "already on");
    let changed = harness.click_segment(control, &labels, 1);
    assert_eq!(
        changed,
        Some(NewTabPrefs { start_from: StartFrom::DefaultBranch, ..NewTabPrefs::default() })
    );

    let mut harness = Harness::new(NewTabPrefs::default());
    let control = harness.placed().right_panel.unwrap();
    let labels = RIGHT_PANEL.map(|(_, label)| label);
    assert_eq!(harness.click_segment(control, &labels, 0), None);
    let changed = harness.click_segment(control, &labels, 1);
    assert_eq!(
        changed,
        Some(NewTabPrefs { right_panel: RightPanel::History, ..NewTabPrefs::default() })
    );
}

fn open_dropdown(harness: &mut Harness) -> Vec<Rect> {
    let field = harness.placed().agent.unwrap();
    assert_eq!(harness.click(field.center()), None);
    harness.frame(Vec::new());
    harness.ctx.data(|data| data.get_temp(options_id())).expect("the dropdown is open")
}

#[test]
fn the_agent_dropdown_lists_last_used_and_every_profile_and_picks_one() {
    let mut harness = Harness::new(NewTabPrefs::default());
    let options = open_dropdown(&mut harness);
    assert_eq!(options.len(), 4);
    let changed = harness.click(options[2].center());
    assert_eq!(
        changed,
        Some(NewTabPrefs { agent: Some("reviewer".into()), ..NewTabPrefs::default() })
    );
}

#[test]
fn last_used_clears_the_default_agent() {
    let prefs = NewTabPrefs { agent: Some("builder".into()), ..NewTabPrefs::default() };
    let mut harness = Harness::new(prefs);
    let options = open_dropdown(&mut harness);
    assert_eq!(harness.click(options[0].center()), Some(NewTabPrefs::default()));
}

#[test]
fn a_default_agent_that_no_longer_exists_still_shows_what_the_file_says() {
    let prefs = NewTabPrefs { agent: Some("deleted".into()), ..NewTabPrefs::default() };
    let mut harness = Harness::new(prefs);
    assert_eq!(harness.frame(Vec::new()), None);
    let options = open_dropdown(&mut harness);
    assert_eq!(options.len(), 4);
}
