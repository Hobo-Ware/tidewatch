use super::*;

#[test]
fn the_tooltip_matches_the_board_for_each_state() {
    let input = |session, lid| Input {
        look: Look::Off,
        session,
        wall: 1000.0,
        total: None,
        lid,
        gap: 100.0,
        force_short: false,
        pinned: false,
    };
    let (title, text) = tip_text(&input(State::Off, true));
    assert_eq!(
        (title, text.as_str()),
        ("Amphetamine", "Keep this Mac awake, even with the lid closed on battery. Pick a timer.")
    );
    let (title, text) =
        tip_text(&input(State::On { until: Some(1000.0 + 42.0 * 60.0 - 10.0), told: 1 }, true));
    assert_eq!(title, "Amphetamine is on");
    assert_eq!(
        text,
        "42 min left. The Mac stays awake with the lid closed on battery. Click to change or stop."
    );
    let (_, text) = tip_text(&input(State::On { until: None, told: 0 }, true));
    assert_eq!(text, "No timer. The Mac stays awake until you turn it off.");
    let (_, text) = tip_text(&input(State::Off, false));
    assert!(
        text.contains("no lid mode on Linux") || text.contains("There is no lid mode on Linux")
    );
}

#[test]
fn a_window_that_is_not_animating_shows_the_steady_look_or_nothing_in_motion() {
    assert_eq!(still(Look::On { age: 0.3 }), Look::On { age: STILL_AGE });
    assert_eq!(still(Look::Stopping { age: 0.1 }), Look::Off);
    assert_eq!(still(Look::Off), Look::Off);
    assert_eq!(still(Look::Pending), Look::Pending);
}

mod frames {
    use std::time::Duration;

    use egui::{Context, RawInput, Rect, ViewportId};
    use tidewatch_core::amphetamine::{Choice, State};

    use super::super::*;
    use crate::amphetamine_motion::Look;

    const WALL: f64 = 5000.0;

    fn input(session: State, look: Look, gap: f32) -> Input {
        Input {
            look,
            session,
            wall: WALL,
            total: Some(3600.0),
            lid: true,
            gap,
            force_short: false,
            pinned: false,
        }
    }

    struct Out {
        slot_after: Rect,
        pill_left: Option<f32>,
        delay: Duration,
    }

    fn frame(ctx: &Context, memory: &mut Memory, input: &Input, now: f64, focused: bool) -> Out {
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 80.0))),
            time: Some(now),
            ..Default::default()
        };
        let viewport = raw.viewports.entry(ViewportId::ROOT).or_default();
        viewport.focused = Some(focused);
        let mut after = Rect::NOTHING;
        let mut slot = Rect::NOTHING;
        let full = ctx.run_ui(raw, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(400.0);
                let before = ui.next_widget_position();
                show(ui, input, memory, now);
                slot = Rect::from_min_size(before + vec2(4.0, 0.0), vec2(SLOT_W, 28.0));
                after = ui.allocate_exact_size(vec2(60.0, 28.0), Sense::hover()).0;
            });
        });
        let _ = slot;
        let delay = full.viewport_output[&ViewportId::ROOT].repaint_delay;
        Out { slot_after: after, pill_left: memory.pill_left(), delay }
    }

    #[test]
    fn nothing_to_the_right_of_the_button_moves_between_off_and_on_with_any_label() {
        let ctx = Context::default();
        let mut memory = Memory::default();
        let off = frame(&ctx, &mut memory, &input(State::Off, Look::Off, 300.0), 1.0, true);
        let timed = State::On { until: Some(WALL + 3600.0), told: 1 };
        let mut on_rects = Vec::new();
        for (state, gap) in [
            (timed, 300.0),
            (State::On { until: None, told: 0 }, 300.0),
            (timed, 40.0),
            (timed, 2.0),
        ] {
            let mut fresh = Memory::default();
            for t in [1.0, 1.1, 1.5, 3.0] {
                let out =
                    frame(&ctx, &mut fresh, &input(state, Look::On { age: 3.0 }, gap), t, true);
                on_rects.push(out.slot_after);
            }
        }
        assert!(
            on_rects.iter().all(|rect| *rect == off.slot_after),
            "{on_rects:?} vs {:?}",
            off.slot_after
        );
    }

    #[test]
    fn the_pill_never_grows_past_the_gap_before_the_new_tab_button() {
        let ctx = Context::default();
        let timed = State::On { until: Some(WALL + 3.0 * 3600.0 + 12.0 * 60.0), told: 4 };
        for gap in [400.0, 120.0, 80.0, 60.0, 40.0, 10.0, 0.0] {
            let mut memory = Memory::default();
            let mut out =
                frame(&ctx, &mut memory, &input(timed, Look::On { age: 3.0 }, gap), 1.0, true);
            for step in 1..=40 {
                out = frame(
                    &ctx,
                    &mut memory,
                    &input(timed, Look::On { age: 3.0 }, gap),
                    1.0 + f64::from(step) * 0.02,
                    true,
                );
            }
            let grown = out.pill_left.map_or(0.0, |left| 400.0 + 4.0 + SLOT_W - left);
            assert!(
                grown <= SLOT_W + (gap - pill::GAP_MARGIN).max(0.0) + 0.5,
                "gap {gap}: pill grew {grown}"
            );
        }
    }

    #[test]
    fn it_asks_for_no_repaint_while_off_and_at_most_every_33_ms_while_on_in_a_focused_window() {
        let settled = |session, look, focused| {
            let ctx = Context::default();
            let mut memory = Memory::default();
            let input = input(session, look, 300.0);
            frame(&ctx, &mut memory, &input, 1.0, focused);
            frame(&ctx, &mut memory, &input, 4.0, focused).delay
        };
        let on = State::On { until: None, told: 0 };
        assert_eq!(settled(State::Off, Look::Off, true), Duration::MAX);
        assert!(settled(on, Look::On { age: 9.0 }, true) <= Duration::from_millis(33));
        assert_eq!(settled(on, Look::On { age: 9.0 }, false), Duration::MAX);
        let asking = State::Starting { choice: Choice::Hour };
        assert_eq!(settled(asking, Look::Pending, true), Duration::MAX);
    }

    fn click_at(
        ctx: &Context,
        memory: &mut Memory,
        input: &Input,
        at: egui::Pos2,
        time: f64,
    ) -> bool {
        let raw = |events: Vec<egui::Event>, time| RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 300.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let run = |ctx: &Context, memory: &mut Memory, events, time| {
            let _ = ctx.run_ui(raw(events, time), |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(400.0);
                    show(ui, input, memory, time);
                });
            });
        };
        run(ctx, memory, vec![egui::Event::PointerMoved(at)], time);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(ctx, memory, vec![press(true)], time + 0.1);
        run(ctx, memory, vec![press(false)], time + 0.2);
        run(ctx, memory, vec![], time + 0.3);
        egui::Popup::is_any_open(ctx)
    }

    #[test]
    fn a_click_on_the_tablet_opens_the_menu_and_a_click_elsewhere_leaves_it_shut() {
        let session = State::Off;
        let ctx = Context::default();
        let mut memory = Memory::default();
        let mut found = None;
        let _ = ctx.run_ui(RawInput::default(), |_| {});
        let centre = pos2(400.0 + 8.0 + SLOT_W / 2.0, 20.0);
        let wanted = input(session, Look::Off, 300.0);
        for dy in [0.0, 4.0, 8.0] {
            let ctx = Context::default();
            let mut fresh = Memory::default();
            let at = centre + vec2(0.0, dy);
            if click_at(&ctx, &mut fresh, &wanted, at, 1.0) {
                found = Some(at);
                break;
            }
        }
        assert!(found.is_some(), "no click on the slot opened the menu");
        let ctx = Context::default();
        let elsewhere = pos2(100.0, 20.0);
        assert!(!click_at(&ctx, &mut memory, &wanted, elsewhere, 1.0));
    }
}
