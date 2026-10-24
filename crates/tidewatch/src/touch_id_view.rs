//! The "Touch ID for Amphetamine" row of Settings > General > Keep awake.

use egui::Ui;

use crate::screen_kit::Size;
use crate::settings_ui::{self, Control, Tone};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TouchId {
    Missing,
    Installed,
    Working,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    SetUp,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    pub(crate) desc: &'static str,
    pub(crate) button: &'static str,
    pub(crate) click: Option<Click>,
}

pub(crate) fn look(state: TouchId) -> Look {
    match state {
        TouchId::Missing => Look {
            desc: "Ask for your fingerprint instead of your password when Amphetamine turns on. Setting it up asks for your password once and installs a small helper owned by root.",
            button: "Set up",
            click: Some(Click::SetUp),
        },
        TouchId::Installed => Look {
            desc: "Amphetamine asks for your fingerprint, or your login password on a Mac without Touch ID. Remove deletes the helper and its sudo rule.",
            button: "Remove",
            click: Some(Click::Remove),
        },
        TouchId::Working => {
            Look { desc: "Waiting for macOS to finish.", button: "Working...", click: None }
        }
    }
}

pub(crate) fn show(ui: &mut Ui, state: TouchId) -> Option<Click> {
    let look = look(state);
    let control = Control::button(ui, look.button, Size::Compact);
    settings_ui::row(ui, "Touch ID for Amphetamine", look.desc, control, |ui| {
        ui.add_enabled_ui(look.click.is_some(), |ui| {
            settings_ui::button(ui, look.button, Tone::Quiet, Size::Compact).clicked()
        })
        .inner
        .then_some(look.click)
        .flatten()
    })
}

#[cfg(test)]
mod tests;
