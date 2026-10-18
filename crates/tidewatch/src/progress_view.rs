//! The Progress panel in the History card: ring and goal, a bar per phase, the question, the
//! current phase's rows and the tags that open the lists. Which side a tab shows is kept here.

use std::collections::HashMap;
use std::time::Duration;
use tidewatch_core::prefs::RightPanel;

use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, Frame, Margin, Rect, ScrollArea, Sense, Shape, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::progress::{Entry, Ledger, Mark};

use crate::brand;
use crate::progress_model::{
    self as model, Bar, BarKind, ItemRow, Ring, TAGS, Tag, Tone, entries, item_rows, questions,
    rows_shown,
};
use crate::progress_ring::{paint, paint_mark, tone_color};
use crate::progress_state::TabLedger;
use crate::reader_view::{ellipsized, paint_item};
use crate::screen_kit::{Size, body, heading, mono, primary_button};

pub(crate) const SIDES: [&str; 2] = ["Progress", "History"];

const WIDE_PAD: f32 = 22.0;
const NARROW_PAD: f32 = 14.0;
const NARROW_BELOW: f32 = 330.0;
const GAP: f32 = 22.0;
const ROW_H: f32 = 30.0;
const MARK_W: f32 = 18.0;
const BAR_H: f32 = 6.0;
const BAR_MIN_W: f32 = 52.0;
const BAR_GAP: f32 = 6.0;
const TAG_H: f32 = 26.0;
const TAG_GAP: f32 = 8.0;
const LIST_LIMIT: usize = 6;
const SPIN_EVERY: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Progress,
    History,
}

impl Side {
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Progress => 0,
            Self::History => 1,
        }
    }

    pub(crate) const fn at(index: usize) -> Self {
        if index == 0 { Self::Progress } else { Self::History }
    }
}

impl From<RightPanel> for Side {
    fn from(panel: RightPanel) -> Self {
        match panel {
            RightPanel::Progress => Self::Progress,
            RightPanel::History => Self::History,
        }
    }
}

/// The side the card shows: History until a ledger exists, then the user's choice, else `first`
/// (the Settings default).
pub(crate) fn side_shown(chosen: Option<Side>, has_ledger: bool, first: Side) -> Side {
    match (has_ledger, chosen) {
        (false, _) => Side::History,
        (true, Some(side)) => side,
        (true, None) => first,
    }
}

/// What the panel remembers of one tab while the app runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TabState {
    pub(crate) chosen: Option<Side>,
    open: [bool; 3],
    all_rows: bool,
}

impl TabState {
    pub(crate) fn toggle(&mut self, tag: Tag) {
        self.open[tag.index()] ^= true;
    }

    pub(crate) fn is_open(self, tag: Tag) -> bool {
        self.open[tag.index()]
    }

    pub(crate) fn show_all_rows(self) -> bool {
        self.all_rows
    }

    pub(crate) fn toggle_rows(&mut self) {
        self.all_rows ^= true;
    }
}

#[derive(Default)]
pub(crate) struct Memory {
    tabs: HashMap<u64, TabState>,
}

impl Memory {
    pub(crate) fn tab(&mut self, id: u64) -> &mut TabState {
        self.tabs.entry(id).or_default()
    }

    pub(crate) fn forget_except(&mut self, open: impl Fn(u64) -> bool) {
        self.tabs.retain(|id, _| open(*id));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Answer,
}

pub(crate) struct Input<'a> {
    pub(crate) ledger: &'a TabLedger,
    pub(crate) now_ms: i64,
    pub(crate) time: f64,
    /// Whether things that turn may ask for frames.
    pub(crate) animate: bool,
    /// The side shown first once a ledger exists.
    pub(crate) first: Side,
}

fn pad_of(width: f32) -> f32 {
    if width < NARROW_BELOW { NARROW_PAD } else { WIDE_PAD }
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    input: &Input<'_>,
    state: &mut TabState,
) -> Option<Action> {
    let mut action = None;
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).id_salt("progress-panel"));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let pad = pad_of(rect.width()) as i8;
    ScrollArea::vertical().auto_shrink([false, false]).show(&mut child, |ui| {
        Frame::new().inner_margin(Margin::symmetric(pad, pad)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            action = content(ui, input, state);
        });
    });
    action
}

fn content(ui: &mut Ui, input: &Input<'_>, state: &mut TabState) -> Option<Action> {
    let ledger = &input.ledger.ledger;
    let width = ui.available_width();
    let ring = model::ring(ledger);
    let mut action = None;
    header(ui, width, ledger, &ring, input);
    ui.add_space(GAP);
    phase_bars(ui, width, ledger, ring.tone);
    let asked = questions(ledger);
    if !asked.is_empty() {
        ui.add_space(GAP);
        if question_card(ui, width, &asked) {
            action = Some(Action::Answer);
        }
    }
    let rows = item_rows(ledger);
    if !rows.is_empty() {
        ui.add_space(GAP);
        item_rows_block(ui, width, &rows, input, state);
    }
    if TAGS.iter().any(|tag| !entries(ledger, *tag).is_empty()) {
        ui.add_space(GAP);
        tags(ui, width, ledger, state);
    }
    action
}

fn single_line(
    painter: &egui::Painter,
    text: &str,
    font: egui::FontId,
    ink: Color32,
    w: f32,
) -> std::sync::Arc<egui::Galley> {
    ellipsized(painter, text, font, ink, w)
}

fn header(ui: &mut Ui, width: f32, ledger: &Ledger, ring: &Ring, input: &Input<'_>) {
    let narrow = width < NARROW_BELOW - 2.0 * NARROW_PAD;
    let size = if narrow { 52.0 } else { 72.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(width, size), Sense::hover());
    let painter = ui.painter();
    let center = pos2(rect.left() + size / 2.0, rect.center().y);
    let color = tone_color(ring.tone);
    paint(painter, center, size, ring.fraction, color, brand::CARD_LINE);
    let label = ring.label();
    if !label.is_empty() {
        let mut font = heading(size * 0.24);
        let mut galley = painter.layout_no_wrap(label.clone(), font.clone(), brand::TEXT);
        let room = size * 0.62;
        if galley.size().x > room {
            font.size *= room / galley.size().x;
            galley = painter.layout_no_wrap(label, font, brand::TEXT);
        }
        painter.galley(center - galley.size() / 2.0, galley, brand::TEXT);
    }
    let x = rect.left() + size + if narrow { 12.0 } else { 16.0 };
    let room = (rect.right() - x).max(1.0);
    let goal_size = if narrow { 14.0 } else { 16.0 };
    let goal = single_line(painter, ledger.goal.trim(), heading(goal_size), brand::TEXT, room);
    let meta = meta_line(painter, ring.tone, input, room);
    let block = goal.size().y + 4.0 + meta.size().y;
    let top = rect.center().y - block / 2.0;
    let meta_top = top + goal.size().y + 4.0;
    painter.galley(pos2(x, top), goal, brand::TEXT);
    painter.galley(pos2(x, meta_top), meta, brand::MUTED);
    let text_area = Rect::from_min_max(pos2(x, top), pos2(rect.right(), top + block));
    let hover = ui.interact(text_area, ui.id().with("progress-goal"), Sense::hover());
    hover.on_hover_text(ledger.goal.trim());
    let _ = response;
}

fn meta_line(
    painter: &egui::Painter,
    tone: Tone,
    input: &Input<'_>,
    room: f32,
) -> std::sync::Arc<egui::Galley> {
    let ink = tone_color(tone);
    let quiet = TextFormat::simple(body(12.5), brand::MUTED);
    let mut job = LayoutJob::default();
    job.append("\u{25cf} ", 0.0, TextFormat::simple(body(12.5), ink));
    job.append(tone.word(), 0.0, TextFormat::simple(body(12.5), ink));
    if let Some(updated) = model::updated(input.now_ms, input.ledger.updated_ms) {
        job.append(&format!(" \u{b7} {updated}"), 0.0, quiet);
    }
    job.wrap = TextWrapping::truncate_at_width(room.max(1.0));
    painter.layout_job(job)
}

/// How many bars fit in a row of `width`, and so how the phases wrap.
pub(crate) fn bar_columns(width: f32, count: usize) -> usize {
    let fit = ((width + BAR_GAP) / (BAR_MIN_W + BAR_GAP)).floor() as usize;
    fit.clamp(1, count.max(1))
}

fn phase_bars(ui: &mut Ui, width: f32, ledger: &Ledger, tone: Tone) {
    let bars = model::bars(ledger);
    if bars.is_empty() {
        return;
    }
    let columns = bar_columns(width, bars.len());
    let col_w = (width - BAR_GAP * (columns - 1) as f32) / columns as f32;
    let label_h = 16.0;
    let row_h = BAR_H + 7.0 + label_h;
    for (line, chunk) in bars.chunks(columns).enumerate() {
        if line > 0 {
            ui.add_space(12.0);
        }
        let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
        for (index, bar) in chunk.iter().enumerate() {
            let left = rect.left() + (col_w + BAR_GAP) * index as f32;
            let column = Rect::from_min_size(pos2(left, rect.top()), vec2(col_w, row_h));
            paint_bar(ui, column, bar, tone, line * columns + index);
        }
    }
}

fn paint_bar(ui: &mut Ui, column: Rect, bar: &Bar, tone: Tone, index: usize) {
    let painter = ui.painter();
    let track = Rect::from_min_size(column.min, vec2(column.width(), BAR_H));
    let radius = CornerRadius::same((BAR_H / 2.0) as u8);
    match bar.kind {
        BarKind::Skipped => {
            let y = track.center().y;
            painter.extend(Shape::dashed_line(
                &[pos2(track.left() + 2.0, y), pos2(track.right() - 2.0, y)],
                Stroke::new(1.5, brand::FAINT.gamma_multiply(0.6)),
                4.0,
                3.0,
            ));
        }
        BarKind::Plain => {
            painter.rect_filled(track, radius, brand::FIELD_LINE);
            if bar.fill > 0.0 {
                let fill = Rect::from_min_size(track.min, vec2(track.width() * bar.fill, BAR_H));
                let ink = if tone == Tone::Done { brand::DONE } else { brand::TEAL };
                painter.rect_filled(fill, radius, ink);
            }
        }
    }
    let ink = if bar.current { brand::TEXT } else { brand::MUTED };
    let label = single_line(painter, &bar.label, body(12.0), ink, column.width());
    let at = pos2(column.left(), track.bottom() + 7.0);
    painter.galley(at, label, ink);
    let area = Rect::from_min_size(column.min, vec2(column.width(), column.height()));
    let tip = if bar.total == 0 {
        bar.label.clone()
    } else {
        format!("{}: {}/{}", bar.label, bar.done, bar.total)
    };
    ui.interact(area, ui.id().with(("progress-bar", index)), Sense::hover()).on_hover_text(tip);
}

fn question_card(ui: &mut Ui, width: f32, asked: &[&Entry]) -> bool {
    const PAD_X: f32 = 14.0;
    const PAD_Y: f32 = 12.0;
    const GLYPH: f32 = 28.0;
    const SPACE: f32 = 12.0;
    let first = asked[0];
    let button_w = crate::screen_kit::button_width(ui, "Answer", Size::Compact);
    let stacked = width < 300.0;
    let text_w = if stacked {
        width - 2.0 * PAD_X - GLYPH - SPACE
    } else {
        width - 2.0 * PAD_X - GLYPH - SPACE * 2.0 - button_w
    }
    .max(40.0);
    let mut job = LayoutJob::simple(first.text.trim().to_owned(), body(13.5), brand::TEXT, text_w);
    job.wrap.max_rows = 2;
    let galley = ui.painter().layout_job(job);
    let text_h = galley.size().y;
    let inner_h = if stacked {
        text_h.max(GLYPH) + SPACE + Size::Compact.height()
    } else {
        text_h.max(GLYPH).max(Size::Compact.height())
    };
    let (rect, _) = ui.allocate_exact_size(vec2(width, inner_h + 2.0 * PAD_Y), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), brand::ORANGE.gamma_multiply(0.08));
    let glyph_top = rect.top() + PAD_Y + (text_h.max(GLYPH) - GLYPH) / 2.0;
    let glyph = Rect::from_min_size(pos2(rect.left() + PAD_X, glyph_top), vec2(GLYPH, GLYPH));
    painter.rect_filled(glyph, CornerRadius::same(8), brand::ORANGE.gamma_multiply(0.16));
    painter.text(glyph.center(), Align2::CENTER_CENTER, "?", heading(15.0), brand::ORANGE);
    let text_x = glyph.right() + SPACE;
    let text_top = rect.top() + PAD_Y + (text_h.max(GLYPH) - text_h) / 2.0;
    painter.galley(pos2(text_x, text_top), galley, brand::TEXT);
    let button_rect = if stacked {
        let top = rect.top() + PAD_Y + text_h.max(GLYPH) + SPACE;
        Rect::from_min_size(pos2(text_x, top), vec2(button_w, Size::Compact.height()))
    } else {
        Rect::from_center_size(
            pos2(rect.right() - PAD_X - button_w / 2.0, rect.center().y),
            vec2(button_w, Size::Compact.height()),
        )
    };
    let hover = ui.interact(
        Rect::from_min_max(rect.min, pos2(rect.right(), button_rect.top().max(rect.top() + 1.0))),
        ui.id().with("progress-question"),
        Sense::hover(),
    );
    let tip = asked.iter().map(|entry| entry.text.trim()).collect::<Vec<_>>().join("\n");
    hover.on_hover_text(tip);
    let mut clicked = false;
    ui.scope_builder(UiBuilder::new().max_rect(button_rect), |ui| {
        clicked = primary_button(ui, "Answer", true, Size::Compact).clicked();
    });
    clicked
}

fn item_rows_block(
    ui: &mut Ui,
    width: f32,
    rows: &[ItemRow<'_>],
    input: &Input<'_>,
    state: &mut TabState,
) {
    let (shown, hidden) = rows_shown(rows.len(), state.show_all_rows());
    let mut spinning = false;
    for row in &rows[..shown] {
        spinning |= row.mark == Mark::Doing;
        item_row(ui, width, row, input.time);
    }
    if spinning && input.animate {
        ui.ctx().request_repaint_after(SPIN_EVERY);
    }
    if hidden > 0 || state.show_all_rows() {
        let text = if state.show_all_rows() {
            "Show less".to_owned()
        } else {
            format!("Show all {}", rows.len())
        };
        ui.add_space(4.0);
        if link(ui, &text) {
            state.toggle_rows();
        }
    }
}

fn item_row(ui: &mut Ui, width: f32, row: &ItemRow<'_>, time: f64) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    let painter = ui.painter();
    paint_mark(painter, pos2(rect.left() + MARK_W / 2.0, rect.center().y), 14.0, row.mark, time);
    let text_x = rect.left() + MARK_W + 12.0;
    let muted = matches!(row.mark, Mark::Skipped);
    let ink = if muted { brand::FAINT } else { brand::TEXT };
    let room = (rect.right() - text_x).max(1.0);
    let why = row.why.map(|why| {
        let galley = painter.layout_no_wrap(why.to_owned(), body(12.0), brand::FAINT);
        let cap = (room * 0.42).max(40.0);
        if galley.size().x > cap {
            single_line(painter, why, body(12.0), brand::FAINT, cap)
        } else {
            galley
        }
    });
    let why_w = why.as_ref().map_or(0.0, |galley| galley.size().x + 12.0);
    let label = single_line(painter, row.text, body(13.5), ink, (room - why_w).max(1.0));
    painter.galley(pos2(text_x, rect.center().y - label.size().y / 2.0), label, ink);
    if let Some(galley) = why {
        let at = pos2(rect.right() - galley.size().x, rect.center().y - galley.size().y / 2.0);
        painter.galley(at, galley, brand::FAINT);
    }
    let tip = match row.why {
        Some(why) => format!("{} - {why}", row.text),
        None => row.text.to_owned(),
    };
    response.on_hover_text(tip);
}

fn link(ui: &mut Ui, text: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), body(12.5), brand::MUTED);
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 8.0, 22.0), Sense::click());
    let ink = if response.hovered() { brand::TEXT } else { brand::MUTED };
    ui.painter().galley(rect.left_center() + vec2(0.0, -galley.size().y / 2.0), galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Where pills of `row_h` tall, `gap` apart, sit when they flow in rows of `width`: the top left
/// of each, and the height of everything.
pub(crate) fn flow(widths: &[f32], width: f32, row_h: f32, gap: f32) -> (Vec<(f32, f32)>, f32) {
    let mut at = Vec::with_capacity(widths.len());
    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    for pill in widths {
        if x > 0.0 && x + pill > width {
            x = 0.0;
            y += row_h + gap;
        }
        at.push((x, y));
        x += pill + gap;
    }
    (at, if widths.is_empty() { 0.0 } else { y + row_h })
}

fn tags(ui: &mut Ui, width: f32, ledger: &Ledger, state: &mut TabState) {
    let shown: Vec<(Tag, usize)> = TAGS
        .iter()
        .map(|tag| (*tag, entries(ledger, *tag).len()))
        .filter(|(_, count)| *count > 0)
        .collect();
    let labels: Vec<_> = shown.iter().map(|(tag, count)| tag.label(*count)).collect();
    let icon_w = |tag: Tag| if tag == Tag::Decisions { 0.0 } else { 16.0 };
    let widths: Vec<f32> = shown
        .iter()
        .zip(&labels)
        .map(|((tag, _), label)| {
            let text = ui.painter().layout_no_wrap(label.clone(), body(12.5), brand::SOFT);
            (text.size().x + 22.0 + icon_w(*tag)).min(width)
        })
        .collect();
    let (places, height) = flow(&widths, width, TAG_H, TAG_GAP);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    for (index, ((tag, _), label)) in shown.iter().zip(&labels).enumerate() {
        let (x, y) = places[index];
        let pill = Rect::from_min_size(rect.min + vec2(x, y), vec2(widths[index], TAG_H));
        let response =
            ui.interact(pill, ui.id().with(("progress-tag", tag.index())), Sense::click());
        let open = state.is_open(*tag);
        let painter = ui.painter();
        let line = if open {
            brand::TEAL.gamma_multiply(0.6)
        } else if response.hovered() {
            brand::FIELD_LINE
        } else {
            brand::ITEM_LINE
        };
        painter.rect(
            pill,
            CornerRadius::same(255),
            Color32::TRANSPARENT,
            Stroke::new(1.0, line),
            StrokeKind::Inside,
        );
        let ink = if open { brand::TEXT } else { brand::SOFT };
        let mut left = pill.left() + 11.0;
        match tag {
            Tag::Verified => {
                paint_mark(painter, pos2(left + 5.0, pill.center().y), 12.0, Mark::Done, 0.0);
                left += 16.0;
            }
            Tag::NotVerified => {
                paint_mark(painter, pos2(left + 5.0, pill.center().y), 12.0, Mark::Todo, 0.0);
                left += 16.0;
            }
            Tag::Decisions => {}
        }
        let text =
            single_line(painter, label, body(12.5), ink, (pill.right() - 11.0 - left).max(1.0));
        painter.galley(pos2(left, pill.center().y - text.size().y / 2.0), text, ink);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            state.toggle(*tag);
        }
    }
    for (tag, _) in &shown {
        if state.is_open(*tag) {
            ui.add_space(14.0);
            entry_list(ui, width, *tag, &entries(ledger, *tag));
        }
    }
}

fn entry_list(ui: &mut Ui, width: f32, tag: Tag, list: &[&Entry]) {
    const PAD: f32 = 10.0;
    const BULLET: f32 = 16.0;
    let painter = ui.painter().clone();
    let caption = painter.layout_no_wrap(tag.caption().to_owned(), body(11.5), brand::FAINT);
    let (cap_rect, _) = ui.allocate_exact_size(vec2(width, caption.size().y + 6.0), Sense::hover());
    painter.galley(cap_rect.left_top(), caption, brand::FAINT);
    let text_w = (width - 2.0 * PAD - BULLET).max(40.0);
    let galleys: Vec<_> = list
        .iter()
        .take(LIST_LIMIT)
        .map(|entry| {
            let mut job = LayoutJob::default();
            job.append(entry.text.trim(), 0.0, TextFormat::simple(body(12.5), brand::SOFT));
            if let Some(why) = entry.why.as_deref().map(str::trim).filter(|why| !why.is_empty()) {
                job.append(&format!("  {why}"), 0.0, TextFormat::simple(body(12.0), brand::FAINT));
            }
            job.wrap.max_width = text_w;
            job.wrap.max_rows = 3;
            painter.layout_job(job)
        })
        .collect();
    let more = list.len().saturating_sub(LIST_LIMIT);
    let more_h = if more > 0 { 20.0 } else { 0.0 };
    let body_h: f32 = galleys.iter().map(|galley| galley.size().y + 8.0).sum::<f32>() - 8.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, body_h + more_h + 2.0 * PAD), Sense::hover());
    paint_item(&painter, rect, false);
    let mut y = rect.top() + PAD;
    for (index, galley) in galleys.into_iter().enumerate() {
        let center = pos2(rect.left() + PAD + 5.0, y + 8.0);
        match tag {
            Tag::Verified => paint_mark(&painter, center, 12.0, Mark::Done, 0.0),
            Tag::NotVerified => paint_mark(&painter, center, 12.0, Mark::Todo, 0.0),
            Tag::Decisions => {
                painter.circle_filled(center, 2.0, brand::FAINT);
            }
        }
        let height = galley.size().y;
        painter.galley(pos2(rect.left() + PAD + BULLET, y), galley, brand::SOFT);
        y += height + if index + 1 < LIST_LIMIT { 8.0 } else { 0.0 };
    }
    if more > 0 {
        painter.text(
            pos2(rect.left() + PAD + BULLET, rect.bottom() - PAD - 8.0),
            Align2::LEFT_CENTER,
            format!("+{more} more"),
            mono(11.5),
            brand::FAINT,
        );
    }
}

#[cfg(test)]
mod tests;
