//! Settings shell: every page renders, the sidebar switches pages, Esc closes, General reports changes.

use egui::{Event, PointerButton, Pos2, RawInput, Vec2};
use tidewatch_core::prefs::{ResumeAgents, WindowPrefs};
use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use super::*;
use crate::catalog::Availability;
use crate::macos::LoginItem;
use crate::window_view;

fn settings() -> Settings {
    let profiles = Profiles::seeded(&["claude".to_owned()]);
    let agents = vec![("claude".to_owned(), "Claude Code".to_owned())];
    Settings::new(&profiles, &Workflows::defaults(), agents, &WindowPrefs::default())
}

fn files() -> Files {
    Files {
        profiles: "profiles.toml".into(),
        workflows: "workflows.toml".into(),
        trust: "trust.toml".into(),
        general: "settings.toml".into(),
        house_style: "house-style.md".into(),
    }
}

fn trust_page() -> trust_view::Page<'static> {
    trust_view::Page {
        consent: tidewatch_core::consent::Consent::Undecided,
        state: None,
        root: "/home/u/.tidewatch",
        added: 0,
        busy: false,
    }
}

fn general(restore: bool, resume: ResumeAgents, login_offered: bool) -> General {
    General {
        restore,
        resume,
        skipped_by_flag: false,
        login_offered,
        window: window_view::View {
            status: crate::shell::HotkeyStatus::Registered,
            menu_bar_available: true,
        },
        github: tidewatch_core::prefs::GithubPrefs::default(),
    }
}

fn updates(ctx: &egui::Context) -> Updates {
    Updates::new(ctx, tidewatch_core::prefs::UpdatePrefs::default(), None, true)
}

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1280.0, 800.0))),
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

fn escape() -> Event {
    Event::Key {
        key: Key::Escape,
        physical_key: Some(Key::Escape),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

struct Harness {
    ctx: egui::Context,
    settings: Settings,
    updates: Updates,
    restore: bool,
    resume: ResumeAgents,
    login_offered: bool,
    esc_closes: bool,
}

impl Harness {
    fn new(section: Section) -> Self {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        let mut settings = settings();
        settings.section = section;
        let updates = updates(&ctx);
        Self {
            ctx,
            settings,
            updates,
            restore: true,
            resume: ResumeAgents::Auto,
            login_offered: false,
            esc_closes: true,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Action> {
        let mut answer = None;
        let (settings, updates) = (&mut self.settings, &mut self.updates);
        let probes =
            Probes::from([("claude".to_owned(), Availability::Found(Some("2.1.0".into())))]);
        let (files, trust, general) =
            (files(), trust_page(), general(self.restore, self.resume, self.login_offered));
        let host = Host {
            files: &files,
            trust: &trust,
            general: &general,
            probes: &probes,
            read_login: || LoginItem::Off,
            esc_closes: self.esc_closes,
        };
        let _ = self.ctx.run_ui(input(events), |ui| {
            let area = ui.max_rect();
            answer = show(ui, area, settings, &host, updates);
        });
        answer
    }

    fn click(&mut self, at: Pos2) -> Option<Action> {
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![press(at, true)]);
        self.frame(vec![press(at, false)])
    }

    fn click_scan(&mut self, xs: std::ops::Range<i32>, ys: std::ops::Range<i32>) -> Option<Action> {
        self.frame(vec![]);
        for y in ys.step_by(6) {
            for x in xs.clone().step_by(6) {
                if let Some(action) = self.click(pos2(x as f32, y as f32)) {
                    return Some(action);
                }
            }
        }
        None
    }
}

#[test]
fn every_page_renders_headless_and_asks_for_nothing_until_clicked() {
    for (section, _, _) in settings().sections() {
        let mut harness = Harness::new(section);
        for _ in 0..3 {
            assert!(harness.frame(vec![]).is_none(), "{section:?}");
        }
        assert!(!harness.settings.profiles.dirty && !harness.settings.workflows.dirty);
    }
}

#[test]
fn a_click_on_a_sidebar_item_opens_its_page_in_sidebar_order() {
    let sections: Vec<Section> = settings().sections().iter().map(|(s, _, _)| *s).collect();
    assert_eq!(sections.len(), 6);
    let mut harness = Harness::new(Section::General);
    harness.frame(vec![]);
    for (index, want) in sections.iter().enumerate() {
        let at = pos2(100.0, 60.0 + index as f32 * 40.0 + 19.0);
        assert!(harness.click(at).is_none());
        assert_eq!(harness.settings.section, *want, "item {index}");
    }
}

#[test]
fn escape_closes_the_settings_unless_something_else_owns_the_key() {
    let mut harness = Harness::new(Section::General);
    harness.frame(vec![]);
    assert!(matches!(harness.frame(vec![escape()]), Some(Action::Close)));
    harness.esc_closes = false;
    assert!(harness.frame(vec![escape()]).is_none());
}

#[test]
fn escape_while_a_text_field_is_focused_only_leaves_the_field() {
    let mut harness = Harness::new(Section::Profiles);
    harness.frame(vec![]);
    let mut found = None;
    'scan: for y in (150..260).step_by(6) {
        for x in (620..900).step_by(12) {
            harness.click(pos2(x as f32, y as f32));
            if harness.ctx.text_edit_focused() {
                found = Some(());
                break 'scan;
            }
        }
    }
    assert!(found.is_some(), "a click in the Name or Model field focuses it");
    assert!(harness.frame(vec![escape()]).is_none(), "the first Escape is the field's");
    assert!(!harness.ctx.text_edit_focused());
    assert!(matches!(harness.frame(vec![escape()]), Some(Action::Close)), "the second closes");
}

#[test]
fn the_restore_switch_is_the_first_switch_on_general_and_reports_the_new_value() {
    let mut harness = Harness::new(Section::General);
    let action = harness.click_scan(1100..1240, 90..150);
    assert!(matches!(action, Some(Action::Restore(false))), "{}", action.is_some());
    harness.restore = false;
    let action = harness.click_scan(1100..1240, 90..150);
    assert!(matches!(action, Some(Action::Restore(true))));
}

fn every_action_on_general(harness: &mut Harness) -> Vec<Action> {
    harness.frame(vec![]);
    let mut found = Vec::new();
    for y in (100..480).step_by(10) {
        for x in (760..1240).step_by(20) {
            found.extend(harness.click(pos2(x as f32, y as f32)));
        }
    }
    found
}

#[test]
fn the_window_section_sits_under_the_startup_card_and_reports_its_own_change() {
    let mut harness = Harness::new(Section::General);
    let found = every_action_on_general(&mut harness);
    let quake_off = WindowPrefs { quake: false, ..WindowPrefs::default() };
    assert!(
        found.iter().any(|action| matches!(action, Action::Window(prefs) if *prefs == quake_off)),
        "no Window action turned the drop-down off"
    );
}

#[test]
fn the_resume_control_sits_under_the_restore_row_and_offers_the_other_two_modes() {
    let mut harness = Harness::new(Section::General);
    let found = every_action_on_general(&mut harness);
    let modes: Vec<ResumeAgents> = found
        .iter()
        .filter_map(|action| match action {
            Action::ResumeAgents(mode) => Some(*mode),
            _ => None,
        })
        .collect();
    assert!(modes.contains(&ResumeAgents::Prefill) && modes.contains(&ResumeAgents::Off));
    assert!(!modes.contains(&ResumeAgents::Auto), "the mode already set reports nothing");
    harness.resume = ResumeAgents::Off;
    let modes: Vec<ResumeAgents> = every_action_on_general(&mut harness)
        .into_iter()
        .filter_map(|action| match action {
            Action::ResumeAgents(mode) => Some(mode),
            _ => None,
        })
        .collect();
    assert!(modes.contains(&ResumeAgents::Auto) && !modes.contains(&ResumeAgents::Off));
}

#[test]
fn the_resume_control_does_nothing_while_restoring_is_off() {
    let mut harness = Harness::new(Section::General);
    harness.restore = false;
    let found = every_action_on_general(&mut harness);
    assert!(!found.iter().any(|action| matches!(action, Action::ResumeAgents(_))));
}

#[test]
fn the_sidebar_shows_the_version_and_marks_a_page_with_unsaved_edits() {
    let mut harness = Harness::new(Section::General);
    harness.settings.profiles.touched();
    for _ in 0..2 {
        assert!(harness.frame(vec![]).is_none());
    }
    assert!(harness.settings.sections()[1].2);
}

fn login_actions(offered: bool) -> Vec<login_item::Click> {
    let mut harness = Harness::new(Section::General);
    harness.login_offered = offered;
    harness.frame(vec![]);
    let mut found = Vec::new();
    for y in (150..420).step_by(6) {
        for x in (1100..1240).step_by(12) {
            if let Some(Action::LoginItem(click)) = harness.click(pos2(x as f32, y as f32)) {
                found.push(click);
            }
        }
    }
    found
}

#[test]
fn open_at_login_sits_in_the_startup_card_and_asks_the_app_to_register() {
    let found = login_actions(true);
    assert_ne!(found.len(), 0);
    assert!(found.iter().all(|click| *click == login_item::Click::Set(true)), "{found:?}");
}

#[test]
fn a_platform_without_a_login_item_shows_no_row_for_it() {
    assert_eq!(login_actions(false), Vec::new());
}
