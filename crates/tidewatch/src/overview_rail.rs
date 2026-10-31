//! The right rail of the Overview: Coming up by day, then Poofed today (the last three, the rest
//! on Show all). Running elsewhere is drawn under them by the caller.

use egui::{Align2, CornerRadius, FontId, Painter, Rect, Sense, Ui, pos2, vec2};

use crate::agenda::{self, Day, Item, Mark};
use crate::brand;
use crate::overview::{POOFED_CAP, PoofedRow, capped, poofed_header};
use crate::overview_view::{
    HEAD_GAP, Head, Out, SECTION_GAP, SECTION_HEAD_H, SECTION_PAD, State, ghost, in_rect,
    repo_label, section_frame, section_head,
};
use crate::reader_view::ellipsized;
use crate::screen_kit::TEXT_TAG;
use crate::screen_kit::{
    Size, TagFill, body, button_width, heading, paint_tag, primary_button, tag_width,
};

const DAY_H: f32 = 22.0;
const DAY_GAP: f32 = 6.0;
const MONTH_H: f32 = 18.0;
const MONTH_GAP: f32 = 10.0;
const ITEM_H: f32 = 44.0;
const POOFED_ROW_H: f32 = 28.0;
const DOT: f32 = 6.0;
const TEXT_X: f32 = 16.0;
const TAG_GAP: f32 = 6.0;
const ABOUT: &str = "about";

pub(crate) fn show(
    ui: &mut Ui,
    width: f32,
    agenda: &[Day],
    poofed: &[PoofedRow],
    state: &mut State,
    out: &mut Out,
) {
    if !agenda.is_empty() {
        out.agenda = out.agenda.or(coming_up(ui, width, agenda));
    }
    if !poofed.is_empty() {
        if !agenda.is_empty() {
            ui.add_space(SECTION_GAP);
        }
        out.poofed = out.poofed.or(poofed_today(ui, width, poofed, state));
    }
}

fn coming_up(ui: &mut Ui, width: f32, days: &[Day]) -> Option<usize> {
    let body_h: f32 = days
        .iter()
        .enumerate()
        .map(|(at, day)| month_h(at, day) + DAY_H + day.items.len() as f32 * ITEM_H + DAY_GAP)
        .sum();
    let height = SECTION_PAD * 2.0 + SECTION_HEAD_H + HEAD_GAP + body_h - DAY_GAP;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    section_frame(&painter, rect, false);
    let count = format!("{} scheduled", agenda::count(days));
    section_head(
        &painter,
        rect,
        &Head { title: "Coming up", caption: "", count: &count, dot: None, ink: brand::TEXT },
    );
    let mut y = rect.top() + SECTION_PAD + SECTION_HEAD_H + HEAD_GAP;
    let mut clicked = None;
    for (at, day) in days.iter().enumerate() {
        if let (true, Some(month)) = (day.starts_month, &day.month) {
            if at > 0 {
                y += MONTH_GAP;
            }
            month_header(&painter, rect, y, month);
            y += MONTH_H;
        }
        let ink = if day.today { brand::TEAL_TEXT } else { brand::MUTED };
        painter.text(
            pos2(rect.left() + SECTION_PAD, y + DAY_H / 2.0),
            Align2::LEFT_CENTER,
            &day.label,
            heading(TEXT_TAG),
            ink,
        );
        y += DAY_H;
        for item in &day.items {
            let row = Rect::from_min_size(
                pos2(rect.left() + SECTION_PAD - 6.0, y),
                vec2(width - SECTION_PAD * 2.0 + 12.0, ITEM_H),
            );
            if item_row(ui, &painter, row, item) {
                clicked = Some(item.row);
            }
            y += ITEM_H;
        }
        y += DAY_GAP;
    }
    clicked
}

fn month_h(at: usize, day: &Day) -> f32 {
    if day.starts_month { MONTH_H + if at > 0 { MONTH_GAP } else { 0.0 } } else { 0.0 }
}

fn month_header(painter: &Painter, rect: Rect, y: f32, month: &str) {
    let left = rect.left() + SECTION_PAD;
    let mid = y + MONTH_H / 2.0;
    let name =
        painter.text(pos2(left, mid), Align2::LEFT_CENTER, month, body(TEXT_TAG), brand::MUTED);
    painter.hline(
        (name.right() + 8.0)..=(rect.right() - SECTION_PAD),
        mid,
        egui::Stroke::new(1.0, brand::ITEM_LINE),
    );
}

fn item_row(ui: &mut Ui, painter: &Painter, rect: Rect, item: &Item) -> bool {
    let resp = ui.interact(rect, ui.id().with(("agenda-row", item.row)), Sense::click());
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(8), brand::ITEM);
    }
    let parked = item.mark == Mark::Parked;
    let left = rect.left() + 6.0;
    let dot = if parked { brand::FAINT } else { brand::TEAL };
    painter.circle_filled(pos2(left + DOT / 2.0, rect.top() + 14.0), DOT / 2.0, dot);
    let text_x = left + TEXT_X;
    let mut room = rect.right() - 6.0 - text_x;
    let mut button = None;
    if item.mark == Mark::Decide {
        let w = button_width(ui, "Decide", Size::Compact);
        button = Some(Rect::from_min_size(
            pos2(rect.right() - 6.0 - w, rect.center().y - Size::Compact.height() / 2.0),
            vec2(w, Size::Compact.height()),
        ));
        room -= w + 8.0;
    }
    let ink = if parked { brand::MUTED } else { brand::TEXT };
    let tag = item.about.then(|| tag_width(painter, ABOUT));
    let title_room = tag.map_or(room, |tag| room - tag - TAG_GAP);
    let title = ellipsized(painter, &item.what, FontId::proportional(13.0), ink, title_room);
    let title_w = title.size().x;
    painter.galley(pos2(text_x, rect.top() + 6.0), title, ink);
    if tag.is_some() {
        paint_tag(
            painter,
            (text_x + title_w + TAG_GAP, rect.top() + 14.0),
            ABOUT,
            brand::MUTED,
            TagFill::Line(brand::ITEM_LINE),
        );
    }
    let mid = rect.top() + 29.0;
    let right = repo_label(painter, text_x, mid, &item.place, room * 0.4);
    let mut tail = format!("\u{b7} {}", item.tab);
    if let Some(note) = &item.note {
        tail = format!("{tail} \u{b7} {note}");
    }
    let tail = ellipsized(
        painter,
        &tail,
        FontId::proportional(12.0),
        brand::MUTED,
        (text_x + room - right - 6.0).max(1.0),
    );
    painter.galley(pos2(right + 6.0, mid - tail.size().y / 2.0), tail, brand::MUTED);
    let clicked = button.is_some_and(|button| {
        in_rect(ui, button, |ui| primary_button(ui, "Decide", true, Size::Compact).clicked())
    });
    clicked
        | resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&item.tip).clicked()
}

fn poofed_today(ui: &mut Ui, width: f32, rows: &[PoofedRow], state: &mut State) -> Option<usize> {
    let (shown, _) = capped(rows, POOFED_CAP, state.poofed_all);
    let button = (rows.len() > POOFED_CAP).then(|| {
        if state.poofed_all { "Show fewer".to_owned() } else { format!("Show all {}", rows.len()) }
    });
    let more_h = if button.is_some() { 8.0 + Size::Compact.height() } else { 0.0 };
    let height =
        SECTION_PAD * 2.0 + SECTION_HEAD_H + HEAD_GAP + shown.len() as f32 * POOFED_ROW_H + more_h;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    section_frame(&painter, rect, false);
    let count = poofed_header(rows);
    section_head(
        &painter,
        rect,
        &Head { title: "Poofed today", caption: "", count: &count, dot: None, ink: brand::TEXT },
    );
    let mut y = rect.top() + SECTION_PAD + SECTION_HEAD_H + HEAD_GAP;
    let mut clicked = None;
    for (index, row) in shown.iter().enumerate() {
        let line = Rect::from_min_size(
            pos2(rect.left() + SECTION_PAD - 6.0, y),
            vec2(width - SECTION_PAD * 2.0 + 12.0, POOFED_ROW_H),
        );
        let resp = ui.interact(
            line,
            ui.id().with(("poofed-row", row.name.as_str(), index)),
            Sense::click(),
        );
        if resp.hovered() {
            painter.rect_filled(line, CornerRadius::same(6), brand::ITEM);
        }
        let outcome_ink = if row.outcome == "answered" { brand::MUTED } else { brand::TEAL_TEXT };
        let took =
            painter.layout_no_wrap(row.took.clone(), FontId::proportional(12.0), brand::MUTED);
        let outcome =
            painter.layout_no_wrap(row.outcome.clone(), FontId::proportional(12.0), outcome_ink);
        let right = line.right() - 6.0;
        painter.galley(
            pos2(right - took.size().x, line.center().y - took.size().y / 2.0),
            took.clone(),
            brand::MUTED,
        );
        let outcome_x = right - took.size().x - 10.0 - outcome.size().x;
        painter.galley(
            pos2(outcome_x, line.center().y - outcome.size().y / 2.0),
            outcome,
            outcome_ink,
        );
        let name = ellipsized(
            &painter,
            &row.name,
            FontId::proportional(13.0),
            brand::SOFT,
            (outcome_x - line.left() - 6.0 - 12.0).max(1.0),
        );
        painter.galley(
            pos2(line.left() + 6.0, line.center().y - name.size().y / 2.0),
            name,
            brand::SOFT,
        );
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&row.summary)
            .clicked()
        {
            clicked = Some(row.index);
        }
        y += POOFED_ROW_H;
    }
    if let Some(label) = button
        && ghost(ui, pos2(rect.left() + SECTION_PAD, y + 8.0), &label)
    {
        state.poofed_all = !state.poofed_all;
    }
    clicked
}
