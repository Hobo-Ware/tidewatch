use super::*;

#[test]
fn the_tooltip_matches_the_board_for_each_state() {
    let input = |session, lid| Input {
        look: Look::Off,
        session,
        wall: 1000.0,
        total: None,
        lid,
        route: Route::Password,
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
fn the_copy_says_what_the_first_start_will_ask_for() {
    let input = |session, route| Input {
        look: Look::Off,
        session,
        wall: 1000.0,
        total: None,
        lid: true,
        route,
        gap: 100.0,
        force_short: false,
        pinned: false,
    };
    let starting = State::Starting { choice: Choice::Hour };
    let cases = [
        (
            Route::SetUp,
            "Asks for your password once to set up Touch ID.",
            "Waiting for the password prompt.",
        ),
        (Route::Fingerprint, "Starts with Touch ID.", "Waiting for Touch ID."),
        (Route::Password, "Pick a timer.", "Waiting for the password prompt."),
    ];
    for (route, off, waiting) in cases {
        let (_, text) = tip_text(&input(State::Off, route));
        assert!(text.ends_with(off), "{route:?}: {text}");
        let (_, text) = tip_text(&input(starting, route));
        assert_eq!(text, waiting, "{route:?}");
    }
    assert!(menu_note(&input(State::Off, Route::SetUp)).starts_with("Asks for your password once"));
    assert_eq!(
        menu_note(&input(State::Off, Route::Fingerprint)),
        "Lid closed on battery starts with Touch ID."
    );
    assert!(menu_note(&input(State::Off, Route::Password)).contains("needs your password"));
    for route in [Route::SetUp, Route::Fingerprint, Route::Password] {
        let text = menu_note(&input(State::Off, route));
        assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
    }
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
            route: Route::Password,
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
                slot = Rect::from_min_size(before + vec2(4.0, 0.0), vec2(SLOT_W, 44.0));
                after = ui.allocate_exact_size(vec2(60.0, 44.0), Sense::hover()).0;
            });
        });
        let _ = slot;
        let delay = full.viewport_output[&ViewportId::ROOT].repaint_delay;
        Out { slot_after: after, pill_left: memory.pill_left(), delay }
    }

    #[test]
    fn nothing_to_the_right_of_the_button_moves_between_off_and_on_with_any_label() {
        let ctx = crate::fonts::test_context();
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
        let ctx = crate::fonts::test_context();
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
            assert!(grown <= SLOT_W + gap.max(0.0), "gap {gap}: pill grew {grown}");
        }
    }

    #[test]
    fn it_asks_for_no_repaint_while_off_and_at_most_every_33_ms_while_on_in_a_focused_window() {
        let settled = |session, look, focused| {
            let ctx = crate::fonts::test_context();
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
        let ctx = crate::fonts::test_context();
        let mut memory = Memory::default();
        let mut found = None;
        let _ = ctx.run_ui(RawInput::default(), |_| {});
        let centre = pos2(400.0 + 8.0 + SLOT_W / 2.0, 20.0);
        let wanted = input(session, Look::Off, 300.0);
        for dy in [0.0, 4.0, 8.0] {
            let ctx = crate::fonts::test_context();
            let mut fresh = Memory::default();
            let at = centre + vec2(0.0, dy);
            if click_at(&ctx, &mut fresh, &wanted, at, 1.0) {
                found = Some(at);
                break;
            }
        }
        assert!(found.is_some(), "no click on the slot opened the menu");
        let ctx = crate::fonts::test_context();
        let elsewhere = pos2(100.0, 20.0);
        assert!(!click_at(&ctx, &mut memory, &wanted, elsewhere, 1.0));
    }

    struct Open {
        ctx: Context,
        memory: Memory,
        input: Input,
        texts: Vec<(String, Rect)>,
        boxes: Vec<(Rect, egui::Color32)>,
        action: Option<Action>,
        time: f64,
    }

    fn collect(shape: &egui::Shape, open: &mut Open) {
        match shape {
            egui::Shape::Text(text) => open.texts.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::Shape::Rect(rect) => open.boxes.push((rect.rect, rect.fill)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, open)),
            _ => {}
        }
    }

    impl Open {
        fn new(session: State, lid: bool) -> Self {
            let mut open = Self {
                ctx: crate::fonts::test_context(),
                memory: Memory { force_menu: true, ..Memory::default() },
                input: Input { lid, ..input(session, Look::Off, 300.0) },
                texts: Vec::new(),
                boxes: Vec::new(),
                action: None,
                time: 1.0,
            };
            for _ in 0..4 {
                open.frame(vec![]);
            }
            open
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.1;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 500.0))),
                time: Some(self.time),
                events,
                focused: true,
                ..Default::default()
            };
            let (input, memory, time) = (&self.input, &mut self.memory, self.time);
            let mut action = None;
            let output = self.ctx.run_ui(raw, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(700.0);
                    action = show(ui, input, memory, time).action;
                });
            });
            self.texts.clear();
            self.boxes.clear();
            for clipped in &output.shapes {
                collect(&clipped.shape, self);
            }
            self.action = action.or(self.action);
        }

        fn text(&self, wanted: &str) -> Rect {
            self.texts
                .iter()
                .find(|(text, _)| text == wanted)
                .map_or_else(|| panic!("{wanted:?} in {:?}", self.texts), |(_, rect)| *rect)
        }

        fn click(&mut self, at: egui::Pos2) {
            let press = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![press(true)]);
            self.frame(vec![press(false)]);
        }
    }

    #[test]
    fn the_menu_is_a_label_four_duration_rows_with_room_and_the_note_in_a_sunken_box() {
        let open = Open::new(State::Off, true);
        let label = open.text("KEEP AWAKE WITH THE LID CLOSED");
        let words = ["30 minutes", "1 hour", "4 hours", "Until I turn it off"];
        let rows: Vec<Rect> = words.iter().map(|word| open.text(word)).collect();
        assert!(rows[0].top() > label.bottom() + 6.0, "space under the label");
        for pair in rows.windows(2) {
            let step = pair[1].center().y - pair[0].center().y;
            assert_eq!(step, crate::screen_kit::MENU_ROW_H + 2.0, "{pair:?}");
        }
    }

    #[test]
    fn the_note_sits_inside_one_sunken_box_under_the_rows() {
        let open = Open::new(State::Off, true);
        let sunken: Vec<Rect> = open
            .boxes
            .iter()
            .filter(|(_, fill)| *fill == brand::SUNKEN)
            .map(|(rect, _)| *rect)
            .collect();
        assert_eq!(sunken.len(), 1, "{:?}", open.boxes);
        let last_row = open.text("Until I turn it off");
        assert!(sunken[0].top() > last_row.bottom());
        let inside: Vec<&str> = open
            .texts
            .iter()
            .filter(|(_, rect)| sunken[0].contains_rect(*rect))
            .map(|(text, _)| text.as_str())
            .collect();
        assert!(inside.iter().any(|text| text.contains("macOS asks once")), "{inside:?}");
        assert!(
            inside.iter().any(|text| text.contains("already keeps the Mac awake")),
            "{inside:?}"
        );
    }

    #[test]
    fn without_lid_mode_the_label_drops_the_lid() {
        let open = Open::new(State::Off, false);
        open.text("KEEP AWAKE");
    }

    #[test]
    fn a_click_on_a_duration_row_picks_that_duration() {
        let mut open = Open::new(State::Off, true);
        let at = open.text("1 hour").center();
        open.click(at);
        assert_eq!(open.action, Some(Action::Pick(Choice::Hour)));
    }

    #[test]
    fn the_menu_of_a_running_session_keeps_add_an_hour_and_turn_off() {
        let on = State::On { until: Some(WALL + 1800.0), told: 0 };
        let mut open = Open::new(on, true);
        open.text("AMPHETAMINE IS ON");
        let at = open.text("Turn off").center();
        open.click(at);
        assert_eq!(open.action, Some(Action::TurnOff));
        let mut open = Open::new(on, true);
        let at = open.text("Add 1 hour").center();
        open.click(at);
        assert_eq!(open.action, Some(Action::AddHour));
    }
}
