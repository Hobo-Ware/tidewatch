//! The GitHub card of Settings, General: what a new tab does when its pull request changes (the
//! `[github]` table of `settings.toml`). A tab keeps its own choice from the PR strip.

use egui::Ui;
use tidewatch_core::prefs::{GithubPrefs, PrMode};

use crate::settings_ui::{self, Control, section, switch_row};

const MODES: [(PrMode, &str); 3] =
    [(PrMode::Ask, "Ask me"), (PrMode::Auto, "Auto-fix when idle"), (PrMode::Off, "Off")];

/// The card. Returns the whole table when something changed.
pub(crate) fn card(ui: &mut Ui, github: GithubPrefs) -> Option<GithubPrefs> {
    let mut changed = None;
    section(ui, "GitHub", |ui| {
        let labels = MODES.map(|(_, label)| label);
        let active = MODES.iter().position(|(mode, _)| *mode == github.mode);
        let picked = settings_ui::row(
            ui,
            "When a pull request changes",
            "Ask me shows a card with one button. Auto-fix sends the same message itself, only when the agent is idle, at most 3 times per PR. A tab can change it from its PR strip.",
            Control::new(settings_ui::inline_width(ui, &labels), 28.0),
            |ui| settings_ui::pick(ui, "github-mode", &labels, active),
        );
        if let Some(index) = picked {
            changed = Some(GithubPrefs { mode: MODES[index].0, ..github });
        }
        if let Some(on) =
            switch_row(ui, "Failed checks", "With the failing log lines.", github.failed_checks)
        {
            changed = Some(GithubPrefs { failed_checks: on, ..github });
        }
        if let Some(on) = switch_row(
            ui,
            "Review comments and requested changes",
            "Comments on the pull request and in a review.",
            github.reviews,
        ) {
            changed = Some(GithubPrefs { reviews: on, ..github });
        }
        if let Some(on) =
            switch_row(ui, "Merge conflicts", "With the base branch.", github.conflicts)
        {
            changed = Some(GithubPrefs { conflicts: on, ..github });
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, RawInput, Rect, pos2, vec2};

    #[test]
    fn the_card_draws_headless_and_changes_nothing_by_itself() {
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        for github in [
            GithubPrefs::default(),
            GithubPrefs { mode: PrMode::Auto, failed_checks: false, ..GithubPrefs::default() },
        ] {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                ..RawInput::default()
            };
            let mut changed = None;
            let _ = ctx.run_ui(raw, |ui| changed = card(ui, github));
            assert_eq!(changed, None);
        }
    }
}
