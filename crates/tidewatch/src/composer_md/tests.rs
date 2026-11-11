use egui::Context;
use egui::epaint::text::LayoutJob;

use super::*;

const NAMES: [(Flags, &str); 9] = [
    (Flags::BOLD, "bold"),
    (Flags::ITALIC, "italic"),
    (Flags::STRIKE, "strike"),
    (Flags::CODE, "code"),
    (Flags::LINK, "link"),
    (Flags::MARK, "mark"),
    (Flags::QUOTE, "quote"),
    (Flags::FENCED, "fenced"),
    (Flags::DIM, "dim"),
];

fn styled(text: &str) -> Vec<(String, String)> {
    parse(text)
        .runs()
        .iter()
        .map(|run| {
            let mut names: Vec<String> = NAMES
                .iter()
                .filter(|(flag, _)| run.flags.has(*flag))
                .map(|(_, name)| (*name).to_owned())
                .collect();
            if run.heading > 0 {
                names.push(format!("h{}", run.heading));
            }
            (text[run.range.clone()].to_owned(), names.join(" "))
        })
        .collect()
}

fn runs(text: &str, want: &[(&str, &str)]) {
    let want: Vec<(String, String)> =
        want.iter().map(|(piece, names)| ((*piece).to_owned(), (*names).to_owned())).collect();
    assert_eq!(styled(text), want, "text {text:?}");
}

#[test]
fn every_construct_gets_its_style_and_its_marks() {
    runs("a **b** c", &[("**", "bold mark"), ("b", "bold"), ("**", "bold mark")]);
    runs("a _b_ c", &[("_", "italic mark"), ("b", "italic"), ("_", "italic mark")]);
    runs("*b*", &[("*", "italic mark"), ("b", "italic"), ("*", "italic mark")]);
    runs("~~b~~", &[("~~", "strike mark"), ("b", "strike"), ("~~", "strike mark")]);
    runs("x `c` y", &[("`", "code mark"), ("c", "code"), ("`", "code mark")]);
    runs("# T", &[("# ", "mark h1"), ("T", "h1")]);
    runs("###### T", &[("###### ", "mark h6"), ("T", "h6")]);
    runs("> q", &[("> ", "mark quote"), ("q", "quote")]);
    runs("> > q", &[("> > ", "mark quote"), ("q", "quote")]);
}

#[test]
fn bold_italic_nests_and_a_heading_holds_inline_styles() {
    runs(
        "***b***",
        &[
            ("*", "italic mark"),
            ("**", "bold italic mark"),
            ("b", "bold italic"),
            ("**", "bold italic mark"),
            ("*", "italic mark"),
        ],
    );
    runs(
        "# a `c`",
        &[
            ("# ", "mark h1"),
            ("a ", "h1"),
            ("`", "code mark h1"),
            ("c", "code h1"),
            ("`", "code mark h1"),
        ],
    );
}

#[test]
fn a_task_item_marks_its_box_and_dims_the_text_once_done() {
    runs("- [ ] a", &[("[ ] ", "mark")]);
    runs("- [x] a", &[("[x] ", "mark"), ("a", "dim")]);
}

#[test]
fn links_and_bare_addresses_are_links_with_their_brackets_as_marks() {
    runs("[t](http://x.y)", &[("[", "link mark"), ("t", "link"), ("](http://x.y)", "link mark")]);
    runs("see https://a.b/c_d. ok", &[("https://a.b/c_d", "link")]);
    runs("(http://a.b/x)", &[("http://a.b/x", "link")]);
    runs("no http:// here", &[]);
    runs("<https://a.b>", &[("<", "link mark"), ("https://a.b", "link"), (">", "link mark")]);
}

#[test]
fn a_fenced_block_is_mono_lines_between_two_mark_lines() {
    runs(
        "```rs\nlet a = 1;\n```",
        &[("```rs", "mark fenced"), ("let a = 1;", "fenced"), ("```", "mark fenced")],
    );
    runs("```\nopen", &[("```", "mark fenced"), ("open", "fenced")]);
    runs("~~~\nx\n~~~", &[("~~~", "mark fenced"), ("x", "fenced"), ("~~~", "mark fenced")]);
}

#[test]
fn text_markdown_does_not_reach_stays_plain() {
    runs("plain words", &[]);
    runs("2 * 3 * 4", &[]);
    runs("snake_case_name", &[]);
    runs("| **a** | b |\n|---|---|\n| 1 | 2 |", &[]);
    runs("    indented code", &[]);
    runs("#nospace", &[]);
    runs("ab\n---", &[]);
    runs("- ab\n    - ", &[]);
    runs("Title\n=====", &[]);
}

#[test]
fn lines_know_their_block() {
    let parsed = parse("a\n> b\n> > c\n```\n- d\n```\ne");
    let blocks: Vec<Block> = (0..7).map(|line| parsed.line(line).unwrap().block).collect();
    assert_eq!(
        blocks,
        vec![
            Block::Plain,
            Block::Quote { depth: 1, prefix: 2 },
            Block::Quote { depth: 2, prefix: 4 },
            Block::Code { fence: true },
            Block::Code { fence: false },
            Block::Code { fence: true },
            Block::Plain,
        ]
    );
}

#[test]
fn the_cursor_lines_are_those_the_cursor_or_selection_touches() {
    let cases = [
        ("a\nb\nc", (0, 0), 0..=0),
        ("a\nb\nc", (2, 2), 1..=1),
        ("a\nb\nc", (0, 4), 0..=2),
        ("a\nb\nc", (4, 0), 0..=2),
        ("a\nb\nc", (1, 1), 0..=0),
        ("", (0, 0), 0..=0),
    ];
    for (text, cursor, want) in cases {
        assert_eq!(shown_for(text, cursor), want, "{text:?} {cursor:?}");
    }
}

#[test]
fn the_parse_is_kept_for_the_same_text() {
    let a = parse("**a**");
    let b = parse("**a**");
    assert!(Arc::ptr_eq(&a, &b));
    assert!(!Arc::ptr_eq(&a, &parse("**b**")));
}

fn with_ui<R>(run: impl FnOnce(&Ui) -> R) -> R {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let (mut run, mut out) = (Some(run), None);
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        out = run.take().map(|run| run(ui));
    });
    out.expect("the frame ran")
}

fn built(text: &str, shown: &Shown) -> LayoutJob {
    with_ui(|ui| {
        let font = FontId::proportional(16.0);
        let look = Look { font: &font, ink: Color32::WHITE, tint: &None, shown };
        let mut job = LayoutJob::default();
        append(ui, &mut job, (text, 0), &look, &parse(text));
        job
    })
}

fn sections_of(job: &LayoutJob) -> Vec<(String, f32, Color32)> {
    job.sections
        .iter()
        .map(|section| {
            (
                job.text[section.byte_range.start.0..section.byte_range.end.0].to_owned(),
                section.format.font_id.size,
                section.format.color,
            )
        })
        .collect()
}

#[test]
fn a_mark_is_hidden_off_the_cursor_line_and_drawn_faint_on_it() {
    let text = "**a**\n**b**";
    let on_first = sections_of(&built(text, &Some(0..=0)));
    let marks: Vec<(&str, bool)> = on_first
        .iter()
        .filter(|(piece, ..)| piece == "**")
        .map(|(_, size, color)| ("**", *size < 1.0 && *color == Color32::TRANSPARENT))
        .collect();
    assert_eq!(marks, vec![("**", false), ("**", false), ("**", true), ("**", true)]);
    let faint: Vec<bool> = on_first
        .iter()
        .filter(|(piece, ..)| piece == "**")
        .map(|s| s.2 == brand::faint())
        .collect();
    assert_eq!(faint, vec![true, true, false, false]);
}

#[test]
fn hiding_marks_never_changes_the_text() {
    let cases =
        ["**a** _b_\n> q\n# h", "```rs\nx\n```\n- [x] t\n[l](http://a.b)", "`c`\n\n> > d\n~~s~~"];
    for text in cases {
        for shown in [None, Some(0..=0), Some(1..=2)] {
            assert_eq!(built(text, &shown).text, text, "{text:?} {shown:?}");
        }
    }
}

#[test]
fn a_hidden_fence_line_keeps_a_thin_row_and_a_shown_one_a_full_row() {
    let text = "```\nx\n```";
    let rows = |shown: &Shown| {
        with_ui(|ui| {
            let font = FontId::proportional(16.0);
            let look = Look { font: &font, ink: Color32::WHITE, tint: &None, shown };
            crate::bullets::layout(ui, text, 300.0, &look)
                .rows
                .iter()
                .map(|row| row.size.y)
                .collect::<Vec<f32>>()
        })
    };
    let hidden = rows(&None);
    let shown = rows(&Some(0..=0));
    assert_eq!(hidden.len(), 3);
    assert!(hidden[0] < 8.0 && hidden[2] < 8.0, "{hidden:?}");
    assert!(shown[0] > 12.0, "{shown:?}");
    assert!(hidden[1] > 12.0);
}

#[test]
fn a_heading_row_is_taller_than_a_plain_one() {
    let heights = |text: &str| {
        with_ui(|ui| {
            let font = FontId::proportional(16.0);
            let look = Look { font: &font, ink: Color32::WHITE, tint: &None, shown: &None };
            crate::bullets::layout(ui, text, 300.0, &look).rows[0].size.y
        })
    };
    assert!(heights("# T") > heights("T") * 1.4);
    assert!(heights("###### T") < heights("# T"));
}

#[test]
fn a_quote_line_is_drawn_one_indent_in_with_a_bar_and_raw_on_the_cursor_line() {
    with_ui(|ui| {
        let font = FontId::proportional(16.0);
        let galley = |shown: &Shown| {
            let look = Look { font: &font, ink: Color32::WHITE, tint: &None, shown };
            crate::bullets::layout(ui, "> q\nplain", 300.0, &look)
        };
        let hidden = galley(&None);
        assert_eq!(hidden.rows[0].pos.x, QUOTE_INDENT);
        assert_eq!(hidden.rows[1].pos.x, 0.0);
        let bars = blocks(&hidden, Pos2::ZERO, &None).bars;
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].0, BAR_INSET);
        let raw = galley(&Some(0..=0));
        assert_eq!(raw.rows[0].pos.x, 0.0);
        assert_eq!(blocks(&raw, Pos2::ZERO, &Some(0..=0)).bars, Vec::new());
    });
}

#[test]
fn quote_lines_share_one_bar_and_a_code_block_one_background() {
    with_ui(|ui| {
        let font = FontId::proportional(16.0);
        let look = Look { font: &font, ink: Color32::WHITE, tint: &None, shown: &None };
        let galley = crate::bullets::layout(ui, "> a\n> b\nx\n```\ny\nz\n```", 300.0, &look);
        let found = blocks(&galley, Pos2::ZERO, &None);
        assert_eq!(found.bars.len(), 1);
        assert_eq!(found.codes.len(), 1);
        let (top, bottom) = found.codes[0];
        let rows = &galley.rows;
        assert_eq!(top, rows[3].pos.y);
        assert_eq!(bottom, rows[6].pos.y + rows[6].size.y);
    });
}

#[test]
fn a_task_item_draws_its_box_in_the_gutter_and_no_bracket_text_off_the_cursor_line() {
    with_ui(|ui| {
        let font = FontId::proportional(16.0);
        let look = Look { font: &font, ink: Color32::WHITE, tint: &None, shown: &None };
        let galley = crate::bullets::layout(ui, "- [ ] a", 300.0, &look);
        assert_eq!(galley.text(), "- [ ] a");
        assert_eq!(galley.rows[0].pos.x, crate::bullets::text_edge(0));
        let plain = crate::bullets::layout(ui, "- a", 300.0, &look);
        assert!(
            (galley.rows[0].glyphs.last().unwrap().pos.x
                - plain.rows[0].glyphs.last().unwrap().pos.x)
                .abs()
                < 1.5
        );
    });
}
