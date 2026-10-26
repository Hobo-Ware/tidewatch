//! The new-tab form's lane: the filter and a vertical list of profiles (grouped by CLI) or workflows.

use egui::{
    Align2, CornerRadius, Id, Rect, RichText, ScrollArea, Sense, TextEdit, Ui, UiBuilder, pos2,
    vec2,
};
use tidewatch_core::profiles::Profile;

use crate::brand;
use crate::catalog::{Availability, Catalog};
use crate::new_tab::{Kind, NewTab};
use crate::new_tab_layout::{self as layout, Slot};
use crate::new_tab_meeseeks;
use crate::new_tab_workflow;
use crate::reader_view::{ellipsized, field_frame_on};
use crate::screen_kit::{body, clickable, heading, mono, paint_cli_badge, selection_row};
use crate::settings_ui::fade_edges;

const BADGE: f32 = 28.0;
const PAD: f32 = 12.0;
const NOT_INSTALLED: f32 = 0.45;
const ROW_RADIUS: u8 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Badge {
    Cli(String),
    Stages(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) index: usize,
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) badge: Badge,
    pub(crate) dim: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Group(String),
    Row(Row),
}

/// `opus · medium`, `sonnet · default (high)`, `opus`, or `default model`; `not installed` when the CLI is missing.
pub(crate) fn subtitle(profile: &Profile, availability: &Availability) -> String {
    if *availability == Availability::Missing {
        return "not installed".to_owned();
    }
    let model = profile.model.as_deref().unwrap_or("default model");
    let chosen = profile.effort.as_deref().is_some_and(|effort| !effort.is_empty());
    match (chosen, profile.effective_effort()) {
        (true, Some(effort)) => format!("{model} \u{b7} {effort}"),
        (false, Some(default)) => format!("{model} \u{b7} default ({default})"),
        (_, None) => model.to_owned(),
    }
}

/// `Builder · opus · medium` for the profile a Meeseeks on this CLI will use (the CLI's own name
/// left off the profile's), `default model` when the CLI has no profile; `not installed` when the
/// CLI is missing.
pub(crate) fn meeseeks_subtitle(
    cli: &str,
    key: &str,
    profile: &Profile,
    availability: &Availability,
) -> String {
    let line = subtitle(profile, availability);
    if key == Profile::DEFAULTS_ID || *availability == Availability::Missing {
        return line;
    }
    let name = profile.name.strip_prefix(&format!("{cli} ")).unwrap_or(&profile.name);
    format!("{name} \u{b7} {line}")
}

/// The stage names of a workflow in one line: `Plan › Build › Review`.
pub(crate) fn stages_line(chips: &[&str]) -> String {
    chips.join(" \u{203a} ")
}

/// The lane's list for the form's kind, in the order the number keys count: groups by CLI with
/// their profiles, or the workflows with no groups.
pub(crate) fn entries(state: &NewTab, catalog: &Catalog) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut index = 0;
    if state.kind() == Kind::Workflow {
        let Some(all) = catalog.workflows() else { return out };
        for id in state.visible(catalog) {
            let Some(workflow) = all.workflow.get(&id) else { continue };
            out.push(Entry::Row(Row {
                index,
                title: workflow.name.clone(),
                subtitle: stages_line(&new_tab_workflow::chips(workflow)),
                badge: Badge::Stages(workflow.stages.len()),
                dim: false,
                id,
            }));
            index += 1;
        }
        return out;
    }
    let profiles = catalog.entries();
    if state.kind() == Kind::Meeseeks {
        let ids = state.visible(catalog);
        for (index, id) in ids.into_iter().enumerate() {
            let Some(agent) = catalog.agent(&id) else { continue };
            let (key, profile) = new_tab_meeseeks::profile_for(&id, &profiles);
            let availability = catalog.availability(&id);
            out.push(Entry::Row(Row {
                index,
                title: agent.name.clone(),
                subtitle: meeseeks_subtitle(&agent.name, &key, &profile, &availability),
                badge: Badge::Cli(id.clone()),
                dim: availability == Availability::Missing,
                id,
            }));
        }
        return out;
    }
    let names = catalog.agent_names();
    for (agent_id, ids) in state.agent_groups(catalog) {
        let label = names.iter().find(|(id, _)| *id == agent_id).map_or(&agent_id, |(_, n)| n);
        out.push(Entry::Group(label.clone()));
        let availability = catalog.availability(&agent_id);
        for id in ids {
            let Some((_, profile, _)) = profiles.iter().find(|(entry, _, _)| *entry == id) else {
                continue;
            };
            out.push(Entry::Row(Row {
                index,
                title: profile.name.clone(),
                subtitle: subtitle(profile, &availability),
                badge: Badge::Cli(agent_id.clone()),
                dim: availability == Availability::Missing,
                id,
            }));
            index += 1;
        }
    }
    out
}

pub(crate) fn slots(entries: &[Entry]) -> Vec<Slot> {
    entries
        .iter()
        .map(|entry| match entry {
            Entry::Group(_) => Slot::Group,
            Entry::Row(_) => Slot::Row,
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct Out {
    pub(crate) picked: Option<usize>,
    pub(crate) filter: Option<String>,
    pub(crate) selected: Option<Rect>,
}

fn top_row(ui: &mut Ui, rect: Rect, state: &NewTab) -> Option<String> {
    if let Some(title) = match state.kind() {
        Kind::Workflow => Some("WORKFLOW"),
        Kind::Meeseeks => Some("AGENT"),
        Kind::Agent | Kind::Shell => None,
    } {
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, title, body(11.5), brand::MUTED);
        return None;
    }
    let current = state.filter();
    let mut text = current.to_owned();
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    field_frame_on(&mut child, brand::SEGMENT, false, |ui| {
        ui.add(
            TextEdit::singleline(&mut text)
                .id(Id::new("new-tab-filter"))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(body(12.5))
                .text_color(brand::TEXT)
                .hint_text(RichText::new("Filter").color(brand::FAINT))
                .desired_width(ui.available_width()),
        )
    });
    (text != current).then_some(text)
}

fn paint_badge(ui: &Ui, rect: Rect, badge: &Badge) {
    match badge {
        Badge::Cli(id) => paint_cli_badge(ui.painter(), rect, id),
        Badge::Stages(count) => {
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(8), brand::ITEM);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                count.to_string(),
                mono(12.0),
                brand::SOFT,
            );
        }
    }
}

fn paint_row(ui: &mut Ui, rect: Rect, row: &Row, selected: bool) -> bool {
    let response = clickable(ui, rect, ("lane-row", &row.id));
    selection_row(ui.painter(), rect, selected, response.hovered());
    let badge = Rect::from_min_size(
        pos2(rect.left() + PAD, rect.center().y - BADGE / 2.0),
        vec2(BADGE, BADGE),
    );
    paint_badge(ui, badge, &row.badge);
    let painter = ui.painter();
    let text_x = badge.right() + 10.0;
    let room = rect.right() - PAD - 16.0 - text_x;
    let title = ellipsized(painter, &row.title, heading(13.5), brand::TEXT, room);
    painter.galley(pos2(text_x, rect.top() + 6.0), title, brand::TEXT);
    let sub = ellipsized(painter, &row.subtitle, body(11.5), brand::MUTED, room);
    painter.galley(pos2(text_x, rect.top() + 25.0), sub, brand::MUTED);
    if row.index < 9 {
        painter.text(
            pos2(rect.right() - PAD, rect.center().y),
            Align2::RIGHT_CENTER,
            (row.index + 1).to_string(),
            mono(11.0),
            brand::FAINT,
        );
    }
    if row.dim {
        painter.rect_filled(
            rect,
            CornerRadius::same(ROW_RADIUS),
            brand::GROUND.gamma_multiply(1.0 - NOT_INSTALLED),
        );
    }
    response.clicked()
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    state: &NewTab,
    entries: &[Entry],
    empty_text: &str,
    scroll_to_selected: bool,
) -> Out {
    let (top, list) = layout::lane_parts(rect);
    let mut out = Out { picked: None, filter: top_row(ui, top, state), selected: None };
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    if entries.is_empty() {
        child.label(RichText::new(empty_text).size(13.0).color(brand::MUTED));
        return out;
    }
    let (placed, total) = layout::lane_slots(&slots(entries));
    let scroll = ScrollArea::vertical()
        .id_salt("new-tab-lane")
        .auto_shrink([false, false])
        .max_height(list.height())
        .show(&mut child, |ui| {
            let (area, _) = ui.allocate_exact_size(vec2(list.width(), total), Sense::hover());
            for (entry, (top, height)) in entries.iter().zip(placed) {
                let rect = Rect::from_min_size(
                    pos2(area.left(), area.top() + top),
                    vec2(area.width(), height),
                );
                match entry {
                    Entry::Group(label) => {
                        ui.painter().text(
                            pos2(rect.left() + 4.0, rect.bottom() - 4.0),
                            Align2::LEFT_BOTTOM,
                            label.to_uppercase(),
                            body(11.0),
                            brand::FAINT,
                        );
                    }
                    Entry::Row(row) => {
                        let selected = row.index == state.selected();
                        if paint_row(ui, rect, row, selected) {
                            out.picked = Some(row.index);
                        }
                        if selected {
                            out.selected = Some(rect);
                        }
                        if selected && scroll_to_selected {
                            ui.scroll_to_rect(rect, None);
                        }
                    }
                }
            }
        });
    fade_edges(&child, &scroll, brand::GROUND);
    out
}

#[cfg(test)]
mod tests;
