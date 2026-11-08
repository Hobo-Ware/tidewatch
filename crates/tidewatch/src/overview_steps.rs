//! The checklist on an Overview card: the last ticked step and the open ones, each with its day tag
//! at the right, and a `+ N more` line, so the owner reads what is left without opening the tab.

use egui::{FontId, Painter, Ui, pos2};
use tidewatch_core::civil_date::Date;
use tidewatch_core::progress::Mark;

use crate::brand;
use crate::progress_blocks::TagLayout;
use crate::progress_model::{CardChecklist, step_tag};
use crate::progress_ring::paint_mark;
use crate::reader_view::ellipsized;
use crate::screen_kit::TEXT_SECONDARY;

const GAP: f32 = 8.0;
const ROW_H: f32 = 22.0;
const MORE_H: f32 = 18.0;
const MARK: f32 = 12.0;
const TEXT_X: f32 = MARK + 8.0;
const TAG_GAP: f32 = 8.0;
const MIN_TEXT_W: f32 = 40.0;

/// The height the checklist adds under a card's text, none when nothing is left.
pub(crate) fn height(list: &CardChecklist) -> f32 {
    if list.steps.is_empty() {
        return 0.0;
    }
    let more = if list.more > 0 { MORE_H } else { 0.0 };
    GAP + list.steps.len() as f32 * ROW_H + more
}

pub(crate) fn more_label(more: usize) -> String {
    format!("+ {more} more")
}

/// Paints the checklist from `top` (the end of the card's text) between `left` and `right`.
pub(crate) fn paint(
    ui: &Ui,
    painter: &Painter,
    (left, right, top): (f32, f32, f32),
    list: &CardChecklist,
    (today, salt): (Date, u64),
) {
    let mut y = top + GAP;
    for (index, step) in list.steps.iter().enumerate() {
        let mid = y + ROW_H / 2.0;
        paint_mark(painter, pos2(left + MARK / 2.0, mid), MARK, step.mark, 0.0);
        let tag = step_tag(&step.row(), today);
        let layout = tag.as_ref().map(|tag| TagLayout::new(ui, tag, false));
        let tag_w = layout.as_ref().map_or(0.0, |layout| layout.width() + TAG_GAP);
        let room = (right - left - TEXT_X - tag_w).max(MIN_TEXT_W);
        let ink = match step.mark {
            Mark::Done => brand::FAINT,
            Mark::Doing => brand::TEXT,
            _ => brand::STEP_TEXT,
        };
        let text = ellipsized(painter, &step.text, FontId::proportional(TEXT_SECONDARY), ink, room);
        painter.galley(pos2(left + TEXT_X, mid - text.size().y / 2.0), text, ink);
        if let (Some(layout), Some(tag)) = (layout, tag) {
            layout.paint(ui, right, mid, &tag.text, ui.id().with(("card_step_tag", salt, index)));
        }
        y += ROW_H;
    }
    if list.more > 0 {
        let more = painter.layout_no_wrap(
            more_label(list.more),
            FontId::proportional(TEXT_SECONDARY),
            brand::FAINT,
        );
        painter.galley(pos2(left + TEXT_X, y + (MORE_H - more.size().y) / 2.0), more, brand::FAINT);
    }
}
