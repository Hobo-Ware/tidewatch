//! The Agents and profiles page: profiles grouped by agent on the left, the editor card on the right.

use std::collections::HashMap;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Rect, ScrollArea, Sense, Ui,
    UiBuilder, pos2, vec2,
};
use tidewatch_core::agents::Level;
use tidewatch_core::profiles::default_effort_for;

use crate::catalog::Availability;
use crate::profile_editor::{Draft, Editor, Group};
use crate::screen_kit::{badge_colors, badge_letter, field_label};
use crate::segmented;
use crate::settings_ui::editor::{
    Bar, BarAction, Item, card_header, columns, list_item, page_area, page_top, save_bar,
    split_card,
};
use crate::settings_ui::{
    CARD, FAINT, GREEN, MUTED, ORANGE, RED, SOFT, TEAL_TEXT, caption, disclosure, fade_edges,
    field, note, switch_line,
};
use crate::settings_view::Action;

pub(crate) type Probes = HashMap<String, Availability>;

const EFFORTS: [(&str, &str); 6] = [
    ("", "Default"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "Extra high"),
    ("max", "Max"),
];

pub(crate) fn level_note(level: Level) -> &'static str {
    match level {
        Level::Plan => "Looks, never changes",
        Level::Ask => "Asks before each step",
        Level::Edits => "Edits, asks to run",
        Level::Auto => "Edits and runs",
        Level::Yolo => "No limits, worktree only",
    }
}

fn tint(agent: &str) -> (String, (Color32, Color32)) {
    let letter = if agent.is_empty() { String::new() } else { badge_letter(agent) };
    (letter, badge_colors(agent))
}

fn probe_label(availability: Option<&Availability>) -> (String, Color32) {
    match availability {
        Some(Availability::Found(Some(version))) => (format!("{version} found"), SOFT),
        Some(Availability::Found(None)) => ("found".to_owned(), SOFT),
        Some(Availability::Missing) => ("not installed".to_owned(), ORANGE),
        Some(Availability::Probing) | None => ("checking...".to_owned(), FAINT),
    }
}

fn summary(draft: &Draft) -> String {
    let mut parts = Vec::new();
    for part in [draft.model.trim(), draft.effort.trim()] {
        if !part.is_empty() {
            parts.push(part.to_owned());
        }
    }
    parts.push(draft.level.label().to_owned());
    parts.join(" \u{b7} ")
}

fn effort_line(effort: &str, default: Option<&str>) -> String {
    match (effort.trim(), default) {
        ("", Some(default)) => format!("The agent's own effort, {default} for this model."),
        ("", None) => "The agent's own effort.".to_owned(),
        ("low", _) => "Quick answers with little thinking.".to_owned(),
        ("medium", _) => "Balanced speed and depth.".to_owned(),
        ("high", _) => "Thinks longer before it answers.".to_owned(),
        ("xhigh", _) => "Deep thinking for hard problems.".to_owned(),
        ("max", _) => "The most thinking the model allows.".to_owned(),
        _ => "Passed to the agent as written.".to_owned(),
    }
}

fn effort_choices(current: &str, default: Option<&str>) -> Vec<(String, String)> {
    let mut choices: Vec<(String, String)> =
        EFFORTS.iter().map(|(value, label)| ((*value).to_owned(), (*label).to_owned())).collect();
    if let Some(default) = default {
        choices[0].1 = format!("Default ({default})");
    }
    let current = current.trim();
    if !current.is_empty() && !EFFORTS.iter().any(|(value, _)| *value == current) {
        choices.push((current.to_owned(), current.to_owned()));
    }
    choices
}

pub(crate) fn page(
    ui: &mut Ui,
    area: Rect,
    editor: &mut Editor,
    probes: &Probes,
    file: &str,
) -> Option<Action> {
    let inner = page_area(area);
    let (bottom, new) = page_top(
        ui,
        area,
        inner,
        (
            "Agents and profiles",
            "A profile is an agent with its model, effort and permission level.",
            file,
        ),
        "New profile",
    );
    if new {
        editor.add();
    }
    let (list, card) = columns(inner, bottom);
    list_column(ui, list, editor, probes);
    editor_card(ui, card, editor, file)
}

fn list_column(ui: &mut Ui, rect: Rect, editor: &mut Editor, probes: &Probes) {
    let groups = editor.groups();
    let mut picked = None;
    let mut column =
        ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
    ScrollArea::vertical().id_salt("profile_list").auto_shrink(false).show(&mut column, |ui| {
        ui.set_width(rect.width());
        ui.spacing_mut().item_spacing.y = 6.0;
        for (position, group) in groups.iter().enumerate() {
            if position > 0 {
                ui.add_space(6.0);
            }
            let availability = probes.get(&group.agent);
            group_header(ui, group, availability);
            let missing = matches!(availability, Some(Availability::Missing));
            for &index in &group.drafts {
                let draft = &editor.drafts[index];
                let (letter, colors) = tint(&draft.agent);
                let selected = index == editor.selected;
                let item = Item {
                    badge: Some((&letter, colors)),
                    name: &draft.name,
                    tag: draft.level.label(),
                    tag_color: if selected { TEAL_TEXT } else { MUTED },
                    selected,
                    dimmed: missing,
                };
                if list_item(ui, &item).clicked() {
                    picked = Some(index);
                }
            }
        }
    });
    if let Some(index) = picked {
        editor.select(index);
    }
}

fn group_header(ui: &mut Ui, group: &Group, availability: Option<&Availability>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let painter = ui.painter();
    painter.text(
        rect.left_center() + vec2(4.0, 2.0),
        Align2::LEFT_CENTER,
        group.name.to_uppercase(),
        FontId::proportional(12.0),
        FAINT,
    );
    let (text, color) = if group.known {
        probe_label(availability)
    } else {
        ("not in the agent table".to_owned(), ORANGE)
    };
    let right = painter.text(
        rect.right_center() + vec2(-4.0, 2.0),
        Align2::RIGHT_CENTER,
        text,
        FontId::proportional(12.0),
        color,
    );
    let dot = match availability {
        Some(Availability::Found(_)) if group.known => GREEN,
        _ => color,
    };
    painter.circle_filled(pos2(right.left() - 8.0, rect.center().y + 2.0), 3.0, dot);
}

fn editor_card(ui: &mut Ui, rect: Rect, editor: &mut Editor, file: &str) -> Option<Action> {
    let Some(selected) = editor.drafts.get(editor.selected).cloned() else {
        ui.painter().rect_filled(rect, CornerRadius::same(14), CARD);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No profiles. Add one with New profile.",
            FontId::proportional(14.0),
            MUTED,
        );
        return None;
    };
    let agents = editor.agents().to_vec();
    let error = editor.error.clone();
    let bar = Bar {
        dirty: editor.dirty,
        saving: editor.saving,
        error: error.as_deref(),
        can_revert: true,
        file,
    };
    let parts = split_card(ui, rect, bar.shows());
    let (letter, colors) = tint(&selected.agent);
    let delete =
        card_header(ui, parts.header, Some((&letter, colors)), &selected.name, &summary(&selected));
    let mut changed = false;
    let mut body =
        ui.new_child(UiBuilder::new().max_rect(parts.body).layout(Layout::top_down(Align::Min)));
    let form_scroll =
        ScrollArea::vertical().id_salt("profile_form").auto_shrink(false).show(&mut body, |ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(20, 18)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if let Some(draft) = editor.selected_draft() {
                    changed = form(ui, draft, &agents);
                }
            });
        });
    fade_edges(&body, &form_scroll, CARD);
    let mut action = None;
    if bar.shows() {
        let mut bar_ui = ui.new_child(UiBuilder::new().max_rect(parts.bar));
        match save_bar(&mut bar_ui, &bar) {
            Some(BarAction::Save) => action = Some(Action::Save),
            Some(BarAction::Revert) => editor.revert(),
            None => {}
        }
    }
    if changed {
        editor.touched();
    }
    if delete {
        editor.remove_selected();
    }
    action
}

fn form(ui: &mut Ui, draft: &mut Draft, agents: &[(String, String)]) -> bool {
    let mut changed = false;
    ui.spacing_mut().item_spacing.x = 20.0;
    let previous_model = draft.model.clone();
    ui.columns(2, |columns| {
        changed |= field(&mut columns[0], "Name", &mut draft.name, "Builder");
        changed |= field(&mut columns[1], "Model", &mut draft.model, "empty: the agent's default");
    });
    if draft.model != previous_model {
        draft.follow_model(&previous_model);
    }
    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
    ui.add_space(12.0);
    field_label(ui, "Agent");
    ui.add_space(2.0);
    let names: Vec<&str> = agents.iter().map(|(_, name)| name.as_str()).collect();
    let active = agents.iter().position(|(id, _)| *id == draft.agent).unwrap_or(usize::MAX);
    let id = ui.id().with("profile_agent");
    if let Some(index) = segmented::inline_or_chips(ui, id, &names, &[], &[], active) {
        draft.agent.clone_from(&agents[index].0);
        changed = true;
    }
    ui.add_space(12.0);
    field_label(ui, "Effort");
    ui.add_space(2.0);
    let default = default_effort_for(&draft.agent, &draft.model);
    let efforts = effort_choices(&draft.effort, default);
    let labels: Vec<&str> = efforts.iter().map(|(_, label)| label.as_str()).collect();
    let current = draft.effort.trim();
    let active = efforts.iter().position(|(value, _)| value == current).unwrap_or(usize::MAX);
    let id = ui.id().with("profile_effort");
    if let Some(index) = segmented::inline_or_chips(ui, id, &labels, &[], &[], active) {
        draft.effort.clone_from(&efforts[index].0);
        changed = true;
    }
    caption(ui, &effort_line(&draft.effort, default));
    ui.add_space(12.0);
    field_label(ui, "Permission level");
    ui.add_space(2.0);
    changed |= levels(ui, draft);
    ui.add_space(12.0);
    disclosure(
        ui,
        "profile_advanced",
        "Advanced: setup command, extra arguments, push guard, house style",
        |ui| {
            ui.add_space(8.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            changed |= field(
                ui,
                "Setup command",
                &mut draft.setup,
                "runs in the worktree before the agent, for example npm ci",
            );
            changed |= field(
                ui,
                "Extra arguments",
                &mut draft.extra_args,
                "added to the agent's command line, quote words with spaces",
            );
            ui.add_space(2.0);
            changed |=
                switch_line(ui, "Keep this agent off the default branch", &mut draft.never_push);
            changed |= switch_line(ui, "Add the Tidewatch house style", &mut draft.house_style);
        },
    );
    changed
}

fn levels(ui: &mut Ui, draft: &mut Draft) -> bool {
    let labels = Level::ALL.map(Level::label);
    let tones = Level::ALL.map(|level| (level == Level::Yolo).then_some(RED));
    let active = Level::ALL.iter().position(|level| *level == draft.level).unwrap_or(usize::MAX);
    let id = ui.id().with("profile_level");
    let picked = segmented::inline_or_chips(ui, id, &labels, &tones, &[], active);
    if let Some(index) = picked {
        draft.level = Level::ALL[index];
    }
    caption(ui, level_note(draft.level));
    if draft.level == Level::Yolo {
        ui.add_space(2.0);
        note(ui, "YOLO only runs in agent worktree tabs; Other refuses it.", RED);
    }
    picked.is_some()
}

#[cfg(test)]
mod tests;
