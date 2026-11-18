//! Draws the Agents panel of the Overview: one full-width row per agent under its state group (face,
//! name and meta line, steps, progress, time of the last word), and the Finished today panel under it.
//! A click on a row jumps to the agent's tab.

use egui::{Align2, CornerRadius, FontId, Painter, Rect, Sense, Ui, pos2, vec2};
use jiff::tz::TimeZone;

use crate::agent_avatar::{self, Face};
use crate::brand;
use crate::overview::{Row, card_view, fmt_elapsed};
use crate::overview_agents::{
    COL_GAP, Columns, Density, FACE, FINISHED_ROW_H, Finished, LINE_H, Lane, Lines, ROW_PAD, clock,
    columns, lines, progress_text, row_height, segments,
};
use crate::overview_steps;
use crate::overview_view::{
    HEAD_GAP, Head, SECTION_HEAD_H, SECTION_PAD, ghost, repo_label, section_frame, section_head,
};
use crate::panel_blocks;
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, TEXT_BODY, TEXT_SECONDARY, TEXT_TAG, body, heading};
use crate::status_view;

pub(crate) const GROUP_H: f32 = 30.0;
const MORE_H: f32 = 8.0 + Size::Compact.height();
const NAME_MID: f32 = 10.0;
const META_MID: f32 = 31.0;
const BAR_H: f32 = 4.0;
const SEGMENT_GAP: f32 = 2.0;
const FACE_TOP: f32 = 2.0;

/// An agent group to draw: its rows and how many the group has in all.
pub(crate) struct Group<'a> {
    pub(crate) lane: Lane,
    pub(crate) rows: &'a [Row],
    pub(crate) total: usize,
    /// The label of the button that folds the rest in, when the group is capped.
    pub(crate) more: Option<String>,
}

#[derive(Default)]
pub(crate) struct AgentsOut {
    pub(crate) jump: Option<u64>,
    pub(crate) toggled: bool,
}

const PANEL_CHROME: f32 = SECTION_PAD * 2.0 + SECTION_HEAD_H + HEAD_GAP;

/// The panel's height with every row in `density`.
pub(crate) fn height(groups: &[Group<'_>], density: Density) -> f32 {
    let body: f32 = groups
        .iter()
        .map(|group| {
            let rows: f32 = group
                .rows
                .iter()
                .map(|row| row_height(lines(row, group.lane, density).count()))
                .sum();
            GROUP_H + rows + if group.more.is_some() { MORE_H } else { 0.0 }
        })
        .sum();
    PANEL_CHROME + body
}

pub(crate) fn show(
    ui: &mut Ui,
    width: f32,
    groups: &[Group<'_>],
    density: Density,
    total: usize,
) -> AgentsOut {
    let (rect, _) = ui.allocate_exact_size(vec2(width, height(groups, density)), Sense::hover());
    let painter = ui.painter_at(rect);
    section_frame(&painter, rect, false);
    section_head(
        &painter,
        rect,
        &Head {
            title: "Agents",
            caption: "",
            count: &total.to_string(),
            dot: None,
            ink: brand::text(),
        },
    );
    let inner = Rect::from_min_max(
        pos2(rect.left() + SECTION_PAD, rect.top() + PANEL_CHROME - HEAD_GAP + 4.0),
        pos2(rect.right() - SECTION_PAD, rect.bottom()),
    );
    let cols = columns(inner.width() - 12.0);
    let clip = ui.clip_rect();
    let mut y = inner.top();
    let mut out = AgentsOut::default();
    for group in groups {
        group_head(&painter, inner, y, group);
        y += GROUP_H;
        for (index, row) in group.rows.iter().enumerate() {
            let content = lines(row, group.lane, density);
            let h = row_height(content.count());
            let rect = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), h));
            y += h;
            if !rect.intersects(clip) {
                continue;
            }
            if row_view(ui, &painter, rect, row, (&content, &cols), index > 0) {
                out.jump = Some(row.tab_id);
            }
        }
        if let Some(label) = &group.more {
            out.toggled |= ghost(ui, pos2(inner.left(), y + 8.0), label);
            y += MORE_H;
        }
    }
    out
}

fn group_head(painter: &Painter, inner: Rect, y: f32, group: &Group<'_>) {
    let mid = y + GROUP_H / 2.0 + 2.0;
    painter.circle_filled(pos2(inner.left() + 10.0, mid), 4.0, group.lane.dot());
    let title = painter.text(
        pos2(inner.left() + 22.0, mid),
        Align2::LEFT_CENTER,
        group.lane.title(),
        heading(TEXT_SECONDARY),
        group.lane.dot(),
    );
    let count = painter.text(
        pos2(title.right() + 8.0, mid),
        Align2::LEFT_CENTER,
        group.total.to_string(),
        FontId::proportional(TEXT_SECONDARY),
        brand::muted(),
    );
    let caption = group.lane.caption();
    if !caption.is_empty() {
        painter.text(
            pos2(count.right() + 8.0, mid),
            Align2::LEFT_CENTER,
            caption,
            FontId::proportional(TEXT_TAG),
            brand::faint(),
        );
    }
}

fn tip_of(row: &Row) -> String {
    let status = match row.stamp.age.as_deref() {
        Some(age) => format!("{} \u{b7} {age}", status_view::describe(&row.status)),
        None => status_view::describe(&row.status),
    };
    let status = match &row.tokens {
        Some(tokens) => format!("{status}\n{}", tokens.tip),
        None => status,
    };
    match row.summary.as_deref() {
        Some(summary) => format!("{summary}\n\n{status}"),
        None => status,
    }
}

fn row_view(
    ui: &mut Ui,
    painter: &Painter,
    rect: Rect,
    row: &Row,
    (content, cols): (&Lines, &Columns),
    divider: bool,
) -> bool {
    let resp = ui.interact(rect, ui.id().with(("agent-row", row.tab_id)), Sense::click());
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(8), brand::item());
    } else if divider {
        painter.hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, brand::item_line()));
    }
    let left = rect.left() + 6.0;
    let right = rect.right() - 6.0;
    let top = rect.top() + ROW_PAD;
    let face = Rect::from_min_size(pos2(left, top + FACE_TOP), vec2(FACE, FACE));
    paint_face(ui, painter, face, row);
    if row.avatar.is_some() {
        agent_avatar::menu(&resp, row.tab_id);
    }
    let name_x = left + FACE + COL_GAP;
    name_and_meta(ui, painter, (name_x, top), cols.name, row);
    let steps_x = name_x + cols.name + COL_GAP;
    steps_column(ui, painter, (steps_x, top), cols.steps, (row, content));
    let time_right = right;
    let view = card_view(row);
    if cols.progress > 0.0 {
        let x = time_right - cols.time - COL_GAP - cols.progress;
        progress(painter, (x, top), cols.progress, row, view.ring);
    }
    let age = row.stamp.age.clone().unwrap_or_else(|| fmt_elapsed(row.idle_secs));
    let galley = ellipsized(painter, &age, body(TEXT_TAG), brand::muted(), cols.time);
    let at = pos2(time_right - galley.size().x, top + NAME_MID - galley.size().y / 2.0);
    painter.galley(at, galley, brand::muted());
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip_of(row)).clicked()
}

fn paint_face(ui: &Ui, painter: &Painter, tile: Rect, row: &Row) {
    let face = Face::of(&row.profile, &row.agent_id, row.avatar.clone());
    if agent_avatar::paint_face(ui.ctx(), painter, tile, &face)
        && let Some(avatar) = face.avatar()
    {
        agent_avatar::hover(ui, tile, row.tab_id, avatar);
    }
}

fn name_and_meta(ui: &Ui, painter: &Painter, (x, top): (f32, f32), width: f32, row: &Row) {
    let name = ellipsized(painter, &row.title, heading(TEXT_BODY), brand::text(), width);
    painter.galley(pos2(x, top + NAME_MID - name.size().y / 2.0), name, brand::text());
    let mid = top + META_MID;
    let limit = x + width;
    let mut at = x;
    if !matches!(row.kind, crate::overview::AgentKind::Cli(_)) {
        let kind = painter.layout_no_wrap(
            row.kind.label(),
            FontId::proportional(TEXT_SECONDARY),
            row.kind.ink(),
        );
        let w = kind.size().x;
        painter.galley(pos2(at, mid - kind.size().y / 2.0), kind, row.kind.ink());
        at += w + 8.0;
    }
    if limit - at >= 50.0 {
        at = repo_label(painter, at, mid, &row.place, (limit - at) * 0.6) + 8.0;
    }
    let stage = row.look.as_ref().and_then(|look| look.kind).map(|kind| capitalized(kind.word()));
    if let Some(stage) = stage.filter(|_| limit - at >= 40.0) {
        let text = ellipsized(
            painter,
            &stage,
            FontId::proportional(TEXT_SECONDARY),
            brand::muted(),
            limit - at,
        );
        let w = text.size().x;
        painter.galley(pos2(at, mid - text.size().y / 2.0), text, brand::muted());
        at += w + 8.0;
    }
    if let Some(why) = row.stamp.out_of_date
        && limit - at >= panel_blocks::out_of_date_width(painter)
    {
        panel_blocks::paint_out_of_date(ui, at, mid, why);
    }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

fn steps_column(
    ui: &Ui,
    painter: &Painter,
    (x, top): (f32, f32),
    width: f32,
    (row, content): (&Row, &Lines),
) {
    let mut y = top;
    if let Some((text, ink)) = &content.lead {
        let galley = ellipsized(painter, text, FontId::proportional(TEXT_SECONDARY), *ink, width);
        painter.galley(pos2(x, y + LINE_H / 2.0 - galley.size().y / 2.0), galley, *ink);
        y += LINE_H;
    }
    if !content.steps.steps.is_empty() {
        overview_steps::paint(
            ui,
            painter,
            (x, x + width, y - overview_steps::GAP),
            &content.steps,
            (crate::progress_glue::today(), row.tab_id),
        );
        y += content.steps.steps.len() as f32 * LINE_H;
    }
    if let Some(next) = &content.next {
        let galley = ellipsized(
            painter,
            next,
            FontId::proportional(TEXT_SECONDARY),
            brand::muted(),
            width - 20.0,
        );
        painter.galley(
            pos2(x + 20.0, y + LINE_H / 2.0 - galley.size().y / 2.0),
            galley,
            brand::muted(),
        );
    }
}

fn progress(painter: &Painter, (x, top): (f32, f32), width: f32, row: &Row, fill: egui::Color32) {
    if let Some(text) = progress_text(row) {
        let galley =
            ellipsized(painter, &text, FontId::proportional(TEXT_TAG), brand::soft(), width);
        painter.galley(pos2(x, top + NAME_MID - galley.size().y / 2.0), galley, brand::soft());
    }
    let Some(look) = row.look.as_ref().filter(|look| look.ring.total > 0) else { return };
    let (filled, count) = segments(look.ring.done, look.ring.total);
    let gaps = SEGMENT_GAP * count.saturating_sub(1) as f32;
    let each = ((width - gaps) / count as f32).max(1.0);
    let y = top + META_MID - BAR_H / 2.0;
    for index in 0..count {
        let seg = Rect::from_min_size(
            pos2(x + index as f32 * (each + SEGMENT_GAP), y),
            vec2(each, BAR_H),
        );
        let ink = if index < filled { fill } else { brand::item_line() };
        painter.rect_filled(seg, CornerRadius::same(2), ink);
    }
}

const FINISHED_CHROME: f32 = SECTION_PAD * 2.0 + SECTION_HEAD_H + HEAD_GAP;

/// The Finished today panel's height with `shown` rows of `total`, and its button when rows hide.
pub(crate) fn finished_height(shown: usize, total: usize) -> f32 {
    let more = if total > shown { MORE_H } else { 0.0 };
    FINISHED_CHROME + shown as f32 * FINISHED_ROW_H + more
}

/// The height the Finished today panel takes beside its rows.
pub(crate) const fn finished_chrome() -> f32 {
    FINISHED_CHROME + MORE_H
}

/// Draws the panel: the first `shown` of `done` and a button that folds the rest in or out.
pub(crate) fn finished(
    ui: &mut Ui,
    width: f32,
    done: &[Finished],
    (shown, all): (usize, bool),
    zone: &TimeZone,
) -> (Option<u64>, bool) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, finished_height(shown, done.len())), Sense::hover());
    let painter = ui.painter_at(rect);
    section_frame(&painter, rect, false);
    section_head(
        &painter,
        rect,
        &Head {
            title: "Finished today",
            caption: "",
            count: &done.len().to_string(),
            dot: Some(brand::done()),
            ink: brand::text(),
        },
    );
    let mut y = rect.top() + SECTION_PAD + SECTION_HEAD_H + HEAD_GAP;
    let mut jump = None;
    for (index, item) in done.iter().take(shown).enumerate() {
        let line = Rect::from_min_size(
            pos2(rect.left() + SECTION_PAD - 6.0, y),
            vec2(width - SECTION_PAD * 2.0 + 12.0, FINISHED_ROW_H),
        );
        let resp =
            ui.interact(line, ui.id().with(("finished-row", item.tab_id, index)), Sense::click());
        if resp.hovered() {
            painter.rect_filled(line, CornerRadius::same(6), brand::item());
        }
        finished_row(&painter, line, item, zone);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&item.text).clicked()
        {
            jump = Some(item.tab_id);
        }
        y += FINISHED_ROW_H;
    }
    let mut toggled = false;
    if done.len() > shown || all {
        let label =
            if all { "Show fewer".to_owned() } else { format!("{} more", done.len() - shown) };
        toggled = ghost(ui, pos2(rect.left() + SECTION_PAD, y + 8.0), &label);
    }
    (jump, toggled)
}

fn finished_row(painter: &Painter, line: Rect, item: &Finished, zone: &TimeZone) {
    let mid = line.center().y;
    let time = painter.text(
        pos2(line.left() + 6.0, mid),
        Align2::LEFT_CENTER,
        clock(item.at, zone),
        FontId::monospace(TEXT_SECONDARY),
        brand::faint(),
    );
    let agent = painter.layout_no_wrap(
        item.agent.clone(),
        FontId::proportional(TEXT_SECONDARY),
        brand::muted(),
    );
    let agent_w = agent.size().x.min(line.width() * 0.3);
    let agent = ellipsized(
        painter,
        &item.agent,
        FontId::proportional(TEXT_SECONDARY),
        brand::muted(),
        agent_w,
    );
    let agent_left = line.right() - 6.0 - agent.size().x;
    painter.galley(pos2(agent_left, mid - agent.size().y / 2.0), agent, brand::muted());
    let text = ellipsized(
        painter,
        &item.text,
        FontId::proportional(TEXT_SECONDARY),
        brand::step_text(),
        (agent_left - time.right() - 24.0).max(20.0),
    );
    painter.galley(pos2(time.right() + 12.0, mid - text.size().y / 2.0), text, brand::step_text());
}
