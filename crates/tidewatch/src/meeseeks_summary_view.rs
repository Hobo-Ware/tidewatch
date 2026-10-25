//! Draws the summary tab of a poofed Meeseeks in the place of the terminal and the panels: the task
//! and when it ended beside the Meeseeks face, the stage card (the Progress panel's look, all four
//! bars), each check with its result and line, what the agent said, the pull request link, the
//! branch it left, the last turn and the ledger's stages. Read-only: its one button is Close.

use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Color32, CornerRadius, Id, Layout, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, pos2, vec2,
};
use tidewatch_core::progress::Mark;

use crate::agent_avatar;
use crate::avatar_glue::meeseeks_avatar;
use crate::brand;
use crate::last_turn::LastTurn;
use crate::last_turn_view;
use crate::link_text::Text;
use crate::meeseeks_model::Look;
use crate::meeseeks_summary::{BranchLine, CheckRow, StageRow, Summary};
use crate::meeseeks_view;
use crate::progress_ring::paint_mark;
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, body, button_width, heading, mono, quiet_button};
use crate::selectable;

const COLUMN_W: f32 = 760.0;
const PAD: f32 = 24.0;
const SECTION_GAP: f32 = 22.0;
const FACE: f32 = 44.0;
const FACE_GAP: f32 = 14.0;
const CHECK_H: f32 = 34.0;
const CHECK_PAD: f32 = 16.0;
const LABEL_W: f32 = 84.0;
const RADIUS: u8 = 12;
const STAGE_NAME_W: f32 = 150.0;
const STAGE_COUNT_W: f32 = 64.0;
const STAGE_ROW_H: f32 = 28.0;
const SAID_ROWS: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Close,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Placed {
    pub(crate) header: Option<Rect>,
    pub(crate) close: Option<Rect>,
    pub(crate) card: Option<Rect>,
    pub(crate) checks: Vec<Rect>,
    pub(crate) pr: Option<Rect>,
    pub(crate) branch: Option<Rect>,
}

pub(crate) fn placed_id() -> Id {
    Id::new("meeseeks-summary-placed")
}

pub(crate) fn show(ui: &mut Ui, area: Rect, summary: &Summary) -> Option<Action> {
    ui.painter().rect_filled(area, 0.0, brand::GROUND);
    let width = (area.width() - 2.0 * PAD).clamp(1.0, COLUMN_W);
    let left = area.center().x - width / 2.0;
    let column =
        Rect::from_min_max(pos2(left, area.top() + PAD), pos2(left + width, area.bottom()));
    let mut content = ui.new_child(UiBuilder::new().max_rect(column));
    let mut placed = Placed::default();
    let mut action = None;
    ScrollArea::vertical().auto_shrink([false, false]).show(&mut content, |ui| {
        let width = ui.available_width();
        action = header(ui, width, summary, &mut placed);
        ui.add_space(SECTION_GAP);
        meeseeks_view::show(ui, width, &summary.card);
        placed.card = ui.ctx().data(|data| {
            data.get_temp::<meeseeks_view::Placed>(meeseeks_view::placed_id())
                .and_then(|card| card.card)
        });
        ui.add_space(SECTION_GAP);
        checks(ui, width, &summary.checks, &mut placed);
        if let Some(said) = &summary.said {
            ui.add_space(SECTION_GAP);
            section_label(ui, "Summary");
            linked(ui, width, said, None);
        }
        if let Some(pr) = &summary.pr {
            ui.add_space(SECTION_GAP);
            section_label(ui, "Pull request");
            placed.pr = Some(linked(ui, width, &pr.url, Some(&pr.label)));
        }
        if let Some(branch) = &summary.branch {
            ui.add_space(SECTION_GAP);
            section_label(ui, "Branch");
            placed.branch = Some(branch_rows(ui, width, branch));
        }
        if !summary.done.is_empty() || !summary.next.is_empty() {
            ui.add_space(SECTION_GAP);
            let turn = LastTurn {
                done: summary.done.clone(),
                needs: Vec::new(),
                next: summary.next.clone(),
                check: None,
                can_ask: Ok(()),
                owner_asks: false,
            };
            last_turn_view::show(ui, width, &turn, false);
        }
        if !summary.stages.is_empty() {
            ui.add_space(SECTION_GAP);
            section_label(ui, "Ledger");
            stages(ui, width, &summary.stages);
        }
        ui.add_space(PAD);
    });
    ui.ctx().data_mut(|data| data.insert_temp(placed_id(), placed));
    action
}

fn header(ui: &mut Ui, width: f32, summary: &Summary, placed: &mut Placed) -> Option<Action> {
    let (row, _) = ui.allocate_exact_size(vec2(width, FACE), Sense::hover());
    placed.header = Some(row);
    let painter = ui.painter().clone();
    let face = Rect::from_min_size(row.min, vec2(FACE, FACE));
    agent_avatar::paint(ui.ctx(), &painter, face, &meeseeks_avatar());
    let button_w = button_width(ui, "Close", Size::Regular);
    let text_left = face.right() + FACE_GAP;
    let room = (row.right() - button_w - FACE_GAP - text_left).max(1.0);
    let title = ellipsized(&painter, &summary.title, heading(18.0), brand::TEXT, room);
    let meta = ellipsized(&painter, &summary.meta, body(13.5), brand::MUTED, room);
    let block = title.size().y + 4.0 + meta.size().y;
    let top = row.center().y - block / 2.0;
    let meta_top = top + title.size().y + 4.0;
    selectable::galley(ui, pos2(text_left, top), title, brand::TEXT);
    selectable::galley(ui, pos2(text_left, meta_top), meta, brand::MUTED);
    let button_row =
        Rect::from_min_max(pos2(row.right() - button_w, row.top()), row.right_bottom());
    let mut action = None;
    ui.scope_builder(
        UiBuilder::new().max_rect(button_row).layout(Layout::right_to_left(Align::Center)),
        |ui| {
            let close = quiet_button(ui, "Close", true, Size::Regular);
            placed.close = Some(close.rect);
            if close
                .on_hover_text(format!("Close ({})", crate::commands::Command::CloseTab.keys()))
                .clicked()
            {
                action = Some(Action::Close);
            }
        },
    );
    action
}

fn section_label(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), heading(13.0), brand::MUTED);
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y + 6.0), Sense::hover());
    selectable::galley(ui, rect.left_top(), galley, brand::MUTED);
}

fn mark_of(look: Look) -> Mark {
    match look {
        Look::Skipped => Mark::Skipped,
        Look::Done | Look::Active | Look::Pending => Mark::Done,
    }
}

fn checks(ui: &mut Ui, width: f32, rows: &[CheckRow], placed: &mut Placed) {
    section_label(ui, "Checks");
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, CHECK_H * rows.len() as f32), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect(
        rect,
        CornerRadius::same(RADIUS),
        brand::ITEM,
        Stroke::new(1.0, brand::ITEM_LINE),
        StrokeKind::Inside,
    );
    for (index, row) in rows.iter().enumerate() {
        let line = Rect::from_min_size(
            pos2(rect.left(), rect.top() + CHECK_H * index as f32),
            vec2(width, CHECK_H),
        );
        placed.checks.push(line);
        if index > 0 {
            painter.hline(
                (line.left() + CHECK_PAD)..=(line.right() - CHECK_PAD),
                line.top(),
                Stroke::new(1.0, brand::ITEM_LINE),
            );
        }
        let mid = line.center().y;
        paint_mark(
            &painter,
            pos2(line.left() + CHECK_PAD + 6.0, mid),
            14.0,
            mark_of(row.look),
            0.0,
        );
        let result_ink = if row.look == Look::Skipped { brand::FAINT } else { brand::DONE };
        let result = painter.layout_no_wrap(row.result.to_owned(), body(13.0), result_ink);
        let label_left = line.left() + CHECK_PAD + 26.0;
        let detail_left = label_left + LABEL_W;
        let room = (line.right() - CHECK_PAD - result.size().x - 12.0 - detail_left).max(1.0);
        let label = painter.layout_no_wrap(row.label.to_owned(), heading(14.0), brand::TEXT);
        selectable::galley(ui, pos2(label_left, mid - label.size().y / 2.0), label, brand::TEXT);
        let detail = ellipsized(&painter, &row.detail, body(13.0), brand::MUTED, room);
        selectable::galley(
            ui,
            pos2(detail_left, mid - detail.size().y / 2.0),
            detail,
            brand::MUTED,
        );
        selectable::galley(
            ui,
            pos2(line.right() - CHECK_PAD - result.size().x, mid - result.size().y / 2.0),
            result,
            result_ink,
        );
    }
}

/// Text with its web links clickable, with an optional muted `lead` before it. Returns where it sits.
fn linked(ui: &mut Ui, width: f32, text: &str, lead: Option<&str>) -> Rect {
    let painter = ui.painter().clone();
    let mut line = Text::new();
    if let Some(lead) = lead.filter(|lead| *lead != text) {
        line.run(&format!("{lead}  "), &TextFormat::simple(body(14.5), brand::MUTED));
    }
    line.run(text, &TextFormat::simple(body(14.5), brand::TEXT));
    let laid = line.wrapped(width, SAID_ROWS).lay_out(&painter);
    let (rect, _) = ui.allocate_exact_size(vec2(width, laid.size().y), Sense::hover());
    laid.paint_selectable(ui, ui.id().with(("summary-text", text)), rect.min, brand::TEXT);
    rect
}

fn branch_rows(ui: &mut Ui, width: f32, branch: &BranchLine) -> Rect {
    let painter = ui.painter().clone();
    let mut job = LayoutJob::default();
    job.append(&branch.name, 0.0, TextFormat::simple(mono(13.5), brand::SOFT));
    job.append(&format!("   {}", branch.text), 0.0, TextFormat::simple(body(14.0), brand::MUTED));
    if let Some(files) = &branch.files {
        job.append("\n", 0.0, TextFormat::simple(body(14.0), brand::MUTED));
        job.append(&files.text, 0.0, TextFormat::simple(body(14.0), brand::MUTED));
        job.append(
            &format!("   {}", files.added),
            0.0,
            TextFormat::simple(mono(13.0), brand::GREEN),
        );
        job.append(
            &format!("  {}", files.removed),
            0.0,
            TextFormat::simple(mono(13.0), brand::RED),
        );
    }
    job.wrap = TextWrapping::wrap_at_width(width.max(1.0));
    let galley = painter.layout_job(job);
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    selectable::galley(ui, rect.min, galley, brand::SOFT);
    rect
}

fn stages(ui: &mut Ui, width: f32, rows: &[StageRow]) {
    let painter = ui.painter().clone();
    for row in rows {
        let (line, _) = ui.allocate_exact_size(vec2(width, STAGE_ROW_H), Sense::hover());
        let mid = line.center().y;
        let name = ellipsized(&painter, &row.name, body(13.5), brand::SOFT, STAGE_NAME_W - 12.0);
        selectable::galley(ui, pos2(line.left(), mid - name.size().y / 2.0), name, brand::SOFT);
        let count = painter.layout_no_wrap(
            format!("{} of {}", row.done, row.total),
            mono(12.0),
            brand::MUTED,
        );
        selectable::galley(
            ui,
            pos2(line.right() - count.size().x, mid - count.size().y / 2.0),
            count,
            brand::MUTED,
        );
        let track = Rect::from_min_max(
            pos2(line.left() + STAGE_NAME_W, mid - 3.0),
            pos2(line.right() - STAGE_COUNT_W - 12.0, mid + 3.0),
        );
        painter.rect_filled(track, CornerRadius::same(3), brand::ITEM_LINE);
        let fraction = if row.total == 0 { 0.0 } else { row.done as f32 / row.total as f32 };
        let fill =
            Rect::from_min_size(track.min, vec2(track.width() * fraction.clamp(0.0, 1.0), 6.0));
        let ink: Color32 = brand::TEAL;
        painter.rect_filled(fill, CornerRadius::same(3), ink);
    }
}

#[cfg(test)]
mod tests;
