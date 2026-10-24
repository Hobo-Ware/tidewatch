use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use tidewatch_core::avatar::clean;

use super::*;
use crate::avatar_glue::FAKE;

fn avatar(index: usize) -> Avatar {
    clean(FAKE[index].0, FAKE[index].1).unwrap()
}

#[test]
fn an_avatar_is_drawn_into_a_square_of_real_pixels() {
    let image = render(&avatar(0).svg, 40).unwrap();
    assert_eq!(image.size, [40, 40]);
    let corner = image.pixels[0];
    let middle = image.pixels[20 * 40 + 20];
    assert_eq!(corner.a(), 0, "the rounded corner is clear");
    assert!(middle.a() > 0, "the tile is filled");
}

#[test]
fn every_mockup_icon_renders_with_colour_in_it() {
    for (index, (_, reason)) in FAKE.iter().enumerate() {
        let image = render(&avatar(index).svg, 64).unwrap();
        let colours: std::collections::HashSet<_> =
            image.pixels.iter().filter(|p| p.a() == 255).map(egui::Color32::to_array).collect();
        assert!(colours.len() >= 2, "{reason}");
    }
}

#[test]
fn a_wide_picture_is_fitted_not_cropped_off_the_square() {
    let wide = clean(
        "<svg viewBox=\"0 0 128 64\"><rect width=\"128\" height=\"64\" fill=\"#fff\"/></svg>",
        "x",
    )
    .unwrap();
    let image = render(&wide.svg, 32).unwrap();
    assert!(image.pixels[0].a() > 0);
    assert_eq!(image.pixels[31 * 32].a(), 0, "below the fitted picture is clear");
}

#[test]
fn the_texture_is_made_once_per_picture_and_size() {
    let ctx = Context::default();
    let one = texture(&ctx, &avatar(0), 20).unwrap();
    let again = texture(&ctx, &avatar(0), 20).unwrap();
    assert_eq!(one.id(), again.id());
    assert_ne!(one.id(), texture(&ctx, &avatar(0), 40).unwrap().id());
    assert_ne!(one.id(), texture(&ctx, &avatar(1), 20).unwrap().id());
}

#[test]
fn a_picture_that_cannot_be_drawn_is_remembered_as_none() {
    let ctx = Context::default();
    let broken = Avatar { svg: "<svg".into(), reason: "x".into() };
    assert!(texture(&ctx, &broken, 20).is_none());
    assert!(texture(&ctx, &broken, 20).is_none());
}

struct Frame {
    ctx: Context,
    rect: Rect,
}

impl Frame {
    fn new() -> Self {
        Self {
            ctx: Context::default(),
            rect: Rect::from_min_size(pos2(40.0, 40.0), vec2(20.0, 20.0)),
        }
    }

    fn run(&self, events: Vec<Event>) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 200.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| {
            let response = ui.interact(self.rect, ui.id().with("face"), Sense::click());
            menu(&response, 9);
        });
    }

    fn button(at: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE }
    }
}

#[test]
fn right_click_on_the_avatars_place_opens_a_menu() {
    let frame = Frame::new();
    frame.run(Vec::new());
    let at = frame.rect.center();
    frame.run(vec![Event::PointerMoved(at)]);
    frame.run(vec![Frame::button(at, PointerButton::Secondary, true)]);
    frame.run(vec![Frame::button(at, PointerButton::Secondary, false)]);
    frame.run(Vec::new());
    frame.run(Vec::new());
    let layers: Vec<_> = frame.ctx.memory(|m| m.areas().visible_layer_ids().into_iter().collect());
    assert!(layers.len() >= 2, "a popup is open: {layers:?}");
}

#[test]
fn a_choice_is_taken_once_and_nothing_is_chosen_before_it() {
    let ctx = Context::default();
    assert_eq!(take_choice(&ctx), None);
    ctx.data_mut(|d| d.insert_temp(Id::new(CHOICE), Some((4_u64, Choice::Redraw))));
    assert_eq!(take_choice(&ctx), Some((4, Choice::Redraw)));
    assert_eq!(take_choice(&ctx), None);
}

#[test]
fn a_picture_offers_redraw_and_use_letter_and_a_letter_offers_draw_avatar() {
    let labels = |picture| choices(picture).iter().map(|(label, _)| *label).collect::<Vec<_>>();
    assert_eq!(labels(true), ["Redraw", "Use letter"]);
    assert_eq!(labels(false), ["Draw avatar"]);
    assert_eq!(choices(false)[0].1, Choice::Draw);
}
