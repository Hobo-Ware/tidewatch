//! The new-tab form's cards: profiles and workflows in two columns, the stages of the picked workflow.

use egui::{
    Align2, CornerRadius, Painter, Pos2, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use tidewatch_core::agents::Agent;
use tidewatch_core::profiles::Profile;

use crate::brand;
use crate::catalog::{Availability, Catalog};
use crate::commands::Command;
use crate::new_tab_workflow::{self, StageCard};
use crate::reader_view::{ellipsized, paint_item};
use crate::screen_kit::{body, clickable, heading, mono, paint_cli_badge};

pub(crate) const GAP: f32 = 8.0;
pub(crate) const AGENT_H: f32 = 52.0;
pub(crate) const WORKFLOW_H: f32 = 62.0;
pub(crate) const STAGE_H: f32 = 48.0;
const COLUMNS: usize = 2;
const STAGE_COLUMNS: usize = 4;
const RADIUS: u8 = 9;
const PAD: f32 = 12.0;
const BADGE: f32 = 30.0;
const CHIP_PAD: f32 = 7.0;
const CHIP_H: f32 = 20.0;
const CHIP_GAP: f32 = 4.0;
const NOT_INSTALLED: f32 = 0.45;

/// Where each of `count` cards goes: `columns` of them across `width`, rows `height` tall, from
/// `origin` down.
pub(crate) fn grid(
    origin: Pos2,
    width: f32,
    columns: usize,
    count: usize,
    height: f32,
) -> Vec<Rect> {
    let card_w = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
    (0..count)
        .map(|index| {
            let (row, column) = (index / columns, index % columns);
            let at = pos2(
                origin.x + column as f32 * (card_w + GAP),
                origin.y + row as f32 * (height + GAP),
            );
            Rect::from_min_size(at, vec2(card_w, height))
        })
        .collect()
}

pub(crate) fn grid_height(columns: usize, count: usize, height: f32) -> f32 {
    let rows = count.div_ceil(columns);
    if rows == 0 { 0.0 } else { rows as f32 * height + (rows as f32 - 1.0) * GAP }
}

/// The height the profile cards take, and the same for the workflow cards.
pub(crate) fn agents_height(count: usize) -> f32 {
    grid_height(COLUMNS, count, AGENT_H)
}

pub(crate) fn workflows_height(count: usize) -> f32 {
    grid_height(COLUMNS, count, WORKFLOW_H)
}

pub(crate) fn stages_height(count: usize) -> f32 {
    grid_height(STAGE_COLUMNS, count, STAGE_H)
}

/// `Claude Code · opus · high`, or `Claude Code · default model`.
pub(crate) fn subline(profile: &Profile, cli: &str) -> String {
    let model = profile.model.as_deref().unwrap_or("default model");
    match profile.effort.as_deref() {
        Some(effort) => format!("{cli} \u{b7} {model} \u{b7} {effort}"),
        None => format!("{cli} \u{b7} {model}"),
    }
}

fn card_subline(profile: &Profile, agent: &Agent, availability: &Availability) -> String {
    match availability {
        Availability::Missing => format!("{} \u{b7} not installed", agent.name),
        Availability::Probing | Availability::Found(_) => subline(profile, &agent.name),
    }
}

fn paint_card(painter: &Painter, rect: Rect, selected: bool, hovered: bool) {
    let (fill, line) = match (selected, hovered) {
        (true, _) => (brand::TEAL_NAV, brand::TEAL),
        (false, true) => (brand::ITEM, brand::FIELD_LINE),
        (false, false) => (brand::ITEM, brand::ITEM_LINE),
    };
    painter.rect(
        rect,
        CornerRadius::same(RADIUS),
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
}

fn paint_number(painter: &Painter, rect: Rect, index: usize) {
    if index < 9 {
        painter.text(
            pos2(rect.right() - PAD, rect.center().y),
            Align2::RIGHT_CENTER,
            (index + 1).to_string(),
            mono(11.0),
            brand::FAINT,
        );
    }
}

fn dim(painter: &Painter, rect: Rect) {
    painter.rect_filled(
        rect,
        CornerRadius::same(RADIUS),
        brand::GROUND.gamma_multiply(1.0 - NOT_INSTALLED),
    );
}

/// What a card list asks for this frame.
pub(crate) struct Cards<'a> {
    pub(crate) ids: &'a [String],
    pub(crate) selected: usize,
    pub(crate) scroll_to_selected: bool,
    pub(crate) max_height: f32,
}

/// The profile cards, in a scroll area that only scrolls past `max_height`. Returns the card
/// clicked.
pub(crate) fn agents(ui: &mut Ui, catalog: &Catalog, cards: &Cards) -> Option<usize> {
    let entries = catalog.entries();
    let width = ui.available_width();
    let mut clicked = None;
    ScrollArea::vertical().max_height(cards.max_height).auto_shrink([false, true]).show(ui, |ui| {
        let (area, _) =
            ui.allocate_exact_size(vec2(width, agents_height(cards.ids.len())), Sense::hover());
        let rects = grid(area.min, width, COLUMNS, cards.ids.len(), AGENT_H);
        for (index, (rect, id)) in rects.into_iter().zip(cards.ids).enumerate() {
            let Some((profile, agent)) = entries
                .iter()
                .find(|(entry, _, _)| entry == id)
                .map(|(_, profile, agent)| (*profile, *agent))
            else {
                continue;
            };
            let selected = index == cards.selected;
            let response = clickable(ui, rect, ("agent-card", id));
            let availability = catalog.availability(&agent.id);
            let painter = ui.painter();
            paint_card(painter, rect, selected, response.hovered());
            let badge = Rect::from_min_size(
                pos2(rect.left() + PAD, rect.center().y - BADGE / 2.0),
                vec2(BADGE, BADGE),
            );
            paint_cli_badge(painter, badge, &agent.id);
            let text_x = badge.right() + PAD;
            let room = rect.right() - PAD - 18.0 - text_x;
            let name = ellipsized(painter, &profile.name, heading(14.0), brand::TEXT, room);
            painter.galley(pos2(text_x, rect.top() + 9.0), name, brand::TEXT);
            let line = card_subline(profile, agent, &availability);
            let sub = ellipsized(painter, &line, body(12.0), brand::MUTED, room);
            painter.galley(pos2(text_x, rect.top() + 29.0), sub, brand::MUTED);
            paint_number(painter, rect, index);
            if availability == Availability::Missing {
                dim(painter, rect);
            }
            if selected && cards.scroll_to_selected {
                ui.scroll_to_rect(rect, None);
            }
            if response.clicked() {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// How many of the chips whose widths are `widths` fit one row of `avail` when the rest is
/// replaced by a `more` pill `more_w` wide.
pub(crate) fn chips_that_fit(widths: &[f32], arrow_w: f32, more_w: f32, avail: f32) -> usize {
    let row = |count: usize| -> f32 {
        let chips: f32 = widths[..count].iter().sum();
        chips + (count.saturating_sub(1) as f32) * (arrow_w + 2.0 * CHIP_GAP)
    };
    if row(widths.len()) <= avail {
        return widths.len();
    }
    (0..widths.len())
        .rev()
        .find(|count| row(*count) + arrow_w + 2.0 * CHIP_GAP + more_w <= avail)
        .unwrap_or(0)
}

fn paint_chips(painter: &Painter, rect: Rect, names: &[&str]) {
    let font = body(11.5);
    let galleys: Vec<_> = names
        .iter()
        .map(|name| painter.layout_no_wrap((*name).to_owned(), font.clone(), brand::SOFT))
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|galley| galley.size().x + 2.0 * CHIP_PAD).collect();
    let arrow = painter.layout_no_wrap("\u{203a}".to_owned(), font.clone(), brand::FAINT);
    let more_text = format!("+{}", names.len());
    let more_w =
        painter.layout_no_wrap(more_text, font.clone(), brand::SOFT).size().x + 2.0 * CHIP_PAD;
    let shown = chips_that_fit(&widths, arrow.size().x, more_w, rect.width());
    let mut x = rect.left();
    let mid = rect.center().y;
    for (index, (galley, width)) in galleys.into_iter().zip(&widths).enumerate().take(shown) {
        if index > 0 {
            x += CHIP_GAP;
            painter.galley(pos2(x, mid - arrow.size().y / 2.0), arrow.clone(), brand::FAINT);
            x += arrow.size().x + CHIP_GAP;
        }
        let chip = Rect::from_min_size(pos2(x, mid - CHIP_H / 2.0), vec2(*width, CHIP_H));
        painter.rect_filled(chip, CornerRadius::same(5), brand::SEGMENT);
        painter.galley(chip.center() - galley.size() / 2.0, galley, brand::SOFT);
        x += width;
    }
    if shown < names.len() {
        let rest = names.len() - shown;
        x += CHIP_GAP;
        painter.text(pos2(x, mid), Align2::LEFT_CENTER, format!("+{rest}"), font, brand::FAINT);
    }
}

/// The workflow cards: name, number key and the stages as chips. Returns the card clicked.
pub(crate) fn workflows(ui: &mut Ui, catalog: &Catalog, cards: &Cards) -> Option<usize> {
    let all = catalog.workflows();
    let width = ui.available_width();
    let mut clicked = None;
    ScrollArea::vertical().max_height(cards.max_height).auto_shrink([false, true]).show(ui, |ui| {
        let (area, _) =
            ui.allocate_exact_size(vec2(width, workflows_height(cards.ids.len())), Sense::hover());
        let rects = grid(area.min, width, COLUMNS, cards.ids.len(), WORKFLOW_H);
        for (index, (rect, id)) in rects.into_iter().zip(cards.ids).enumerate() {
            let Some(workflow) = all.and_then(|all| all.workflow.get(id)) else { continue };
            let selected = index == cards.selected;
            let response = clickable(ui, rect, ("workflow-card", id));
            let painter = ui.painter();
            paint_card(painter, rect, selected, response.hovered());
            let room = rect.width() - 2.0 * PAD - 18.0;
            let name = ellipsized(painter, &workflow.name, heading(14.0), brand::TEXT, room);
            painter.galley(pos2(rect.left() + PAD, rect.top() + 9.0), name, brand::TEXT);
            paint_number(painter, Rect::from_min_size(rect.min, vec2(rect.width(), 34.0)), index);
            let chips = Rect::from_min_size(
                pos2(rect.left() + PAD, rect.top() + 33.0),
                vec2(rect.width() - 2.0 * PAD, CHIP_H + 2.0),
            );
            paint_chips(painter, chips, &new_tab_workflow::chips(workflow));
            if selected && cards.scroll_to_selected {
                ui.scroll_to_rect(rect, None);
            }
            if response.clicked() {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// The stages of the picked workflow, one small card each.
pub(crate) fn stages(ui: &mut Ui, stages: &[StageCard]) {
    let width = ui.available_width();
    let (area, _) =
        ui.allocate_exact_size(vec2(width, stages_height(stages.len())), Sense::hover());
    let rects = grid(area.min, width, STAGE_COLUMNS, stages.len(), STAGE_H);
    for (rect, stage) in rects.into_iter().zip(stages) {
        paint_item(ui.painter(), rect, false);
        let painter = ui.painter();
        let room = rect.width() - 20.0;
        let title = format!("{} {}", stage.number, stage.title);
        let title = ellipsized(painter, &title, heading(13.0), brand::TEXT, room);
        painter.galley(rect.left_top() + vec2(10.0, 7.0), title, brand::TEXT);
        let detail = ellipsized(painter, &stage.detail, body(12.0), brand::MUTED, room);
        painter.galley(rect.left_top() + vec2(10.0, 26.0), detail, brand::MUTED);
        ui.interact(rect, ui.id().with(("stage-card", stage.number)), Sense::hover())
            .on_hover_text(format!("Gate: {}", stage.gate));
    }
}

/// What a list with no cards says: where to add one, that it is loading, or that nothing matches.
pub(crate) fn empty_text(filter: &str, loaded: bool, noun: &str) -> String {
    if !filter.is_empty() {
        format!("No {noun} matches \"{filter}\".")
    } else if loaded {
        format!("No {noun}s yet. Add one in Settings ({}).", Command::Settings.keys())
    } else {
        format!("The {noun}s are still loading...")
    }
}

pub(crate) fn empty(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(14.0).color(brand::MUTED));
}

#[cfg(test)]
mod tests;
