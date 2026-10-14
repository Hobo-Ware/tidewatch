//! Draws the Window section of General: the quake switch, the hotkey recorder with its status, and
//! the menu-bar switch. It returns the new window settings when something was applied.

use egui::{
    Align, CornerRadius, Event, Layout, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, vec2,
};
use tidewatch_core::prefs::WindowPrefs;

use crate::hotkey;
use crate::settings_ui::{
    self, Control, FAINT, FIELD_LINE, GROUND, RED, TEAL, TEAL_TEXT, key_cap, key_cap_width,
    row_with, switch_row,
};
use crate::shell::HotkeyStatus;
use crate::window_settings::Editor;

const RECORDER_WIDTH: f32 = 230.0;
const RECORDER_HEIGHT: f32 = 38.0;
const MIN_RECORDER: f32 = 112.0;
const CAP_GAP: f32 = 6.0;

/// What the section shows beside the editor's own state.
pub(crate) struct View {
    pub(crate) status: HotkeyStatus,
    pub(crate) menu_bar_available: bool,
}

pub(crate) fn section(ui: &mut Ui, editor: &mut Editor, view: &View) -> Option<WindowPrefs> {
    settings_ui::section(ui, "Window", |ui| {
        let mut applied = None;
        if let Some(on) = switch_row(
            ui,
            "Drop-down window",
            "Slides down from the top on the hotkey, hides when you click away.",
            editor.quake(),
        ) {
            applied = Some(editor.set_quake(on));
        }
        if editor.quake() {
            applied = hotkey_row(ui, editor, &view.status).or(applied);
        }
        if view.menu_bar_available
            && let Some(on) = switch_row(
                ui,
                "Menu-bar item",
                "Show or hide, the Overview and Settings from the menu bar.",
                editor.menu_bar(),
            )
        {
            applied = Some(editor.set_menu_bar(on));
        }
        applied
    })
}

fn hotkey_row(ui: &mut Ui, editor: &mut Editor, status: &HotkeyStatus) -> Option<WindowPrefs> {
    let problem = editor.problem().map(|problem| problem.to_string());
    let mut applied = None;
    row_with(
        ui,
        Control::new(RECORDER_WIDTH, RECORDER_HEIGHT),
        |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new("Hotkey").size(15.0).color(settings_ui::TEXT));
            settings_ui::note(ui, "Click and press the keys you want.", settings_ui::MUTED);
            status_line(ui, problem.as_deref(), status);
        },
        |ui| applied = recorder(ui, editor, problem.is_some()),
    );
    applied
}

fn status_line(ui: &mut Ui, problem: Option<&str>, status: &HotkeyStatus) {
    if let Some(problem) = problem {
        settings_ui::note(ui, problem, RED);
        return;
    }
    match status {
        HotkeyStatus::Registered => settings_ui::note(ui, "Registered", TEAL_TEXT),
        HotkeyStatus::Problem(reason) => {
            settings_ui::note(ui, &format!("Not registered: {reason}"), RED);
        }
        HotkeyStatus::NoGlobalHotkeys => settings_ui::note(
            ui,
            "This desktop has no global hotkeys. Bind a shortcut to `tidewatch --toggle`.",
            FAINT,
        ),
        HotkeyStatus::Off => {}
    }
}

fn held_labels(modifiers: egui::Modifiers) -> Vec<String> {
    [
        (modifiers.ctrl, "Ctrl"),
        (modifiers.alt, "Alt"),
        (modifiers.shift, "Shift"),
        (modifiers.mac_cmd, "Cmd"),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .map(|(_, name)| name.to_owned())
    .collect()
}

fn recorder(ui: &mut Ui, editor: &mut Editor, bad: bool) -> Option<WindowPrefs> {
    let recording = editor.recording;
    let labels = if recording {
        held_labels(ui.input(|input| input.modifiers))
    } else {
        hotkey::cap_labels(&editor.hotkey)
    };
    let widths: Vec<f32> = labels.iter().map(|label| key_cap_width(ui, label, 13.0)).collect();
    let caps = widths.iter().sum::<f32>() + CAP_GAP * widths.len().saturating_sub(1) as f32;
    let width = (caps + 16.0).max(MIN_RECORDER);
    let (rect, response) = ui.allocate_exact_size(vec2(width, RECORDER_HEIGHT), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(10), GROUND);
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 6.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = CAP_GAP;
    for label in &labels {
        key_cap(&mut inner, label, 13.0);
    }
    if recording && labels.is_empty() {
        inner.label(RichText::new("Press keys").size(13.0).color(FAINT));
    }
    let line = match (bad, recording, response.hovered()) {
        (true, _, _) => RED,
        (false, true, _) => TEAL,
        (false, false, true) => FAINT,
        (false, false, false) => FIELD_LINE,
    };
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    if response.clicked() {
        if recording {
            editor.revert_hotkey();
        } else {
            editor.recording = true;
        }
    } else if recording && response.clicked_elsewhere() {
        editor.revert_hotkey();
    }
    if !recording {
        return None;
    }
    let pressed = ui.input(|input| {
        input.events.iter().find_map(|event| match event {
            Event::Key { key, physical_key, pressed: true, repeat: false, modifiers } => {
                Some((physical_key.unwrap_or(*key), *modifiers))
            }
            _ => None,
        })
    });
    ui.input_mut(|input| input.events.retain(|event| !matches!(event, Event::Key { .. })));
    let (key, modifiers) = pressed?;
    if key == egui::Key::Escape && !modifiers.any() {
        editor.revert_hotkey();
        return None;
    }
    hotkey::spell(modifiers, key).and_then(|chord| editor.record(chord))
}

#[cfg(test)]
mod tests;
