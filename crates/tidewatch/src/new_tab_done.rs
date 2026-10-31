//! The "Done means" ticks under a Meeseeks form's task box: a check mark when on, an empty box
//! when off, the reason in muted text when a row cannot be ticked.

use egui::{Align2, CursorIcon, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::brand;
use crate::new_tab_layout::LABEL_H;
use crate::new_tab_meeseeks::{Row, Tick};
use crate::screen_kit::{body, field_label};

const CAPTION_GAP: f32 = 8.0;
const ROW_H: f32 = 22.0;
const ROW_GAP: f32 = 4.0;
const MARK: f32 = 14.0;
const MARK_GAP: f32 = 8.0;

pub(crate) fn height(rows: usize) -> f32 {
    let gaps = rows.saturating_sub(1) as f32 * ROW_GAP;
    LABEL_H + CAPTION_GAP + rows as f32 * ROW_H + gaps
}

#[derive(Default)]
pub(crate) struct Shown {
    pub(crate) rects: [Option<Rect>; 3],
    pub(crate) clicked: Option<Tick>,
}

fn paint_mark(ui: &Ui, rect: Rect, on: bool, enabled: bool) {
    let painter = ui.painter();
    if on {
        let ink = if enabled { brand::TEAL } else { brand::FAINT };
        let at = |x: f32, y: f32| rect.min + vec2(x, y) * (rect.width() / 24.0);
        painter.add(Shape::line(
            vec![at(5.0, 12.0), at(10.0, 17.0), at(19.0, 7.0)],
            Stroke::new(2.2, ink),
        ));
    } else {
        painter.rect_stroke(rect, 4.0, Stroke::new(1.5, brand::FIELD_LINE), StrokeKind::Inside);
    }
}

pub(crate) fn show(ui: &mut Ui, rows: &[Row; 3], tests_tip: Option<&str>, pr_label: &str) -> Shown {
    let mut shown = Shown::default();
    field_label(ui, "Done means");
    ui.add_space(CAPTION_GAP);
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            ui.add_space(ROW_GAP);
        }
        let enabled = row.disabled.is_none();
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
        shown.rects[index] = Some(rect);
        let mark = Rect::from_center_size(
            pos2(rect.left() + MARK / 2.0, rect.center().y),
            vec2(MARK, MARK),
        );
        paint_mark(ui, mark, row.on, enabled);
        let painter = ui.painter();
        let ink = if enabled { brand::TEXT } else { brand::MUTED };
        let text = if row.tick == Tick::Pr { pr_label } else { row.tick.label() };
        let label = painter.text(
            pos2(mark.right() + MARK_GAP, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            body(13.0),
            ink,
        );
        if let Some(reason) = row.disabled {
            painter.text(
                pos2(label.right() + MARK_GAP, rect.center().y),
                Align2::LEFT_CENTER,
                reason,
                body(12.0),
                brand::FAINT,
            );
        }
        if enabled {
            let mut response = response.on_hover_cursor(CursorIcon::PointingHand);
            if let Some(tip) = tests_tip.filter(|_| row.tick == Tick::Tests) {
                response = response.on_hover_text(tip);
            }
            if response.clicked() {
                shown.clicked = Some(row.tick);
            }
        } else if let Some(reason) = row.disabled {
            response.on_hover_text(reason);
        }
    }
    shown
}
