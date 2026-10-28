//! Draws Recently closed under the Running elsewhere card at the end of the Overview's rail: a
//! compact line with a fold button, and the rows it folds out (badge, title, repo and when it
//! closed, a Reopen button). Drawn from plain data (`closed_state.rs`); reports what was clicked.

use egui::{
    Align, Align2, CornerRadius, Id, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, pos2, vec2,
};

use crate::brand;
use crate::closed_state::{Card, CardAction, Row};
use crate::reader_view::ellipsized;
use crate::screen_kit::{
    Size, body, button_width, heading, mono, paint_cli_badge, primary_button, quiet_button,
};
use crate::screen_kit::{TEXT_SECONDARY, TEXT_TAG, TEXT_TITLE};

const LINE_H: f32 = 60.0;
const LINE_GAP: f32 = 8.0;
const LINE_TAIL: f32 = 22.0;
const ROW_H: f32 = 46.0;
const SIDE: f32 = 16.0;
const BADGE: f32 = 26.0;
const TEXT_GAP: f32 = 12.0;

fn count_text(rows: usize) -> String {
    let noun = if rows == 1 { "agent" } else { "agents" };
    format!("{rows} {noun} closed in the last 14 days")
}

pub(crate) fn section(ui: &mut Ui, data: &Card) -> Option<CardAction> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, LINE_H), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(14),
        brand::CARD,
        Stroke::new(1.0, brand::CARD_LINE),
        StrokeKind::Inside,
    );
    let painter = ui.painter_at(rect);
    painter.text(
        pos2(rect.left() + 14.0, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        "Recently closed",
        heading(TEXT_TITLE),
        brand::TEXT,
    );
    painter.text(
        pos2(rect.left() + 14.0, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        count_text(data.rows.len()),
        body(12.0),
        brand::MUTED,
    );
    let label = if data.open { "Hide" } else { "Show" };
    let button_w = button_width(ui, label, Size::Compact);
    let button = Rect::from_center_size(
        pos2(rect.right() - 14.0 - button_w / 2.0, rect.center().y),
        vec2(button_w, Size::Compact.height()),
    );
    let toggled = crate::overview_view::in_rect(ui, button, |ui| {
        primary_button(ui, label, true, Size::Compact).clicked()
    });
    let mut action = toggled.then_some(CardAction::Toggle);
    if data.open {
        ui.add_space(LINE_GAP);
        action = action.or(rows(ui, data));
        ui.add_space(LINE_TAIL);
    }
    action
}

fn rows(ui: &mut Ui, data: &Card) -> Option<CardAction> {
    let width = ui.available_width();
    let height = data.rows.len() as f32 * ROW_H;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        brand::CARD,
        Stroke::new(1.0, brand::CARD_LINE),
        StrokeKind::Inside,
    );
    let mut action = None;
    for (index, row) in data.rows.iter().enumerate() {
        let top = rect.top() + index as f32 * ROW_H;
        let row_rect = Rect::from_min_size(pos2(rect.left(), top), vec2(width, ROW_H));
        action = action.or(draw_row(ui, row_rect, row, index > 0));
    }
    action
}

fn draw_row(ui: &mut Ui, rect: Rect, row: &Row, divider: bool) -> Option<CardAction> {
    let painter = ui.painter_at(rect);
    if divider {
        painter.hline(
            (rect.left() + 12.0)..=(rect.right() - 12.0),
            rect.top(),
            Stroke::new(1.0, brand::CARD_LINE),
        );
    }
    paint_cli_badge(
        &painter,
        Rect::from_center_size(
            pos2(rect.left() + SIDE + BADGE / 2.0, rect.center().y),
            vec2(BADGE, BADGE),
        ),
        &row.cli_id,
    );
    let mut cluster = ui.new_child(
        UiBuilder::new()
            .id_salt(("closed_row", &row.session_id))
            .max_rect(rect.shrink2(vec2(SIDE, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    let label = if row.busy { "Opening..." } else { "Reopen" };
    let response = quiet_button(&mut cluster, label, !row.busy, Size::Compact)
        .on_hover_text("Bring the worktree back and continue the conversation in a new tab");
    let text_x = rect.left() + SIDE + BADGE + TEXT_GAP;
    let room = (cluster.min_rect().left() - text_x - TEXT_GAP).max(40.0);
    let title = ellipsized(&painter, &row.title, body(13.0), brand::TEXT, room);
    let info = format!("{} \u{b7} {}", row.repo, row.closed);
    let info = ellipsized(&painter, &info, mono(TEXT_TAG), brand::FAINT, room);
    let title_at = pos2(text_x, rect.center().y - 15.0);
    let info_at = pos2(text_x, rect.center().y + 3.0);
    let hover =
        [Rect::from_min_size(title_at, title.size()), Rect::from_min_size(info_at, info.size())];
    painter.galley(title_at, title, brand::TEXT);
    painter.galley(info_at, info, brand::FAINT);
    for (part, area) in hover.into_iter().enumerate() {
        ui.interact(area, Id::new(("closed_name", &row.session_id, part)), Sense::hover())
            .on_hover_ui(|ui| {
                ui.label(RichText::new(&row.title).size(TEXT_SECONDARY).color(brand::TEXT));
                ui.label(RichText::new(&row.repo).font(mono(TEXT_TAG)).color(brand::MUTED));
            });
    }
    response.clicked().then(|| CardAction::Reopen(row.session_id.clone()))
}
