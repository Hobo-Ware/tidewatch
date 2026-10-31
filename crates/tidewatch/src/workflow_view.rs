//! The workflow profile on screen: the confirm-once card under the tab strip, and the Workflow card
//! of Settings, General.

use egui::{Context, Id, Margin, Order, Rect, RichText, Stroke, Ui, pos2};
use tidewatch_core::workflow_detect::{Detected, Source};
use tidewatch_core::workflow_profile::{CatchUp, Fixes, PullRequests, Workflow};

use crate::brand;
use crate::layout;
use crate::screen_kit::{Size, TEXT_SECONDARY, primary_button, quiet_button};
use crate::settings_ui::{self, Control, row, section};

const CATCH_UP: [(CatchUp, &str); 2] = [(CatchUp::Rebase, "Rebase"), (CatchUp::Merge, "Merge")];
const FIXES: [(Fixes, &str); 2] =
    [(Fixes::Fixup, "Into their commit (fixup)"), (Fixes::NewCommits, "New commits")];
const PULL_REQUESTS: [(PullRequests, &str); 2] =
    [(PullRequests::Draft, "Draft"), (PullRequests::Ready, "Ready")];

const MAX_WIDTH: f32 = 560.0;
const BELOW_STRIP: f32 = layout::CHROME_H + 12.0;
const CARD_ID: &str = "workflow_card";

fn label_of<T: PartialEq + Copy>(table: &[(T, &'static str)], value: T) -> &'static str {
    table.iter().find(|(have, _)| *have == value).map_or("", |(_, label)| label)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Accept,
    Change,
    NotNow,
}

/// The card that offers the detected values once. Returns what was pressed. Esc answers `NotNow`
/// only while the pointer is over the card or one of its buttons has the keyboard, so an agent's
/// own Esc is never taken.
pub(crate) fn card(ctx: &Context, repo: &str, detected: &Detected, column: Rect) -> Option<Answer> {
    let mut shown = Shown::default();
    let area = egui::Area::new(Id::new(CARD_ID))
        .order(Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(pos2(column.center().x, column.top() + BELOW_STRIP))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(brand::ITEM)
                .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(16, 12))
                .shadow(egui::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(100),
                })
                .show(ui, |ui| {
                    ui.set_max_width((column.width() - 32.0).clamp(240.0, MAX_WIDTH));
                    ui.spacing_mut().item_spacing.y = 8.0;
                    shown = body(ui, repo, detected);
                });
        });
    let engaged = area.response.contains_pointer() || shown.focused;
    if shown.answer.is_none()
        && engaged
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        return Some(Answer::NotNow);
    }
    shown.answer
}

#[derive(Default)]
struct Shown {
    answer: Option<Answer>,
    focused: bool,
}

fn body(ui: &mut Ui, repo: &str, detected: &Detected) -> Shown {
    let mut shown = Shown::default();
    ui.label(
        RichText::new(format!("Workflow for {repo}"))
            .font(crate::screen_kit::heading(15.0))
            .color(brand::TEXT),
    );
    ui.label(
        RichText::new("What Tidewatch may say about branches and pull requests when it talks to an agent. Your rules win; these are what I found in them.")
            .size(TEXT_SECONDARY)
            .color(brand::MUTED),
    );
    let rows = [
        (
            "Branches catch up by",
            label_of(&CATCH_UP, detected.catch_up.value),
            &detected.catch_up.source,
        ),
        ("Fixes go", label_of(&FIXES, detected.fixes.value), &detected.fixes.source),
        (
            "New pull requests open as",
            label_of(&PULL_REQUESTS, detected.pull_requests.value),
            &detected.pull_requests.source,
        ),
    ];
    for (title, value, source) in rows {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(title).size(TEXT_SECONDARY).color(brand::SOFT));
            ui.label(
                RichText::new(value)
                    .font(crate::screen_kit::heading(TEXT_SECONDARY))
                    .color(brand::TEXT),
            );
        });
        evidence(ui, source);
    }
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let accept = primary_button(ui, "Accept", true, Size::Compact);
        let change = quiet_button(ui, "Change", true, Size::Compact);
        let not_now = quiet_button(ui, "Not now", true, Size::Compact);
        shown.focused = accept.has_focus() || change.has_focus() || not_now.has_focus();
        shown.answer = if accept.clicked() {
            Some(Answer::Accept)
        } else if change.clicked() {
            Some(Answer::Change)
        } else if not_now.clicked() {
            Some(Answer::NotNow)
        } else {
            None
        };
    });
    shown
}

fn evidence(ui: &mut Ui, source: &Source) {
    ui.add(egui::Label::new(RichText::new(source.line()).size(12.0).color(brand::MUTED)).wrap());
}

/// What the Workflow card of Settings shows for the repo on show.
#[derive(Clone, Debug)]
pub(crate) struct View {
    pub(crate) path: std::path::PathBuf,
    pub(crate) repo: String,
    pub(crate) workflow: Workflow,
    pub(crate) detected: Option<Detected>,
    pub(crate) confirmed: bool,
}

impl View {
    fn evidence<T: PartialEq + Copy>(
        &self,
        chosen: T,
        pick: impl Fn(&Detected) -> (T, &Source),
    ) -> String {
        match self.detected.as_ref().map(pick) {
            Some((found, source)) if found == chosen => source.line(),
            Some(_) if self.confirmed => "set by you".to_owned(),
            _ => String::new(),
        }
    }
}

/// The card. Returns the whole profile when something changed.
pub(crate) fn settings_card(ui: &mut Ui, view: Option<&View>) -> Option<Workflow> {
    let mut changed = None;
    let Some(view) = view else {
        section(ui, "Workflow", |ui| {
            row(
                ui,
                "Open a git repo to set its workflow",
                "The profile belongs to a repo: how its branches catch up, where fixes go and how pull requests open.",
                Control::NONE,
                |_| {},
            );
        });
        return None;
    };
    let current = view.workflow;
    section(ui, &format!("Workflow: {}", view.repo), |ui| {
        let labels = CATCH_UP.map(|(_, label)| label);
        let active = CATCH_UP.iter().position(|(value, _)| *value == current.catch_up);
        let note = view.evidence(current.catch_up, |d| (d.catch_up.value, &d.catch_up.source));
        let picked = row(
            ui,
            "Branches catch up by",
            &note,
            Control::new(settings_ui::inline_width(ui, &labels), 28.0),
            |ui| settings_ui::pick(ui, "workflow-catch-up", &labels, active),
        );
        if let Some(index) = picked {
            changed = Some(Workflow { catch_up: CATCH_UP[index].0, ..current });
        }
        let labels = FIXES.map(|(_, label)| label);
        let active = FIXES.iter().position(|(value, _)| *value == current.fixes);
        let note = view.evidence(current.fixes, |d| (d.fixes.value, &d.fixes.source));
        let picked = row(
            ui,
            "Fixes go",
            &note,
            Control::new(settings_ui::inline_width(ui, &labels), 28.0),
            |ui| settings_ui::pick(ui, "workflow-fixes", &labels, active),
        );
        if let Some(index) = picked {
            changed = Some(Workflow { fixes: FIXES[index].0, ..current });
        }
        let push = if current.pushes_unpushed_with_lease() {
            "--force-with-lease"
        } else {
            "a plain push"
        };
        row(
            ui,
            "Push after a rewrite",
            &format!("{push}, from the two choices above"),
            Control::NONE,
            |_| {},
        );
        let labels = PULL_REQUESTS.map(|(_, label)| label);
        let active = PULL_REQUESTS.iter().position(|(value, _)| *value == current.pull_requests);
        let note = view
            .evidence(current.pull_requests, |d| (d.pull_requests.value, &d.pull_requests.source));
        let picked = row(
            ui,
            "New pull requests open as",
            &note,
            Control::new(settings_ui::inline_width(ui, &labels), 28.0),
            |ui| settings_ui::pick(ui, "workflow-pull-requests", &labels, active),
        );
        if let Some(index) = picked {
            changed = Some(Workflow { pull_requests: PULL_REQUESTS[index].0, ..current });
        }
    });
    changed
}

#[cfg(test)]
mod tests;
