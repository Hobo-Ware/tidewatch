use egui::{Event, Modifiers, RawInput, Rect, pos2, vec2};
use tidewatch_core::agents::Level;

use super::*;
use crate::change_tab::{Facts, Running};

fn card(id: &str, name: &str) -> Card {
    Card {
        id: id.to_owned(),
        name: name.to_owned(),
        detail: "opus \u{b7} high".to_owned(),
        agent_id: "claude".to_owned(),
        cli_name: "Claude".to_owned(),
        unsupported: Vec::new(),
        installed: true,
        level: Level::Edits,
    }
}

fn facts(working: bool) -> Facts {
    Facts {
        tab: 9,
        in_worktree: true,
        running: Running::Agent { agent_id: "claude".into(), cli_name: "Claude".into(), working },
        stored: None,
        current: Some(("builder".into(), Level::Edits)),
        seen_level: None,
        cards: vec![card("builder", "Builder"), card("reviewer", "Reviewer")],
        follows: None,
        busy: false,
        meeseeks: false,
        pinned: false,
        replaces: None,
    }
}

struct Harness {
    ctx: egui::Context,
    facts: Facts,
    pick: Pick,
}

impl Harness {
    fn new(facts: Facts, pick: Pick) -> Self {
        let mut harness = Self { ctx: crate::fonts::test_context(), facts, pick };
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Outcome> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let (facts, pick) = (&self.facts, &mut self.pick);
        let mut outcome = None;
        let _ = self.ctx.run_ui(raw, |ui| outcome = show(ui.ctx(), facts, pick));
        outcome
    }
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn reviewer_pick() -> Pick {
    Pick { kind: Kind::Agent, profile: "reviewer".into(), level: Level::Plan }
}

#[test]
fn enter_changes_with_the_picked_profile_and_level() {
    let mut harness = Harness::new(facts(false), reviewer_pick());
    assert_eq!(harness.frame(vec![key(Key::Enter)]), Some(Outcome::Change { interrupt: false }));
    let change = crate::change_tab::change(&harness.facts, &harness.pick, false).unwrap();
    assert_eq!(
        change.target,
        crate::retarget::Target::Agent { profile: "reviewer".into(), level: Some(Level::Plan) }
    );
}

#[test]
fn escape_closes_without_changing_anything() {
    let mut harness = Harness::new(facts(false), reviewer_pick());
    assert_eq!(harness.frame(vec![key(Key::Escape)]), Some(Outcome::Close));
    assert_eq!(harness.pick, reviewer_pick());
}

#[test]
fn enter_on_what_is_already_so_changes_again_and_a_missing_cli_asks_to_be_set_up() {
    let same = Pick { kind: Kind::Agent, profile: "builder".into(), level: Level::Edits };
    let mut harness = Harness::new(facts(false), same);
    assert_eq!(harness.frame(vec![key(Key::Enter)]), Some(Outcome::Change { interrupt: false }));
    let mut missing = facts(false);
    missing.cards[1].installed = false;
    let mut harness = Harness::new(missing, reviewer_pick());
    assert_eq!(harness.frame(vec![key(Key::Enter)]), Some(Outcome::SetUp));
}
