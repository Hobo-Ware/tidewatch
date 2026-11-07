//! Tidewatch's own marks in the Reader: the badge on the owner's face for a message Tidewatch typed
//! for them, the buttons under that message, the quiet line a run of reminders folds into and the
//! line after the cap.

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, Layout, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    Ui, pos2, vec2,
};
use jiff::tz::TimeZone;
use tidewatch_core::github_html;
use tidewatch_core::sent::{Record, Switch};

use crate::brand;
use crate::job_model::clock_label;
use crate::quiet_notes;
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, body, button_width, quiet_button};
use crate::screen_kit::{TEXT_SECONDARY, TEXT_TAG};

const FOOTER_SHARE: f32 = 0.7;
const MARK_W: f32 = 18.0;
const MARK_H: f32 = 12.0;
const FOOTER_PX: f32 = 12.0;
const FOOTER_GAP: f32 = 8.0;
const ACTION_GAP: f32 = 8.0;
const OUTER: f32 = 10.0;
const MARK_INK: Color32 = brand::TEAL;
const FOOTER_INK: Color32 = brand::FAINT;
const BADGE_D: f32 = 16.0;
const BADGE_RING: f32 = 1.5;
const BADGE_MARK: f32 = 0.55;

fn footer_width(width: f32) -> f32 {
    (width * FOOTER_SHARE).round()
}

/// What a click on a Tidewatch row asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    ChangeWorkflow,
    TurnOff(Switch),
}

/// What a frame of a row drew: whether its fold was clicked, and the button pressed under it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Clicked {
    pub(crate) toggled: bool,
    pub(crate) click: Option<Click>,
}

/// The buttons under a row: the profile its words came from, and the setting that stops its kind.
pub(crate) fn buttons_of(record: &Record) -> Vec<(&'static str, Click)> {
    let mut buttons = Vec::new();
    if record.reason.uses_workflow() {
        buttons.push(("Change workflow", Click::ChangeWorkflow));
    }
    if let Some(switch) = record.switch() {
        buttons.push((switch.label(), Click::TurnOff(switch)));
    }
    buttons
}

/// The buttons of `buttons_of` in a row ending `right` pixels from the column's left edge, where
/// the bubble ends.
pub(crate) fn actions(ui: &mut Ui, right: f32, record: &Record) -> Option<Click> {
    let buttons = buttons_of(record);
    if buttons.is_empty() {
        return None;
    }
    ui.add_space(FOOTER_GAP);
    let widths: Vec<f32> =
        buttons.iter().map(|(label, _)| button_width(ui, label, Size::Compact)).collect();
    let total = widths.iter().sum::<f32>() + ACTION_GAP * (buttons.len() - 1) as f32;
    let mut pressed = None;
    ui.allocate_ui_with_layout(
        vec2(right, Size::Compact.height()),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = ACTION_GAP;
            ui.add_space((right - total).max(0.0));
            for (label, click) in &buttons {
                if quiet_button(ui, label, true, Size::Compact).clicked() {
                    pressed = Some(*click);
                }
            }
        },
    );
    ui.add_space(OUTER);
    pressed
}

/// Where Tidewatch's badge sits on a face: over its lower right corner.
pub(crate) fn badge_rect(face: Rect) -> Rect {
    Rect::from_center_size(
        face.right_bottom() - vec2(BADGE_D / 2.0, BADGE_D / 2.0),
        vec2(BADGE_D, BADGE_D),
    )
}

/// Tidewatch's mark in a small dark disc, ringed in the panel's colour so it reads on a face.
pub(crate) fn paint_badge(painter: &Painter, face: Rect) {
    let disc = badge_rect(face);
    let centre = disc.center();
    painter.circle_filled(centre, BADGE_D / 2.0 + BADGE_RING, brand::GROUND);
    painter.circle_filled(centre, BADGE_D / 2.0, brand::TEAL_NAV);
    let size = vec2(MARK_W, MARK_H) * BADGE_MARK;
    paint_mark(painter, centre - size / 2.0, BADGE_MARK);
}

fn paint_mark(painter: &Painter, top_left: Pos2, scale: f32) {
    let at = |x: f32, y: f32| top_left + vec2(x, y) * scale;
    let stroke = |width: f32| Stroke::new((width * scale).max(1.0), MARK_INK);
    painter.add(Shape::line(vec![at(1.0, 1.5), at(6.0, 6.0), at(1.0, 10.5)], stroke(1.6)));
    let wave: Vec<Pos2> = (0..=10)
        .map(|n| {
            let t = n as f32 / 10.0;
            at(8.0 + 9.0 * t, 10.0 - 1.6 * (t * std::f32::consts::TAU * 1.5).sin())
        })
        .collect();
    painter.add(Shape::line(wave, stroke(1.4)));
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

fn footer(ui: &mut Ui, column: f32, text: &str) {
    let galley =
        ui.painter().layout(text.to_owned(), body(FOOTER_PX), FOOTER_INK, footer_width(column));
    let (rect, _) = ui.allocate_exact_size(vec2(column, galley.size().y), Sense::hover());
    let at = pos2(rect.center().x - galley.size().x / 2.0, rect.top());
    ui.painter().galley(at, galley, FOOTER_INK);
}

const QUIET_H: f32 = 24.0;
pub(crate) const QUIET_GUESS_H: f32 = QUIET_H + 4.0;
const QUIET_PX: f32 = TEXT_SECONDARY;
const QUIET_GAP: f32 = 8.0;
const QUIET_LIST_INDENT: f32 = 10.0;
const QUIET_LIST_PAD: f32 = 12.0;
const QUIET_ROWS: usize = 3;
const QUIET_CHEVRON_W: f32 = 8.0;
const QUIET_ENTRY_GAP: f32 = 6.0;

pub(crate) struct Quiet<'a> {
    pub(crate) run: &'a [&'a Record],
    pub(crate) zone: &'a TimeZone,
    pub(crate) open: bool,
}

/// A run of Tidewatch's reminders as one muted line: the mark, what was asked, a count when there
/// are several, a chevron and the time. A click opens the list under it and the next one folds it.
pub(crate) fn quiet(ui: &mut Ui, shown: &Quiet<'_>) -> Clicked {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), QUIET_H), Sense::click());
    let painter = ui.painter_at(rect);
    let mid = rect.center().y;
    let ink = if response.hovered() || shown.open { brand::SOFT } else { brand::MUTED };
    paint_mark(&painter, pos2(rect.left() + 2.0, mid - MARK_H / 2.0), 1.0);
    let mut x = rect.left() + 2.0 + MARK_W + QUIET_GAP;
    let at = quiet_notes::span(shown.run, shown.zone)
        .map(|span| painter.layout_no_wrap(span, body(TEXT_TAG), brand::MUTED));
    let right = rect.right() - at.as_ref().map_or(0.0, |at| at.size().x);
    if let Some(at) = at {
        painter.galley(pos2(right, mid - at.size().y / 2.0), at, brand::MUTED);
    }
    let tag = quiet_notes::count(shown.run)
        .map(|number| painter.layout_no_wrap(number, body(TEXT_TAG), brand::MUTED));
    let pill_w = tag.as_ref().map_or(0.0, |tag| tag.size().x + 14.0 + QUIET_GAP);
    let room = right - QUIET_GAP - x - pill_w - QUIET_CHEVRON_W - QUIET_GAP;
    let sentence =
        ellipsized(&painter, quiet_notes::words(shown.run), body(QUIET_PX), ink, room.max(0.0));
    painter.galley(pos2(x, mid - sentence.size().y / 2.0), sentence.clone(), ink);
    x += sentence.size().x + QUIET_GAP;
    if let Some(tag) = tag {
        let pill = Rect::from_min_size(
            pos2(x, mid - tag.size().y / 2.0 - 1.0),
            tag.size() + vec2(14.0, 2.0),
        );
        painter.rect_stroke(
            pill,
            CornerRadius::same((pill.height() / 2.0).round() as u8),
            Stroke::new(1.0, brand::ITEM_LINE),
            StrokeKind::Inside,
        );
        painter.galley(pill.center() - tag.size() / 2.0, tag, brand::MUTED);
        x = pill.right() + QUIET_GAP;
    }
    let tip = if shown.open { -3.0 } else { 3.0 };
    let (centre, stroke) = (pos2(x + 4.0, mid), Stroke::new(1.3, ink));
    if shown.open {
        painter
            .line_segment([centre + vec2(-4.0, -tip / 2.0), centre + vec2(0.0, tip / 2.0)], stroke);
        painter
            .line_segment([centre + vec2(0.0, tip / 2.0), centre + vec2(4.0, -tip / 2.0)], stroke);
    } else {
        painter
            .line_segment([centre + vec2(-tip / 2.0, -4.0), centre + vec2(tip / 2.0, 0.0)], stroke);
        painter
            .line_segment([centre + vec2(tip / 2.0, 0.0), centre + vec2(-tip / 2.0, 4.0)], stroke);
    }
    let click = if shown.open { quiet_list(ui, shown) } else { None };
    let toggled = response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    Clicked { toggled, click }
}

fn quiet_list(ui: &mut Ui, shown: &Quiet<'_>) -> Option<Click> {
    let left = ui.max_rect().left() + QUIET_LIST_INDENT;
    let width = (ui.available_width() - QUIET_LIST_INDENT - QUIET_LIST_PAD).max(40.0);
    let mut top = None;
    let mut bottom = 0.0_f32;
    ui.add_space(2.0);
    for record in shown.run {
        let clock = clock_label(record.at_ms, shown.zone).unwrap_or_default();
        let mut job = LayoutJob::default();
        job.append(
            &format!("{clock} \u{b7} {}", preview_text(&record.text)),
            0.0,
            TextFormat::simple(body(QUIET_PX), brand::SOFT),
        );
        job.append(
            &format!("   {}", record.because),
            0.0,
            TextFormat::simple(body(TEXT_TAG), brand::MUTED),
        );
        job.wrap.max_width = width;
        job.wrap.max_rows = QUIET_ROWS;
        job.wrap.overflow_character = Some('\u{2026}');
        let galley = ui.painter().layout_job(job);
        let (rect, _) = ui.allocate_exact_size(
            vec2(QUIET_LIST_INDENT + QUIET_LIST_PAD + width, galley.size().y),
            Sense::hover(),
        );
        let at = pos2(left + QUIET_LIST_PAD, rect.top());
        top.get_or_insert(rect.top());
        bottom = rect.bottom();
        ui.painter().galley(at, galley, brand::SOFT);
        ui.add_space(QUIET_ENTRY_GAP);
    }
    if let Some(top) = top {
        ui.painter().line_segment(
            [pos2(left, top), pos2(left, bottom)],
            Stroke::new(1.0, brand::ITEM_LINE),
        );
    }
    let click = shown.run.first().and_then(|record| record.switch()).and_then(|switch| {
        ui.horizontal(|ui| {
            ui.add_space(QUIET_LIST_INDENT + QUIET_LIST_PAD);
            quiet_button(ui, switch.label(), true, Size::Compact).clicked()
        })
        .inner
        .then_some(Click::TurnOff(switch))
    });
    ui.add_space(2.0);
    click
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::painted_text_rects_nonblank;
    use tidewatch_core::sent::Reason;

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

    struct Line {
        ctx: egui::Context,
        run: Vec<Record>,
        open: bool,
        width: f32,
        texts: Vec<(String, Rect)>,
    }

    impl Line {
        fn new(run: Vec<Record>, width: f32) -> Self {
            let mut line = Self {
                ctx: crate::fonts::test_context(),
                run,
                open: false,
                width,
                texts: Vec::new(),
            };
            line.frame(Vec::new());
            line.frame(Vec::new());
            line
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> bool {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, 400.0))),
                events,
                focused: true,
                ..egui::RawInput::default()
            };
            let zone = TimeZone::UTC;
            let refs: Vec<&Record> = self.run.iter().collect();
            let open = self.open;
            let mut clicked = false;
            let output = self.ctx.run_ui(raw, |ui| {
                clicked = quiet(ui, &Quiet { run: &refs, zone: &zone, open }).toggled;
            });
            self.texts.clear();
            for shape in &output.shapes {
                painted_text_rects_nonblank(&shape.shape, &mut self.texts);
            }
            if clicked {
                self.open = !self.open;
            }
            clicked
        }

        fn click_on(&mut self, text: &str) {
            let at = self.rect_of(text).center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(Vec::new());
        }

        fn rect_of(&self, text: &str) -> Rect {
            self.texts
                .iter()
                .find(|(drawn, _)| drawn.starts_with(text))
                .unwrap_or_else(|| panic!("{text:?} in {:?}", self.texts))
                .1
        }

        fn has(&self, text: &str) -> bool {
            self.texts.iter().any(|(drawn, _)| drawn.starts_with(text))
        }
    }

    fn reminders(count: i64) -> Vec<Record> {
        (0..count)
            .map(|n| Record {
                at_ms: (22 * 60 + 22 + n * 6) * 60_000,
                ..Record::new(
                    Reason::LedgerGuard,
                    "you answered and the progress report should show it",
                    "The user answered; update your report before your final reply.",
                    None,
                )
            })
            .collect()
    }

    const SENTENCE: &str = "Tidewatch reminded the agent to update its report";

    #[test]
    fn a_run_of_reminders_is_one_line_with_a_count_and_the_span_at_the_right() {
        let line = Line::new(reminders(4), 700.0);
        assert!(line.has(SENTENCE) && line.has("4") && line.has("22:22 to 22:40"));
        assert!(!line.has("The user answered"), "folded: {:?}", line.texts);
        let (words, count, span) =
            (line.rect_of(SENTENCE), line.rect_of("4"), line.rect_of("22:22 to"));
        assert!(words.right() < count.left() && count.right() < span.left());
        assert!(span.right() <= 700.0);
        let one = Line::new(reminders(1), 700.0);
        assert!(one.has("22:22") && !one.has("22:22 to") && !one.has("1"), "{:?}", one.texts);
    }

    #[test]
    fn a_click_opens_the_list_of_what_was_sent_and_the_next_click_folds_it() {
        let mut line = Line::new(reminders(3), 700.0);
        line.click_on(SENTENCE);
        assert!(line.open);
        let sent = "22:22 \u{b7} The user answered";
        assert!(line.has(sent), "{:?}", line.texts);
        let rows: Vec<&Rect> = line
            .texts
            .iter()
            .filter(|(text, _)| text.contains("The user answered"))
            .map(|(_, r)| r)
            .collect();
        assert_eq!(rows.len(), 3);
        assert!(rows.windows(2).all(|pair| pair[0].bottom() <= pair[1].top()));
        let because =
            line.texts.iter().find(|(text, _)| text.contains("you answered and")).unwrap();
        assert!(because.0.starts_with("22:22"), "the reason sits with its message");
        line.click_on(SENTENCE);
        assert!(!line.open && !line.has(sent));
    }

    #[test]
    fn the_line_stays_inside_the_column_from_a_narrow_width_up() {
        for width in [320.0, 480.0, 900.0] {
            let mut line = Line::new(reminders(4), width);
            line.click_on(SENTENCE);
            for (text, rect) in &line.texts {
                assert!(
                    rect.left() >= 0.0 && rect.right() <= width + 0.5,
                    "{width}: {text:?} {rect:?}"
                );
            }
        }
    }

    fn record_of(reason: Reason, because: &str, hand: bool) -> Record {
        Record { hand, at_ms: 60_000, ..Record::new(reason, because, "Do the thing.", None) }
    }

    #[test]
    fn the_buttons_under_a_row_are_its_workflow_for_a_pr_ask_and_the_switch_of_its_kind() {
        use tidewatch_core::prefs::AutoJob;
        use tidewatch_core::sent::because;
        let table = [
            (
                record_of(Reason::AutoFix, because::AUTO_CONFLICTS, false),
                vec![("Turn off for conflicts", Click::TurnOff(Switch::Auto(AutoJob::Conflicts)))],
            ),
            (
                record_of(Reason::PullRequest, because::ASK_PR_CLICK, true),
                vec![("Change workflow", Click::ChangeWorkflow)],
            ),
            (
                record_of(Reason::CommentGuard, because::COMMENTS, false),
                vec![("Turn off the comment guard for this tab", Click::TurnOff(Switch::Comments))],
            ),
            (record_of(Reason::LedgerGuard, because::LEDGER_STOP, false), Vec::new()),
            (record_of(Reason::MeeseeksCheck, because::MEESEEKS, false), Vec::new()),
        ];
        for (record, want) in table {
            assert_eq!(buttons_of(&record), want, "{:?}", record.reason);
        }
    }

    #[test]
    fn the_opened_list_of_a_reminder_run_offers_the_switch_of_its_kind() {
        use tidewatch_core::sent::because;
        let mut line =
            Line::new(vec![record_of(Reason::CommentGuard, because::COMMENTS, false)], 700.0);
        line.click_on("Tidewatch told the agent about new branch comments");
        assert!(line.has("Turn off the comment guard for this tab"), "{:?}", line.texts);
        let closed =
            Line::new(vec![record_of(Reason::LedgerGuard, because::LEDGER_STOP, false)], 700.0);
        assert!(!closed.has("Turn off"));
    }

    #[test]
    fn a_footer_wraps_at_seventy_percent_of_the_column() {
        assert_eq!(footer_width(1000.0), 700.0);
        assert_eq!(footer_width(641.0), 449.0);
    }
}
