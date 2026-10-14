//! The Agents and profiles page: profiles grouped by agent on the left, the editor card on the right.

use std::collections::HashMap;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Rect, ScrollArea, Sense, Ui,
    UiBuilder, vec2,
};
use tidewatch_core::agents::Level;

use crate::catalog::Availability;
use crate::profile_editor::{Draft, Editor, Group};
use crate::settings_ui::editor::{
    Bar, BarAction, Item, card_header, columns, list_item, option_card, option_card_height,
    page_area, page_top, save_bar, split_card,
};
use crate::settings_ui::{
    CARD, FAINT, MUTED, ORANGE, RED, TEAL_TEXT, caption, choice, disclosure, field, note,
    switch_line,
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
        Level::Plan => "looks, never changes",
        Level::Ask => "asks before each step",
        Level::Edits => "edits, asks to run",
        Level::Auto => "edits and runs",
        Level::Yolo => "no limits, worktree only",
    }
}

fn tint(agent: &str) -> (String, (Color32, Color32)) {
    let pair = |bg: [u8; 3], fg: [u8; 3]| {
        (Color32::from_rgb(bg[0], bg[1], bg[2]), Color32::from_rgb(fg[0], fg[1], fg[2]))
    };
    match agent {
        "claude" => ("C".to_owned(), pair([0x3b, 0x24, 0x18], [0xfb, 0x92, 0x3c])),
        "codex" => ("X".to_owned(), pair([0x1e, 0x2a, 0x22], [0x8f, 0xd1, 0x6a])),
        "gemini" => ("G".to_owned(), pair([0x1d, 0x24, 0x40], [0x93, 0xc5, 0xfd])),
        "copilot" => ("P".to_owned(), pair([0x2a, 0x21, 0x40], [0xc0, 0x84, 0xfc])),
        other => {
            let letter =
                other.chars().next().map_or_else(String::new, |c| c.to_uppercase().to_string());
            (letter, pair([0x22, 0x29, 0x37], [0xa7, 0xb0, 0xbf]))
        }
    }
}

fn probe_label(availability: Option<&Availability>) -> (String, Color32) {
    match availability {
        Some(Availability::Found(Some(version))) => (format!("{version} found"), TEAL_TEXT),
        Some(Availability::Found(None)) => ("found".to_owned(), TEAL_TEXT),
        Some(Availability::Missing) => ("not installed".to_owned(), ORANGE),
        Some(Availability::Probing) | None => ("checking...".to_owned(), FAINT),
    }
}

fn summary(draft: &Draft, agent: &str) -> String {
    let mut parts = vec![agent.to_owned()];
    let model = draft.model.trim();
    if !model.is_empty() {
        parts.push(model.to_owned());
    }
    let effort = draft.effort.trim();
    if !effort.is_empty() {
        parts.push(format!("{effort} effort"));
    }
    parts.push(draft.level.label().to_owned());
    parts.join(" \u{b7} ")
}

fn effort_choices(current: &str) -> Vec<(String, String)> {
    let mut choices: Vec<(String, String)> =
        EFFORTS.iter().map(|(value, label)| ((*value).to_owned(), (*label).to_owned())).collect();
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
        inner,
        "Agents and profiles",
        "A profile is an agent with its model, effort and permission level.",
        file,
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
    painter.text(
        rect.right_center() + vec2(-4.0, 2.0),
        Align2::RIGHT_CENTER,
        text,
        FontId::proportional(12.0),
        color,
    );
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
    let agent_name = agents
        .iter()
        .find(|(id, _)| *id == selected.agent)
        .map_or_else(|| format!("{} (unknown)", selected.agent), |(_, name)| name.clone());
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
    let delete = card_header(
        ui,
        parts.header,
        Some((&letter, colors)),
        &selected.name,
        &summary(&selected, &agent_name),
    );
    let mut changed = false;
    let mut body =
        ui.new_child(UiBuilder::new().max_rect(parts.body).layout(Layout::top_down(Align::Min)));
    ScrollArea::vertical().id_salt("profile_form").auto_shrink(false).show(&mut body, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(20, 18)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(draft) = editor.selected_draft() {
                changed = form(ui, draft, &agents);
            }
        });
    });
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
    let width = ui.available_width();
    ui.spacing_mut().item_spacing.x = 20.0;
    ui.columns(2, |columns| {
        changed |= field(&mut columns[0], "Name", &mut draft.name, "Builder");
        changed |= field(&mut columns[1], "Model", &mut draft.model, "empty: the agent's default");
    });
    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
    ui.add_space(12.0);
    caption(ui, "Agent");
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        for (id, name) in agents {
            if choice(ui, name, draft.agent == *id).clicked() {
                draft.agent.clone_from(id);
                changed = true;
            }
        }
    });
    ui.add_space(12.0);
    caption(ui, "Effort");
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        let current = draft.effort.trim().to_owned();
        for (value, label) in effort_choices(&draft.effort) {
            if choice(ui, &label, current == value).clicked() {
                draft.effort = value;
                changed = true;
            }
        }
    });
    ui.add_space(12.0);
    caption(ui, "Permission level");
    ui.add_space(2.0);
    changed |= levels(ui, draft, width);
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
            changed |= switch_line(ui, "Block git push in this agent's tab", &mut draft.never_push);
            changed |= switch_line(ui, "Add the Tidewatch house style", &mut draft.house_style);
        },
    );
    changed
}

fn levels(ui: &mut Ui, draft: &mut Draft, width: f32) -> bool {
    let gap = 6.0;
    let count = Level::ALL.len() as f32;
    let each = ((width - gap * (count - 1.0)) / count).floor();
    let height = Level::ALL
        .iter()
        .map(|level| option_card_height(ui, each, level_note(*level)))
        .fold(0.0, f32::max);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for level in Level::ALL {
            let selected = draft.level == level;
            let card = option_card(
                ui,
                vec2(each, height),
                level.label(),
                level_note(level),
                selected,
                level == Level::Yolo,
            );
            if card.clicked() {
                draft.level = level;
                changed = true;
            }
        }
    });
    if draft.level == Level::Yolo {
        ui.add_space(2.0);
        note(ui, "YOLO only runs in agent worktree tabs; Other refuses it.", RED);
    }
    changed
}

#[cfg(test)]
mod tests;
