//! A fenced code block in the Reader: a well inside the bubble with a header strip (the language,
//! the Copy button) over the code. Long lines wrap with a hanging indent instead of scrolling.

use egui::{
    Align2, Color32, CornerRadius, Frame, Id, Label, Margin, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, StrokeKind, TextStyle, Ui, text::LayoutJob, vec2,
};
use egui_extras::syntax_highlighting::{CodeTheme, highlight};

use crate::brand;
use crate::screen_kit::{Size, body, mono};

const RADIUS: u8 = 8;
const PAD: i8 = 12;
const CODE_PX: f32 = 14.0;
const HEADER_H: f32 = Size::Compact.height() + 8.0;
const HEADER_PAD: f32 = 10.0;
const BUTTON_H: f32 = Size::Compact.height();
const BUTTON_PAD: f32 = Size::Compact.pad();
const ICON: f32 = 12.0;
const ICON_GAP: f32 = 6.0;
const LABEL_PX: f32 = 12.0;
const HANG_COLS: usize = 2;
const MIN_ROW_COLS: f32 = 8.0;
const COPIED_SECS: f64 = 1.5;
const HIGHLIGHTED: [&str; 5] = ["rs", "rust", "py", "python", "toml"];

/// What a code block sits on: the fill of its well.
#[derive(Clone, Copy)]
pub(crate) struct Look {
    pub(crate) fill: Color32,
}

impl Look {
    pub(crate) const AGENT: Self = Self { fill: brand::SUNKEN };
    pub(crate) const USER: Self = Self { fill: Color32::from_black_alpha(90) };
}

/// Where the parts of a drawn block landed, left in egui's memory for the headless tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shown {
    pub(crate) frame: Rect,
    pub(crate) header: Rect,
    pub(crate) copy: Rect,
    pub(crate) text: Rect,
}

/// The width a block needs to show its longest line unwrapped, padding and border included.
pub(crate) fn natural_width(painter: &Painter, text: &str) -> f32 {
    let widest = text
        .lines()
        .map(|line| painter.layout_no_wrap(line.to_owned(), mono(CODE_PX), Color32::WHITE).size().x)
        .fold(0.0, f32::max);
    widest.ceil() + 2.0 * f32::from(PAD) + 2.0
}

pub(crate) fn show(ui: &mut Ui, lang: Option<&str>, text: &str, look: Look) {
    let outer = ui.available_width();
    let id = ui.id().with("code-block");
    let (mut header, mut copy, mut shown_text) = (Rect::NOTHING, Rect::NOTHING, Rect::NOTHING);
    #[cfg_attr(not(test), allow(unused_variables))]
    let framed = Frame::new()
        .fill(look.fill)
        .stroke(Stroke::new(1.0, brand::ITEM_LINE))
        .corner_radius(CornerRadius::same(RADIUS))
        .show(ui, |ui| {
            ui.set_width(outer - 2.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let strip = strip(ui, id, lang, text);
            header = strip.0;
            copy = strip.1;
            let inner = Frame::new().inner_margin(Margin::same(PAD)).show(ui, |ui| {
                ui.set_width(outer - 2.0 - 2.0 * f32::from(PAD));
                code(ui, lang, text)
            });
            shown_text = inner.inner;
        });
    #[cfg(test)]
    ui.data_mut(|data| {
        data.insert_temp(
            id.with("shown"),
            Shown { frame: framed.response.rect, header, copy, text: shown_text },
        );
    });
}

fn strip(ui: &mut Ui, id: Id, lang: Option<&str>, text: &str) -> (Rect, Rect) {
    let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let painter = ui.painter();
    painter.hline(header.x_range(), header.bottom(), Stroke::new(1.0, brand::ITEM_LINE));
    if let Some(lang) = lang {
        painter.text(
            header.left_center() + vec2(f32::from(PAD), 0.0),
            Align2::LEFT_CENTER,
            lang,
            mono(LABEL_PX),
            brand::MUTED,
        );
    }
    let now = ui.input(|input| input.time);
    let until = ui.data(|data| data.get_temp::<f64>(id)).unwrap_or(0.0);
    let copied = now < until;
    let width = button_width(ui);
    let rect = Rect::from_center_size(
        header.right_center() - vec2(HEADER_PAD + width / 2.0, 0.0),
        vec2(width, BUTTON_H),
    );
    let response = ui.interact(rect, id.with("copy"), Sense::click());
    paint_button(ui.painter(), rect, copied, &response);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.clone().on_hover_text("Copy to clipboard");
    if response.clicked() {
        ui.ctx().copy_text(text.to_owned());
        ui.data_mut(|data| data.insert_temp(id, now + COPIED_SECS));
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(COPIED_SECS));
    } else if copied {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(until - now));
    }
    (header, rect)
}

fn label_width(ui: &Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), body(Size::Compact.font()), Color32::WHITE)
        .size()
        .x
}

fn button_width(ui: &Ui) -> f32 {
    let text = label_width(ui, "Copy").max(label_width(ui, "Copied"));
    2.0 * BUTTON_PAD + ICON + ICON_GAP + text.ceil()
}

fn paint_button(painter: &Painter, rect: Rect, copied: bool, response: &Response) {
    let (line, ink, fill) = if copied {
        (brand::GREEN.gamma_multiply(0.5), brand::GREEN, Color32::TRANSPARENT)
    } else if response.is_pointer_button_down_on() {
        (brand::FAINT, brand::TEXT, brand::SEGMENT)
    } else if response.hovered() {
        (brand::FAINT, brand::TEXT, brand::BUTTON)
    } else {
        (brand::FIELD_LINE, brand::SOFT, Color32::TRANSPARENT)
    };
    painter.rect(
        rect,
        CornerRadius::same(Size::Compact.radius()),
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let icon =
        Rect::from_min_size(rect.left_center() + vec2(BUTTON_PAD, -ICON / 2.0), vec2(ICON, ICON));
    if copied {
        paint_check(painter, icon, ink);
    } else {
        paint_copy(painter, icon, ink);
    }
    painter.text(
        rect.left_center() + vec2(BUTTON_PAD + ICON + ICON_GAP, 0.0),
        Align2::LEFT_CENTER,
        if copied { "Copied" } else { "Copy" },
        body(Size::Compact.font()),
        ink,
    );
}

fn grid_point(icon: Rect, x: f32, y: f32) -> Pos2 {
    icon.min + vec2(x, y) * (icon.width() / 24.0)
}

fn paint_copy(painter: &Painter, icon: Rect, ink: Color32) {
    let stroke = Stroke::new(1.3, ink);
    let at = |x, y| grid_point(icon, x, y);
    let back = [(15.0, 9.0), (15.0, 5.0), (13.0, 3.0), (5.0, 3.0), (3.0, 5.0), (3.0, 13.0)];
    let back = back.iter().map(|(x, y)| at(*x, *y)).chain([at(5.0, 15.0), at(9.0, 15.0)]);
    painter.add(Shape::line(back.collect(), stroke));
    let front = Rect::from_min_max(at(9.0, 9.0), at(21.0, 21.0));
    painter.rect_stroke(front, CornerRadius::same(2), stroke, StrokeKind::Middle);
}

fn paint_check(painter: &Painter, icon: Rect, ink: Color32) {
    let points = [(4.5, 12.5), (9.5, 17.5), (19.5, 7.0)];
    painter.add(Shape::line(
        points.iter().map(|(x, y)| grid_point(icon, *x, *y)).collect(),
        Stroke::new(1.6, ink),
    ));
}

fn code(ui: &mut Ui, lang: Option<&str>, text: &str) -> Rect {
    ui.scope(|ui| {
        ui.style_mut().text_styles.insert(TextStyle::Monospace, mono(CODE_PX));
        let width = ui.available_width();
        let wrapped = wrap(ui, text, width);
        let job = styled(ui, lang, &wrapped);
        ui.add(Label::new(job).selectable(true)).rect
    })
    .inner
}

fn styled(ui: &Ui, lang: Option<&str>, shown: &str) -> LayoutJob {
    let supported = lang.is_some_and(|lang| HIGHLIGHTED.contains(&lang.to_lowercase().as_str()));
    match lang {
        Some(lang) if supported => {
            let style = ui.style();
            highlight(ui.ctx(), style, &CodeTheme::from_style(style), shown, lang)
        }
        _ => LayoutJob::simple(shown.to_owned(), mono(CODE_PX), brand::TEXT, f32::INFINITY),
    }
}

fn wrap(ui: &Ui, text: &str, width: f32) -> String {
    let font = mono(CODE_PX);
    let column = ui.fonts_mut(|fonts| fonts.glyph_width(&font, ' '));
    let mut first_row = |rest: &str, room: f32| {
        let job = LayoutJob::simple(rest.to_owned(), font.clone(), Color32::WHITE, room);
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        galley.rows.first().map_or(0, |row| row.char_count_including_newline().0)
    };
    wrap_text(text, width, column, &mut first_row)
}

/// `text` with every line that is wider than `width` broken into rows. `column` is the width of
/// one mono character and `first_row(rest, room)` how many characters of `rest` fit a row `room`
/// wide. A row after the first is indented by the line's own indent plus two columns.
pub(crate) fn wrap_text(
    text: &str,
    width: f32,
    column: f32,
    first_row: &mut dyn FnMut(&str, f32) -> usize,
) -> String {
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| wrap_line(&line.replace('\t', "    "), width, column, first_row))
        .collect();
    lines.join("\n")
}

fn wrap_line(
    line: &str,
    width: f32,
    column: f32,
    first_row: &mut dyn FnMut(&str, f32) -> usize,
) -> String {
    if line.chars().count() as f32 * column <= width {
        return line.to_owned();
    }
    let lead = line.chars().take_while(|c| *c == ' ').count();
    let columns = (width / column).floor();
    let hang = (lead + HANG_COLS).min((columns / 3.0) as usize);
    let narrow = (width - hang as f32 * column).max(MIN_ROW_COLS * column);
    let indent = " ".repeat(hang);
    let mut out = String::new();
    let mut rest = line;
    let mut room = width;
    loop {
        let total = rest.chars().count();
        let take = first_row(rest, room).clamp(1, total);
        if take >= total {
            out.push_str(rest);
            return out;
        }
        let at = rest.char_indices().nth(take).map_or(rest.len(), |(byte, _)| byte);
        out.push_str(rest[..at].trim_end());
        out.push('\n');
        out.push_str(&indent);
        rest = &rest[at..];
        room = narrow;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, FullOutput, OutputCommand, PointerButton, RawInput, pos2};

    const COMMAND: &str = "! git push --force-with-lease=agent/clean-up-schema:3f9c2a1b7d \
        origin agent/clean-up-schema && gh pr create --draft --fill --base main --head agent/clean-up-schema";

    fn measure(rest: &str, room: f32) -> usize {
        let fit = (room / 10.0) as usize;
        let chars: Vec<char> = rest.chars().collect();
        if chars.len() <= fit {
            return chars.len();
        }
        chars[..=fit].iter().rposition(|c| *c == ' ').map_or(fit, |space| space + 1)
    }

    #[test]
    fn long_lines_wrap_with_a_hanging_indent_and_short_ones_stay() {
        let cases = [
            ("short", "short"),
            ("a\n\nb\n", "a\n\nb\n"),
            ("aaaa bbbb cccc dddd", "aaaa bbbb\n  cccc\n  dddd"),
            ("  xxxx yyyy zzzz", "  xxxx\n   yyyy\n   zzzz"),
            (&"x".repeat(25), "xxxxxxxxxx\n  xxxxxxxx\n  xxxxxxx"),
            ("\tab", "    ab"),
            ("one\naaaa bbbb cccc dddd\ntwo", "one\naaaa bbbb\n  cccc\n  dddd\ntwo"),
        ];
        for (input, want) in cases {
            assert_eq!(wrap_text(input, 100.0, 10.0, &mut measure), want, "input {input:?}");
        }
    }

    fn painted(out: &FullOutput) -> Vec<String> {
        out.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    fn frame(
        ctx: &egui::Context,
        (width, lang, text): (f32, Option<&str>, &str),
        events: Vec<Event>,
        time: f64,
    ) -> (FullOutput, Shown) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 900.0))),
            events,
            focused: true,
            time: Some(time),
            ..RawInput::default()
        };
        let mut shown = None;
        let out = ctx.run_ui(raw, |ui| {
            ui.set_width(width);
            show(ui, lang, text, Look::AGENT);
            shown =
                ui.data(|data| data.get_temp::<Shown>(ui.id().with("code-block").with("shown")));
        });
        (out, shown.expect("the block recorded where it drew"))
    }

    fn settled(ctx: &egui::Context, block: (f32, Option<&str>, &str), time: f64) -> Shown {
        let _ = frame(ctx, block, vec![], time);
        frame(ctx, block, vec![], time).1
    }

    #[test]
    fn the_code_never_runs_under_the_copy_slot_at_any_width() {
        for width in [180.0, 240.0, 320.0, 480.0, 720.0] {
            for text in [COMMAND, "cargo test", "a\nb\nc", ""] {
                let ctx = crate::fonts::test_context();
                let shown = settled(&ctx, (width, Some("sh"), text), 1.0);
                let room = format!("width {width}, text {text:?}: {shown:?}");
                assert!(shown.copy.min.y >= shown.header.min.y, "{room}");
                assert!(shown.copy.max.y <= shown.header.max.y, "{room}");
                assert!(shown.copy.max.x <= shown.header.max.x - HEADER_PAD + 0.5, "{room}");
                assert!(shown.text.min.y >= shown.header.max.y + f32::from(PAD) - 0.5, "{room}");
                assert!(!shown.text.intersects(shown.copy), "{room}");
                assert!(shown.text.min.x >= shown.frame.min.x + f32::from(PAD), "{room}");
                assert!(shown.text.max.x <= shown.frame.max.x - f32::from(PAD) + 0.5, "{room}");
                assert!((shown.frame.width() - width).abs() < 0.5, "{room}");
            }
        }
    }

    #[test]
    fn a_long_command_wraps_inside_the_block_with_a_hanging_indent() {
        let ctx = crate::fonts::test_context();
        let block = (360.0, Some("sh"), COMMAND);
        let _ = frame(&ctx, block, vec![], 1.0);
        let (out, shown) = frame(&ctx, block, vec![], 1.0);
        let code = painted(&out)
            .into_iter()
            .find(|text| text.contains("git push"))
            .expect("the command is painted");
        let rows: Vec<&str> = code.split('\n').collect();
        assert!(rows.len() >= 3, "{rows:?}");
        assert!(
            rows[1..].iter().all(|row| row.starts_with("  ") && !row.starts_with("   ")),
            "{rows:?}"
        );
        assert!(shown.text.width() <= 360.0 - 2.0 - 2.0 * f32::from(PAD) + 0.5);
        let squeezed = |text: &str| text.split_whitespace().collect::<String>();
        assert_eq!(squeezed(&code), squeezed(COMMAND));
    }

    #[test]
    fn the_language_tag_is_painted_when_the_fence_has_one() {
        let ctx = crate::fonts::test_context();
        let (with, _) = frame(&ctx, (400.0, Some("rust"), "fn main() {}"), vec![], 1.0);
        let (without, _) = frame(&ctx, (400.0, None, "fn main() {}"), vec![], 1.0);
        assert!(painted(&with).contains(&"rust".to_owned()));
        assert!(!painted(&without).contains(&"rust".to_owned()));
    }

    fn click(
        ctx: &egui::Context,
        block: (f32, Option<&str>, &str),
        at: Pos2,
        time: f64,
    ) -> Vec<String> {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut copied = Vec::new();
        for (step, events) in
            [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)]]
                .into_iter()
                .enumerate()
        {
            let (out, _) = frame(ctx, block, events, time + step as f64 * 0.01);
            copied.extend(out.platform_output.commands.into_iter().filter_map(|command| {
                match command {
                    OutputCommand::CopyText(text) => Some(text),
                    _ => None,
                }
            }));
        }
        copied
    }

    #[test]
    fn copy_puts_the_exact_text_on_the_clipboard_and_reads_copied_for_a_moment() {
        let text = "! git push --force-with-lease=x\n  indented\tTab\n";
        let block = (400.0, Some("sh"), text);
        let ctx = crate::fonts::test_context();
        let shown = settled(&ctx, block, 1.0);
        assert!(painted(&frame(&ctx, block, vec![], 1.0).0).contains(&"Copy".to_owned()));
        let copied = click(&ctx, block, shown.copy.center(), 1.0);
        assert_eq!(copied, [text]);
        let (out, _) = frame(&ctx, block, vec![], 1.2);
        assert!(painted(&out).contains(&"Copied".to_owned()));
        assert!(!painted(&out).contains(&"Copy".to_owned()));
        let (later, _) = frame(&ctx, block, vec![], 3.0);
        assert!(painted(&later).contains(&"Copy".to_owned()));
        assert!(!painted(&later).contains(&"Copied".to_owned()));
    }

    #[test]
    fn a_click_on_the_code_copies_nothing() {
        let block = (400.0, None, "cargo test");
        let ctx = crate::fonts::test_context();
        let shown = settled(&ctx, block, 1.0);
        assert_eq!(click(&ctx, block, shown.text.center(), 1.0).len(), 0);
    }

    #[test]
    fn the_button_keeps_its_width_between_copy_and_copied() {
        let block = (400.0, None, "x");
        let ctx = crate::fonts::test_context();
        let before = settled(&ctx, block, 1.0).copy;
        let _ = click(&ctx, block, before.center(), 1.0);
        let during = frame(&ctx, block, vec![], 1.2).1.copy;
        assert_eq!(before, during);
    }
}
