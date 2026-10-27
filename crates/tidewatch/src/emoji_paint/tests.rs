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
            paint(ui, pos2(0.0, 0.0), &galley);
        });
        out.shapes
            .iter()
            .filter(|clipped| matches!(&clipped.shape, Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default()))
            .count()
    };
    assert_eq!(count("no emoji at all"), 0);
    assert_eq!(count("a \u{1f600} b \u{1f680}"), 2);
}
