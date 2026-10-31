//! The Progress panel's blocks under the title: Waiting on you, Now, Waiting for, the steps by
//! stage, Coming up and the Decisions, Not verified and Verified lists.

use egui::epaint::text::TextFormat;
use egui::{Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tidewatch_core::panel_state::{CheckSeen, MarkSeen};
use tidewatch_core::progress::{Entry, Mark};

use crate::brand;
use crate::link_text::Text;
use crate::panel_blocks::{self, Blocks, Proof, QUIET_PREFIX};
use crate::progress_model::{Asked, Coming, ItemRow, JobNow, Now, StageRows, Tag};
use crate::progress_ring::{paint_mark, tone_color};
use crate::reader_view::ellipsized;
use crate::screen_kit::{TEXT_BODY, TEXT_SECONDARY, body, heading};

pub(crate) const MARK_W: f32 = 18.0;
const TEXT_X: f32 = MARK_W + 12.0;
const MIN_TEXT_W: f32 = 60.0;
const ROW_GAP: f32 = 10.0;
const CARD_PAD: f32 = 12.0;
const LABEL_H: f32 = 22.0;
const BAR_HEIGHT: f32 = 6.0;
const BAR_SPACE: f32 = 6.0;
const NAME_H: f32 = 20.0;
const COMING_WHEN_W: f32 = 18.0;
pub(crate) const LIST_LIMIT: usize = 4;

/// How a block draws: its column, whether labels stack over values, and whether it holds a report
/// from the last turn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Column {
    pub(crate) width: f32,
    pub(crate) stack: bool,
    pub(crate) dim: bool,
}

impl Column {
    fn ink(self, ink: Color32) -> Color32 {
        if self.dim { ink.gamma_multiply(0.6) } else { ink }
    }
}

fn format(font: FontId, ink: Color32) -> TextFormat {
    TextFormat::simple(font, ink)
}

/// A wrapped, selectable paragraph in `ink`, `indent` in from the column's left.
fn paragraph(
    ui: &mut Ui,
    column: Column,
    indent: f32,
    runs: &[(&str, FontId, Color32)],
    salt: &str,
) -> Rect {
    let painter = ui.painter().clone();
    let mut text = Text::new();
    for (run, font, ink) in runs {
        text.run(run, &format(font.clone(), *ink));
    }
    let room = (column.width - indent).max(MIN_TEXT_W);
    let laid = text.wrapped(room, usize::MAX).lay_out(&painter);
    let (rect, _) = ui.allocate_exact_size(vec2(column.width, laid.size().y), Sense::hover());
    let fallback = runs.first().map_or(brand::TEXT, |run| run.2);
    laid.paint_selectable(ui, ui.id().with(salt), pos2(rect.left() + indent, rect.top()), fallback);
    rect
}

/// The label of a section: 13 px semibold, with its count when it has one.
pub(crate) fn section_label(ui: &mut Ui, column: Column, text: &str, count: Option<usize>) {
    let painter = ui.painter().clone();
    let ink = column.ink(brand::SOFT);
    let galley = painter.layout_no_wrap(text.to_owned(), heading(TEXT_SECONDARY), ink);
    let (rect, _) = ui.allocate_exact_size(vec2(column.width, LABEL_H), Sense::hover());
    let at = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    let right = at.x + galley.size().x;
    painter.galley(at, galley, ink);
    if let Some(count) = count {
        let number = painter.layout_no_wrap(count.to_string(), body(TEXT_SECONDARY), brand::MUTED);
        painter.galley(
            pos2(right + 8.0, rect.center().y - number.size().y / 2.0),
            number,
            brand::MUTED,
        );
    }
}

/// Waiting on you: the question, and where to answer it.
pub(crate) fn asked_block(ui: &mut Ui, column: Column, asked: &Asked) {
    let painter = ui.painter().clone();
    let inner = (column.width - 2.0 * CARD_PAD).max(MIN_TEXT_W);
    let label =
        painter.layout_no_wrap("Waiting on you".to_owned(), heading(TEXT_SECONDARY), brand::NEEDS);
    let mut question = Text::new();
    question.run(&asked.text, &format(body(TEXT_BODY), brand::TEXT));
    let question = question.wrapped(inner, 4).lay_out(&painter);
    let hint = match asked.more {
        0 => "Answer above the reply box".to_owned(),
        1 => "Answer above the reply box \u{b7} 1 more question".to_owned(),
        more => format!("Answer above the reply box \u{b7} {more} more questions"),
    };
    let mut note = Text::new();
    note.run(&hint, &format(body(TEXT_SECONDARY), brand::MUTED));
    let note = note.wrapped(inner, 2).lay_out(&painter);
    let height = 2.0 * CARD_PAD + label.size().y + 6.0 + question.size().y + 6.0 + note.size().y;
    let (rect, _) = ui.allocate_exact_size(vec2(column.width, height), Sense::hover());
    painter.rect(
        rect,
        CornerRadius::same(12),
        brand::NEEDS_FILL,
        Stroke::new(1.0, brand::NEEDS.gamma_multiply(0.45)),
        StrokeKind::Inside,
    );
    let (left, mut y) = (rect.left() + CARD_PAD, rect.top() + CARD_PAD);
    painter.circle_filled(pos2(left + 3.0, y + label.size().y / 2.0), 3.0, brand::NEEDS);
    let label_h = label.size().y;
    painter.galley(pos2(left + 12.0, y), label, brand::NEEDS);
    y += label_h + 6.0;
    let question_h = question.size().y;
    question.paint_selectable(ui, ui.id().with("asked-question"), pos2(left, y), brand::TEXT);
    y += question_h + 6.0;
    note.paint_selectable(ui, ui.id().with("asked-note"), pos2(left, y), brand::MUTED);
}

/// The parts of the Now block's second line: the steps done, how old the report is, what happened
/// since. A turn gone quiet says so in place of the report's age.
pub(crate) fn now_parts(now: &Now, blocks: &Blocks) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some((done, total)) = now.steps {
        let noun = if total == 1 { "step" } else { "steps" };
        parts.push(format!("{done} of {total} {noun}"));
    }
    match (blocks.card_age(), blocks.report.as_deref()) {
        (Some(quiet), _) if quiet.starts_with(QUIET_PREFIX) => parts.push(quiet.to_owned()),
        (_, Some(age)) => parts.push(format!("updated {age}")),
        _ => {}
    }
    if let Some(since) = &blocks.since {
        parts.push(since.clone());
    }
    parts
}

pub(crate) fn now_block(ui: &mut Ui, column: Column, now: &Now, blocks: &Blocks) {
    let painter = ui.painter().clone();
    let ink = tone_color(now.tone);
    let word = painter.layout_no_wrap(now.tone.word().to_owned(), heading(TEXT_SECONDARY), ink);
    let (rect, _) = ui.allocate_exact_size(vec2(column.width, LABEL_H), Sense::hover());
    painter.circle_filled(pos2(rect.left() + 3.0, rect.center().y), 3.0, ink);
    painter.galley(pos2(rect.left() + 12.0, rect.center().y - word.size().y / 2.0), word, ink);
    if !now.text.is_empty() {
        paragraph(
            ui,
            column,
            0.0,
            &[(&now.text, body(TEXT_BODY), column.ink(brand::TEXT))],
            "now-text",
        );
    }
    let parts = now_parts(now, blocks);
    if !parts.is_empty() {
        ui.add_space(2.0);
        paragraph(
            ui,
            column,
            0.0,
            &[(&parts.join(" \u{b7} "), body(TEXT_SECONDARY), brand::MUTED)],
            "now-meta",
        );
    }
    if let Some(job) = &now.job {
        ui.add_space(8.0);
        job_line(ui, column, job);
    }
}

fn job_line(ui: &mut Ui, column: Column, job: &JobNow) {
    let painter = ui.painter().clone();
    let right = job
        .right
        .as_ref()
        .map(|text| painter.layout_no_wrap(text.clone(), body(TEXT_SECONDARY), brand::MUTED));
    let right_w = right.as_ref().map_or(0.0, |galley| galley.size().x + 12.0);
    let stacked = right.is_some() && (column.stack || column.width - right_w < 96.0);
    let name_room = if stacked { column.width } else { column.width - right_w };
    let name =
        ellipsized(&painter, &job.name, body(TEXT_SECONDARY), brand::SOFT, name_room.max(40.0));
    let rows = if stacked { 2.0 } else { 1.0 };
    let height = NAME_H * rows + BAR_SPACE + BAR_HEIGHT;
    let (rect, _) = ui.allocate_exact_size(vec2(column.width, height), Sense::hover());
    painter.galley(
        pos2(rect.left(), rect.top() + (NAME_H - name.size().y) / 2.0),
        name,
        brand::SOFT,
    );
    if let Some(right) = right {
        let (x, y) = if stacked {
            (rect.left(), rect.top() + NAME_H)
        } else {
            (rect.right() - right.size().x, rect.top())
        };
        painter.galley(pos2(x, y + (NAME_H - right.size().y) / 2.0), right, brand::MUTED);
    }
    let track = Rect::from_min_size(
        pos2(rect.left(), rect.top() + NAME_H * rows + BAR_SPACE),
        vec2(rect.width(), BAR_HEIGHT),
    );
    let radius = CornerRadius::same(3);
    painter.rect_filled(track, radius, brand::ITEM_LINE);
    if job.fraction > 0.0 {
        let fill = Rect::from_min_size(track.min, vec2(track.width() * job.fraction, BAR_HEIGHT));
        painter.rect_filled(fill, radius, brand::TEAL);
    }
}

/// Waiting for: the job, pull request, date or event, on its own line.
pub(crate) fn waiting_block(ui: &mut Ui, column: Column, what: &str) {
    let painter = ui.painter().clone();
    let label =
        painter.layout_no_wrap("Waiting for".to_owned(), body(TEXT_SECONDARY), brand::MUTED);
    if column.stack {
        let (rect, _) =
            ui.allocate_exact_size(vec2(column.width, label.size().y + 2.0), Sense::hover());
        painter.galley(rect.left_top(), label, brand::MUTED);
        paragraph(ui, column, 0.0, &[(what, body(TEXT_BODY), brand::TEXT)], "waiting-for");
        return;
    }
    let indent = label.size().x + 8.0;
    let mut text = Text::new();
    text.run(what, &format(body(TEXT_BODY), brand::TEXT));
    let laid = text.wrapped((column.width - indent).max(MIN_TEXT_W), usize::MAX).lay_out(&painter);
    let height = laid.size().y.max(label.size().y);
    let (rect, _) = ui.allocate_exact_size(vec2(column.width, height), Sense::hover());
    painter.galley(
        pos2(rect.left(), rect.top() + (height - label.size().y) / 2.0),
        label,
        brand::MUTED,
    );
    laid.paint_selectable(
        ui,
        ui.id().with("waiting-for"),
        pos2(rect.left() + indent, rect.top()),
        brand::TEXT,
    );
}

/// What a step row shows under its text: the proof word and the evidence of a finished step, the
/// reason of a blocked one (amber), the note of any other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Under {
    pub(crate) proof: Option<Proof>,
    pub(crate) text: String,
    pub(crate) blocked: bool,
}

pub(crate) fn under(
    row: &ItemRow<'_>,
    mark: Mark,
    by_tidewatch: bool,
    checks: &[CheckSeen],
) -> Option<Under> {
    let text = panel_blocks::why_with_tick(row.why, by_tidewatch);
    let proof =
        (mark == Mark::Done).then(|| panel_blocks::proof(by_tidewatch, row.why, checks)).flatten();
    match (&text, proof) {
        (None, None) => None,
        (text, proof) => Some(Under {
            proof,
            text: text.clone().unwrap_or_default(),
            blocked: mark == Mark::Blocked,
        }),
    }
}

pub(crate) struct Steps<'a> {
    pub(crate) stages: &'a [&'a StageRows<'a>],
    pub(crate) marks: &'a [MarkSeen],
    pub(crate) checks: &'a [CheckSeen],
    pub(crate) time: f64,
}

/// The steps by stage. Returns whether any step is under way, so the caller asks for frames.
pub(crate) fn steps_block(ui: &mut Ui, column: Column, steps: &Steps<'_>) -> bool {
    let mut spinning = false;
    for (index, stage) in steps.stages.iter().enumerate() {
        if index > 0 {
            ui.add_space(12.0);
        }
        section_label(ui, column, &stage.title, None);
        for row in &stage.rows {
            let ticked = panel_blocks::ticked_by_tidewatch(steps.marks, row.text);
            let (mark, by_tidewatch) = panel_blocks::tick(row.mark, ticked);
            spinning |= mark == Mark::Doing;
            let below = under(row, mark, by_tidewatch, steps.checks);
            step_row(ui, column, (row.text, mark, below.as_ref()), steps.time);
        }
    }
    spinning
}

fn step_row(
    ui: &mut Ui,
    column: Column,
    (text, mark, below): (&str, Mark, Option<&Under>),
    time: f64,
) {
    let painter = ui.painter().clone();
    let muted = matches!(mark, Mark::Skipped);
    let ink = column.ink(if muted { brand::FAINT } else { brand::TEXT });
    let room = (column.width - TEXT_X).max(MIN_TEXT_W);
    let mut label = Text::new();
    label.run(text, &format(body(TEXT_BODY), ink));
    let label = label.wrapped(room, usize::MAX).lay_out(&painter);
    let note = below.map(|below| {
        let mut note = Text::new();
        if let Some(proof) = below.proof {
            let proof_ink = match proof {
                Proof::Checked => column.ink(brand::DONE),
                Proof::Claimed => column.ink(brand::MUTED),
            };
            note.run(proof.word(), &format(body(TEXT_SECONDARY), proof_ink));
            if !below.text.is_empty() {
                note.run(" \u{b7} ", &format(body(TEXT_SECONDARY), column.ink(brand::FAINT)));
            }
        }
        let note_ink = if below.blocked { brand::NEEDS } else { brand::FAINT };
        note.run(&below.text, &format(body(TEXT_SECONDARY), column.ink(note_ink)));
        note.wrapped(room, usize::MAX).lay_out(&painter)
    });
    let note_h = note.as_ref().map_or(0.0, |note| note.size().y + 2.0);
    let height = label.size().y.max(20.0) + note_h + ROW_GAP;
    let (rect, response) = ui.allocate_exact_size(vec2(column.width, height), Sense::hover());
    paint_mark(&painter, pos2(rect.left() + MARK_W / 2.0, rect.top() + 10.0), 14.0, mark, time);
    let id = response.id;
    let label_h = label.size().y.max(20.0);
    label.paint_selectable(
        ui,
        id.with("label"),
        pos2(rect.left() + TEXT_X, rect.top() + (label_h - label.size().y) / 2.0),
        ink,
    );
    if let Some(note) = note {
        let at = pos2(rect.left() + TEXT_X, rect.top() + label_h + 2.0);
        note.paint_selectable(ui, id.with("note"), at, brand::FAINT);
    }
}

/// Coming up: the report's scheduled items with when they are due.
pub(crate) fn coming_block(ui: &mut Ui, column: Column, rows: &[Coming<'_>]) {
    section_label(ui, column, "Coming up", Some(rows.len()));
    for (index, row) in rows.iter().enumerate() {
        let when = format!("{}  ", row.when);
        let mut runs: Vec<(&str, FontId, Color32)> = vec![
            (&when, body(TEXT_SECONDARY), column.ink(brand::TEAL_TEXT)),
            (row.text, body(TEXT_BODY), column.ink(brand::TEXT)),
        ];
        if let Some(why) = row.why {
            runs.push(("  ", body(TEXT_SECONDARY), brand::FAINT));
            runs.push((why, body(TEXT_SECONDARY), column.ink(brand::FAINT)));
        }
        paragraph(ui, column, COMING_WHEN_W * 0.0, &runs, &format!("coming-{index}"));
        ui.add_space(ROW_GAP - 2.0);
    }
}

/// What a list of entries draws: the first few, or all of them after a click.
pub(crate) fn shown_count(len: usize, all: bool) -> usize {
    if all { len } else { len.min(LIST_LIMIT) }
}

/// A fold's wording: `Show 3 more`, or `Show less` once open.
pub(crate) fn fold_label(len: usize, all: bool) -> Option<String> {
    if len <= LIST_LIMIT {
        return None;
    }
    Some(if all { "Show less".to_owned() } else { format!("Show {} more", len - LIST_LIMIT) })
}

/// One list: Decisions, Not verified or Verified. Returns whether its fold was clicked.
pub(crate) fn list_block(
    ui: &mut Ui,
    column: Column,
    (tag, list): (Tag, &[&Entry]),
    all: bool,
) -> bool {
    section_label(ui, column, tag.caption(), Some(list.len()));
    let painter = ui.painter().clone();
    for (index, entry) in list.iter().take(shown_count(list.len(), all)).enumerate() {
        let mut text = Text::new();
        text.run(entry.text.trim(), &format(body(TEXT_BODY), column.ink(brand::SOFT)));
        if let Some(why) = entry.why.as_deref().map(str::trim).filter(|why| !why.is_empty()) {
            text.run(&format!("  {why}"), &format(body(TEXT_SECONDARY), column.ink(brand::FAINT)));
        }
        let laid = text
            .wrapped((column.width - TEXT_X + MARK_W).max(MIN_TEXT_W), usize::MAX)
            .lay_out(&painter);
        let height = laid.size().y.max(20.0) + ROW_GAP - 2.0;
        let (rect, _) = ui.allocate_exact_size(vec2(column.width, height), Sense::hover());
        let bullet = pos2(rect.left() + 5.0, rect.top() + 10.0);
        match tag {
            Tag::Verified => paint_mark(&painter, bullet, 12.0, Mark::Done, 0.0),
            Tag::NotVerified => paint_mark(&painter, bullet, 12.0, Mark::Todo, 0.0),
            Tag::Decisions => {
                painter.circle_filled(bullet, 2.0, column.ink(brand::FAINT));
            }
        }
        let id = ui.id().with(("entry", tag.caption(), index));
        laid.paint_selectable(
            ui,
            id,
            pos2(rect.left() + 16.0, rect.top() + (20.0 - laid.size().y.min(20.0)) / 2.0),
            brand::SOFT,
        );
    }
    fold_label(list.len(), all).is_some_and(|label| fold_link(ui, &label, tag.caption()))
}

/// A muted link that answers a click. `salt` tells folds apart.
pub(crate) fn fold_link(ui: &mut Ui, text: &str, salt: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), body(TEXT_SECONDARY), brand::MUTED);
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 8.0, 22.0), Sense::click());
    let _ = salt;
    let ink = if response.hovered() { brand::TEXT } else { brand::MUTED };
    ui.painter().galley(rect.left_center() + vec2(0.0, -galley.size().y / 2.0), galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
