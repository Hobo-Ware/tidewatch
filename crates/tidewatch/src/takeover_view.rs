//! Draws take-over: the "Running elsewhere" card at the end of the Overview's rail, and the dialog that
//! asks before a session is moved. Both are drawn from plain data (`takeover.rs`) and report what
//! was clicked.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Id, Key, Layout, Margin, Modifiers, Painter, Rect,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::elsewhere::Key as SessionKey;

use crate::brand;
use crate::reader_view::ellipsized;
use crate::screen_kit::TEXT_TITLE;
use crate::screen_kit::{
    Size, body, button_width, dialog_frame, footer_row, heading, mono, paint_cli_badge,
    primary_button, quiet_button,
};
use crate::screen_kit::{TEXT_BODY, TEXT_SECONDARY, TEXT_TAG};
use crate::settings_ui::{pill, switch_line};
use crate::takeover::{CardAction, DialogView, Offer, Row, Status};

const HEADER_H: f32 = 46.0;
const ROW_H: f32 = 46.0;
const ROW_H_NARROW: f32 = 64.0;
const NARROW_BELOW: f32 = 520.0;
const NARROW_LINE_OFFSET: f32 = 14.0;
const NARROW_GAP: f32 = 6.0;
const HEADER_GAP: f32 = 12.0;
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

    fn ended_rows_h(&self, row_h: f32) -> f32 {
        if self.ended_open { self.ended.len() as f32 * row_h } else { 0.0 }
    }
}

fn row_h(width: f32) -> f32 {
    if width < NARROW_BELOW { ROW_H_NARROW } else { ROW_H }
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
        heading(TEXT_TITLE),
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
    let row_h = row_h(width);
    let running_h = data.rows.len() as f32 * row_h;
    let height = HEADER_H + running_h + data.disclosure_h() + data.ended_rows_h(row_h);
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
        let top = rect.top() + HEADER_H + index as f32 * row_h;
        let row_rect = Rect::from_min_size(pos2(rect.left(), top), vec2(width, row_h));
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
            let row_top = bar.bottom() + index as f32 * row_h;
            let row_rect = Rect::from_min_size(pos2(rect.left(), row_top), vec2(width, row_h));
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
        body(TEXT_SECONDARY),
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
    painter.hline(area.x_range(), area.bottom(), Stroke::new(1.0, brand::CARD_LINE));
    let label = "Move all idle here";
    let button_w = button_width(ui, label, Size::Compact);
    let room = area.width() - 2.0 * SIDE - button_w - HEADER_GAP;
    let summary = ellipsized(&painter, &data.summary, body(TEXT_SECONDARY), brand::MUTED, room);
    let at = pos2(area.left() + SIDE, area.center().y - summary.size().y / 2.0);
    painter.galley(at, summary, brand::MUTED);
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
    primary_button(&mut inner, label, data.can_move_all, Size::Compact)
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
    if rect.width() < NARROW_BELOW {
        draw_narrow(ui, &painter, rect, row)
    } else {
        draw_wide(ui, &painter, rect, row)
    }
}

fn row_cluster(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, rect: Rect) -> Ui {
    ui.new_child(
        UiBuilder::new().id_salt(id).max_rect(rect).layout(Layout::right_to_left(Align::Center)),
    )
}

fn level_ink(row: &Row) -> Color32 {
    if row.yolo { brand::RED } else { brand::SOFT }
}

fn hover_whole_name(ui: &mut Ui, row: &Row, rects: [Rect; 2]) {
    for (part, rect) in rects.into_iter().enumerate() {
        ui.interact(rect, Id::new(("takeover_name", &row.key, part)), Sense::hover()).on_hover_ui(
            |ui| {
                ui.label(RichText::new(&row.name).size(TEXT_SECONDARY).color(brand::TEXT));
                ui.label(RichText::new(&row.folder).font(mono(TEXT_TAG)).color(brand::MUTED));
            },
        );
    }
}

fn draw_wide(ui: &mut Ui, painter: &Painter, rect: Rect, row: &Row) -> Option<CardAction> {
    let mut action = None;
    let mut cluster = row_cluster(ui, ("takeover_row", &row.key), rect.shrink2(vec2(SIDE, 0.0)));
    cluster.spacing_mut().item_spacing.x = 10.0;
    if let Some(clicked) = offer_button(&mut cluster, row) {
        action = Some(CardAction::Press(clicked));
    }
    if row.status != Status::NotRunning {
        pill(&mut cluster, row.status.label(), status_ink(row.status));
    }
    pill(&mut cluster, &row.level, level_ink(row));
    cluster.label(RichText::new(&row.host).size(TEXT_SECONDARY).color(brand::MUTED));
    let text_x = rect.left() + SIDE + BADGE + 12.0;
    let room = (cluster.min_rect().left() - text_x - 12.0).max(40.0);
    let name = ellipsized(painter, &row.name, body(13.0), brand::TEXT, room);
    let folder = ellipsized(painter, &row.folder, mono(TEXT_TAG), brand::FAINT, room);
    let name_at = pos2(text_x, rect.center().y - 15.0);
    let folder_at = pos2(text_x, rect.center().y + 3.0);
    let rects =
        [Rect::from_min_size(name_at, name.size()), Rect::from_min_size(folder_at, folder.size())];
    painter.galley(name_at, name, brand::TEXT);
    painter.galley(folder_at, folder, brand::FAINT);
    hover_whole_name(ui, row, rects);
    action
}

fn draw_narrow(ui: &mut Ui, painter: &Painter, rect: Rect, row: &Row) -> Option<CardAction> {
    let text_x = rect.left() + SIDE + BADGE + 12.0;
    let right = rect.right() - SIDE;
    let line = |center_y: f32| {
        Rect::from_min_max(
            pos2(text_x, center_y - NARROW_LINE_OFFSET),
            pos2(right, center_y + NARROW_LINE_OFFSET),
        )
    };
    let top_y = rect.center().y - 16.0;
    let bottom_y = rect.center().y + 12.0;

    let mut pills = row_cluster(ui, ("takeover_pills", &row.key), line(top_y));
    pills.spacing_mut().item_spacing.x = NARROW_GAP;
    if row.status != Status::NotRunning {
        pill(&mut pills, row.status.label(), status_ink(row.status));
    }
    pill(&mut pills, &row.level, level_ink(row));
    let name_room = pills.min_rect().left() - text_x - HEADER_GAP;
    let name = ellipsized(painter, &row.name, body(TEXT_SECONDARY), brand::TEXT, name_room);
    let name_at = pos2(text_x, top_y - name.size().y / 2.0);

    let mut offer = row_cluster(ui, ("takeover_offer", &row.key), line(bottom_y));
    let clicked = offer_button(&mut offer, row);
    let info_room = offer.min_rect().left() - text_x - HEADER_GAP;
    let info = info_galley(painter, row, info_room);
    let info_at = pos2(text_x, bottom_y - info.size().y / 2.0);

    let rects =
        [Rect::from_min_size(name_at, name.size()), Rect::from_min_size(info_at, info.size())];
    painter.galley(name_at, name, brand::TEXT);
    painter.galley(info_at, info, brand::FAINT);
    hover_whole_name(ui, row, rects);
    clicked.map(CardAction::Press)
}

fn info_galley(painter: &Painter, row: &Row, room: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    let format = |font| TextFormat::simple(font, brand::FAINT);
    job.append(&row.folder, 0.0, format(mono(TEXT_TAG)));
    job.append(&format!(" \u{b7} {}", row.host), 0.0, format(body(TEXT_TAG)));
    job.wrap = TextWrapping::truncate_at_width(room.max(1.0));
    painter.layout_job(job)
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
                ui.add(
                    egui::Label::new(RichText::new(line).size(TEXT_BODY).color(brand::SOFT)).wrap(),
                );
            });
        }
        commands(ui, view);
        for note in &view.notes {
            ui.label(RichText::new(note).size(TEXT_SECONDARY).color(brand::ORANGE));
        }
        if let Some(note) = &view.yolo_note {
            ui.label(RichText::new(note).size(TEXT_SECONDARY).color(brand::MUTED));
        }
        if view.offer_worktree {
            switch_line(ui, "Move it into a new worktree first", worktree);
        }
        ui.add_space(4.0);
        outcome = buttons(ui, view);
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
                                .size(TEXT_SECONDARY)
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
    footer_row(ui, "Enter moves  -  Esc cancels", |ui| {
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
