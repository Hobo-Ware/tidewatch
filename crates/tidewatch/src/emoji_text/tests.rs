use egui::Color32;
use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{Context, FontId};

use super::*;
use crate::fonts;

fn context() -> Context {
    let ctx = fonts::test_context();
    let _ = ctx.run_ui(egui::RawInput::default(), |_ui| {});
    ctx
}

fn found(text: &str) -> Vec<&str> {
    clusters(text).into_iter().map(|cluster| &text[cluster.bytes]).collect()
}

#[test]
fn a_cluster_is_named_by_its_code_points_without_variation_selectors() {
    let cases = [
        ("\u{1f600}", "1f600"),
        ("\u{2764}\u{fe0f}", "2764"),
        ("\u{26a0}\u{fe0f}", "26a0"),
        ("\u{1f1e7}\u{1f1ea}", "1f1e7_1f1ea"),
        ("\u{1f468}\u{200d}\u{1f4bb}", "1f468_200d_1f4bb"),
        ("\u{1f44d}\u{1f3fd}", "1f44d_1f3fd"),
        ("1\u{fe0f}\u{20e3}", "31_20e3"),
        (
            "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}",
            "1f3f4_e0067_e0062_e0065_e006e_e0067_e007f",
        ),
        ("\u{2764}\u{fe0f}\u{200d}\u{1f525}", "2764_200d_1f525"),
    ];
    for (cluster, key) in cases {
        assert_eq!(key_of(cluster), key, "{cluster:?}");
    }
}

#[test]
fn the_emoji_of_a_message_are_found_with_their_sequences_whole() {
    let cases = [
        (
            "Looks great \u{1f600} ship it \u{1f680} thanks \u{2764}\u{fe0f} \u{1f44d} and \u{1f389}",
            5,
        ),
        ("\u{1f600}\u{1f600}", 2),
        ("a \u{1f468}\u{200d}\u{1f4bb} b", 1),
        ("\u{1f1e7}\u{1f1ea}\u{1f1eb}\u{1f1f7}", 2),
        ("\u{1f44d}\u{1f3fd} ok", 1),
        ("1\u{fe0f}\u{20e3} 2\u{fe0f}\u{20e3}", 2),
        ("\u{2764}\u{fe0f}\u{200d}\u{1f525}", 1),
    ];
    for (text, count) in cases {
        assert_eq!(found(text).len(), count, "{text:?}");
    }
    assert_eq!(found("a \u{1f468}\u{200d}\u{1f4bb} b"), ["\u{1f468}\u{200d}\u{1f4bb}"]);
    assert_eq!(found("x\u{1f44d}\u{1f3fd}"), ["\u{1f44d}\u{1f3fd}"]);
}

#[test]
fn a_flag_takes_two_regional_letters_and_no_more() {
    assert_eq!(found("\u{1f1e7}\u{1f1ea}\u{1f1eb}"), ["\u{1f1e7}\u{1f1ea}"]);
}

#[test]
fn characters_that_are_text_stay_text() {
    for text in [
        "plain words and 123 #1 * 2",
        "(c) caf\u{e9} \u{65e5}\u{672c}",
        "\u{2713} done \u{2192} next",
        "\u{a9} 2026 \u{2764} \u{26a0}",
        "1\u{fe0f}",
        "",
    ] {
        assert_eq!(found(text), Vec::<&str>::new(), "{text:?}");
    }
}

#[test]
fn an_emoji_without_a_picture_is_left_alone() {
    let text = "a \u{1f600} b";
    assert_eq!(find(text, &|_| false), Vec::new());
    assert_eq!(find(text, &|key| key == "1f600").len(), 1);
}

#[test]
fn a_cluster_remembers_where_it_starts_and_ends_in_bytes() {
    let text = "\u{e9}\u{1f600}x\u{1f680}";
    let all = clusters(text);
    assert_eq!(all[0].bytes, 2..6);
    assert_eq!(all[1].bytes, 7..11);
    assert_eq!(all[1].key, "1f680");
}

fn format(line: Option<f32>) -> TextFormat {
    let mut format = TextFormat::simple(FontId::proportional(16.0), Color32::WHITE);
    format.line_height = line;
    format
}

fn slotted(ctx: &Context, text: &str, line: Option<f32>) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(text, 0.0, format(line));
    slot_job(ctx, job)
}

#[test]
fn a_text_with_no_emoji_comes_back_untouched() {
    let ctx = context();
    let mut job = LayoutJob::default();
    job.append("no emoji here \u{2713}", 0.0, format(None));
    assert_eq!(slot_job(&ctx, job.clone()), job);
}

#[test]
fn the_slot_keeps_the_text_and_hides_only_the_picture_part() {
    let ctx = context();
    let text = "hi \u{1f468}\u{200d}\u{1f4bb} there";
    let job = slotted(&ctx, text, Some(25.6));
    assert_eq!(job.text, text);
    let looks: Vec<(&str, f32, Color32, Option<f32>)> = job
        .sections
        .iter()
        .map(|s| {
            let range = usize::from(s.byte_range.start)..usize::from(s.byte_range.end);
            (&job.text[range], s.format.font_id.size, s.format.color, s.format.line_height)
        })
        .collect();
    assert_eq!(
        looks,
        [
            ("hi ", 16.0, Color32::WHITE, Some(25.6)),
            ("\u{1f468}", 20.0, Color32::TRANSPARENT, Some(25.6)),
            ("\u{200d}\u{1f4bb}", TAIL_PX, Color32::TRANSPARENT, Some(25.6)),
            (" there", 16.0, Color32::WHITE, Some(25.6)),
        ]
    );
}

#[test]
fn a_text_with_no_line_height_gets_its_fonts_so_the_rows_do_not_grow() {
    let ctx = context();
    let row = ctx.fonts_mut(|fonts| fonts.row_height(&FontId::proportional(16.0)));
    let job = slotted(&ctx, "a \u{1f600}", None);
    assert!(job.sections.iter().all(|s| s.format.line_height == Some(row)), "{row}");
}
