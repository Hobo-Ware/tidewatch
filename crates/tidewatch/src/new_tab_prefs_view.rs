//! The New tabs card of Settings, General: what the new-tab form opens with (the `[new_tab]` table
//! of `settings.toml`).

use egui::{CornerRadius, Id, Popup, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use tidewatch_core::agents::Level;
use tidewatch_core::prefs::{NewTabPrefs, RightPanel, StartFrom};

use crate::brand;
use crate::screen_kit::{body, paint_cli_badge};
use crate::segmented;
use crate::settings_ui::{self, Control, note, section};

const FIELD_W: f32 = 240.0;
const FIELD_H: f32 = 34.0;
const BADGE: f32 = 22.0;
const LAST_USED: &str = "Last used";
const SEGMENT_H: f32 = 28.0;
const AGENT_ID: &str = "new-tab-prefs-agent";
const PLACED: &str = "new-tab-prefs-placed";
const OPTIONS: &str = "new-tab-prefs-options";
const START_FROM: [(StartFrom, &str); 2] =
    [(StartFrom::Current, "Current branch"), (StartFrom::DefaultBranch, "Default branch")];
const RIGHT_PANEL: [(RightPanel, &str); 2] =
    [(RightPanel::Progress, "Progress"), (RightPanel::History, "History")];

/// A profile the Agent dropdown offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileOption {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) agent_id: String,
}

/// Where each control was drawn in the last frame, for a headless test to aim at.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Placed {
    pub(crate) agent: Option<Rect>,
    pub(crate) permission: Option<Rect>,
    pub(crate) remote: Option<Rect>,
    pub(crate) start_from: Option<Rect>,
    pub(crate) right_panel: Option<Rect>,
}

pub(crate) fn placed_id() -> Id {
    Id::new(PLACED)
}

/// Where the options of the open Agent dropdown are: Last used, then each profile.
pub(crate) fn options_id() -> Id {
    Id::new(OPTIONS)
}

fn level_labels() -> [&'static str; 5] {
    Level::ALL.map(Level::label)
}

fn control_rect(ui: &Ui, width: f32, height: f32) -> Rect {
    let free = ui.available_rect_before_wrap();
    Rect::from_min_size(
        pos2(free.right() - width, free.center().y - height / 2.0),
        vec2(width, height),
    )
}

/// The card. Returns the whole table when something changed.
pub(crate) fn card(
    ui: &mut Ui,
    prefs: &NewTabPrefs,
    profiles: &[ProfileOption],
) -> Option<NewTabPrefs> {
    let mut changed = None;
    let mut placed = Placed::default();
    section(ui, "New tabs", |ui| {
        let picked = settings_ui::row(ui, "Agent", "", Control::new(FIELD_W, FIELD_H), |ui| {
            placed.agent = Some(control_rect(ui, FIELD_W, FIELD_H));
            agent_field(ui, prefs.agent.as_deref(), profiles)
        });
        if let Some(pick) = picked {
            let agent = match pick {
                AgentPick::LastUsed => None,
                AgentPick::Profile(id) => Some(id),
            };
            changed = Some(NewTabPrefs { agent, ..prefs.clone() });
        }

        let labels = level_labels();
        let width = segmented::inline_width(ui, &labels);
        let active = prefs.permission.and_then(|level| Level::ALL.iter().position(|l| *l == level));
        let tones = Level::ALL.map(|level| (level == Level::Yolo).then_some(brand::FAINT));
        let clicked =
            settings_ui::row(ui, "Permission", "", Control::new(width, SEGMENT_H), |ui| {
                placed.permission = Some(control_rect(ui, width, SEGMENT_H));
                let id = ui.id().with("new-tab-prefs-permission");
                segmented::inline_toned(ui, id, &labels, &tones, active.unwrap_or(usize::MAX))
            });
        if let Some(index) = clicked.filter(|index| Level::ALL[*index] != Level::Yolo) {
            changed = Some(NewTabPrefs { permission: Some(Level::ALL[index]), ..prefs.clone() });
        }

        let mut on = prefs.remote;
        let desc = "Claude Code and Copilot CLI";
        let flipped = settings_ui::row(ui, "Remote control", desc, Control::SWITCH, |ui| {
            let switch = settings_ui::switch(ui, &mut on);
            placed.remote = Some(switch.rect);
            switch.changed()
        });
        if flipped {
            changed = Some(NewTabPrefs { remote: on, ..prefs.clone() });
        }

        let labels = START_FROM.map(|(_, label)| label);
        let active = START_FROM.iter().position(|(from, _)| *from == prefs.start_from);
        let picked = pick_row(ui, "Start from", "new-tab-prefs-start", &labels, active);
        placed.start_from = picked.0;
        if let Some(index) = picked.1 {
            changed = Some(NewTabPrefs { start_from: START_FROM[index].0, ..prefs.clone() });
        }

        let labels = RIGHT_PANEL.map(|(_, label)| label);
        let active = RIGHT_PANEL.iter().position(|(panel, _)| *panel == prefs.right_panel);
        let picked = pick_row(ui, "Right panel", "new-tab-prefs-panel", &labels, active);
        placed.right_panel = picked.0;
        if let Some(index) = picked.1 {
            changed = Some(NewTabPrefs { right_panel: RIGHT_PANEL[index].0, ..prefs.clone() });
        }
    });
    ui.add_space(8.0);
    note(ui, "The new tab form opens with these; you can still change them per tab.", brand::FAINT);
    ui.ctx().data_mut(|data| data.insert_temp(placed_id(), placed));
    changed
}

fn pick_row(
    ui: &mut Ui,
    title: &str,
    id: &str,
    labels: &[&str],
    active: Option<usize>,
) -> (Option<Rect>, Option<usize>) {
    let width = segmented::inline_width(ui, labels);
    settings_ui::row(ui, title, "", Control::new(width, SEGMENT_H), |ui| {
        let rect = control_rect(ui, width, SEGMENT_H);
        let id = ui.id().with(id);
        (Some(rect), segmented::inline(ui, id, labels, active.unwrap_or(usize::MAX)))
    })
}

/// What a pick in the Agent dropdown chose.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AgentPick {
    LastUsed,
    Profile(String),
}

/// The dropdown of profiles. Returns the pick.
fn agent_field(
    ui: &mut Ui,
    current: Option<&str>,
    profiles: &[ProfileOption],
) -> Option<AgentPick> {
    let (rect, response) = ui.allocate_exact_size(vec2(FIELD_W, FIELD_H), Sense::click());
    let shown = current.and_then(|id| profiles.iter().find(|profile| profile.id == id));
    let painter = ui.painter().clone();
    painter.rect(
        rect,
        CornerRadius::same(8),
        brand::SEGMENT,
        Stroke::new(1.0, if response.hovered() { brand::FAINT } else { brand::FIELD_LINE }),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + 10.0;
    if let Some(profile) = shown {
        let badge = Rect::from_min_size(pos2(x, rect.center().y - BADGE / 2.0), vec2(BADGE, BADGE));
        paint_cli_badge(&painter, badge, &profile.agent_id);
        x += BADGE + 10.0;
    }
    let label = match (shown, current) {
        (Some(profile), _) => profile.name.as_str(),
        (None, Some(id)) => id,
        (None, None) => LAST_USED,
    };
    let ink = if current.is_some() { brand::TEXT } else { brand::SOFT };
    let room = rect.right() - 30.0 - x;
    let galley = crate::reader_view::ellipsized(&painter, label, body(14.0), ink, room);
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, ink);
    let chevron = pos2(rect.right() - 16.0, rect.center().y);
    let stroke = Stroke::new(1.5, brand::MUTED);
    painter.line_segment([chevron + vec2(-3.0, -1.5), chevron + vec2(0.0, 1.5)], stroke);
    painter.line_segment([chevron + vec2(0.0, 1.5), chevron + vec2(3.0, -1.5)], stroke);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    kelp_ui::widgets::focus_ring(ui, &response, 8.0);
    let mut picked = None;
    Popup::menu(&response).id(Id::new(AGENT_ID)).width(FIELD_W).show(|ui| {
        ui.set_min_width(FIELD_W);
        let mut rects = Vec::with_capacity(profiles.len() + 1);
        let last = ui.add(egui::Button::selectable(current.is_none(), option_text(LAST_USED)));
        rects.push(last.rect);
        if last.clicked() {
            picked = Some(AgentPick::LastUsed);
            ui.close();
        }
        for profile in profiles {
            let on = current == Some(profile.id.as_str());
            let option = ui.add(egui::Button::selectable(on, option_text(&profile.name)));
            rects.push(option.rect);
            if option.clicked() {
                picked = Some(AgentPick::Profile(profile.id.clone()));
                ui.close();
            }
        }
        ui.ctx().data_mut(|data| data.insert_temp(options_id(), rects));
    });
    picked
}

fn option_text(text: &str) -> RichText {
    RichText::new(text).size(14.0)
}

#[cfg(test)]
mod tests;
