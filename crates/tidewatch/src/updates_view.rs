//! Draws the Updates section of General: what the updater says, Check now with the update pill
//! beside it, and the switches. It returns the new update settings when a switch was clicked.

use egui::{Align, Layout, RichText, Ui};
use tidewatch_core::prefs::UpdatePrefs;
use tidewatch_core::update::Mode;

use crate::screen_kit::{Size, mono};
use crate::settings_ui::{self, Control, MUTED, TEXT, Tone, button, row_with, switch_row};
use crate::updates::{INSTALL_HINT, NOTIFY_ONLY_HINT, Updates};

const PILL_ROOM: f32 = 150.0;

pub(crate) fn section(ui: &mut Ui, updates: &mut Updates) -> Option<UpdatePrefs> {
    let mode = updates.mode();
    section_in(ui, updates, mode)
}

fn section_in(ui: &mut Ui, updates: &mut Updates, mode: Mode) -> Option<UpdatePrefs> {
    let mut applied = None;
    settings_ui::section(ui, "Updates", |ui| {
        status_row(ui, updates, mode);
        if updates.is_stuck() {
            repair_row(ui, &Updates::repair_script());
        }
        if let Some(on) = switch_row(
            ui,
            "Check for updates automatically",
            "In the background. Check now works either way.",
            updates.prefs().check,
        ) {
            applied = Some(updates.set_check(on));
        }
        if mode == Mode::Install
            && let Some(on) = switch_row(
                ui,
                "Install updates in the background",
                INSTALL_HINT,
                updates.prefs().install,
            )
        {
            applied = Some(updates.set_install(on));
        }
    });
    applied
}

fn repair_row(ui: &mut Ui, script: &str) {
    row_with(
        ui,
        Control::NONE,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                RichText::new("Fix runs this once Tidewatch has quit").size(13.0).color(MUTED),
            );
            ui.add(egui::Label::new(RichText::new(script).font(mono(11.5)).color(TEXT)).wrap());
        },
        |_| {},
    );
}

fn status_row(ui: &mut Ui, updates: &mut Updates, mode: Mode) {
    let line = updates.status_line();
    let check = Control::button(ui, "Check now", Size::Compact);
    row_with(
        ui,
        Control::new(check.width + PILL_ROOM, check.height),
        |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(&line).size(15.0).color(TEXT));
            if mode == Mode::NotifyOnly {
                ui.label(RichText::new(NOTIFY_ONLY_HINT).size(13.0).color(MUTED));
            }
        },
        |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if button(ui, "Check now", Tone::Quiet, Size::Compact).clicked() {
                    updates.check_now();
                }
                updates.pill(ui);
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::FakeState;
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

    fn click(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn run_in(
        ctx: &egui::Context,
        updates: &mut Updates,
        mode: Mode,
        events: Vec<Event>,
    ) -> (Option<UpdatePrefs>, Rect) {
        let mut answer = None;
        let mut used = Rect::NOTHING;
        let _ = ctx.run_ui(input(events), |ui| {
            used = ui
                .scope_builder(UiBuilder::new().max_rect(ui.max_rect().shrink(8.0)), |ui| {
                    answer = section_in(ui, updates, mode);
                })
                .response
                .rect;
        });
        (answer, used)
    }

    fn fake() -> Release {
        Release { version: "9.9.9".to_owned(), url: "https://example.test".to_owned() }
    }

    #[test]
    fn the_section_renders_in_every_mode_with_and_without_news_and_asks_for_nothing_until_touched()
    {
        for mode in [Mode::Install, Mode::NotifyOnly] {
            for fake in [None, Some(fake())] {
                let ctx = crate::fonts::test_context();
                ctx.set_fonts(crate::fonts::definitions());
                let mut updates =
                    Updates::new(&ctx, UpdatePrefs::default(), fake, true, std::env::temp_dir());
                for _ in 0..2 {
                    assert_eq!(run_in(&ctx, &mut updates, mode, vec![]).0, None);
                }
            }
        }
    }

    #[test]
    fn a_failed_or_stuck_update_renders_and_the_stuck_one_lists_the_fix() {
        let heights: Vec<f32> = [None, Some(FakeState::Failed), Some(FakeState::Stuck)]
            .into_iter()
            .map(|state| {
                let ctx = crate::fonts::test_context();
                ctx.set_fonts(crate::fonts::definitions());
                let mut updates =
                    Updates::new(&ctx, UpdatePrefs::default(), None, true, std::env::temp_dir())
                        .with_fake_state(state);
                run_in(&ctx, &mut updates, Mode::Install, vec![]);
                run_in(&ctx, &mut updates, Mode::Install, vec![]).1.height()
            })
            .collect();
        assert!(heights.iter().all(|h| *h > 0.0), "{heights:?}");
        assert!(heights[2] > heights[1], "the repair script adds a row: {heights:?}");
    }

    #[test]
    fn the_check_switch_applies_at_once_and_leaves_install_alone() {
        let ctx = crate::fonts::test_context();
        let mut updates =
            Updates::new(&ctx, UpdatePrefs::default(), None, true, std::env::temp_dir());
        let (_, used) = run_in(&ctx, &mut updates, Mode::NotifyOnly, vec![]);
        let at = pos2(used.right() - 40.0, used.bottom() - 36.0);
        let mut run = |events| run_in(&ctx, &mut updates, Mode::NotifyOnly, events).0;
        run(vec![Event::PointerMoved(at)]);
        run(vec![click(at, true)]);
        let applied = run(vec![click(at, false)]).expect("the last switch is the check one");
        assert_eq!(applied, UpdatePrefs { check: false, install: true });
        assert!(!updates.prefs().check);
    }

    #[test]
    fn the_install_switch_only_exists_in_install_mode() {
        let ctx = crate::fonts::test_context();
        let mut updates =
            Updates::new(&ctx, UpdatePrefs::default(), None, true, std::env::temp_dir());
        let height = |mode, updates: &mut Updates| run_in(&ctx, updates, mode, vec![]).1.height();
        let install = height(Mode::Install, &mut updates);
        let notify = height(Mode::NotifyOnly, &mut updates);
        assert!(install > 0.0 && notify > 0.0);
        let at = {
            let (_, used) = run_in(&ctx, &mut updates, Mode::Install, vec![]);
            pos2(used.right() - 40.0, used.bottom() - 36.0)
        };
        let mut run = |events| run_in(&ctx, &mut updates, Mode::Install, events).0;
        run(vec![Event::PointerMoved(at)]);
        run(vec![click(at, true)]);
        let applied = run(vec![click(at, false)]).expect("the last switch is the install one");
        assert_eq!(applied, UpdatePrefs { check: true, install: false });
    }
}
