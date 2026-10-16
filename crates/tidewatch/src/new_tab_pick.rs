//! Draws the new-tab dialog's agent and workflow lists.

use egui::{
    Align, Color32, CornerRadius, Layout, Mesh, Painter, Rect, RichText, Sense, Stroke, StrokeKind,
    Ui, UiBuilder, epaint::Vertex, pos2, vec2,
};
use tidewatch_core::agents::{Agent, Level};
use tidewatch_core::profiles::Profile;
use tidewatch_core::workflows::Workflow;

use crate::brand;
use crate::catalog::{Availability, Catalog};
use crate::commands::Command;
use crate::new_tab::{NewTab, level_hint, level_refusal};
use crate::new_tab_dialog::Outcome;
use crate::new_tab_dialog::start;
use crate::new_tab_workflow;
use crate::notice::key_hint;
use crate::screen_kit::{
    body, clickable, heading, paint_cli_badge, primary_button, quiet_button, selection_row,
};
use crate::segmented;
use crate::settings_ui::text_input;

const ROW_H: f32 = 58.0;
const OPEN_EXTRA: f32 = 84.0;
const LIST_MAX_H: f32 = 470.0;
const LIST_MIN_H: f32 = 160.0;
const WINDOW_MARGIN: f32 = 320.0;
const BADGE: f32 = 30.0;
const FADE_H: f32 = 28.0;

pub(crate) fn agent(
    ui: &mut Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    outcome: &mut Option<Outcome>,
) {
    filter_field(ui, state, catalog);
    ui.add_space(8.0);
    let groups = state.agent_groups(catalog);
    let mut clicked = None;
    let scroll_to = std::mem::take(&mut state.scroll_to_selected);
    let list = egui::ScrollArea::vertical()
        .max_height(list_height(ui))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let mut index = 0;
            for (agent_id, ids) in &groups {
                let Some(agent) = catalog.agent(agent_id) else { continue };
                group_title(ui, &agent.name, &catalog.availability(agent_id));
                for id in ids {
                    let Some((profile, agent)) = catalog
                        .entries()
                        .into_iter()
                        .find(|(x, _, _)| x == id)
                        .map(|(_, p, a)| (p, a))
                    else {
                        continue;
                    };
                    let selected = index == state.selected();
                    let response = agent_row(ui, state, catalog, profile, agent, selected);
                    if selected && scroll_to {
                        response.scroll_to_me(None);
                    }
                    if response.double_clicked() {
                        clicked = Some((index, true));
                    } else if response.clicked() {
                        clicked = Some((index, false));
                    }
                    index += 1;
                }
            }
            if groups.is_empty() {
                empty_list(ui, state, catalog, "profile");
            }
        });
    fade_edges(ui, &list);
    if let Some((index, now)) = clicked {
        state.select(index, catalog);
        if now {
            *outcome = start(state, catalog);
        }
    }
    footer(
        ui,
        state,
        catalog,
        outcome,
        "Up and Down pick  -  Left and Right change the level  -  Enter opens the tab  -  Esc goes back",
    );
}

pub(crate) fn workflow(
    ui: &mut Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    outcome: &mut Option<Outcome>,
) {
    filter_field(ui, state, catalog);
    ui.add_space(8.0);
    let ids = state.workflow_ids(catalog);
    let mut clicked = None;
    let scroll_to = std::mem::take(&mut state.scroll_to_selected);
    let list = egui::ScrollArea::vertical()
        .max_height(list_height(ui))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (index, id) in ids.iter().enumerate() {
                let Some(workflow) = catalog.workflows().and_then(|w| w.workflow.get(id)) else {
                    continue;
                };
                let selected = index == state.selected();
                let response = workflow_row(ui, workflow, selected);
                if selected && scroll_to {
                    response.scroll_to_me(None);
                }
                if response.double_clicked() {
                    clicked = Some((index, true));
                } else if response.clicked() {
                    clicked = Some((index, false));
                }
            }
            if ids.is_empty() {
                empty_list(ui, state, catalog, "workflow");
            }
        });
    fade_edges(ui, &list);
    if let Some((index, now)) = clicked {
        state.select(index, catalog);
        if now {
            *outcome = start(state, catalog);
        }
    }
    footer(
        ui,
        state,
        catalog,
        outcome,
        "Up and Down pick  -  Enter opens the tab, you set the task there  -  Esc goes back",
    );
}

fn list_height(ui: &Ui) -> f32 {
    LIST_MAX_H.min((ui.ctx().content_rect().height() - WINDOW_MARGIN).max(LIST_MIN_H))
}

fn filter_field(ui: &mut Ui, state: &mut NewTab, catalog: &Catalog) {
    let mut text = state.filter().to_owned();
    let field = text_input(ui, &mut text, "Filter");
    if !field.has_focus() {
        field.request_focus();
    }
    if field.changed() {
        state.set_filter(&text, catalog);
    }
}

fn fade_edges(ui: &Ui, list: &egui::scroll_area::ScrollAreaOutput<()>) {
    let view = list.inner_rect;
    let below = list.content_size.y - list.state.offset.y - view.height() > 1.0;
    if below {
        fade(
            ui.painter(),
            Rect::from_min_max(pos2(view.left(), view.bottom() - FADE_H), view.max),
            true,
        );
    }
    if list.state.offset.y > 1.0 {
        fade(
            ui.painter(),
            Rect::from_min_max(view.min, pos2(view.right(), view.top() + FADE_H)),
            false,
        );
    }
}

fn fade(painter: &Painter, rect: Rect, to_bottom: bool) {
    let clear = Color32::TRANSPARENT;
    let (top, bottom) = if to_bottom { (clear, brand::CARD) } else { (brand::CARD, clear) };
    let vertex = |pos, color| Vertex { pos, uv: egui::epaint::WHITE_UV, color };
    let mut mesh = Mesh::default();
    mesh.vertices = vec![
        vertex(rect.left_top(), top),
        vertex(rect.right_top(), top),
        vertex(rect.right_bottom(), bottom),
        vertex(rect.left_bottom(), bottom),
    ];
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    painter.add(mesh);
}

fn group_title(ui: &mut Ui, name: &str, availability: &Availability) {
    let title = match availability {
        Availability::Probing => format!("{name}  checking..."),
        Availability::Found(Some(version)) => format!("{name} {version}"),
        Availability::Found(None) => name.to_owned(),
        Availability::Missing => format!("{name}  not found on PATH"),
    };
    ui.add_space(8.0);
    let color = if *availability == Availability::Missing { brand::ORANGE } else { brand::MUTED };
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(title).font(heading(12.5)).color(color));
    });
    ui.add_space(4.0);
}

fn subline(profile: &Profile) -> String {
    let model = profile.model.as_deref().unwrap_or("default model");
    match profile.effort.as_deref() {
        Some(effort) => format!("{model} \u{b7} {effort} effort"),
        None => model.to_owned(),
    }
}

pub(crate) fn paint_card(painter: &Painter, rect: Rect, selected: bool, hovered: bool) {
    if !selected {
        painter.rect(
            rect,
            CornerRadius::same(8),
            brand::ITEM,
            Stroke::new(1.0, brand::ITEM_LINE),
            StrokeKind::Inside,
        );
    }
    selection_row(painter, rect, selected, hovered);
}

fn agent_row(
    ui: &mut Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    profile: &Profile,
    agent: &Agent,
    selected: bool,
) -> egui::Response {
    let height = if selected { ROW_H + OPEN_EXTRA } else { ROW_H };
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), height + 4.0), Sense::hover());
    let row = Rect::from_min_size(rect.min, vec2(rect.width(), height));
    let response = clickable(ui, row, ("agent-row", &profile.name, &agent.id));
    let painter = ui.painter();
    paint_card(painter, row, selected, response.hovered());
    let badge = Rect::from_min_size(pos2(row.left() + 16.0, row.top() + 14.0), vec2(BADGE, BADGE));
    paint_cli_badge(painter, badge, &agent.id);
    let text_x = badge.right() + 12.0;
    painter.text(
        pos2(text_x, row.top() + 16.0),
        egui::Align2::LEFT_TOP,
        &profile.name,
        body(16.0),
        brand::TEXT,
    );
    let quiet = if selected { brand::SOFT } else { brand::MUTED };
    painter.text(
        pos2(text_x, row.top() + 36.0),
        egui::Align2::LEFT_TOP,
        subline(profile),
        body(13.0),
        quiet,
    );
    let level = if selected { state.level() } else { profile.level };
    chip_pill(ui, pos2(row.right() - 16.0, row.top() + 29.0), level.label(), selected);
    if selected {
        ladder(ui, state, catalog, agent, row, text_x);
    }
    response
}

fn chip_pill(ui: &Ui, right_center: egui::Pos2, text: &str, active: bool) {
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(text.to_owned(), body(12.0), brand::TEXT);
    let size = galley.size() + vec2(20.0, 8.0);
    let rect = Rect::from_min_size(right_center - vec2(size.x, size.y / 2.0), size);
    let (fill, ink) = if active {
        (brand::TEAL.gamma_multiply(0.18), brand::TEAL_TEXT)
    } else {
        (brand::SEGMENT, brand::MUTED)
    };
    painter.rect_filled(rect, CornerRadius::same(255), fill);
    painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
}

fn ladder(ui: &mut Ui, state: &mut NewTab, catalog: &Catalog, agent: &Agent, row: Rect, left: f32) {
    let area =
        Rect::from_min_max(pos2(left, row.top() + ROW_H), pos2(row.right() - 16.0, row.bottom()));
    let mut child = ui.new_child(UiBuilder::new().max_rect(area));
    child.spacing_mut().item_spacing = vec2(6.0, 6.0);
    let refusals = Level::ALL.map(|level| level_refusal(state.group(), agent, level));
    let labels = Level::ALL.map(Level::label);
    let tones = Level::ALL.map(|level| (level == Level::Yolo).then_some(brand::RED));
    let active = Level::ALL.iter().position(|level| *level == state.level()).unwrap_or(0);
    let picked = segmented::inline_toned(
        &mut child,
        egui::Id::new("new_tab_level"),
        &labels,
        &tones,
        active,
    );
    child.label(RichText::new(level_hint(state.level())).size(13.0).color(brand::SOFT));
    let note = refusals.iter().flatten().next().cloned().or_else(|| {
        let flags = agent.level_args(state.level()).map(|args| args.join(" "))?;
        (!flags.is_empty()).then(|| format!("Adds {flags}"))
    });
    if let Some(note) = note {
        child.label(RichText::new(note).size(12.0).color(brand::MUTED));
    }
    if let Some(index) = picked
        && refusals[index].is_none()
    {
        state.set_level(Level::ALL[index], catalog);
    }
}

fn workflow_row(ui: &mut Ui, workflow: &Workflow, selected: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let response = clickable(ui, rect, ("workflow-row", &workflow.id));
    let painter = ui.painter();
    paint_card(painter, rect, selected, response.hovered());
    painter.text(
        pos2(rect.left() + 16.0, rect.top() + 14.0),
        egui::Align2::LEFT_TOP,
        &workflow.name,
        body(16.0),
        brand::TEXT,
    );
    let description = if selected { brand::SOFT } else { brand::MUTED };
    painter.text(
        pos2(rect.left() + 16.0, rect.top() + 35.0),
        egui::Align2::LEFT_TOP,
        &workflow.description,
        body(13.0),
        description,
    );
    let tag = new_tab_workflow::tag(workflow);
    chip_pill(ui, pos2(rect.right() - 16.0, rect.center().y), &tag, selected);
    response
}

fn empty_list(ui: &mut Ui, state: &NewTab, catalog: &Catalog, noun: &str) {
    let text = if state.filter().is_empty() && catalog.is_loaded() {
        format!("No {noun}s yet. Add one in Settings ({}).", Command::Settings.keys())
    } else if state.filter().is_empty() {
        format!("The {noun}s are still loading...")
    } else {
        format!("No {noun} matches \"{}\".", state.filter())
    };
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(14.0).color(brand::MUTED));
}

fn footer(
    ui: &mut Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    outcome: &mut Option<Outcome>,
    hint: &str,
) {
    ui.add_space(8.0);
    for warning in state.warnings(catalog) {
        ui.label(RichText::new(warning).size(12.0).color(brand::ORANGE));
    }
    if let Some(error) = &state.error {
        ui.label(RichText::new(error).size(13.0).color(brand::RED));
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if primary_button(ui, "Open tab", true).clicked() {
                *outcome = start(state, catalog);
            }
            if quiet_button(ui, "Back", true).clicked() && !state.back() {
                *outcome = Some(Outcome::Cancel);
            }
        });
    });
    key_hint(ui, hint);
}
