//! The "Open at login" row of Settings > General.

use egui::Ui;

use crate::macos::LoginItem;
use crate::screen_kit::Size;
use crate::settings_ui::{self, Control, Tone};

const REFRESH_SECONDS: f64 = 1.0;
const CONTROL_GAP: f32 = 10.0;
const CONTROL_HEIGHT: f32 = Size::Regular.height();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    pub(crate) on: bool,
    pub(crate) enabled: bool,
    pub(crate) approve: bool,
    pub(crate) text: &'static str,
}

const ASK: &str = "Start Tidewatch when you log in to your Mac.";

pub(crate) fn look(status: LoginItem) -> Look {
    match status {
        LoginItem::Off | LoginItem::NotFound => {
            Look { on: false, enabled: true, approve: false, text: ASK }
        }
        LoginItem::On => Look { on: true, enabled: true, approve: false, text: ASK },
        LoginItem::NeedsApproval => Look {
            on: true,
            enabled: true,
            approve: true,
            text: "Allow Tidewatch in System Settings > General > Login Items to finish.",
        },
        LoginItem::Unavailable => Look {
            on: false,
            enabled: false,
            approve: false,
            text: "Needs the installed Tidewatch.app on macOS 13 or newer.",
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Click {
    Set(bool),
    Approve,
}

#[derive(Default)]
pub(crate) struct Row {
    status: Option<LoginItem>,
    checked_at: Option<f64>,
}

impl Row {
    fn refresh(&mut self, now: f64, read: impl FnOnce() -> LoginItem) -> LoginItem {
        let due = self.checked_at.is_none_or(|last| now < last || now - last >= REFRESH_SECONDS);
        if let Some(status) = self.status.filter(|_| !due) {
            return status;
        }
        let status = read();
        self.status = Some(status);
        self.checked_at = Some(now);
        status
    }

    pub(crate) fn read_again(&mut self) {
        self.status = None;
        self.checked_at = None;
    }
}

pub(crate) fn show(
    ui: &mut Ui,
    row: &mut Row,
    now: f64,
    read: impl FnOnce() -> LoginItem,
) -> Option<Click> {
    let look = look(row.refresh(now, read));
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(REFRESH_SECONDS));
    let approve_label = "Approve in System Settings";
    let approve_width = if look.approve {
        settings_ui::button_width(ui, approve_label, Size::Compact) + CONTROL_GAP
    } else {
        0.0
    };
    let control = Control::new(Control::SWITCH.width + approve_width, CONTROL_HEIGHT);
    settings_ui::row(ui, "Open at login", look.text, control, |ui| {
        let mut on = look.on;
        let switched =
            ui.add_enabled_ui(look.enabled, |ui| settings_ui::switch(ui, &mut on).changed()).inner;
        ui.spacing_mut().item_spacing.x = CONTROL_GAP;
        let approved = look.approve
            && settings_ui::button(ui, approve_label, Tone::Quiet, Size::Compact).clicked();
        if approved { Some(Click::Approve) } else { switched.then_some(Click::Set(on)) }
    })
}

#[cfg(test)]
mod tests;
