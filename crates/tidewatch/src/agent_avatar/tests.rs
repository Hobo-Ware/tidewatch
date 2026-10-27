use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use tidewatch_core::avatar::clean;

use super::*;
use crate::avatar_glue::FAKE;

fn avatar(index: usize) -> Avatar {
    clean(FAKE[index].0, FAKE[index].1).unwrap()
}

#[test]
fn an_avatar_is_drawn_into_a_square_of_real_pixels_inset_from_the_edge() {
    let image = render(&avatar(0).svg, 40).unwrap();
    assert_eq!(image.size, [40, 40]);
    assert_eq!(image.pixels[0].a(), 0, "the corner is clear");
    assert!(image.pixels.iter().any(|p| p.a() == 255), "the art is drawn");
    let edge: Vec<_> = (0..40).map(|i| image.pixels[20 * 40 + i].a()).collect();
    assert_eq!((edge[0], edge[1], edge[38], edge[39]), (0, 0, 0, 0), "nothing touches the edge");
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
fn a_wide_picture_is_fitted_and_centered_not_cropped_off_the_square() {
    let wide = clean(
        "<svg viewBox=\"0 0 128 64\"><rect width=\"128\" height=\"64\" fill=\"#fff\"/></svg>",
        "x",
    )
    .unwrap();
    let image = render(&wide.svg, 32).unwrap();
    assert!(image.pixels[16 * 32 + 16].a() > 0);
    assert_eq!(image.pixels[3 * 32 + 16].a(), 0, "above the fitted picture is clear");
    assert_eq!(image.pixels[29 * 32 + 16].a(), 0, "below the fitted picture is clear");
    assert!(image.pixels[14 * 32 + 8].a() > 0 && image.pixels[14 * 32 + 23].a() > 0);
}

fn outside_the_circle(image: &ColorImage) -> usize {
    let size = image.size[0];
    let half = size as f32 / 2.0;
    image
        .pixels
        .iter()
        .enumerate()
        .filter(|(index, pixel)| {
            let x = (index % size) as f32 + 0.5 - half;
            let y = (index / size) as f32 + 0.5 - half;
            x.hypot(y) > half && pixel.a() > 0
        })
        .count()
}

#[test]
fn the_face_is_cut_to_a_circle_whatever_the_art_does() {
    let full_bleed = clean(
        "<svg viewBox=\"0 0 64 64\"><rect x=\"-40\" y=\"-40\" width=\"144\" height=\"144\" fill=\"#fff\"/></svg>",
        "x",
    )
    .unwrap();
    for px in [20, 28, 36, 40, 72] {
        let image = render(&full_bleed.svg, px).unwrap();
        let corner = px as usize - 1;
        let corners = [0, corner, corner * px as usize, corner * px as usize + corner];
        for at in corners {
            assert_eq!(image.pixels[at].a(), 0, "{px} px, pixel {at}");
        }
        assert_eq!(outside_the_circle(&image), 0, "{px} px");
        assert!(image.pixels[(px as usize / 2) * px as usize + px as usize / 2].a() > 0);
    }
}

#[test]
fn a_cut_that_is_the_whole_of_a_big_square_leaves_the_art_inside_the_plate_ring() {
    let image = render(&avatar(0).svg, 100).unwrap();
    let half = 50.0_f32;
    let reach = image
        .pixels
        .iter()
        .enumerate()
        .filter(|(_, pixel)| pixel.a() > 0)
        .map(|(index, _)| {
            ((index % 100) as f32 + 0.5 - half).hypot((index / 100) as f32 + 0.5 - half)
        })
        .fold(0.0_f32, f32::max);
    assert!(reach < half * 0.7, "the art reaches {reach} of {half}");
}

struct Painted {
    shapes: Vec<egui::epaint::ClippedShape>,
}

fn painted(face: &Face, rect: Rect) -> Painted {
    let ctx = crate::fonts::test_context();
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 200.0))),
        ..RawInput::default()
    };
    let output = ctx.run_ui(raw, |ui| {
        paint_face(ui.ctx(), ui.painter(), rect, face);
    });
    Painted { shapes: output.shapes }
}

fn flat(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| flat(shape, out)),
        other => out.push(other.clone()),
    }
}

#[test]
fn a_picture_face_is_a_plate_circle_the_art_and_a_one_pixel_ring_and_a_letter_face_is_round_too() {
    let rect = Rect::from_min_size(pos2(40.0, 40.0), vec2(36.0, 36.0));
    let faces = [Face::Picture(Arc::new(avatar(0))), Face::of("Builder", "claude", None)];
    for face in faces {
        let mut shapes = Vec::new();
        for clipped in painted(&face, rect).shapes {
            flat(&clipped.shape, &mut shapes);
        }
        let circles: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Circle(circle) => Some(circle),
                _ => None,
            })
            .collect();
        assert!(
            shapes.iter().all(
                |shape| !matches!(shape, egui::Shape::Rect(r) if r.rect.size() == rect.size())
            ),
            "no square tile: {face:?}"
        );
        assert_eq!(circles.len(), 2, "a plate and a ring: {face:?}");
        assert_eq!(circles[0].center, rect.center());
        assert_eq!(circles[0].radius, 18.0);
        assert_eq!(circles[1].stroke.width, 1.0);
        assert_eq!(circles[1].stroke.color, brand::FACE_RING);
        assert_eq!(circles[1].radius, 17.5);
        if matches!(face, Face::Picture(_)) {
            assert_eq!(circles[0].fill, brand::FACE_PLATE);
        }
    }
}

#[test]
fn the_plate_is_the_color_the_readability_check_judges_against() {
    assert_eq!(brand::FACE_PLATE.to_array()[..3], tidewatch_core::avatar_check::PLATE,);
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
