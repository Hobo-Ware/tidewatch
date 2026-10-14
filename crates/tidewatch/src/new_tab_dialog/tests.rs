use std::ffi::OsString;
use std::path::PathBuf;

use egui::{Event, Key, Modifiers, RawInput, Rect, pos2, vec2};
use tidewatch_core::agents::Level;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use super::*;
use crate::new_tab::AgentRequest;

struct Harness {
    ctx: egui::Context,
    state: NewTab,
    catalog: Catalog,
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

impl Harness {
    fn new(group: Group) -> Self {
        let mut catalog = Catalog::new(OsString::from("/usr/bin"));
        catalog.set_profiles(Profiles::seeded(&["claude".to_owned()]));
        catalog.set_workflows(Workflows::defaults());
        let state = NewTab::open(group, "web".into(), PathBuf::from("/work/web"), Vec::new(), None);
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut harness = Self { ctx, state, catalog };
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Outcome> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let (state, catalog) = (&mut self.state, &self.catalog);
        let mut outcome = None;
        let _ = self.ctx.run_ui(raw, |ui| {
            outcome = show(ui.ctx(), state, catalog);
        });
        outcome
    }

    fn press(&mut self, k: Key) -> Option<Outcome> {
        self.frame(vec![key(k)])
    }

    fn agent_request(outcome: Option<Outcome>) -> AgentRequest {
        match outcome {
            Some(Outcome::Start(Request::Agent(request))) => *request,
            _ => panic!("expected an agent request"),
        }
    }
}

#[test]
fn the_dialog_opens_on_the_first_screen_and_stays_until_a_key_answers_it() {
    let mut harness = Harness::new(Group::Repo);
    assert_eq!(harness.state.step(), Step::Choose);
    for _ in 0..3 {
        assert!(harness.frame(Vec::new()).is_none());
    }
    assert!(matches!(harness.press(Key::Escape), Some(Outcome::Cancel)));
}

#[test]
fn a_w_and_s_pick_a_card_or_the_plain_shell() {
    let mut harness = Harness::new(Group::Repo);
    assert!(harness.press(Key::A).is_none());
    assert_eq!(harness.state.step(), Step::Agent);

    let mut harness = Harness::new(Group::Repo);
    assert!(harness.press(Key::W).is_none());
    assert_eq!(harness.state.step(), Step::Workflow);

    let mut harness = Harness::new(Group::Repo);
    assert!(matches!(harness.press(Key::S), Some(Outcome::Start(Request::Shell))));
}

#[test]
fn in_other_w_does_nothing_and_a_and_s_still_work() {
    let mut harness = Harness::new(Group::Other);
    assert!(harness.press(Key::W).is_none());
    assert_eq!(harness.state.step(), Step::Choose);
    assert!(harness.press(Key::A).is_none());
    assert_eq!(harness.state.step(), Step::Agent);
    let mut harness = Harness::new(Group::Other);
    assert!(matches!(harness.press(Key::S), Some(Outcome::Start(Request::Shell))));
}

#[test]
fn the_arrows_and_enter_pick_the_highlighted_card() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::ArrowRight);
    assert!(harness.press(Key::Enter).is_none());
    assert_eq!(harness.state.step(), Step::Workflow);

    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::ArrowRight);
    harness.press(Key::ArrowRight);
    assert!(matches!(harness.press(Key::Enter), Some(Outcome::Start(Request::Shell))));
}

#[test]
fn escape_goes_back_from_a_list_and_closes_from_the_first_screen() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::A);
    assert!(harness.press(Key::Escape).is_none());
    assert_eq!(harness.state.step(), Step::Choose);
    assert!(matches!(harness.press(Key::Escape), Some(Outcome::Cancel)));
}

#[test]
fn enter_opens_the_picked_profile_at_its_own_level_with_no_task() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::A);
    let request = Harness::agent_request(harness.press(Key::Enter));
    assert_eq!(request.profile_id, "builder");
    assert_eq!(request.profile.level, Level::Edits);
    assert_eq!(request.task, "");
}

#[test]
fn up_down_pick_and_left_right_change_the_level_of_the_picked_profile() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::A);
    harness.press(Key::ArrowDown);
    harness.press(Key::ArrowDown);
    harness.press(Key::ArrowLeft);
    let request = Harness::agent_request(harness.press(Key::Enter));
    assert_eq!(request.profile_id, "reviewer");
    assert_eq!(request.profile.level, Level::Plan);

    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::A);
    harness.press(Key::ArrowRight);
    let request = Harness::agent_request(harness.press(Key::Enter));
    assert_eq!((request.profile_id.as_str(), request.profile.level), ("builder", Level::Auto));
}

#[test]
fn typing_filters_the_list_and_enter_still_opens_the_tab() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::A);
    harness.frame(Vec::new());
    assert!(harness.frame(vec![Event::Text("review".into())]).is_none());
    assert_eq!(harness.state.filter(), "review");
    let request = Harness::agent_request(harness.press(Key::Enter));
    assert_eq!(request.profile_id, "reviewer");
}

#[test]
fn in_other_the_arrows_never_reach_yolo_and_enter_never_opens_a_yolo_agent() {
    let mut harness = Harness::new(Group::Other);
    harness.press(Key::A);
    for _ in 0..3 {
        harness.press(Key::ArrowRight);
    }
    let request = Harness::agent_request(harness.press(Key::Enter));
    assert_eq!(request.profile.level, Level::Auto);
}

#[test]
fn the_workflow_list_opens_a_request_with_no_task() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::W);
    harness.press(Key::ArrowDown);
    let Some(Outcome::Start(Request::Workflow(request))) = harness.press(Key::Enter) else {
        panic!("expected a workflow request")
    };
    assert_eq!(request.workflow.id, "fix");
    assert_eq!(request.task, "");
}

#[test]
fn enter_on_an_empty_list_says_why_instead_of_opening() {
    let mut harness = Harness::new(Group::Repo);
    harness.press(Key::A);
    harness.frame(vec![Event::Text("zzzz".into())]);
    assert!(harness.press(Key::Enter).is_none());
    assert_eq!(harness.state.error.as_deref(), Some("No profile to start. Add one in Settings."));
}
