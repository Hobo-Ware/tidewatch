//! Draws the Updates section of General: what the updater says, Check now with the update pill
//! beside it, and the switches. It returns the new update settings when a switch was clicked.

use egui::RichText;
use stdusk_egui::colors;
use stdusk_egui::widgets::action_button;
use tidewatch_core::prefs::UpdatePrefs;
use tidewatch_core::update::Mode;

use crate::updates::{INSTALL_HINT, NOTIFY_ONLY_HINT, Updates};
use crate::window_view::{note, switch_row};

pub(crate) fn section(ui: &mut egui::Ui, updates: &mut Updates) -> Option<UpdatePrefs> {
    let mode = updates.mode();
    section_in(ui, updates, mode)
}

fn section_in(ui: &mut egui::Ui, updates: &mut Updates, mode: Mode) -> Option<UpdatePrefs> {
    let mut applied = None;
    ui.add_space(10.0);
    ui.label(RichText::new("Updates").strong().color(colors::fg()));
    note(ui, &updates.status_line());
    ui.horizontal(|ui| {
        if action_button(ui, "Check now", false).clicked() {
            updates.check_now();
        }
        updates.pill(ui);
    });
    if let Some(on) = switch_row(ui, "Check for updates automatically", updates.prefs().check) {
        applied = Some(updates.set_check(on));
    }
    match mode {
        Mode::Install => {
            if let Some(on) =
                switch_row(ui, "Install updates in the background", updates.prefs().install)
            {
                applied = Some(updates.set_install(on));
            }
            note(ui, INSTALL_HINT);
        }
        Mode::NotifyOnly => note(ui, NOTIFY_ONLY_HINT),
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, UiBuilder, Vec2, pos2};
    use kelp_core::update::Release;

    fn input(events: Vec<Event>) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(700.0, 500.0))),
            focused: true,
            events,
            ..RawInput::default()
        }
    }

    fn button(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn run(ctx: &egui::Context, updates: &mut Updates, events: Vec<Event>) -> Option<UpdatePrefs> {
        run_in(ctx, updates, Mode::NotifyOnly, events)
    }

    fn run_in(
        ctx: &egui::Context,
        updates: &mut Updates,
        mode: Mode,
        events: Vec<Event>,
    ) -> Option<UpdatePrefs> {
        let mut answer = None;
        let _ = ctx.run_ui(input(events), |ui| {
            ui.scope_builder(UiBuilder::new().max_rect(ui.max_rect().shrink(8.0)), |ui| {
                answer = section_in(ui, updates, mode);
            });
        });
        answer
    }

    fn fake() -> Release {
        Release { version: "9.9.9".to_owned(), url: "https://example.test".to_owned() }
    }

    #[test]
    fn the_section_renders_in_every_mode_with_and_without_news_and_asks_for_nothing_until_touched()
    {
        for mode in [Mode::Install, Mode::NotifyOnly] {
            for fake in [None, Some(fake())] {
                let ctx = egui::Context::default();
                ctx.set_fonts(crate::fonts::definitions());
                let mut updates = Updates::new(&ctx, UpdatePrefs::default(), fake, true);
                for _ in 0..2 {
                    assert_eq!(run_in(&ctx, &mut updates, mode, vec![]), None);
                }
            }
        }
    }

    #[test]
    fn the_check_switch_applies_at_once_and_leaves_install_alone() {
        let ctx = egui::Context::default();
        let mut updates = Updates::new(&ctx, UpdatePrefs::default(), None, true);
        run(&ctx, &mut updates, vec![]);
        let mut applied = None;
        for y in (40..120).step_by(4) {
            let at = pos2(27.0, y as f32);
            run(&ctx, &mut updates, vec![Event::PointerMoved(at)]);
            run(&ctx, &mut updates, vec![button(at, true)]);
            applied = run(&ctx, &mut updates, vec![button(at, false)]);
            if applied.is_some() {
                break;
            }
        }
        let applied = applied.expect("the first switch is the check one");
        assert_eq!(applied, UpdatePrefs { check: false, install: true });
        assert!(!updates.prefs().check);
    }
}
