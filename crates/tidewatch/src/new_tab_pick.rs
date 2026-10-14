//! Draws the new-tab dialog's agent and workflow lists.

use egui::{
    Align, Align2, CornerRadius, Layout, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder,
    pos2, vec2,
};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, text_field};
use tidewatch_core::agents::{Agent, Level};
use tidewatch_core::profiles::Profile;
use tidewatch_core::workflows::Workflow;

use crate::catalog::{Availability, Catalog};
use crate::commands::Command;
use crate::new_tab::{NewTab, level_hint, level_refusal};
use crate::new_tab_dialog::{DIALOG_W, Outcome, start};
use crate::new_tab_workflow;
use crate::screen_kit::{body, clickable, heading, paint_cli_badge, section_label};

const ROW_H: f32 = 58.0;
const OPEN_EXTRA: f32 = 78.0;
const LIST_MAX_H: f32 = 470.0;
const BADGE: f32 = 30.0;

pub(crate) fn agent(
    ui: &mut Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    outcome: &mut Option<Outcome>,
) {
    header(ui, state, catalog, "Start an agent", outcome);
    ui.add_space(16.0);
    let groups = state.agent_groups(catalog);
    let mut clicked = None;
    let scroll_to = std::mem::take(&mut state.scroll_to_selected);
    egui::ScrollArea::vertical().max_height(LIST_MAX_H).auto_shrink([false, true]).show(ui, |ui| {
        let mut index = 0;
        for (agent_id, ids) in &groups {
            let Some(agent) = catalog.agent(agent_id) else { continue };
            group_title(ui, &agent.name, &catalog.availability(agent_id));
            for id in ids {
                let Some((profile, agent)) =
                    catalog.entries().into_iter().find(|(x, _, _)| x == id).map(|(_, p, a)| (p, a))
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
        "Up / Down picks  -  Left / Right changes the level  -  Enter opens the tab",
    );
}

pub(crate) fn workflow(
    ui: &mut Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    outcome: &mut Option<Outcome>,
) {
    header(ui, state, catalog, "Start a workflow", outcome);
    ui.add_space(16.0);
    let ids = state.workflow_ids(catalog);
    let mut clicked = None;
    let scroll_to = std::mem::take(&mut state.scroll_to_selected);
    egui::ScrollArea::vertical().max_height(LIST_MAX_H).auto_shrink([false, true]).show(ui, |ui| {
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
        "Up / Down picks  -  Enter opens the tab, you set the task there",
    );
}

fn header(
    ui: &mut Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    title: &str,
    outcome: &mut Option<Outcome>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
        let back = clickable(ui, rect, "back").on_hover_text("Back (Esc)");
        let fill = if back.hovered() { colors::hover() } else { colors::hover_elevated() };
        ui.painter().rect_filled(rect, 8.0, fill);
        let c = rect.center();
        let chevron = vec![c + vec2(3.0, -6.0), c + vec2(-3.0, 0.0), c + vec2(3.0, 6.0)];
        ui.painter().add(Shape::line(chevron, Stroke::new(1.8, colors::fg())));
        if back.clicked() && !state.back() {
            *outcome = Some(Outcome::Cancel);
        }
        ui.label(egui::RichText::new(title).font(heading(22.0)).color(colors::fg()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut text = state.filter().to_owned();
            let field = text_field(ui, &mut text, "Filter", 220.0, colors::fg());
            if !field.has_focus() {
                field.request_focus();
            }
            if field.changed() {
                state.set_filter(&text, catalog);
            }
        });
    });
}

fn group_title(ui: &mut Ui, name: &str, availability: &Availability) {
    let title = match availability {
        Availability::Probing => format!("{name}  checking..."),
        Availability::Found(Some(version)) => format!("{name} {version}"),
        Availability::Found(None) => name.to_owned(),
        Availability::Missing => format!("{name}  not found on PATH"),
    };
    ui.add_space(8.0);
    let color =
        if *availability == Availability::Missing { colors::yellow() } else { colors::dim() };
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        section_label(ui, &title, color);
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
    if selected {
        painter.rect(
            row,
            12.0,
            colors::selection(),
            Stroke::new(1.0, colors::accent()),
            StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_filled(row, 12.0, colors::hover_elevated());
    }
    let badge = Rect::from_min_size(pos2(row.left() + 16.0, row.top() + 14.0), vec2(BADGE, BADGE));
    paint_cli_badge(painter, badge, &agent.id);
    let text_x = badge.right() + 12.0;
    painter.text(
        pos2(text_x, row.top() + 16.0),
        Align2::LEFT_TOP,
        &profile.name,
        body(16.0),
        colors::fg(),
    );
    let quiet = if selected { colors::fg().gamma_multiply(0.62) } else { colors::dim() };
    painter.text(
        pos2(text_x, row.top() + 36.0),
        Align2::LEFT_TOP,
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
    let galley = painter.layout_no_wrap(text.to_owned(), body(12.0), colors::fg());
    let size = galley.size() + vec2(20.0, 8.0);
    let rect = Rect::from_min_size(right_center - vec2(size.x, size.y / 2.0), size);
    let (fill, ink) = if active {
        (colors::accent().gamma_multiply(0.28), colors::accent())
    } else {
        (colors::hover_elevated(), colors::dim())
    };
    painter.rect_filled(rect, CornerRadius::same(255), fill);
    painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
}

fn ladder(ui: &mut Ui, state: &mut NewTab, catalog: &Catalog, agent: &Agent, row: Rect, left: f32) {
    let area =
        Rect::from_min_max(pos2(left, row.top() + ROW_H), pos2(row.right() - 16.0, row.bottom()));
    let mut child = ui.new_child(UiBuilder::new().max_rect(area));
    child.spacing_mut().item_spacing = vec2(6.0, 8.0);
    let mut picked = None;
    child.horizontal(|ui| {
        for level in Level::ALL {
            let refusal = level_refusal(state.group(), agent, level);
            let response = level_chip(ui, level, level == state.level(), refusal.is_none());
            let tip = refusal.clone().unwrap_or_else(|| {
                let flags = agent.level_args(level).map(|args| args.join(" ")).unwrap_or_default();
                if flags.is_empty() { "Adds no flag.".to_owned() } else { format!("Adds {flags}") }
            });
            let response = response.on_hover_text(tip);
            if refusal.is_none() && response.clicked() {
                picked = Some(level);
            }
        }
    });
    child.label(
        egui::RichText::new(level_hint(state.level()))
            .size(13.0)
            .color(colors::fg().gamma_multiply(0.62)),
    );
    if let Some(level) = picked {
        state.set_level(level, catalog);
    }
}

fn level_chip(ui: &mut Ui, level: Level, selected: bool, enabled: bool) -> egui::Response {
    let danger = level == Level::Yolo;
    let ink = match (selected, danger, enabled) {
        (true, _, _) => colors::bg(),
        (false, true, true) => colors::red(),
        (false, _, true) => colors::fg().gamma_multiply(0.8),
        (false, _, false) => colors::dim().gamma_multiply(0.6),
    };
    let galley = ui.painter().layout_no_wrap(level.label().to_owned(), body(13.0), ink);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(24.0, 12.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 8.0, if danger { colors::red() } else { colors::accent() });
    } else {
        let edge = if response.hovered() && enabled { colors::dim() } else { colors::border() };
        painter.rect(rect, 8.0, colors::elevated(), Stroke::new(1.0, edge), StrokeKind::Inside);
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn workflow_row(ui: &mut Ui, workflow: &Workflow, selected: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let response = clickable(ui, rect, ("workflow-row", &workflow.id));
    let painter = ui.painter();
    if selected {
        painter.rect(
            rect,
            12.0,
            colors::selection(),
            Stroke::new(1.0, colors::accent()),
            StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, 12.0, colors::hover_elevated());
    }
    painter.text(
        pos2(rect.left() + 16.0, rect.top() + 14.0),
        Align2::LEFT_TOP,
        &workflow.name,
        body(16.0),
        colors::fg(),
    );
    painter.text(
        pos2(rect.left() + 16.0, rect.top() + 35.0),
        Align2::LEFT_TOP,
        &workflow.description,
        body(13.0),
        colors::dim(),
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
    ui.label(egui::RichText::new(text).size(14.0).color(colors::dim()));
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
        ui.label(egui::RichText::new(warning).size(12.0).color(colors::yellow()));
    }
    if let Some(error) = &state.error {
        ui.label(egui::RichText::new(error).size(13.0).color(colors::red()));
    }
    ui.add_space(14.0);
    ui.separator();
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let width = DIALOG_W - 56.0 - 190.0;
        ui.allocate_ui_with_layout(
            vec2(width.min(ui.available_width() - 190.0), 20.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(hint).size(13.0).color(colors::dim()))
                        .truncate(),
                );
            },
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if action_button(ui, "Open tab", true).clicked() {
                *outcome = start(state, catalog);
            }
            if action_button(ui, "Back", false).clicked() && !state.back() {
                *outcome = Some(Outcome::Cancel);
            }
        });
    });
}
