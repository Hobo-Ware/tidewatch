//! Settings pieces: switch, rows, buttons, pills, disclosure, save bar.

use std::cell::Cell;

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, pos2};

use super::editor::{Bar, BarAction, save_bar};
use super::*;

fn raw(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(900.0, 400.0))),
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

fn click(ctx: &egui::Context, at: Pos2, frame: impl Fn(&mut Ui)) {
    for events in [vec![Event::PointerMoved(at)], vec![press(at, true)], vec![press(at, false)]] {
        let _ = ctx.run_ui(raw(events), |ui| frame(ui));
    }
}

fn warm(ctx: &egui::Context, frame: impl Fn(&mut Ui)) {
    let _ = ctx.run_ui(raw(vec![]), |ui| frame(ui));
}

fn switch_frame(ctx: &egui::Context, events: Vec<Event>, on: &mut bool) -> (Rect, bool) {
    let mut out = (Rect::NOTHING, false);
    let _ = ctx.run_ui(raw(events), |ui| {
        let response = switch(ui, on);
        out = (response.rect, response.changed());
    });
    out
}

#[test]
fn a_switch_flips_only_on_a_click_and_reports_the_change() {
    let ctx = crate::fonts::test_context();
    let mut on = true;
    let (rect, changed) = switch_frame(&ctx, vec![], &mut on);
    assert!(on && !changed);
    assert_eq!(rect.size(), SWITCH_SIZE);
    let at = rect.center();
    for events in [vec![Event::PointerMoved(at)], vec![press(at, true)]] {
        let (_, changed) = switch_frame(&ctx, events, &mut on);
        assert!(on && !changed, "nothing changes before the button is released");
    }
    let (_, changed) = switch_frame(&ctx, vec![press(at, false)], &mut on);
    assert!(!on && changed);
}

#[test]
fn a_switch_row_gives_back_the_new_value_and_none_while_untouched() {
    let ctx = crate::fonts::test_context();
    let run = |events: Vec<Event>, on: bool| {
        let mut answer = None;
        let mut area = Rect::NOTHING;
        let _ = ctx.run_ui(raw(events), |ui| {
            area = ui
                .scope(|ui| {
                    card(ui, |ui| {
                        answer = switch_row(ui, "Title", "Description", on);
                    });
                })
                .response
                .rect;
        });
        (answer, area)
    };
    let (answer, area) = run(vec![], true);
    assert_eq!(answer, None);
    let at = pos2(area.right() - 40.0, area.center().y);
    assert_eq!(run(vec![Event::PointerMoved(at)], true).0, None);
    assert_eq!(run(vec![press(at, true)], true).0, None);
    assert_eq!(run(vec![press(at, false)], true).0, Some(false));
}

#[test]
fn rows_stack_with_a_divider_between_them_and_none_above_the_first() {
    let ctx = crate::fonts::test_context();
    let mut heights = Vec::new();
    for rows in 1..=3 {
        let mut used = Rect::NOTHING;
        for _ in 0..2 {
            let _ = ctx.run_ui(raw(vec![]), |ui| {
                used = ui
                    .scope(|ui| {
                        card(ui, |ui| {
                            for _ in 0..rows {
                                row(ui, "Title", "Description", Control::SWITCH, |_| {});
                            }
                        });
                    })
                    .response
                    .rect;
            });
        }
        heights.push(used.height());
    }
    let step = heights[1] - heights[0];
    assert!(step > 60.0, "{heights:?}");
    assert!((heights[2] - heights[1] - step).abs() < 0.5, "{heights:?}");
}

#[test]
fn a_button_clicks_only_when_enabled() {
    let ctx = crate::fonts::test_context();
    for enabled in [true, false] {
        let rect = Cell::new(Rect::NOTHING);
        let clicked = Cell::new(false);
        let frame = |ui: &mut Ui| {
            let response = ui
                .add_enabled_ui(enabled, |ui| button(ui, "Save", Tone::Primary, Size::Regular))
                .inner;
            rect.set(response.rect);
            clicked.set(clicked.get() || response.clicked());
        };
        warm(&ctx, frame);
        click(&ctx, rect.get().center(), frame);
        assert_eq!(clicked.get(), enabled);
    }
}

#[test]
fn a_pick_reports_the_clicked_option_and_a_disclosure_opens_and_closes() {
    let ctx = crate::fonts::test_context();
    let rect = Cell::new(Rect::NOTHING);
    let picked = Cell::new(None);
    let frame = |ui: &mut Ui| {
        let start = ui.cursor().min;
        let index = pick(ui, "level", &["Low", "High"], Some(0));
        rect.set(Rect::from_min_max(start, ui.min_rect().max));
        picked.set(picked.get().or(index));
    };
    warm(&ctx, frame);
    let r = rect.get();
    click(&ctx, egui::pos2(r.min.x + r.width() * 0.75, r.center().y), frame);
    assert_eq!(picked.get(), Some(1));

    let ctx = crate::fonts::test_context();
    let open = Cell::new(false);
    let header = Cell::new(Rect::NOTHING);
    let frame = |ui: &mut Ui| {
        header.set(ui.available_rect_before_wrap());
        let shown = disclosure(ui, "more", "Advanced", |ui| {
            ui.label("inside");
        });
        open.set(shown.is_some());
    };
    warm(&ctx, frame);
    assert!(!open.get());
    let at = header.get().min + vec2(30.0, 10.0);
    click(&ctx, at, frame);
    assert!(open.get(), "a click opens it");
    click(&ctx, at, frame);
    assert!(!open.get(), "and another closes it");
}

#[test]
fn the_save_bar_offers_save_when_dirty_revert_when_asked_and_neither_while_saving() {
    let found = |bar: Bar| {
        let ctx = crate::fonts::test_context();
        let actions = std::cell::RefCell::new(Vec::new());
        for x in (600..900).step_by(8) {
            let at = pos2(x as f32, 29.0);
            let frame = |ui: &mut Ui| {
                if let Some(action) = save_bar(ui, &bar) {
                    actions.borrow_mut().push(action);
                }
            };
            warm(&ctx, frame);
            click(&ctx, at, frame);
        }
        let mut actions = actions.into_inner();
        actions.sort_by_key(|action| *action as u8);
        actions.dedup();
        actions
    };
    let dirty = Bar { dirty: true, can_revert: true, ..Bar::default() };
    assert_eq!(found(dirty), [BarAction::Save, BarAction::Revert]);
    assert_eq!(found(Bar { can_revert: false, ..dirty }), [BarAction::Save]);
    assert_eq!(found(Bar { saving: true, ..dirty }), Vec::<BarAction>::new());
    assert_eq!(
        found(Bar { dirty: false, error: Some("disk full"), ..dirty }),
        [BarAction::Revert],
        "a failed save with nothing left to save can still be reverted"
    );
}

#[test]
fn the_save_bar_shows_for_unsaved_edits_a_save_in_flight_or_an_error() {
    let cases = [
        (Bar::default(), false),
        (Bar { dirty: true, ..Bar::default() }, true),
        (Bar { saving: true, ..Bar::default() }, true),
        (Bar { error: Some("x"), ..Bar::default() }, true),
    ];
    for (bar, shows) in cases {
        assert_eq!(bar.shows(), shows, "{bar:?}");
    }
}

#[test]
fn lerp_color_runs_from_one_end_to_the_other() {
    assert_eq!(lerp_color(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
    assert_eq!(lerp_color(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
}

#[test]
fn key_caps_keep_a_minimum_width_so_single_keys_look_alike() {
    let ctx = crate::fonts::test_context();
    let _ = ctx.run_ui(raw(vec![]), |ui| {
        let one = key_cap(ui, "`", 13.0).rect.width();
        let word = key_cap(ui, "Shift", 13.0).rect.width();
        assert!(one >= 26.0 && word > one, "{one} {word}");
    });
}

#[test]
fn up_to_five_short_options_share_one_segmented_control_and_more_wrap_as_chips() {
    let ctx = crate::fonts::test_context();
    let _ = ctx.run_ui(raw(vec![]), |ui| {
        assert!(fits_inline(ui, &["Low", "High"]));
        assert!(fits_inline(ui, &["A", "B", "C", "D", "E"]));
        assert!(!fits_inline(ui, &["A", "B", "C", "D", "E", "F"]));
        let wide = ["A label wide enough to overflow a row"; 5];
        assert!(!fits_inline(ui, &wide));
    });
}

const LONG: &str = "/Users/dev/.config/tidewatch/projects/acme-web/worktrees/house-style.md";

#[test]
fn a_path_stays_on_one_line_with_its_file_name_and_the_home_folder_as_a_tilde() {
    let home = tidewatch_core::paths::home_dir();
    let under_home = home.as_ref().map(|home| home.join(".config/tidewatch/house-style.md"));
    if let Some(path) = under_home {
        assert_eq!(shown_path(&path.display().to_string()), "~/.config/tidewatch/house-style.md");
    }
    assert_eq!(shown_path("/etc/tidewatch.toml"), "/etc/tidewatch.toml");
    let ctx = crate::fonts::test_context();
    for room in [120.0, 260.0, 600.0] {
        let mut rows = 0;
        let mut width = 0.0;
        for _ in 0..2 {
            let _ = ctx.run_ui(raw(vec![]), |ui| {
                let before = ui.min_rect().height();
                path_line(ui, LONG, (12.0, FAINT), room);
                rows = ((ui.min_rect().height() - before) / 14.0).round() as usize;
                width = ui.min_rect().width();
            });
        }
        assert_eq!(rows, 1, "{room}");
        assert!(width <= room + 0.5, "{room}: {width}");
    }
}

#[test]
fn a_path_with_a_copy_button_leaves_the_button_its_room() {
    assert_eq!(path_room(500.0, 50.0), 442.0);
    assert_eq!(path_room(30.0, 50.0), 0.0);
}

#[test]
fn the_copy_button_puts_the_whole_path_on_the_clipboard() {
    let ctx = crate::fonts::test_context();
    let frame = |ui: &mut Ui| path_with_copy(ui, LONG, (13.0, FAINT));
    warm(&ctx, frame);
    warm(&ctx, frame);
    let mut copied = Vec::new();
    for events in [
        vec![Event::PointerMoved(pos2(880.0, 20.0))],
        vec![press(pos2(880.0, 20.0), true)],
        vec![press(pos2(880.0, 20.0), false)],
    ] {
        let out = ctx.run_ui(raw(events), |ui| {
            ui.set_width(900.0);
            frame(ui);
        });
        copied.extend(out.platform_output.commands.iter().filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        }));
    }
    assert_eq!(copied, [LONG]);
}
