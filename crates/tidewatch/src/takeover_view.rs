//! Draws take-over: the "Running elsewhere" card at the end of the Overview's rail, and the dialog that
//! asks before a session is moved. Both are drawn from plain data (`takeover.rs`) and report what
//! was clicked.

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Id, Key, Layout, Margin, Modifiers, Rect,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::elsewhere::Key as SessionKey;

use crate::brand;
use crate::notice::key_hint;
use crate::reader_view::ellipsized;
use crate::screen_kit::{
    Size, body, dialog_frame, footer_row, heading, mono, paint_cli_badge, primary_button,
    quiet_button,
};
use crate::settings_ui::{pill, switch_line};
use crate::takeover::{CardAction, DialogView, Offer, Row, Status};

const HEADER_H: f32 = 46.0;
const ROW_H: f32 = 46.0;
const DISCLOSURE_H: f32 = 38.0;
const SIDE: f32 = 16.0;
const BADGE: f32 = 26.0;
const DIALOG_W: f32 = 560.0;
const LINE_H: f32 = 60.0;
const LINE_GAP: f32 = 8.0;
const LINE_TAIL: f32 = 22.0;

#[derive(Clone)]
pub(crate) struct Card {
    pub(crate) summary: String,
    pub(crate) can_move_all: bool,
    pub(crate) rows: Vec<Row>,
    pub(crate) ended: Vec<Row>,
    pub(crate) ended_open: bool,
}

impl Card {
    fn disclosure_h(&self) -> f32 {
        if self.ended.is_empty() { 0.0 } else { DISCLOSURE_H }
    }

    fn ended_rows_h(&self) -> f32 {
        if self.ended_open { self.ended.len() as f32 * ROW_H } else { 0.0 }
    }
}

fn status_ink(status: Status) -> Color32 {
    match status {
        Status::Idle => brand::DONE,
        Status::Working => brand::WORKING,
        Status::Quiet | Status::NotRunning => brand::MUTED,
    }
}

fn line_text(live: usize) -> String {
    if live == 1 {
        "1 session in another terminal".to_owned()
    } else {
        format!("{live} sessions in other terminals")
    }
}

/// The compact card at the end of the Overview's rail: how many sessions run in other terminals and
/// a Move here button at the right. A click folds out the full card below it; `reveal` scrolls to
/// both.
pub(crate) fn line(ui: &mut Ui, data: &Card, open: bool, reveal: bool) -> Option<CardAction> {
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
        "Running elsewhere",
        heading(15.0),
        brand::TEXT,
    );
    painter.text(
        pos2(rect.left() + 14.0, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        line_text(data.rows.len()),
        body(12.0),
        brand::MUTED,
    );
    let label = if open { "Hide" } else { "Move here" };
    let button_w = crate::screen_kit::button_width(ui, label, Size::Compact);
    let button = Rect::from_center_size(
        pos2(rect.right() - 14.0 - button_w / 2.0, rect.center().y),
        vec2(button_w, Size::Compact.height()),
    );
    let toggled = crate::overview_view::in_rect(ui, button, |ui| {
        primary_button(ui, label, true, Size::Compact).clicked()
    });
    let mut action = toggled.then_some(CardAction::ToggleLine);
    if open {
        ui.add_space(LINE_GAP);
        action = action.or(card(ui, data, false));
        ui.add_space(LINE_TAIL);
    }
    if reveal {
        let shown = Rect::from_min_max(rect.min, pos2(rect.right(), ui.min_rect().bottom()));
        ui.scroll_to_rect(shown, Some(Align::TOP));
    }
    action
}

pub(crate) fn card(ui: &mut Ui, data: &Card, reveal: bool) -> Option<CardAction> {
    let width = ui.available_width();
    let running_h = data.rows.len() as f32 * ROW_H;
    let height = HEADER_H + running_h + data.disclosure_h() + data.ended_rows_h();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if reveal {
        ui.scroll_to_rect(rect, Some(Align::TOP));
    }
    ui.painter().rect(
        rect,
        CornerRadius::same(12),
        brand::CARD,
        Stroke::new(1.0, brand::CARD_LINE),
        StrokeKind::Inside,
    );
    let mut action = header(ui, rect, data);
    for (index, row) in data.rows.iter().enumerate() {
        let top = rect.top() + HEADER_H + index as f32 * ROW_H;
        let row_rect = Rect::from_min_size(pos2(rect.left(), top), vec2(width, ROW_H));
        action = action.or(draw_row(ui, row_rect, row));
    }
    if data.ended.is_empty() {
        return action;
    }
    let top = rect.top() + HEADER_H + running_h;
    let bar = Rect::from_min_size(pos2(rect.left(), top), vec2(width, DISCLOSURE_H));
    action = action.or(disclosure(ui, bar, data));
    if data.ended_open {
        for (index, row) in data.ended.iter().enumerate() {
            let row_top = bar.bottom() + index as f32 * ROW_H;
            let row_rect = Rect::from_min_size(pos2(rect.left(), row_top), vec2(width, ROW_H));
            action = action.or(draw_row(ui, row_rect, row));
        }
    }
    action
}

fn disclosure(ui: &mut Ui, rect: Rect, data: &Card) -> Option<CardAction> {
    let response = ui.interact(rect, Id::new("takeover_recent"), Sense::click());
    let painter = ui.painter_at(rect);
    painter.hline(
        (rect.left() + 12.0)..=(rect.right() - 12.0),
        rect.top(),
        Stroke::new(1.0, brand::CARD_LINE),
    );
    let chevron = if data.ended_open { "\u{25be}" } else { "\u{25b8}" };
    let ink = if response.hovered() { brand::TEXT } else { brand::MUTED };
    painter.text(
        pos2(rect.left() + SIDE, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{chevron}  Recent sessions ({})", data.ended.len()),
        body(12.5),
        ink,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Sessions that ended and are not open in a tab")
        .clicked()
        .then_some(CardAction::ToggleRecent)
}

fn header(ui: &mut Ui, rect: Rect, data: &Card) -> Option<CardAction> {
    let area = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H));
    let painter = ui.painter_at(area);
    let title = painter.text(
        pos2(area.left() + SIDE, area.center().y),
        Align2::LEFT_CENTER,
        "Running elsewhere",
        heading(14.0),
        brand::TEXT,
    );
    painter.text(
        pos2(title.right() + 12.0, area.center().y),
        Align2::LEFT_CENTER,
        &data.summary,
        body(12.5),
        brand::MUTED,
    );
    painter.hline(area.x_range(), area.bottom(), Stroke::new(1.0, brand::CARD_LINE));
    let mut inner = ui.new_child(
        UiBuilder::new()
            .id_salt("takeover_header")
            .max_rect(area.shrink2(vec2(SIDE, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    let tip = if data.can_move_all {
        "Ends every idle session in the app it runs in and resumes it here"
    } else {
        "No idle session can be moved"
    };
    primary_button(&mut inner, "Move all idle here", data.can_move_all, Size::Compact)
        .on_hover_text(tip)
        .clicked()
        .then_some(CardAction::MoveAllIdle)
}

fn draw_row(ui: &mut Ui, rect: Rect, row: &Row) -> Option<CardAction> {
    let painter = ui.painter_at(rect);
    painter.hline(
        (rect.left() + 12.0)..=(rect.right() - 12.0),
        rect.top(),
        Stroke::new(1.0, brand::CARD_LINE),
    );
    paint_cli_badge(
        &painter,
        Rect::from_center_size(
            pos2(rect.left() + SIDE + BADGE / 2.0, rect.center().y),
            vec2(BADGE, BADGE),
        ),
        &row.cli_id,
    );
    let mut action = None;
    let mut cluster = ui.new_child(
        UiBuilder::new()
            .id_salt(("takeover_row", &row.key))
            .max_rect(rect.shrink2(vec2(SIDE, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    cluster.spacing_mut().item_spacing.x = 10.0;
    if let Some(clicked) = offer_button(&mut cluster, row) {
        action = Some(CardAction::Press(clicked));
    }
    if row.status != Status::NotRunning {
        pill(&mut cluster, row.status.label(), status_ink(row.status));
    }
    let level_ink = if row.yolo { brand::RED } else { brand::SOFT };
    pill(&mut cluster, &row.level, level_ink);
    cluster.label(RichText::new(&row.host).size(12.5).color(brand::MUTED));
    let text_x = rect.left() + SIDE + BADGE + 12.0;
    let room = (cluster.min_rect().left() - text_x - 12.0).max(40.0);
    let name = ellipsized(&painter, &row.name, body(13.0), brand::TEXT, room);
    let folder = ellipsized(&painter, &row.folder, mono(11.0), brand::FAINT, room);
    painter.galley(pos2(text_x, rect.center().y - 15.0), name, brand::TEXT);
    painter.galley(pos2(text_x, rect.center().y + 3.0), folder, brand::FAINT);
    action
}

fn offer_button(ui: &mut Ui, row: &Row) -> Option<SessionKey> {
    let enabled = !matches!(row.offer, Offer::Moving | Offer::Blocked(_));
    let response = quiet_button(ui, row.offer.label(), enabled, Size::Compact);
    let response = match &row.offer {
        Offer::Blocked(reason) => response.on_hover_text(reason.as_str()),
        Offer::Moving => response.on_hover_text("Ending it and opening the tab"),
        Offer::MoveLive => response.on_hover_text("In stdusk: use its Move tabs to Tidewatch"),
        Offer::Waiting => {
            response.on_hover_text("It moves by itself once it is idle. Click to cancel")
        }
        Offer::MoveWhenIdle => {
            response.on_hover_text("Never interrupted: it moves when its turn ends")
        }
        Offer::ResumeHere => {
            response.on_hover_text("Open it in a tab and continue the conversation")
        }
        Offer::MoveHere => response.on_hover_text("End it there and resume it here"),
    };
    response.clicked().then(|| row.key.clone())
}

pub(crate) enum DialogOutcome {
    Cancel,
    Accept,
}

pub(crate) fn dialog(
    ctx: &egui::Context,
    view: &DialogView,
    worktree: &mut bool,
) -> Option<DialogOutcome> {
    let mut outcome = None;
    let modal = dialog_frame(ctx, Id::new("takeover_confirm"), &view.title, DIALOG_W, |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        for (number, line) in view.lines.iter().enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(
                    RichText::new((number + 1).to_string()).font(mono(13.0)).color(brand::TEAL),
                );
                ui.add(egui::Label::new(RichText::new(line).size(13.5).color(brand::SOFT)).wrap());
            });
        }
        commands(ui, view);
        for note in &view.notes {
            ui.label(RichText::new(note).size(12.5).color(brand::ORANGE));
        }
        if let Some(note) = &view.yolo_note {
            ui.label(RichText::new(note).size(12.5).color(brand::MUTED));
        }
        if view.offer_worktree {
            switch_line(ui, "Move it into a new worktree first", worktree);
        }
        ui.add_space(4.0);
        outcome = buttons(ui, view);
        key_hint(ui, "Enter moves  -  Esc cancels");
    });
    if outcome.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        outcome = Some(DialogOutcome::Accept);
    }
    if outcome.is_none() && modal.should_close() {
        outcome = Some(DialogOutcome::Cancel);
    }
    outcome
}

fn commands(ui: &mut Ui, view: &DialogView) {
    let single = view.items.len() == 1;
    let list = |ui: &mut Ui| {
        for item in &view.items {
            Frame::new()
                .fill(brand::ITEM)
                .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if !single {
                        ui.label(
                            RichText::new(format!("{}  {}", item.name, item.folder))
                                .size(12.5)
                                .color(brand::TEXT),
                        );
                    }
                    let command =
                        item.command.as_deref().unwrap_or("this session cannot be resumed");
                    ui.add(
                        egui::Label::new(
                            RichText::new(command).font(mono(12.0)).color(brand::MUTED),
                        )
                        .wrap(),
                    );
                });
        }
    };
    if single {
        list(ui);
    } else {
        ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, list);
    }
}

fn buttons(ui: &mut Ui, view: &DialogView) -> Option<DialogOutcome> {
    let mut outcome = None;
    footer_row(ui, "", |ui| {
        if primary_button(ui, &view.confirm, true, Size::Regular).clicked() {
            outcome = Some(DialogOutcome::Accept);
        }
        if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
            outcome = Some(DialogOutcome::Cancel);
        }
    });
    outcome
}

#[cfg(test)]
mod tests;
