//! The Overview screen (Cmd+0): it replaces the terminal area, like the profile editor, with one
//! row per agent tab. A click on a row asks to jump to that tab. Also the icon button in the tab
//! strip that opens it, with a badge counting the tabs that need you.

use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, ScrollArea, Sense, Shadow, Stroke, StrokeKind,
    pos2, vec2,
};
use stdusk_egui::colors;
use stdusk_egui::ui::ellipsize;

use crate::agent_avatar::{self, Face};
use crate::app::strip::{ICON_BOX, ICON_GLYPH};
use crate::brand;
use crate::coming_up_view;
use crate::commands::Command;
use crate::mascot::{self, Pose};
use crate::overview::{Counts, Overview, PoofedRow, Row, card_view, fmt_elapsed};
use crate::overview_checklist::{Checklist, Chip};
use crate::pr_strip::tone_ink;
use crate::progress_ring;
use crate::progress_view::flow;
use crate::reader_view::ellipsized;
use crate::screen_kit::{Size, button_width, heading, quiet_button};
use crate::status_view::{self, Tone};
use crate::timeline::Block;

const BUTTON_H: f32 = ICON_BOX;
const SIDE: f32 = crate::layout::EDGE;
const SECTION_GAP: f32 = 22.0;
const CARD_MIN_W: f32 = 300.0;
const MAX_COLUMNS: usize = 4;
const CARD_GAP: f32 = 14.0;
const CARD_H: f32 = 166.0;
const CARD_PAD: f32 = 20.0;
const CARD_RADIUS: u8 = 14;
const CARD_SHADOW: f32 = 36.0;
const RING: f32 = 52.0;
const TILE: f32 = 40.0;
const DOT: f32 = 8.0;
const COUNT_GAP: f32 = 18.0;
const TAG_H: f32 = 22.0;
const GROUP_HEAD_H: f32 = 34.0;
const CHIP_H: f32 = 30.0;
const CHIP_GAP: f32 = 8.0;
const CHIP_PAD: f32 = 13.0;
const CHIP_CHARS: usize = 24;
const HEADER_H: f32 = 48.0;
const GRID_ICON: &str = "\u{E464}";
const BADGE_H: f32 = 17.0;
const MASCOT_H: f32 = 120.0;

pub(crate) const BUTTON_W: f32 = ICON_BOX;

/// The tab strip's Overview button. `needs` is how many tabs wait on you in each repo; none draws no
/// badge.
pub(crate) fn button(ui: &mut egui::Ui, active: bool, needs: &[(String, usize)]) -> egui::Response {
    let total: usize = needs.iter().map(|(_, count)| count).sum();
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON_W, BUTTON_H), Sense::click());
    let hovered = resp.hovered();
    if active {
        ui.painter().rect_filled(rect, 8.0, colors::selection());
    } else if hovered {
        ui.painter().rect_filled(rect, 8.0, colors::hover());
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
        FontId::proportional(ICON_GLYPH),
        ink,
    );
    if total > 0 {
        paint_badge(ui, rect, total);
    }
    resp.on_hover_text(button_tip(&Command::Overview.keys(), needs))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn button_tip(keys: &str, needs: &[(String, usize)]) -> String {
    let waiting: Vec<&(String, usize)> = needs.iter().filter(|(_, count)| *count > 0).collect();
    let total: usize = waiting.iter().map(|(_, count)| count).sum();
    let who = match total {
        1 => "1 agent needs".to_owned(),
        n => format!("{n} agents need"),
    };
    match waiting.as_slice() {
        [] => format!("Overview ({keys})"),
        [(repo, _)] => format!("Overview ({keys})\n{who} you in {repo}"),
        several => {
            let split: Vec<String> =
                several.iter().map(|(repo, count)| format!("{count} in {repo}")).collect();
            format!("Overview ({keys})\n{who} you: {}", split.join(", "))
        }
    }
}

fn paint_badge(ui: &egui::Ui, button: Rect, count: usize) {
    let text = if count > 9 { "9+".to_owned() } else { count.to_string() };
    let font = FontId::monospace(11.0);
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

/// What the Overview shows below the agents.
#[derive(Default)]
pub(crate) struct Sections<'a> {
    pub(crate) coming_up: &'a [Block],
    pub(crate) checklist: Option<&'a Checklist>,
    pub(crate) poofed: &'a [PoofedRow],
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Out {
    /// The tab whose card was clicked.
    pub(crate) jump: Option<u64>,
    /// The row of the Coming up card, or of its button, that was clicked.
    pub(crate) coming_up: Option<usize>,
    /// Edit on the checklist row.
    pub(crate) edit_checklist: bool,
}

/// Draws the Overview in `area` and returns what the user clicked. `below` draws last, inside the
/// scrolling list.
pub(crate) fn show(
    ui: &mut egui::Ui,
    area: Rect,
    overview: &Overview,
    sections: &Sections<'_>,
    below: impl FnOnce(&mut egui::Ui),
) -> Out {
    ui.painter().rect_filled(area, 0.0, brand::GROUND);
    header(ui, area, overview);
    let mut out = Out::default();
    let body = Rect::from_min_max(
        pos2(area.left() + SIDE, area.top() + HEADER_H + 14.0),
        pos2(area.right() - SIDE, area.bottom() - 14.0),
    );
    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(body));
    ScrollArea::vertical().auto_shrink([false, false]).show(&mut content, |ui| {
        if overview.counts.total() == 0 {
            empty(ui);
        }
        if !overview.needs.is_empty() {
            let several = single_repo(overview).is_none();
            out.jump = out.jump.or(group(ui, "Needs you", "", &overview.needs, (true, several)));
            ui.add_space(SECTION_GAP);
        }
        for place in &overview.places {
            if place.rows.is_empty() {
                continue;
            }
            out.jump =
                out.jump.or(group(ui, &place.label, &place.path, &place.rows, (false, false)));
            ui.add_space(SECTION_GAP);
        }
        if !sections.poofed.is_empty() {
            poofed_list(ui, sections.poofed);
            ui.add_space(SECTION_GAP);
        }
        if !sections.coming_up.is_empty() {
            out.coming_up = coming_up_view::show(ui, sections.coming_up);
            ui.add_space(SECTION_GAP);
        }
        if let Some(checklist) = sections.checklist {
            out.edit_checklist = checklist_row(ui, checklist);
        }
        ui.add_space(SECTION_GAP);
        below(ui);
    });
    out
}

fn header(ui: &egui::Ui, area: Rect, overview: &Overview) {
    let rect = Rect::from_min_size(area.min, vec2(area.width(), HEADER_H));
    let painter = ui.painter_at(rect);
    let title = painter.text(
        pos2(rect.left() + SIDE, rect.center().y),
        Align2::LEFT_CENTER,
        "Overview",
        heading(16.0),
        brand::TEXT,
    );
    let mut left = title.right();
    if let Some(repo) = single_repo(overview) {
        let galley =
            painter.layout_no_wrap(repo.to_owned(), FontId::proportional(13.0), brand::MUTED);
        let at = pos2(left + 14.0, rect.center().y - galley.size().y / 2.0 + 1.0);
        left = at.x + galley.size().x;
        if left + COUNT_GAP < rect.right() - SIDE - COUNT_GAP {
            painter.galley(at, galley, brand::MUTED);
        } else {
            left = title.right();
        }
    }
    let counts: Vec<_> = counts_row(overview.counts)
        .into_iter()
        .map(|(text, tone)| {
            let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), brand::SOFT);
            (galley, tone)
        })
        .collect();
    let widths: Vec<f32> = counts.iter().map(|(galley, _)| galley.size().x + 8.0 + DOT).collect();
    let room = rect.right() - SIDE - left - COUNT_GAP;
    let kept = fitting(&widths, room);
    let mut right = rect.right() - SIDE;
    let shown: Vec<_> = counts.into_iter().zip(widths).take(kept).collect();
    for ((galley, tone), width) in shown.into_iter().rev() {
        let color = status_view::color(tone).unwrap_or(brand::MUTED);
        let y = rect.center().y;
        painter.galley(
            pos2(right - width + 8.0 + DOT, y - galley.size().y / 2.0),
            galley,
            brand::SOFT,
        );
        painter.circle_filled(pos2(right - width + DOT / 2.0, y), DOT / 2.0, color);
        right -= width + COUNT_GAP;
    }
    painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, brand::CARD_LINE));
}

fn fitting(widths: &[f32], room: f32) -> usize {
    let mut used = 0.0;
    widths
        .iter()
        .take_while(|width| {
            used += **width + COUNT_GAP;
            used - COUNT_GAP <= room
        })
        .count()
}

fn single_repo(overview: &Overview) -> Option<&str> {
    match overview.places.as_slice() {
        [place] => Some(&place.label),
        _ => None,
    }
}

fn counts_row(counts: Counts) -> Vec<(String, Tone)> {
    let mut parts = Vec::new();
    if counts.failed > 0 {
        parts.push((format!("{} failed", counts.failed), Tone::Failed));
    }
    parts.push((format!("{} need you", counts.needs), Tone::Needs));
    parts.push((format!("{} working", counts.working + counts.setup), Tone::Working));
    parts.push((format!("{} idle", counts.done), Tone::Done));
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
            .color(brand::MUTED),
        );
    });
}

fn columns(width: f32) -> usize {
    (((width + CARD_GAP) / (CARD_MIN_W + CARD_GAP)).floor() as usize).clamp(1, MAX_COLUMNS)
}

fn group(
    ui: &mut egui::Ui,
    title: &str,
    path: &str,
    rows: &[Row],
    (needs, name_the_repo): (bool, bool),
) -> Option<u64> {
    let width = ui.available_width();
    let columns = columns(width);
    let card_w = (width - CARD_GAP * (columns - 1) as f32) / columns as f32;
    let lines = rows.len().div_ceil(columns);
    let height = GROUP_HEAD_H + lines as f32 * CARD_H + lines.saturating_sub(1) as f32 * CARD_GAP;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    let title_color =
        if needs { status_view::color(Tone::Needs).unwrap_or(brand::TEXT) } else { brand::TEXT };
    let galley = painter.layout_no_wrap(title.to_owned(), heading(14.0), title_color);
    let title_w = galley.size().x;
    let mid = rect.top() + (GROUP_HEAD_H - 10.0) / 2.0;
    painter.galley(pos2(rect.left(), mid - galley.size().y / 2.0), galley, title_color);
    if !path.is_empty() {
        painter.text(
            pos2(rect.left() + title_w + 10.0, mid),
            Align2::LEFT_CENTER,
            path,
            FontId::monospace(13.0),
            brand::MUTED,
        );
    }
    painter.text(
        pos2(rect.right(), mid),
        Align2::RIGHT_CENTER,
        format!("{} tab{}", rows.len(), if rows.len() == 1 { "" } else { "s" }),
        FontId::proportional(13.0),
        brand::MUTED,
    );
    let mut jump = None;
    for (index, row) in rows.iter().enumerate() {
        let (line, column) = (index / columns, index % columns);
        let at = pos2(
            rect.left() + column as f32 * (card_w + CARD_GAP),
            rect.top() + GROUP_HEAD_H + line as f32 * (CARD_H + CARD_GAP),
        );
        if card(ui, Rect::from_min_size(at, vec2(card_w, CARD_H)), row, needs, name_the_repo) {
            jump = Some(row.tab_id);
        }
    }
    jump
}

fn card(
    ui: &mut egui::Ui,
    rect: Rect,
    row: &Row,
    in_needs_group: bool,
    name_the_repo: bool,
) -> bool {
    let resp = ui.interact(rect, ui.id().with(("overview_card", row.tab_id)), Sense::click());
    let view = card_view(row);
    let painter = ui.painter_at(rect.expand(CARD_SHADOW));
    painter.add(
        Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(64) }
            .as_shape(rect, CornerRadius::same(CARD_RADIUS)),
    );
    let line = if resp.hovered() { brand::FIELD_LINE } else { brand::CARD_LINE };
    painter.rect(
        rect,
        CornerRadius::same(CARD_RADIUS),
        brand::CARD,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let inner = rect.shrink(CARD_PAD);
    let ring_center = pos2(inner.left() + RING / 2.0, inner.top() + RING / 2.0);
    if row.look.is_some() && view.fraction > 0.0 {
        progress_ring::paint(
            &painter,
            ring_center,
            RING,
            view.fraction,
            view.ring,
            brand::CARD_LINE,
        );
    }
    let text_x = inner.left() + RING + 14.0;
    let room = (inner.right() - text_x).max(1.0);
    let what = row.workflow.as_deref().unwrap_or(&row.profile);
    let sub = if in_needs_group && name_the_repo {
        format!("{} \u{b7} {what}", row.place)
    } else {
        what.to_owned()
    };
    let name = ellipsized(&painter, &row.title, heading(15.0), brand::TEXT, room);
    let sub = ellipsized(&painter, &sub, FontId::proportional(13.0), brand::MUTED, room);
    let block = name.size().y + 2.0 + sub.size().y;
    let top = ring_center.y - block / 2.0;
    let sub_top = top + name.size().y + 2.0;
    painter.galley(pos2(text_x, top), name, brand::TEXT);
    painter.galley(pos2(text_x, sub_top), sub, brand::MUTED);
    let line_top = inner.top() + RING + 16.0;
    let said =
        ellipsized(&painter, &view.line, FontId::proportional(14.0), view.line_ink, inner.width());
    painter.galley(pos2(inner.left(), line_top), said, view.line_ink);
    let tag_mid = inner.bottom() - TAG_H / 2.0;
    let tag_right = paint_pill(&painter, inner.left(), tag_mid, &view.tag, view.tag_ink);
    let clock =
        painter.layout_no_wrap(fmt_elapsed(row.elapsed), FontId::monospace(13.0), brand::MUTED);
    let clock_left = inner.right() - clock.size().x;
    let mut pill_right = tag_right;
    let pills = row.pr.iter().map(|(tone, text)| (*tone, text.as_str())).chain(row.done_check);
    for (tone, text) in pills {
        let ink = tone_ink(tone);
        let width =
            painter.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), ink).size().x
                + 20.0;
        if pill_right + 8.0 + width < clock_left - 8.0 {
            pill_right = paint_pill(&painter, pill_right + 8.0, tag_mid, text, ink);
        }
    }
    painter.galley(pos2(clock_left, tag_mid - clock.size().y / 2.0), clock, brand::MUTED);
    let face = Face::of(&row.profile, &row.agent_id, row.avatar.clone());
    let tile = Rect::from_center_size(ring_center, vec2(TILE, TILE));
    if agent_avatar::paint_face(ui.ctx(), &painter, tile, &face)
        && let Some(avatar) = face.avatar()
    {
        agent_avatar::hover(ui, tile, row.tab_id, avatar);
    }
    if row.avatar.is_some() {
        agent_avatar::menu(&resp, row.tab_id);
    }
    let tip = status_view::describe(&row.status);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked()
}

/// A tinted pill whose left edge is at `left`, centered on `mid`. Returns its right edge.
fn paint_pill(painter: &egui::Painter, left: f32, mid: f32, text: &str, ink: Color32) -> f32 {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), ink);
    let size = vec2(galley.size().x + 20.0, TAG_H);
    let pill = Rect::from_min_size(pos2(left, mid - TAG_H / 2.0), size);
    painter.rect_filled(pill, CornerRadius::same(255), ink.gamma_multiply(0.12));
    painter.galley(pill.center() - galley.size() / 2.0, galley, ink);
    pill.right()
}

const POOFED_ROW_H: f32 = 26.0;

fn poofed_list(ui: &mut egui::Ui, rows: &[PoofedRow]) {
    let width = ui.available_width();
    let (head, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    ui.painter_at(head).text(
        head.left_center(),
        Align2::LEFT_CENTER,
        "Poofed today",
        heading(16.0),
        brand::TEXT,
    );
    ui.add_space(6.0);
    for row in rows {
        let (rect, response) = ui.allocate_exact_size(vec2(width, POOFED_ROW_H), Sense::hover());
        let painter = ui.painter_at(rect);
        let detail =
            painter.layout_no_wrap(row.detail.clone(), FontId::proportional(13.0), brand::MUTED);
        let room = (width - detail.size().x - 14.0).max(1.0);
        let name = ellipsized(&painter, &row.name, FontId::proportional(13.0), brand::SOFT, room);
        painter.galley(pos2(rect.left(), rect.center().y - name.size().y / 2.0), name, brand::SOFT);
        painter.galley(
            pos2(rect.right() - detail.size().x, rect.center().y - detail.size().y / 2.0),
            detail,
            brand::MUTED,
        );
        response.on_hover_text(&row.summary);
    }
}

fn chip_title(chip: &Chip) -> String {
    let (short, _) = ellipsize(&chip.title, CHIP_CHARS);
    short
}

fn checklist_row(ui: &mut egui::Ui, checklist: &Checklist) -> bool {
    let width = ui.available_width();
    let (head, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    let painter = ui.painter_at(head);
    let title = painter.text(
        head.left_center(),
        Align2::LEFT_CENTER,
        "Checklist",
        heading(16.0),
        brand::TEXT,
    );
    painter.text(
        pos2(title.right() + 14.0, head.center().y + 1.0),
        Align2::LEFT_CENTER,
        checklist.source.caption(),
        FontId::proportional(13.0),
        brand::MUTED,
    );
    ui.add_space(10.0);
    let font = FontId::proportional(13.0);
    let labels: Vec<(String, String)> = checklist
        .chips
        .iter()
        .map(|chip| (chip.number.map_or_else(String::new, |n| n.to_string()), chip_title(chip)))
        .collect();
    let measure = ui.painter().clone();
    let number_w = |text: &str| {
        if text.is_empty() {
            0.0
        } else {
            measure.layout_no_wrap(text.to_owned(), font.clone(), brand::FAINT).size().x + 6.0
        }
    };
    let mut widths: Vec<f32> = labels
        .iter()
        .map(|(number, title)| {
            let text = measure.layout_no_wrap(title.clone(), font.clone(), brand::TEXT);
            (number_w(number) + text.size().x + 2.0 * CHIP_PAD).min(width)
        })
        .collect();
    let edit_w = button_width(ui, "Edit", Size::Compact);
    widths.push(edit_w);
    let (places, height) = flow(&widths, width, CHIP_H, CHIP_GAP);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    for (index, (number, title)) in labels.iter().enumerate() {
        let (x, y) = places[index];
        let chip = Rect::from_min_size(rect.min + vec2(x, y), vec2(widths[index], CHIP_H));
        painter.rect_filled(chip, CornerRadius::same(255), brand::CARD);
        let mut left = chip.left() + CHIP_PAD;
        if !number.is_empty() {
            let galley = painter.layout_no_wrap(number.clone(), font.clone(), brand::FAINT);
            painter.galley(
                pos2(left, chip.center().y - galley.size().y / 2.0),
                galley,
                brand::FAINT,
            );
            left += number_w(number);
        }
        let galley = painter.layout_no_wrap(title.clone(), font.clone(), brand::TEXT);
        painter.galley(pos2(left, chip.center().y - galley.size().y / 2.0), galley, brand::TEXT);
        let full = &checklist.chips[index].title;
        if full != title {
            ui.interact(chip, ui.id().with(("checklist-chip", index)), Sense::hover())
                .on_hover_text(full);
        }
    }
    let (x, y) = places[labels.len()];
    let edit = Rect::from_min_size(
        rect.min + vec2(x, y + (CHIP_H - Size::Compact.height()) / 2.0),
        vec2(edit_w, Size::Compact.height()),
    );
    let mut clicked = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(edit), |ui| {
        let response = quiet_button(ui, "Edit", true, Size::Compact);
        clicked = response.clicked();
        response.on_hover_text(checklist.file.display().to_string());
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_tip_says_what_the_badge_counts() {
        let repo = |name: &str, count: usize| (name.to_owned(), count);
        let cases = [
            (vec![], "Overview (K)"),
            (vec![repo("acme-web", 0)], "Overview (K)"),
            (vec![repo("acme-web", 1)], "Overview (K)\n1 agent needs you in acme-web"),
            (vec![repo("acme-web", 2)], "Overview (K)\n2 agents need you in acme-web"),
            (
                vec![repo("acme-web", 3), repo("acme-api", 0), repo("acme-docs", 1)],
                "Overview (K)\n4 agents need you: 3 in acme-web, 1 in acme-docs",
            ),
        ];
        for (needs, want) in cases {
            assert_eq!(button_tip("K", &needs), want, "{needs:?}");
        }
    }
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
            pr: None,
            done_check: None,
            look: None,
            job_line: None,
            avatar: None,
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
        frame_with(ctx, data, &Sections::default(), events).jump
    }

    fn frame_with(
        ctx: &egui::Context,
        data: &Overview,
        sections: &Sections<'_>,
        events: Vec<Event>,
    ) -> Out {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut out = Out::default();
        let _ = ctx.run_ui(raw, |ui| {
            let area = ui.max_rect();
            out = show(ui, area, data, sections, |_| {});
        });
        out
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
        let ctx = crate::fonts::test_context();
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
        let ctx = crate::fonts::test_context();
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
    fn the_counts_row_always_names_need_you_working_and_idle() {
        let counts = Counts { setup: 1, working: 2, needs: 1, done: 0, failed: 0 };
        let texts: Vec<String> = counts_row(counts).into_iter().map(|(text, _)| text).collect();
        assert_eq!(texts, ["1 need you", "3 working", "0 idle"]);
        let failed = counts_row(Counts { failed: 2, ..Counts::default() });
        assert_eq!(failed[0].0, "2 failed");
        assert_eq!(failed.len(), 4);
    }

    #[test]
    fn the_repo_is_named_in_the_header_only_when_there_is_one() {
        let place = |label: &str| Place { label: label.into(), path: String::new(), rows: vec![] };
        let one = Overview { needs: vec![], places: vec![place("web")], counts: Counts::default() };
        assert_eq!(single_repo(&one), Some("web"));
        let two = Overview {
            needs: vec![],
            places: vec![place("web"), place("api")],
            counts: Counts::default(),
        };
        assert_eq!(single_repo(&two), None);
    }

    fn chips(titles: &[&str]) -> Checklist {
        Checklist {
            chips: titles
                .iter()
                .enumerate()
                .map(|(i, title)| Chip { number: Some(i as u32 + 1), title: (*title).into() })
                .collect(),
            source: crate::overview_checklist::Source::Default,
            file: "/cfg/checklist.md".into(),
        }
    }

    fn walk(shape: &egui::Shape, found: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(text) if !text.galley.text().trim().is_empty() => found.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, found)),
            _ => {}
        }
    }

    fn texts(
        ctx: &egui::Context,
        data: &Overview,
        sections: &Sections<'_>,
        width: f32,
    ) -> Vec<(String, Rect)> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 900.0))),
            focused: true,
            ..RawInput::default()
        };
        let mut found = Vec::new();
        let output = ctx.run_ui(raw, |ui| {
            let area = ui.max_rect();
            show(ui, area, data, sections, |_| {});
        });
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut found);
        }
        found
    }

    #[test]
    fn cards_flow_into_columns_by_width_up_to_four() {
        let cases = [(250.0, 1), (300.0, 1), (614.0, 2), (928.0, 3), (1242.0, 4), (2000.0, 4)];
        for (width, want) in cases {
            assert_eq!(columns(width), want, "{width}");
        }
    }

    #[test]
    fn the_checklist_row_names_each_phase_and_edit_asks_to_open_the_file() {
        let ctx = crate::fonts::test_context();
        let data = overview(vec![row(9, "busy", AgentStatus::Working)]);
        let list =
            chips(&["Understand and decide", "Build", "A phase with a very long title indeed"]);
        let sections = Sections { checklist: Some(&list), ..Sections::default() };
        frame_with(&ctx, &data, &sections, vec![]);
        let found = texts(&ctx, &data, &sections, 1000.0);
        let names: Vec<&str> = found.iter().map(|(text, _)| text.as_str()).collect();
        for want in
            ["Checklist", "Task to done \u{b7} default", "Understand and decide", "Build", "Edit"]
        {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
        assert!(
            names
                .iter()
                .any(|text| text.starts_with("A phase with a very") && text.ends_with('\u{2026}'))
        );
        let edit = found.iter().find(|(text, _)| text == "Edit").unwrap().1.center();
        let button = |pressed| Event::PointerButton {
            pos: edit,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(&ctx, &data, &sections, vec![Event::PointerMoved(edit)]);
        frame_with(&ctx, &data, &sections, vec![button(true)]);
        let out = frame_with(&ctx, &data, &sections, vec![button(false)]);
        assert!(out.edit_checklist);
        assert_eq!(out.jump, None);
    }

    #[test]
    fn no_card_text_overlaps_at_any_window_width() {
        let ctx = crate::fonts::test_context();
        let mut with_ledger =
            row(4, "needs-you-with-a-long-name-here", AgentStatus::NeedsYou(Reason::Permission));
        with_ledger.look = Some(crate::progress_model::look(
            &tidewatch_core::progress::Ledger::parse(
                "# Goal\nx\n\n## Status\nphase: 2 Build\nstate: working\nnote: a rather long note about what is happening right now in the build\n\n## Checklist\n### 2. Build\n- [~] a\n\n## Needs you\n- Pick\n",
            ),
        ));
        with_ledger.pr = Some((tidewatch_core::pr_status::Tone::Bad, "#42 checks failed".into()));
        let data = overview(vec![with_ledger, row(9, "busy", AgentStatus::Working)]);
        let list = chips(&["One", "Two", "Three"]);
        for width in [360.0, 700.0, 1000.0, 1440.0] {
            let sections = Sections { checklist: Some(&list), ..Sections::default() };
            let found = texts(&ctx, &data, &sections, width);
            for (index, (a, a_rect)) in found.iter().enumerate() {
                for (b, b_rect) in &found[index + 1..] {
                    assert!(
                        !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                        "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {width}"
                    );
                }
            }
        }
    }

    #[test]
    fn coming_up_sits_between_the_agents_and_the_checklist_and_hides_when_empty() {
        use crate::timeline::{Button, Card, Label, Look};
        let ctx = crate::fonts::test_context();
        let data = overview(vec![row(9, "busy", AgentStatus::Working)]);
        let list = chips(&["One"]);
        let block = Block {
            label: Label::Later,
            cards: vec![Card {
                row: 3,
                title: "Install the bot".into(),
                tab: "heron".into(),
                steps: vec![],
                look: Look::You,
                button: Some(Button::Decide),
                tip: String::new(),
            }],
        };
        let blocks = [block];
        let sections =
            Sections { coming_up: &blocks, checklist: Some(&list), ..Sections::default() };
        frame_with(&ctx, &data, &sections, vec![]);
        let found = texts(&ctx, &data, &sections, 1000.0);
        let y =
            |text: &str| found.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| rect.top());
        let (agents, coming, checklist) = (y("busy"), y("Coming up"), y("Checklist"));
        assert!(agents < coming && coming < checklist, "{agents:?} {coming:?} {checklist:?}");
        let at = found.iter().find(|(text, _)| text == "Decide").unwrap().1.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame_with(&ctx, &data, &sections, vec![Event::PointerMoved(at)]);
        frame_with(&ctx, &data, &sections, vec![button(true)]);
        let out = frame_with(&ctx, &data, &sections, vec![button(false)]);
        assert_eq!((out.coming_up, out.jump), (Some(3), None));
        let empty = Sections { coming_up: &[], checklist: Some(&list), ..Sections::default() };
        let found = texts(&ctx, &data, &empty, 1000.0);
        assert!(!found.iter().any(|(text, _)| text == "Coming up"));
    }
}
