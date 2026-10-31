use std::path::Path;

use egui::{Context, Event, PointerButton, Pos2, RawInput, Shape};
use tidewatch_core::comment_flags::{Origin, Record};

use super::*;

fn records(count: usize) -> Vec<Record> {
    (0..count)
        .map(|n| Record {
            at_ms: n as u64,
            origin: Origin::Edit,
            file: "src/a.rs".into(),
            line: n + 1,
            kind: "narration".into(),
            text: "loop over the items".into(),
        })
        .collect()
}

fn roots() -> PathRoots {
    PathRoots::new(
        Path::new("/home/dev/code/shop.worktrees/season-recap"),
        Path::new("/home/dev/code/shop"),
        Some(Path::new("/home/dev")),
    )
}

fn frames(events: Vec<Vec<Event>>, strip: &Strip<'_>) -> (Vec<Option<Action>>, Rect) {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let band = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, HEIGHT));
    let mut seen = Vec::new();
    for batch in std::iter::once(Vec::new()).chain(events) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 300.0))),
            events: batch,
            focused: true,
            ..RawInput::default()
        };
        let mut action = None;
        let _ = ctx.run_ui(raw, |ui| action = show(ui, band, strip));
        seen.push(action);
    }
    (seen, band)
}

fn click_at(pos: Pos2) -> Vec<Vec<Event>> {
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![vec![Event::PointerMoved(pos)], vec![button(true)], vec![button(false)]]
}

#[test]
fn the_off_switch_asks_to_turn_the_tab_off_and_a_click_elsewhere_asks_nothing() {
    let rows = records(3);
    let strip = Strip { records: &rows, roots: &roots(), off: false, dev_open_popover: false };
    let (quiet, band) = frames(vec![vec![]], &strip);
    assert_eq!(quiet, [None, None]);
    let switch = pos2(band.right() - INSET - 14.0 - 20.0, band.center().y);
    let (clicked, _) = frames(click_at(switch), &strip);
    assert_eq!(clicked.last().copied().flatten(), Some(Action::SetOff(true)));
    let blank = pos2(band.left() + 520.0, band.center().y);
    let (clicked, _) = frames(click_at(blank), &strip);
    assert!(clicked.iter().all(Option::is_none));
}

#[test]
fn a_tab_that_is_off_offers_to_turn_the_guard_back_on() {
    let rows = records(2);
    let strip = Strip { records: &rows, roots: &roots(), off: true, dev_open_popover: false };
    let (_, band) = frames(vec![], &strip);
    let switch = pos2(band.right() - INSET - 14.0 - 20.0, band.center().y);
    let (clicked, _) = frames(click_at(switch), &strip);
    assert_eq!(clicked.last().copied().flatten(), Some(Action::SetOff(false)));
}

#[test]
fn the_strip_draws_with_no_flags_one_flag_many_flags_and_the_popover_open() {
    for count in [0, 1, 40] {
        let rows = records(count);
        for off in [false, true] {
            let strip = Strip { records: &rows, roots: &roots(), off, dev_open_popover: true };
            let (seen, _) = frames(vec![vec![], vec![]], &strip);
            assert!(seen.iter().all(Option::is_none), "{count} {off}");
        }
    }
}

const DEEP: &str = "/home/dev/code/shop.worktrees/season-recap/projects/client/src/lib/sections/summary/components/recap/SeasonRecapDrawerHost.svelte";
const COMMENT: &str = "open the recap drawer for the season the viewer picked, then move the focus to its first control once it has mounted and the transition ended";

fn long_records() -> Vec<Record> {
    let at = |n: u64, file: &str, line: usize, kind: &str, text: &str| Record {
        at_ms: n,
        origin: Origin::Edit,
        file: file.into(),
        line,
        kind: kind.into(),
        text: text.into(),
    };
    vec![
        at(
            1,
            "/home/dev/notes/a-rather-long-file-name-with-no-folders.md",
            3,
            "banner",
            "// ===== section =====",
        ),
        at(2, "/home/dev/code/shop/docs/guide.md", 0, "density", ""),
        at(
            3,
            "/opt/vendor/lib/deeper/and/deeper/still/Thing.c",
            4,
            "restates-signature",
            "returns the thing",
        ),
        at(4, DEEP, 58, "dead-code", COMMENT),
    ]
}

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect)> {
    fn walk(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Text(text) => {
                let rect = text.visual_bounding_rect();
                if rect.is_positive() {
                    out.push((text.galley.text().to_owned(), rect));
                }
            }
            Shape::Vec(all) => all.iter().for_each(|inner| walk(inner, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

struct Drawn {
    band: Rect,
    screen: Rect,
    popover: Rect,
    texts: Vec<(String, Rect)>,
}

fn draw_open(width: f32, rows: &[Record]) -> Drawn {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let band = Rect::from_min_size(pos2(40.0, 20.0), vec2(width, HEIGHT));
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(width + 480.0, 720.0));
    let strip = Strip { records: rows, roots: &roots(), off: false, dev_open_popover: true };
    let mut popup_id = None;
    let mut shapes = Vec::new();
    for _ in 0..4 {
        let raw = RawInput { screen_rect: Some(screen), focused: true, ..RawInput::default() };
        let out = ctx.run_ui(raw, |ui| {
            popup_id = Some(ui.id().with("comment-strip-chip").with("popup"));
            let _ = show(ui, band, &strip);
        });
        shapes = out.shapes;
    }
    let popover = ctx.memory(|memory| memory.area_rect(popup_id.unwrap())).expect("popover drawn");
    Drawn { band, screen, popover, texts: texts(&shapes) }
}

#[test]
fn nothing_the_row_or_popover_draws_leaves_its_frame_or_the_column_at_any_width() {
    let rows = long_records();
    for width in [400.0, 560.0, 900.0] {
        let drawn = draw_open(width, &rows);
        let column = drawn.band.shrink2(vec2(INSET, 0.0));
        let tolerance = 0.5;
        assert!(drawn.popover.width() <= POPOVER_W + tolerance, "{width}: {:?}", drawn.popover);
        assert!(
            drawn.popover.left() >= column.left() - tolerance
                && drawn.popover.right() <= column.right() + tolerance,
            "{width}: popover {:?} column {column:?}",
            drawn.popover
        );
        assert!(drawn.popover.bottom() <= drawn.screen.bottom(), "{width}");
        let card_bottom = drawn.band.center().y + CARD_H / 2.0;
        let mut inside_popover = 0;
        for (text, rect) in &drawn.texts {
            let frame = if rect.center().y > card_bottom {
                inside_popover += 1;
                drawn.popover
            } else {
                column
            };
            assert!(
                rect.left() >= frame.left() - tolerance
                    && rect.right() <= frame.right() + tolerance
                    && rect.top() >= frame.top() - tolerance
                    && rect.bottom() <= frame.bottom() + tolerance,
                "{width}: {text:?} {rect:?} leaves {frame:?}"
            );
        }
        assert!(inside_popover >= 9, "{width}: only {inside_popover} popover texts");
    }
}

#[test]
fn the_flagged_file_shows_relative_with_its_name_and_line_and_never_a_home_path() {
    let rows = long_records();
    for width in [400.0, 560.0, 900.0] {
        let drawn = draw_open(width, &rows);
        let shown: Vec<&str> = drawn.texts.iter().map(|(text, _)| text.as_str()).collect();
        assert!(shown.iter().all(|text| !text.contains("/home/dev")), "{width}: {shown:?}");
        assert!(
            shown.iter().any(|text| text.ends_with("/SeasonRecapDrawerHost.svelte:58")
                && (width < 800.0 || text.starts_with("projects/"))),
            "{width}: {shown:?}"
        );
        assert!(
            width < 500.0 || shown.iter().any(|text| text.starts_with("~/")),
            "{width}: {shown:?}"
        );
        assert!(shown.contains(&"commented-out code"), "{width}: {shown:?}");
        assert!(shown.contains(&"restates signature"), "{width}: {shown:?}");
        assert!(
            shown.contains(&"Settings, General, Guards turns the rule off for every tab")
                || shown.iter().any(|text| text.starts_with("Settings, General")),
            "{width}: {shown:?}"
        );
    }
}

#[test]
fn the_row_shows_the_newest_file_cut_in_the_middle_when_there_is_room() {
    let rows = long_records();
    let wide = draw_open(900.0, &rows);
    let row: Vec<&str> = wide
        .texts
        .iter()
        .filter(|(_, rect)| rect.center().y <= wide.band.center().y + CARD_H / 2.0)
        .map(|(text, _)| text.as_str())
        .collect();
    assert!(
        row.iter()
            .any(|text| text.contains('…') && text.ends_with("SeasonRecapDrawerHost.svelte:58")),
        "{row:?}"
    );
}

#[test]
fn a_comment_is_cut_to_two_lines_within_the_width() {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let raw = RawInput { focused: true, ..RawInput::default() };
    let _ = ctx.run_ui(raw, |ui| {
        let painter = ui.painter().clone();
        let long = COMMENT.repeat(4);
        for width in [120.0, 260.0, 420.0] {
            let galley = clipped(&painter, &long, mono(12.0), brand::MUTED, width, COMMENT_ROWS);
            assert_eq!(galley.rows.len(), COMMENT_ROWS, "{width}");
            assert!(galley.size().x <= width + 0.5, "{width}: {}", galley.size().x);
        }
        let unbroken = "x".repeat(300);
        let galley = clipped(&painter, &unbroken, mono(12.0), brand::MUTED, 260.0, COMMENT_ROWS);
        assert!(galley.rows.len() <= COMMENT_ROWS, "{}", galley.rows.len());
        assert!(galley.size().x <= 260.5, "{}", galley.size().x);
        let short = clipped(&painter, "loop over a", mono(12.0), brand::MUTED, 420.0, COMMENT_ROWS);
        assert_eq!(short.rows.len(), 1);
    });
}

#[test]
fn the_popover_is_the_column_wide_at_most_and_never_wider_than_the_window() {
    let cases = [
        (800.0, 1400.0, 460.0),
        (300.0, 1400.0, 300.0),
        (800.0, 320.0, 304.0),
        (0.0, 1400.0, 0.0),
        (800.0, 10.0, 0.0),
    ];
    for (column, frame, want) in cases {
        assert_eq!(popover_width(column, frame), want, "{column} {frame}");
    }
}

#[test]
fn the_popover_header_counts_the_flags_and_its_button_turns_the_tab_off() {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let rows = records(4);
    let band = Rect::from_min_size(pos2(40.0, 20.0), vec2(900.0, HEIGHT));
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 720.0));
    let strip = Strip { records: &rows, roots: &roots(), off: false, dev_open_popover: true };
    let run = |events: Vec<Event>| {
        let raw =
            RawInput { screen_rect: Some(screen), events, focused: true, ..RawInput::default() };
        let mut action = None;
        let out = ctx.run_ui(raw, |ui| action = show(ui, band, &strip));
        (action, texts(&out.shapes))
    };
    let mut shown = Vec::new();
    for _ in 0..4 {
        shown = run(Vec::new()).1;
    }
    let card_bottom = band.center().y + CARD_H / 2.0;
    let in_popover: Vec<&(String, Rect)> =
        shown.iter().filter(|(_, rect)| rect.center().y > card_bottom).collect();
    assert!(in_popover.iter().any(|(text, _)| text == "4 flagged comments"), "{in_popover:?}");
    let (_, button) = in_popover
        .iter()
        .find(|(text, _)| text == "Turn off for this tab")
        .expect("the header button");
    let pos = button.center();
    let press = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut actions = Vec::new();
    for events in [vec![Event::PointerMoved(pos)], vec![press(true)], vec![press(false)]] {
        actions.push(run(events).0);
    }
    assert_eq!(actions.into_iter().flatten().collect::<Vec<_>>(), [Action::SetOff(true)]);
}
