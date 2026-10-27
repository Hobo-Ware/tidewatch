//! Tidewatch as the third speaker of the Reader: a centred header (its mark, its name, the time and
//! the reason it spoke), a dashed outline card with the message and a quiet line under it that says
//! why it was sent. A long message folds to two lines until "Show all" is clicked.

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Painter, Pos2, Rect, Sense, Shape,
    Stroke, StrokeKind, TextStyle, Ui, pos2, vec2,
};
use jiff::tz::TimeZone;
use tidewatch_core::github_html;
use tidewatch_core::sent::{Reason, Record};

use crate::brand;
use crate::code_block::Look;
use crate::job_model::clock_label;
use crate::reader_markdown;
use crate::screen_kit::{Size, body, heading, quiet_button};
use crate::screen_kit::{TEXT_BODY, TEXT_SECONDARY, TEXT_TAG};

const CARD_SHARE: f32 = 0.7;
const CARD_RADIUS: f32 = 16.0;
const CARD_PAD: Margin = Margin { left: 18, right: 18, top: 14, bottom: 14 };
const BODY_PX: f32 = TEXT_BODY;
const CODE_EM: f32 = 0.88;
const FOLDED_ROWS: usize = 2;
const HEADER_H: f32 = 24.0;
const HEADER_GAP: f32 = 7.0;
const MARK_W: f32 = 18.0;
const MARK_H: f32 = 12.0;
const FOOTER_PX: f32 = 12.0;
const FOOTER_GAP: f32 = 8.0;
const OUTER: f32 = 10.0;
const DASH: f32 = 5.0;
const GAP: f32 = 4.0;
const ARC_STEPS: usize = 6;
const BODY_INK: Color32 = Color32::from_rgb(0xc9, 0xd1, 0xdc);
const DASH_INK: Color32 = Color32::from_rgb(0x2f, 0x5f, 0x59);
const MARK_INK: Color32 = brand::TEAL;
const NAME_INK: Color32 = brand::TEAL_TEXT;
const FOOTER_INK: Color32 = brand::FAINT;
const ORANGE_LINE: Color32 = Color32::from_rgb(0x4a, 0x34, 0x18);
const BLUE_LINE: Color32 = Color32::from_rgb(0x1f, 0x3f, 0x5c);
const GREEN_LINE: Color32 = Color32::from_rgb(0x2b, 0x44, 0x22);
const VIOLET_LINE: Color32 = Color32::from_rgb(0x3b, 0x33, 0x66);
const AMBER_LINE: Color32 = Color32::from_rgb(0x4a, 0x3b, 0x12);
const TEAL_LINE: Color32 = Color32::from_rgb(0x1f, 0x3a, 0x3a);

pub(crate) fn tag_colors(reason: Reason) -> (Color32, Color32) {
    match reason {
        Reason::AutoFix => (brand::ORANGE, ORANGE_LINE),
        Reason::LedgerGuard => (brand::BLUE_TEXT, BLUE_LINE),
        Reason::DoneCheck => (brand::GREEN, GREEN_LINE),
        Reason::MeeseeksCheck => (brand::VIOLET_TEXT, VIOLET_LINE),
        Reason::DueToday | Reason::StuckJob | Reason::BackgroundJob => (brand::AMBER, AMBER_LINE),
        Reason::CommentGuard => (brand::ORANGE_SOFT, ORANGE_LINE),
        Reason::AutoPush | Reason::PullRequest | Reason::Avatar => (brand::TEAL_TEXT, TEAL_LINE),
        Reason::Workflow => (brand::SOFT, brand::FIELD_LINE),
        Reason::Resume => (brand::MUTED, brand::ITEM_LINE),
    }
}

pub(crate) fn card_width(width: f32) -> f32 {
    (width * CARD_SHARE).round()
}

pub(crate) struct Shown<'a> {
    pub(crate) record: &'a Record,
    pub(crate) stamp: Option<i64>,
    pub(crate) zone: &'a TimeZone,
    pub(crate) open: bool,
}

pub(crate) fn show(ui: &mut Ui, shown: &Shown<'_>) -> bool {
    let column = ui.available_width();
    ui.add_space(OUTER);
    header(ui, column, shown);
    ui.add_space(HEADER_GAP - 1.0);
    let toggled = card(ui, column, shown);
    ui.add_space(FOOTER_GAP);
    footer(ui, column, &shown.record.footer());
    ui.add_space(OUTER);
    toggled
}

fn header(ui: &mut Ui, column: f32, shown: &Shown<'_>) {
    let painter = ui.painter().clone();
    let name = painter.layout_no_wrap("Tidewatch".to_owned(), heading(TEXT_BODY), NAME_INK);
    let at = shown
        .stamp
        .or(Some(shown.record.at_ms))
        .and_then(|at| clock_label(at, shown.zone))
        .map(|clock| {
            painter.layout_no_wrap(format!("\u{b7} {clock}"), body(TEXT_SECONDARY), brand::MUTED)
        });
    let reason = shown.record.reason;
    let (ink, line) = tag_colors(reason);
    let tag = painter.layout_no_wrap(reason.tag().to_owned(), body(TEXT_TAG), ink);
    let pill = tag.size() + vec2(16.0, 4.0);
    let gap = 7.0;
    let total = MARK_W
        + gap
        + name.size().x
        + at.as_ref().map_or(0.0, |at| gap + at.size().x)
        + gap
        + pill.x;
    let (rect, _) = ui.allocate_exact_size(vec2(column, HEADER_H), Sense::hover());
    let mut x = rect.center().x - total / 2.0;
    let mid = rect.center().y;
    paint_mark(&painter, pos2(x, mid - MARK_H / 2.0));
    x += MARK_W + gap;
    painter.galley(pos2(x, mid - name.size().y / 2.0), name.clone(), NAME_INK);
    x += name.size().x;
    if let Some(at) = at {
        x += gap;
        painter.galley(pos2(x, mid - at.size().y / 2.0), at.clone(), brand::MUTED);
        x += at.size().x;
    }
    x += gap;
    let pill_rect = Rect::from_min_size(pos2(x, mid - pill.y / 2.0), pill);
    painter.rect_stroke(
        pill_rect,
        CornerRadius::same((pill.y / 2.0).round() as u8),
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    painter.galley(pill_rect.center() - tag.size() / 2.0, tag, ink);
}

fn paint_mark(painter: &Painter, top_left: Pos2) {
    let at = |x: f32, y: f32| top_left + vec2(x, y);
    let stroke = Stroke::new(1.6, MARK_INK);
    painter.add(Shape::line(vec![at(1.0, 1.5), at(6.0, 6.0), at(1.0, 10.5)], stroke));
    let wave: Vec<Pos2> = (0..=10)
        .map(|n| {
            let t = n as f32 / 10.0;
            at(8.0 + 9.0 * t, 10.0 - 1.6 * (t * std::f32::consts::TAU * 1.5).sin())
        })
        .collect();
    painter.add(Shape::line(wave, Stroke::new(1.4, MARK_INK)));
}

fn card(ui: &mut Ui, column: f32, shown: &Shown<'_>) -> bool {
    let card_w = card_width(column);
    let mut toggled = false;
    ui.allocate_ui_with_layout(vec2(column, 0.0), Layout::top_down(Align::Center), |ui| {
        let framed = Frame::new().inner_margin(CARD_PAD).show(ui, |ui| {
            let inner = card_w - f32::from(CARD_PAD.left + CARD_PAD.right);
            ui.set_width(inner);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                toggled = message(ui, shown, inner);
            });
        });
        dashed_round_rect(
            ui.painter(),
            framed.response.rect,
            CARD_RADIUS,
            Stroke::new(1.0, DASH_INK),
        );
    });
    toggled
}

fn message(ui: &mut Ui, shown: &Shown<'_>, inner: f32) -> bool {
    let text = &shown.record.text;
    let folded = layout_folded(ui.painter(), text, inner);
    let long = folded.elided;
    if shown.open || !long {
        open_text(ui, text);
    } else {
        let (rect, _) = ui.allocate_exact_size(folded.size(), Sense::hover());
        ui.painter().galley(rect.min, folded, BODY_INK);
    }
    if !long {
        return false;
    }
    let label = if shown.open { "Show less" } else { "Show all" };
    quiet_button(ui, label, true, Size::Compact).clicked()
}

fn preview_text(text: &str) -> String {
    let scrubbed = github_html::scrub(text);
    let lines = scrubbed.lines().map(|line| {
        let line = line.trim_start().trim_start_matches(['>', ' ']);
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            &line[hashes..]
        } else {
            line
        }
    });
    lines.flat_map(str::split_whitespace).collect::<Vec<_>>().join(" ")
}

fn layout_folded(painter: &Painter, text: &str, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job =
        LayoutJob::single_section(preview_text(text), TextFormat::simple(body(BODY_PX), BODY_INK));
    job.wrap.max_width = width;
    job.wrap.max_rows = FOLDED_ROWS;
    job.wrap.overflow_character = Some('\u{2026}');
    painter.layout_job(job)
}

fn open_text(ui: &mut Ui, text: &str) {
    ui.scope(|ui| {
        let styles = &mut ui.style_mut().text_styles;
        styles.insert(TextStyle::Body, FontId::proportional(BODY_PX));
        styles.insert(TextStyle::Monospace, FontId::monospace(BODY_PX * CODE_EM));
        styles.insert(TextStyle::Small, FontId::proportional(13.0));
        let visuals = ui.visuals_mut();
        visuals.override_text_color = Some(BODY_INK);
        visuals.hyperlink_color = brand::TEAL_TEXT;
        visuals.code_bg_color = brand::ITEM;
        reader_markdown::show(ui, text, false, Look::AGENT);
    });
}

fn footer(ui: &mut Ui, column: f32, text: &str) {
    let galley =
        ui.painter().layout(text.to_owned(), body(FOOTER_PX), FOOTER_INK, card_width(column));
    let (rect, _) = ui.allocate_exact_size(vec2(column, galley.size().y), Sense::hover());
    let at = pos2(rect.center().x - galley.size().x / 2.0, rect.top());
    ui.painter().galley(at, galley, FOOTER_INK);
}

pub(crate) fn paused_line(ui: &mut Ui, column: f32) {
    ui.add_space(OUTER);
    footer(
        ui,
        column,
        "Tidewatch paused its automatic messages to this agent after three in a row. Your next message starts them again",
    );
    ui.add_space(OUTER);
}

/// A dashed outline with round corners. egui has no dashed stroke, so the outline is one closed
/// path cut into dashes.
fn dashed_round_rect(painter: &Painter, rect: Rect, radius: f32, stroke: Stroke) {
    let path = round_rect_path(rect, radius.min(rect.height() / 2.0).min(rect.width() / 2.0));
    painter.extend(Shape::dashed_line(&path, stroke, DASH, GAP));
}

fn round_rect_path(rect: Rect, radius: f32) -> Vec<Pos2> {
    let corners = [
        (pos2(rect.right() - radius, rect.top() + radius), -std::f32::consts::FRAC_PI_2),
        (pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (pos2(rect.left() + radius, rect.bottom() - radius), std::f32::consts::FRAC_PI_2),
        (pos2(rect.left() + radius, rect.top() + radius), std::f32::consts::PI),
    ];
    let mut path = Vec::with_capacity(4 * (ARC_STEPS + 1) + 1);
    for (center, start) in corners {
        for step in 0..=ARC_STEPS {
            let angle = start + std::f32::consts::FRAC_PI_2 * step as f32 / ARC_STEPS as f32;
            path.push(center + vec2(angle.cos(), angle.sin()) * radius);
        }
    }
    path.push(path[0]);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_drawn_tags_keep_their_inks_and_outlines() {
        assert_eq!(
            tag_colors(Reason::AutoFix),
            (Color32::from_rgb(0xfb, 0x92, 0x3c), Color32::from_rgb(0x4a, 0x34, 0x18))
        );
        assert_eq!(
            tag_colors(Reason::LedgerGuard),
            (Color32::from_rgb(0x93, 0xc5, 0xfd), Color32::from_rgb(0x1f, 0x3f, 0x5c))
        );
    }

    fn folded(text: &str, width: f32) -> (usize, bool) {
        let ctx = crate::fonts::test_context();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            ..egui::RawInput::default()
        };
        let mut shape = (0, false);
        let _ = ctx.run_ui(raw, |ui| {
            let galley = layout_folded(ui.painter(), text, width);
            shape = (galley.rows.len(), galley.elided);
        });
        shape
    }

    #[test]
    fn the_folded_preview_drops_quote_and_heading_markers_and_html_comments() {
        let text = [
            "@codex commented on PR 7.",
            "",
            "> <!-- codex-review-state -->",
            "> | a | b |",
            ">> deeper",
            "## Heading here",
            "#hashtag stays",
        ]
        .join("\n");
        assert_eq!(
            preview_text(&text),
            "@codex commented on PR 7. | a | b | deeper Heading here #hashtag stays"
        );
    }

    #[test]
    fn a_long_message_folds_to_two_rows_and_a_short_one_does_not_fold() {
        assert_eq!(folded(&"Fix the failing check. ".repeat(40), 400.0), (2, true));
        assert_eq!(folded("Fix the build.\n\nThen the docs.", 400.0), (1, false));
    }

    #[test]
    fn the_card_takes_seventy_percent_of_the_column() {
        assert_eq!(card_width(1000.0), 700.0);
        assert_eq!(card_width(641.0), 449.0);
    }

    #[test]
    fn the_outline_closes_and_stays_inside_the_rect() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 80.0));
        let path = round_rect_path(rect, 16.0);
        assert_eq!(path.first(), path.last());
        for point in &path {
            assert!(rect.expand(0.01).contains(*point), "{point:?}");
        }
    }
}
