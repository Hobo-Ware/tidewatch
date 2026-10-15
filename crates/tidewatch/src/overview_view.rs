//! The Overview screen (Cmd+0): it replaces the terminal area, like the profile editor, with one
//! row per agent tab. A click on a row asks to jump to that tab. Also the icon button in the tab
//! strip that opens it, with a badge counting the tabs that need you.

use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, ScrollArea, Sense, Stroke, StrokeKind, pos2, vec2,
};
use stdusk_egui::colors;
use stdusk_egui::widgets::ICON_BTN_W;

use crate::brand;
use crate::commands::Command;
use crate::end_block_view;
use crate::mascot::{self, Pose};
use crate::overview::{Counts, Overview, Row, fmt_elapsed};
use crate::status_view::{self, Tone};

const BUTTON_H: f32 = 30.0;
const ROW_H: f32 = 42.0;
const END_LINE_H: f32 = 24.0;
const SIDE: f32 = 20.0;
const GRID_ICON: &str = "\u{E464}";
const BADGE_H: f32 = 15.0;
const MASCOT_H: f32 = 120.0;

pub(crate) const BUTTON_W: f32 = ICON_BTN_W;

/// The tab strip's Overview button. `needs` is how many tabs wait on you; none draws no badge.
pub(crate) fn button(ui: &mut egui::Ui, active: bool, needs: usize) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON_W, BUTTON_H), Sense::click());
    let hovered = resp.hovered();
    if active {
        ui.painter().rect_filled(rect, 6.0, colors::selection());
    } else if hovered {
        ui.painter().rect_filled(rect, 6.0, colors::hover());
    }
    let ink = if active {
        brand::TEAL
    } else if hovered {
        colors::fg()
    } else {
        colors::dim()
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        GRID_ICON,
        FontId::proportional(17.0),
        ink,
    );
    if needs > 0 {
        paint_badge(ui, rect, needs);
    }
    let keys = Command::Overview.keys();
    let tip = match needs {
        0 => format!("Overview ({keys})"),
        n => format!("Overview ({keys}) - {n} need you"),
    };
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_badge(ui: &egui::Ui, button: Rect, count: usize) {
    let text = if count > 9 { "9+".to_owned() } else { count.to_string() };
    let font = FontId::monospace(9.5);
    let galley = ui.painter().layout_no_wrap(text, font, colors::bg());
    let width = (galley.size().x + 8.0).max(BADGE_H);
    let badge = Rect::from_min_size(
        pos2(button.right() - width + 4.0, button.top() - 2.0),
        vec2(width, BADGE_H),
    );
    let fill = status_view::color(Tone::Needs).unwrap_or(Color32::YELLOW);
    ui.painter().rect_filled(badge.expand(2.0), BADGE_H, colors::titlebar());
    ui.painter().rect_filled(badge, BADGE_H, fill);
    ui.painter().galley(badge.center() - galley.size() / 2.0, galley, colors::bg());
}

/// Draws the Overview in `area` and returns the tab the user clicked.
pub(crate) fn show(ui: &mut egui::Ui, area: Rect, overview: &Overview) -> Option<u64> {
    ui.painter().rect_filled(area, 0.0, colors::bg());
    let mut jump = None;
    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(area.shrink2(vec2(SIDE, 14.0))));
    ScrollArea::vertical().auto_shrink([false, false]).show(&mut content, |ui| {
        header(ui, overview);
        ui.add_space(12.0);
        if overview.counts.total() == 0 {
            empty(ui);
            return;
        }
        if !overview.needs.is_empty() {
            jump = jump.or(group(ui, "Needs you", "", &overview.needs, true));
            ui.add_space(12.0);
        }
        for place in &overview.places {
            if place.rows.is_empty() {
                continue;
            }
            jump = jump.or(group(ui, &place.label, &place.path, &place.rows, false));
            ui.add_space(12.0);
        }
    });
    jump
}

fn header(ui: &mut egui::Ui, overview: &Overview) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        "Overview",
        FontId::proportional(16.0),
        colors::fg(),
    );
    let mut x = rect.left() + 100.0;
    for (text, tone) in summary(overview.counts) {
        let color = status_view::color(tone).unwrap_or_else(colors::dim);
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), color);
        let width = galley.size().x;
        painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
        x += width + 14.0;
    }
    painter.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        Command::Overview.keys(),
        FontId::monospace(11.0),
        colors::dim(),
    );
}

fn summary(counts: Counts) -> Vec<(String, Tone)> {
    let total = counts.total();
    let noun = if total == 1 { "agent" } else { "agents" };
    let mut parts = vec![(format!("{total} {noun}"), Tone::Shell)];
    for (n, label, tone) in [
        (counts.working, "working", Tone::Working),
        (counts.needs, "need you", Tone::Needs),
        (counts.done, "done", Tone::Done),
        (counts.failed, "failed", Tone::Failed),
    ] {
        if n > 0 {
            parts.push((format!("{n} {label}"), tone));
        }
    }
    parts
}

fn empty(ui: &mut egui::Ui) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        let size = vec2(mascot::width_at(Pose::Asleep, MASCOT_H), MASCOT_H);
        let (area, _) = ui.allocate_exact_size(size, Sense::hover());
        mascot::show(ui, area, Pose::Asleep);
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(format!(
                "Nothing running. Start an agent with {}.",
                Command::NewTab.keys()
            ))
            .size(13.0)
            .color(colors::dim()),
        );
    });
}

fn group(ui: &mut egui::Ui, title: &str, path: &str, rows: &[Row], needs: bool) -> Option<u64> {
    let width = ui.available_width();
    let head_h = 30.0;
    let height = head_h + rows.iter().map(row_height).sum::<f32>();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(10), colors::elevated());
    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, colors::border()),
        StrokeKind::Inside,
    );
    let title_color = if needs {
        status_view::color(Tone::Needs).unwrap_or_else(colors::fg)
    } else {
        colors::fg()
    };
    let galley = painter.layout_no_wrap(title.to_owned(), FontId::proportional(12.5), title_color);
    let title_w = galley.size().x;
    painter.galley(
        pos2(rect.left() + 12.0, rect.top() + head_h / 2.0 - galley.size().y / 2.0),
        galley,
        title_color,
    );
    if !path.is_empty() {
        painter.text(
            pos2(rect.left() + 12.0 + title_w + 10.0, rect.top() + head_h / 2.0),
            Align2::LEFT_CENTER,
            path,
            FontId::monospace(11.0),
            colors::dim(),
        );
    }
    painter.text(
        pos2(rect.right() - 12.0, rect.top() + head_h / 2.0),
        Align2::RIGHT_CENTER,
        format!("{} tab{}", rows.len(), if rows.len() == 1 { "" } else { "s" }),
        FontId::proportional(11.5),
        colors::dim(),
    );
    let mut jump = None;
    let mut top = rect.top() + head_h;
    for row in rows {
        let row_rect = Rect::from_min_size(pos2(rect.left(), top), vec2(width, row_height(row)));
        if draw_row(ui, row_rect, row, needs) {
            jump = Some(row.tab_id);
        }
        top += row_height(row);
    }
    jump
}

fn row_height(row: &Row) -> f32 {
    if row.end_block.is_some() { ROW_H + END_LINE_H } else { ROW_H }
}

fn draw_row(ui: &mut egui::Ui, rect: Rect, row: &Row, in_needs_group: bool) -> bool {
    let resp = ui.interact(rect, ui.id().with(("overview_row", row.tab_id)), Sense::click());
    let painter = ui.painter_at(rect);
    painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, colors::border()));
    if resp.hovered() {
        painter.rect_filled(rect.shrink2(vec2(1.0, 0.0)), 0.0, colors::hover_elevated());
    }
    let tone = status_view::tone(&row.status);
    let accent = status_view::color(tone).unwrap_or_else(colors::dim);
    let mid = rect.top() + ROW_H / 2.0;
    painter.circle_filled(pos2(rect.left() + 18.0, mid), 4.5, accent);
    if let Some(block) = &row.end_block {
        let line = Rect::from_min_max(
            pos2(rect.left() + 36.0, rect.top() + ROW_H - 4.0),
            pos2(rect.right() - 12.0, rect.bottom() - 2.0),
        );
        end_block_view::paint_line(&painter, line, block);
    }

    let title_x = rect.left() + 36.0;
    painter.text(
        pos2(title_x, mid - 8.0),
        Align2::LEFT_CENTER,
        &row.title,
        FontId::proportional(13.0),
        colors::fg(),
    );
    let what = row.workflow.as_deref().unwrap_or(&row.profile);
    let sub = if in_needs_group { format!("{} - {what}", row.place) } else { what.to_owned() };
    painter.text(
        pos2(title_x, mid + 8.0),
        Align2::LEFT_CENTER,
        sub,
        FontId::proportional(11.0),
        colors::dim(),
    );

    let right = rect.right() - 12.0;
    let pill_w = 84.0;
    let pill = Rect::from_center_size(pos2(right - pill_w / 2.0, mid), vec2(pill_w, 20.0));
    painter.rect_filled(pill, 10.0, accent.gamma_multiply(0.16));
    painter.text(
        pill.center(),
        Align2::CENTER_CENTER,
        status_view::label(tone),
        FontId::proportional(11.0),
        accent,
    );
    let time_right = pill.left() - 14.0;
    painter.text(
        pos2(time_right, mid),
        Align2::RIGHT_CENTER,
        fmt_elapsed(row.elapsed),
        FontId::monospace(11.0),
        colors::dim(),
    );
    let branch_right = time_right - 70.0;
    let branch_w = 190.0_f32.min((rect.width() * 0.22).max(80.0));
    clipped(
        &painter,
        pos2(branch_right - branch_w, mid),
        branch_w,
        &row.branch,
        FontId::monospace(11.0),
        colors::dim(),
    );
    let detail_left = title_x + 200.0;
    let detail_w = (branch_right - branch_w - 16.0 - detail_left).max(0.0);
    if detail_w > 20.0
        && let Some(detail) =
            row.detail.as_deref().and_then(|d| d.lines().find(|l| !l.trim().is_empty()))
    {
        let color = if tone == Tone::Needs { accent } else { colors::dim() };
        clipped(
            &painter,
            pos2(detail_left, mid),
            detail_w,
            detail.trim(),
            FontId::monospace(11.5),
            color,
        );
    }
    let tip = status_view::describe(&row.status);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked()
}

fn clipped(
    painter: &egui::Painter,
    left_center: egui::Pos2,
    width: f32,
    text: &str,
    font: FontId,
    color: Color32,
) {
    let clip = Rect::from_min_size(pos2(left_center.x, left_center.y - 12.0), vec2(width, 24.0));
    painter.with_clip_rect(clip.intersect(painter.clip_rect())).text(
        left_center,
        Align2::LEFT_CENTER,
        text,
        font,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overview::{Place, Row};
    use egui::{Event, PointerButton, RawInput, pos2};
    use tidewatch_core::status::{AgentStatus, Reason};

    fn row(id: u64, title: &str, status: AgentStatus) -> Row {
        Row {
            tab_id: id,
            title: title.into(),
            place: "web".into(),
            profile: "Builder".into(),
            workflow: None,
            agent_id: "claude".into(),
            branch: format!("agent/{title}"),
            status,
            detail: Some("Bash: touch hello.txt".into()),
            end_block: None,
            elapsed: 242.0,
        }
    }

    fn overview(rows: Vec<Row>) -> Overview {
        let needs =
            rows.iter().filter(|r| matches!(r.status, AgentStatus::NeedsYou(_))).cloned().collect();
        let rest =
            rows.into_iter().filter(|r| !matches!(r.status, AgentStatus::NeedsYou(_))).collect();
        let counts = Counts { working: 1, needs: 1, ..Counts::default() };
        Overview {
            needs,
            places: vec![Place { label: "web".into(), path: "/work/web".into(), rows: rest }],
            counts,
        }
    }

    fn frame(ctx: &egui::Context, data: &Overview, events: Vec<Event>) -> Option<u64> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut jump = None;
        let _ = ctx.run_ui(raw, |ui| {
            let area = ui.max_rect();
            jump = show(ui, area, data);
        });
        jump
    }

    fn click_at(ctx: &egui::Context, data: &Overview, y: f32) -> Option<u64> {
        let at = pos2(300.0, y);
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, data, vec![Event::PointerMoved(at)]);
        frame(ctx, data, vec![button(true)]);
        frame(ctx, data, vec![button(false)])
    }

    #[test]
    fn clicking_a_row_asks_for_its_tab_and_nothing_else_does() {
        let ctx = egui::Context::default();
        let data = overview(vec![
            row(4, "needs-you", AgentStatus::NeedsYou(Reason::Permission)),
            row(9, "busy", AgentStatus::Working),
        ]);
        frame(&ctx, &data, vec![]);
        let mut hits = Vec::new();
        let mut y = 20.0;
        while y < 560.0 {
            if let Some(tab) = click_at(&ctx, &data, y) {
                hits.push(tab);
            }
            y += 6.0;
        }
        hits.dedup();
        assert_eq!(hits, [4, 9]);
    }

    #[test]
    fn an_empty_overview_has_nothing_to_click() {
        let ctx = egui::Context::default();
        let data = Overview {
            needs: vec![],
            places: vec![Place { label: "web".into(), path: "/work/web".into(), rows: vec![] }],
            counts: Counts::default(),
        };
        frame(&ctx, &data, vec![]);
        for y in [30.0, 100.0, 300.0] {
            assert_eq!(click_at(&ctx, &data, y), None);
        }
    }

    #[test]
    fn the_summary_names_only_the_counts_that_are_not_zero() {
        let counts = Counts { setup: 0, working: 2, needs: 1, done: 0, failed: 0 };
        let texts: Vec<String> = summary(counts).into_iter().map(|(text, _)| text).collect();
        assert_eq!(texts, ["3 agents", "2 working", "1 need you"]);
        let one = summary(Counts { done: 1, ..Counts::default() });
        assert_eq!(one[0].0, "1 agent");
    }
}
