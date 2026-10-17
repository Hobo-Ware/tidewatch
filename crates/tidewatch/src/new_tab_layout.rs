//! The new-tab form's geometry as pure functions of the rect it gets: header, lane, pane and footer.

use egui::{Rect, pos2, vec2};

pub(crate) const SIDE_PAD: f32 = 26.0;
pub(crate) const TOP_PAD: f32 = 8.0;
const COLUMN_MAX_W: f32 = 1100.0;
pub(crate) const HEADER_H: f32 = 46.0;
const HEADER_GAP: f32 = 18.0;
pub(crate) const COLUMN_GAP: f32 = 20.0;
const LANE_TARGET_SHARE: f32 = 0.30;
const LANE_MAX_SHARE: f32 = 0.38;
const LANE_MIN_W: f32 = 240.0;
const LANE_MAX_W: f32 = 280.0;
pub(crate) const FILTER_H: f32 = 28.0;
const LANE_LIST_GAP: f32 = 8.0;

pub(crate) const ROW_H: f32 = 46.0;
pub(crate) const GROUP_H: f32 = 24.0;
const ROW_GAP: f32 = 2.0;
const GROUP_GAP: f32 = 8.0;

pub(crate) const SECTION_GAP: f32 = 16.0;
pub(crate) const LABEL_H: f32 = 14.0;
pub(crate) const LABEL_GAP: f32 = 6.0;
pub(crate) const FIELD_H: f32 = 34.0;
pub(crate) const PROBLEM_H: f32 = 24.0;
pub(crate) const FIELDS_GAP: f32 = 12.0;
const STACK_BELOW: f32 = 380.0;
pub(crate) const MESSAGE_MIN_H: f32 = 96.0;
pub(crate) const FOOTER_H: f32 = 32.0;
const FOOTER_GAP: f32 = 12.0;
const BOTTOM_PAD: f32 = 14.0;
pub(crate) const SCROLL_GUTTER: f32 = 10.0;
const STAGE_MIN_W: f32 = 120.0;
const STAGE_COLUMNS_MAX: usize = 4;
const STAGE_GAP: f32 = 8.0;

pub(crate) fn form_rect(area: Rect) -> Rect {
    let width = (area.width() - 2.0 * SIDE_PAD).clamp(0.0, COLUMN_MAX_W);
    Rect::from_min_size(
        pos2(area.left() + SIDE_PAD, area.top() + TOP_PAD),
        vec2(width, (area.height() - TOP_PAD).max(0.0)),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Columns {
    pub(crate) header: Rect,
    pub(crate) lane: Rect,
    pub(crate) pane: Rect,
}

fn lane_width(form_w: f32) -> f32 {
    (form_w * LANE_TARGET_SHARE).clamp(LANE_MIN_W, LANE_MAX_W).min(form_w * LANE_MAX_SHARE)
}

pub(crate) fn columns(form: Rect) -> Columns {
    let header = Rect::from_min_size(form.min, vec2(form.width(), HEADER_H));
    let top = (form.top() + HEADER_H + HEADER_GAP).min(form.bottom());
    let lane_w = lane_width(form.width());
    let lane =
        Rect::from_min_max(pos2(form.left(), top), pos2(form.left() + lane_w, form.bottom()));
    let pane = Rect::from_min_max(pos2(lane.right() + COLUMN_GAP, top), form.max);
    Columns { header, lane, pane: Rect::from_min_max(pane.min, pane.max.max(pane.min)) }
}

pub(crate) fn lane_parts(lane: Rect) -> (Rect, Rect) {
    let top = Rect::from_min_size(lane.min, vec2(lane.width(), FILTER_H.min(lane.height())));
    let list_top = (top.bottom() + LANE_LIST_GAP).min(lane.bottom());
    (top, Rect::from_min_max(pos2(lane.left(), list_top), lane.max))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Group,
    Row,
}

pub(crate) fn lane_slots(slots: &[Slot]) -> (Vec<(f32, f32)>, f32) {
    let mut y = 0.0;
    let mut placed = Vec::with_capacity(slots.len());
    for (index, slot) in slots.iter().enumerate() {
        let (gap, height) = match slot {
            Slot::Group => (if index == 0 { 0.0 } else { GROUP_GAP }, GROUP_H),
            Slot::Row => (if index == 0 { 0.0 } else { ROW_GAP }, ROW_H),
        };
        y += gap;
        placed.push((y, height));
        y += height;
    }
    (placed, y)
}

pub(crate) fn fields_stacked(width: f32) -> bool {
    width < STACK_BELOW
}

fn field_block_h() -> f32 {
    LABEL_H + LABEL_GAP + FIELD_H
}

/// The worktree name and the start branch: side by side, or the name over the branch when the
/// pane is narrow. A problem under the name adds its line.
pub(crate) fn fields_height(width: f32, problem: bool) -> f32 {
    let name = field_block_h() + if problem { 2.0 + PROBLEM_H } else { 0.0 };
    if fields_stacked(width) { name + FIELDS_GAP + field_block_h() } else { name }
}

pub(crate) fn field_rects(area: Rect, problem: bool) -> (Rect, Rect) {
    let name_h = field_block_h() + if problem { 2.0 + PROBLEM_H } else { 0.0 };
    if fields_stacked(area.width()) {
        let name = Rect::from_min_size(area.min, vec2(area.width(), name_h));
        let base = Rect::from_min_size(
            pos2(area.left(), name.bottom() + FIELDS_GAP),
            vec2(area.width(), field_block_h()),
        );
        (name, base)
    } else {
        let each = (area.width() - FIELDS_GAP) / 2.0;
        let name = Rect::from_min_size(area.min, vec2(each, name_h));
        let base = Rect::from_min_size(
            pos2(area.left() + each + FIELDS_GAP, area.top()),
            vec2(each, field_block_h()),
        );
        (name, base)
    }
}

pub(crate) fn stage_columns(width: f32) -> usize {
    let fit = ((width + STAGE_GAP) / (STAGE_MIN_W + STAGE_GAP)).floor();
    (fit as usize).clamp(1, STAGE_COLUMNS_MAX)
}

/// What stands above the message box, each block as tall as it was measured: the permission
/// control and its text, the stages of a workflow, warnings and the name and branch fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Above {
    pub(crate) permission: f32,
    pub(crate) stages: f32,
    pub(crate) warnings: f32,
    pub(crate) fields: f32,
}

impl Above {
    pub(crate) fn height(&self) -> f32 {
        [self.stages, self.permission, self.warnings, self.fields]
            .into_iter()
            .filter(|height| *height > 0.0)
            .map(|height| height + SECTION_GAP)
            .sum()
    }
}

/// The right pane: the body that scrolls, the footer under it and the share of the body the
/// message box takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pane {
    pub(crate) body: Rect,
    pub(crate) footer: Rect,
    pub(crate) content_w: f32,
    pub(crate) message_h: f32,
    pub(crate) scrolls: bool,
}

fn message_chrome(hint_h: f32) -> f32 {
    LABEL_H + LABEL_GAP + hint_h
}

/// Shares `rect` out. `measure` gives what stands above the message box and the height of the
/// hint under it for a content width. The message box takes what is left but never less than its
/// minimum; when even that does not fit, the body scrolls as a whole and the footer stays.
pub(crate) fn pane(rect: Rect, measure: impl Fn(f32) -> (Above, f32)) -> Pane {
    let footer_bottom = rect.bottom() - BOTTOM_PAD.min(rect.height());
    let footer_top = (footer_bottom - FOOTER_H).max(rect.top());
    let footer =
        Rect::from_min_max(pos2(rect.left(), footer_top), pos2(rect.right(), footer_bottom));
    let body_bottom = (footer_top - FOOTER_GAP).max(rect.top());
    let body = Rect::from_min_max(rect.min, pos2(rect.right(), body_bottom));
    let fit = |content_w: f32| {
        let (above, hint_h) = measure(content_w);
        let fixed = above.height() + message_chrome(hint_h);
        let room = body.height() - fixed;
        (room.max(MESSAGE_MIN_H), room < MESSAGE_MIN_H)
    };
    let (message_h, scrolls) = fit(rect.width());
    if !scrolls {
        return Pane { body, footer, content_w: rect.width(), message_h, scrolls };
    }
    let content_w = (rect.width() - SCROLL_GUTTER).max(0.0);
    let (message_h, _) = fit(content_w);
    Pane { body, footer, content_w, message_h, scrolls }
}

#[cfg(test)]
mod tests;
