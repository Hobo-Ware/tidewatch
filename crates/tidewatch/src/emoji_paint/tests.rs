use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RawInput, Shape, pos2};

use super::*;
use crate::fonts;

const BODY: f32 = 16.0;
const LINE: f32 = 25.6;

fn context() -> Context {
    let ctx = fonts::test_context();
    let _ = ctx.run_ui(RawInput::default(), |_ui| {});
    ctx
}

fn format() -> TextFormat {
    let mut format = TextFormat::simple(FontId::proportional(BODY), Color32::WHITE);
    format.line_height = Some(LINE);
    format
}

fn laid(ctx: &Context, text: &str, wrap: f32) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    job.append(text, 0.0, format());
    let slotted = emoji_text::slot_job(ctx, job);
    ctx.fonts_mut(|fonts| fonts.layout_job(slotted))
}

fn baseline(galley: &Galley, row: usize) -> f32 {
    galley.rows[row].pos.y + galley.rows[row].row.glyphs[0].pos.y
}

#[test]
fn the_picture_is_a_20_point_square_centred_6_points_above_the_baseline_of_the_text_beside_it() {
    let ctx = context();
    let galley = laid(&ctx, "Looks great \u{1f600} thanks", 400.0);
    let slot = &slots(&ctx, pos2(10.0, 30.0), &galley)[0];
    assert_eq!(slot.rect.size(), vec2(20.0, 20.0));
    assert_eq!(slot.rect.center().y, 30.0 + baseline(&galley, 0) - 6.0);
    assert_eq!(slot.key, "1f600");
}

#[test]
fn the_picture_is_centred_on_the_width_its_character_took() {
    let ctx = context();
    let galley = laid(&ctx, "ab \u{1f680} cd", 400.0);
    let glyphs = &galley.rows[0].row.glyphs;
    let at = glyphs.iter().position(|g| g.chr == '\u{1f680}').unwrap();
    let slot = &slots(&ctx, pos2(0.0, 0.0), &galley)[0];
    let (left, advance) = (glyphs[at].pos.x, glyphs[at].advance_width);
    assert!((slot.rect.center().x - (left + advance / 2.0)).abs() < 0.01);
    assert!((advance - 20.0).abs() < 1.0, "{advance}");
    assert!(glyphs[at + 1].pos.x >= slot.rect.right() - 1.0);
}

#[test]
fn a_row_with_emoji_is_as_tall_as_a_row_without() {
    let ctx = context();
    let plain = laid(&ctx, "Looks great x ship it x thanks", 400.0);
    let rich = laid(&ctx, "Looks great \u{1f600} ship it \u{1f680} thanks", 400.0);
    assert_eq!(plain.size().y, rich.size().y);
    let wrapped_plain = laid(&ctx, "one two three four five six seven eight nine", 90.0);
    let wrapped_rich =
        laid(&ctx, "one two three \u{1f600} four five \u{1f680} six seven eight nine", 90.0);
    assert_eq!(wrapped_plain.rows.len(), wrapped_rich.rows.len());
    assert_eq!(wrapped_plain.size().y, wrapped_rich.size().y);
}

#[test]
fn an_emoji_that_wrapped_to_a_later_row_has_its_picture_there() {
    let ctx = context();
    let galley = laid(&ctx, "one two three four five six \u{1f600}", 90.0);
    assert!(galley.rows.len() > 1);
    let slot = &slots(&ctx, pos2(0.0, 0.0), &galley)[0];
    let last = galley.rows.len() - 1;
    assert_eq!(slot.rect.center().y, baseline(&galley, last) - 6.0);
}

#[test]
fn every_emoji_of_a_message_gets_a_slot_in_order() {
    let ctx = context();
    let galley = laid(
        &ctx,
        "Looks great \u{1f600} ship it \u{1f680} thanks \u{2764}\u{fe0f} \u{1f44d} and \u{1f389}",
        600.0,
    );
    let keys: Vec<String> =
        slots(&ctx, pos2(0.0, 0.0), &galley).into_iter().map(|s| s.key).collect();
    assert_eq!(keys, ["1f600", "1f680", "2764", "1f44d", "1f389"]);
}

#[test]
fn a_sequence_has_one_slot_of_the_first_characters_width() {
    let ctx = context();
    let galley = laid(&ctx, "a \u{1f468}\u{200d}\u{1f4bb} b \u{1f1e7}\u{1f1ea} c", 600.0);
    let found = slots(&ctx, pos2(0.0, 0.0), &galley);
    let keys: Vec<&str> = found.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["1f468_200d_1f4bb", "1f1e7_1f1ea"]);
    assert!(found.iter().all(|s| s.rect.size() == vec2(20.0, 20.0)));
    let after_flag = galley.rows[0].row.glyphs.iter().find(|g| g.chr == 'c').unwrap().pos.x;
    assert!(after_flag - found[1].rect.right() < 8.0, "no second slot: {after_flag}");
}

#[test]
fn copy_and_select_still_give_the_real_emoji() {
    let ctx = context();
    let text = "ship \u{1f680} it \u{2764}\u{fe0f}";
    assert_eq!(laid(&ctx, text, 400.0).text(), text);
}

#[test]
fn the_still_picture_is_a_72_pixel_texture_loaded_once_and_none_for_an_unknown_emoji() {
    let ctx = context();
    let first = still(&ctx, "1f600").unwrap();
    assert_eq!(first.size(), [72, 72]);
    assert_eq!(still(&ctx, "1f600").unwrap().id(), first.id());
    assert!(still(&ctx, "ffff_ffff").is_none());
}

#[test]
fn the_painter_draws_one_picture_per_slot_and_nothing_for_plain_text() {
    let ctx = fonts::test_context();
    let count = |text: &str| {
        let out = ctx.run_ui(RawInput::default(), |ui| {
            let galley = laid(ui.ctx(), text, 400.0);
            paint(ui, Id::new("text"), pos2(0.0, 0.0), &galley);
        });
        out.shapes
            .iter()
            .filter(|clipped| matches!(&clipped.shape, Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default()))
            .count()
    };
    assert_eq!(count("no emoji at all"), 0);
    assert_eq!(count("a \u{1f600} b \u{1f680}"), 2);
}

struct Drawn {
    textures: Vec<egui::TextureId>,
    repaint_in: std::time::Duration,
}

fn frame(ctx: &Context, time: f64, pointer: Option<egui::Pos2>, fresh: bool) -> Drawn {
    let raw = RawInput {
        time: Some(time),
        events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
        ..RawInput::default()
    };
    let out = ctx.run_ui(raw, |ui| {
        emoji_motion::fresh_scope(ui.ctx(), fresh, || {
            let galley = laid(ui.ctx(), "ok \u{1f600} go", 400.0);
            paint(ui, Id::new("text"), pos2(0.0, 0.0), &galley);
        });
    });
    let textures = out
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
                Some(mesh.texture_id)
            }
            _ => None,
        })
        .collect();
    let repaint_in = out
        .viewport_output
        .values()
        .map(|viewport| viewport.repaint_delay)
        .min()
        .unwrap_or_default();
    Drawn { textures, repaint_in }
}

fn animated(ctx: &Context, shown: &Drawn) -> bool {
    let rest = still(ctx, "1f600").unwrap().id();
    shown.textures.iter().any(|id| *id != rest)
}

fn slot_centre(ctx: &Context) -> egui::Pos2 {
    slots(ctx, pos2(0.0, 0.0), &laid(ctx, "ok \u{1f600} go", 400.0))[0].rect.center()
}

#[test]
fn a_new_message_plays_its_animation_once_and_then_shows_the_still_picture() {
    let ctx = context();
    let first = frame(&ctx, 10.0, None, true);
    assert_eq!(first.textures.len(), 1);
    assert!(animated(&ctx, &first), "plays on arrival");
    assert!(animated(&ctx, &frame(&ctx, 10.3, None, true)));
    let rest = frame(&ctx, 40.0, None, true);
    assert_eq!(rest.textures, [still(&ctx, "1f600").unwrap().id()], "rests on the still");
}

#[test]
fn a_message_from_history_shows_the_still_picture_and_never_loads_the_animation() {
    let ctx = context();
    let shown = frame(&ctx, 10.0, None, false);
    assert_eq!(shown.textures, [still(&ctx, "1f600").unwrap().id()]);
    assert!(emoji_motion::clip_cached(&ctx, "1f600").is_none());
}

#[test]
fn pointing_at_an_emoji_at_rest_plays_it_again() {
    let ctx = context();
    frame(&ctx, 10.0, None, true);
    assert!(!animated(&ctx, &frame(&ctx, 40.0, None, true)));
    let at = slot_centre(&ctx);
    assert!(animated(&ctx, &frame(&ctx, 41.0, Some(at), false)), "hover starts it");
    assert!(animated(&ctx, &frame(&ctx, 41.2, Some(at), false)));
    assert!(!animated(&ctx, &frame(&ctx, 60.0, Some(at), false)), "and it rests again");
}

#[test]
fn with_animation_off_a_new_message_and_a_hover_both_show_the_still_picture() {
    let ctx = context();
    emoji_motion::set_allowed(&ctx, false);
    assert!(!animated(&ctx, &frame(&ctx, 10.0, None, true)));
    let at = slot_centre(&ctx);
    assert!(!animated(&ctx, &frame(&ctx, 11.0, Some(at), true)));
}

#[test]
fn a_playing_emoji_asks_for_the_next_frame_and_a_resting_one_asks_for_nothing() {
    let ctx = context();
    let playing = frame(&ctx, 10.0, None, true);
    assert!(playing.repaint_in < std::time::Duration::from_millis(200), "{:?}", playing.repaint_in);
    frame(&ctx, 60.0, None, true);
    let resting = frame(&ctx, 61.0, None, true);
    assert!(resting.repaint_in >= std::time::Duration::from_secs(1), "{:?}", resting.repaint_in);
}

#[test]
fn an_emoji_with_no_animation_never_moves() {
    let ctx = context();
    let one = |time, pointer: Option<egui::Pos2>| {
        let raw = RawInput {
            time: Some(time),
            events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
            ..RawInput::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            emoji_motion::fresh_scope(ui.ctx(), true, || {
                let galley = laid(ui.ctx(), "a \u{1f1e7}\u{1f1ea} b", 400.0);
                paint(ui, Id::new("flag"), pos2(0.0, 0.0), &galley);
            });
        });
        out.shapes
            .iter()
            .filter(|clipped| {
                matches!(&clipped.shape, Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default())
            })
            .count()
    };
    assert_eq!(one(0.0, None), 1);
    assert_eq!(one(5.0, Some(pos2(20.0, 15.0))), 1);
    assert!(emoji_motion::clip_cached(&ctx, "1f1e7_1f1ea").is_none());
}
