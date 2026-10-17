//! The new-tab form's stage cards: the stages of the picked workflow, one small card each.

use egui::{Pos2, Rect, Sense, Ui, pos2, vec2};

use crate::brand;
use crate::commands::Command;
use crate::new_tab_workflow::StageCard;
use crate::reader_view::{ellipsized, paint_item};
use crate::screen_kit::{body, heading};

pub(crate) const GAP: f32 = 8.0;
pub(crate) const STAGE_H: f32 = 48.0;

/// Where each of `count` cards goes: `columns` of them across `width`, rows `height` tall, from
/// `origin` down.
pub(crate) fn grid(
    origin: Pos2,
    width: f32,
    columns: usize,
    count: usize,
    height: f32,
) -> Vec<Rect> {
    let card_w = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
    (0..count)
        .map(|index| {
            let (row, column) = (index / columns, index % columns);
            let at = pos2(
                origin.x + column as f32 * (card_w + GAP),
                origin.y + row as f32 * (height + GAP),
            );
            Rect::from_min_size(at, vec2(card_w, height))
        })
        .collect()
}

pub(crate) fn grid_height(columns: usize, count: usize, height: f32) -> f32 {
    let rows = count.div_ceil(columns);
    if rows == 0 { 0.0 } else { rows as f32 * height + (rows as f32 - 1.0) * GAP }
}

pub(crate) fn stages_height(columns: usize, count: usize) -> f32 {
    grid_height(columns, count, STAGE_H)
}

pub(crate) fn stages(ui: &mut Ui, stages: &[StageCard], columns: usize) {
    let width = ui.available_width();
    let (area, _) =
        ui.allocate_exact_size(vec2(width, stages_height(columns, stages.len())), Sense::hover());
    let rects = grid(area.min, width, columns, stages.len(), STAGE_H);
    for (rect, stage) in rects.into_iter().zip(stages) {
        paint_item(ui.painter(), rect, false);
        let painter = ui.painter();
        let room = rect.width() - 20.0;
        let title = format!("{} {}", stage.number, stage.title);
        let title = ellipsized(painter, &title, heading(13.0), brand::TEXT, room);
        painter.galley(rect.left_top() + vec2(10.0, 7.0), title, brand::TEXT);
        let detail = ellipsized(painter, &stage.detail, body(12.0), brand::MUTED, room);
        painter.galley(rect.left_top() + vec2(10.0, 26.0), detail, brand::MUTED);
        ui.interact(rect, ui.id().with(("stage-card", stage.number)), Sense::hover())
            .on_hover_text(format!("Gate: {}", stage.gate));
    }
}

/// What a list with no entries says: where to add one, that it is loading, or that nothing matches.
pub(crate) fn empty_text(filter: &str, loaded: bool, noun: &str) -> String {
    if !filter.is_empty() {
        format!("No {noun} matches \"{filter}\".")
    } else if loaded {
        format!("No {noun}s yet. Add one in Settings ({}).", Command::Settings.keys())
    } else {
        format!("The {noun}s are still loading...")
    }
}

#[cfg(test)]
mod tests;
