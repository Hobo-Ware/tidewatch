//! The General page: the restore switch, the Window section and the Updates section.

use egui::Ui;

use crate::settings_ui::{Control, footer_note, page_header, section, switch_row};
use crate::settings_view::Action;
use crate::updates::Updates;
use crate::updates_view;
use crate::window_settings;
use crate::window_view;

pub(crate) struct General {
    pub(crate) restore: bool,
    pub(crate) skipped_by_flag: bool,
    pub(crate) window: window_view::View,
}

pub(crate) fn page(
    ui: &mut Ui,
    general: &General,
    window: &mut window_settings::Editor,
    updates: &mut Updates,
    file: &str,
) -> Option<Action> {
    page_header(ui, "General", "Saved as you change them");
    let mut restore = None;
    section(ui, "Startup", |ui| {
        restore = switch_row(
            ui,
            "Restore the last session",
            "Tabs reopen where they were and agents pick up their conversation.",
            general.restore,
        );
        if general.skipped_by_flag {
            crate::settings_ui::row(
                ui,
                "Skipped on this run",
                "This run was started with --no-restore, so nothing was restored.",
                Control::NONE,
                |_| {},
            );
        }
    });
    let window = window_view::section(ui, window, &general.window).map(Action::Window);
    let updates = updates_view::section(ui, updates).map(Action::Updates);
    footer_note(ui, file);
    restore.map(Action::Restore).or(window).or(updates)
}
