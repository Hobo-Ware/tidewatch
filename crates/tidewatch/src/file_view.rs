//! The file view of the side layout: a file's diff in the terminal's place, under a header with the
//! Terminal chip that goes back, the file's path and the commit it is from.

use egui::{Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use kelp_ui::theme;
use tidewatch_core::status::AgentStatus;

use crate::brand;
use crate::commit_files::tail_fit;
use crate::diff_pane::{
    PaneAction, PaneOut, SHORT_ID, SIDE_PAD, TOOLBAR_H, layout_switch, message, send_controls,
    show_body, show_toolbar, take_row, toolbar_pr,
};
use crate::flow::PaneExtras;
use crate::notes_strip;
use crate::status_view;
use crate::tab_view::TabView;

const HEADER_H: f32 = 46.0;
const CHIP_H: f32 = 28.0;
const SWITCH_W: f32 = 150.0;
const CONTROLS_W: f32 = 270.0;

/// What the Terminal chip says about the agent behind the terminal: its profile and state.
pub(crate) struct TerminalChip {
    pub(crate) profile: Option<String>,
    pub(crate) status: Option<AgentStatus>,
}

fn state_word(status: &AgentStatus) -> Option<&'static str> {
    match status {
        AgentStatus::SettingUp => Some("starting"),
        AgentStatus::Working => Some("working"),
        AgentStatus::NeedsYou(_) => Some("needs you"),
        AgentStatus::Done => Some("ready"),
        AgentStatus::Failed(_) => Some("failed"),
        AgentStatus::Shell => None,
    }
}

impl TerminalChip {
    /// `Builder working`, or nothing for a plain shell.
    pub(crate) fn text(&self) -> Option<String> {
        let word = state_word(self.status.as_ref()?)?;
        Some(match &self.profile {
            Some(profile) => format!("{profile} {word}"),
            None => word.to_owned(),
        })
    }

    fn dot(&self) -> Option<Color32> {
        self.status.as_ref().and_then(|status| status_view::color(status_view::tone(status)))
    }
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    extras: Option<&PaneExtras>,
    chip: &TerminalChip,
    now: i64,
) -> PaneOut {
    ui.painter().rect_filled(rect, 0.0, theme::bg());
    let mut out = PaneOut::default();
    ui.scope_builder(egui::UiBuilder::new().id_salt("file-view").max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let header = take_row(ui, rect, 0.0, HEADER_H);
        out.back = show_header(ui, header, view, chip);
        let mut top = HEADER_H;
        let toolbar = take_row(ui, rect, top, TOOLBAR_H);
        let controls_w = if extras.is_some() { CONTROLS_W } else { 0.0 };
        let left =
            Rect::from_min_max(toolbar.min, pos2(toolbar.right() - controls_w, toolbar.max.y));
        show_toolbar(ui, left, view, false);
        top += TOOLBAR_H;
        if let Some(extras) = extras {
            send_controls(ui, toolbar.shrink2(vec2(SIDE_PAD - 4.0, 0.0)), extras, &mut out.actions);
            if extras.chip.is_some() {
                let pr = take_row(ui, rect, top, TOOLBAR_H);
                toolbar_pr(ui, pr, extras, &mut out.actions);
                top += TOOLBAR_H;
            }
        }
        let strip_h = extras.map_or(0.0, |e| notes_strip::height(e.rows.len()));
        if let Some(extras) = extras.filter(|_| strip_h > 0.0) {
            let strip = take_row(ui, rect, top, strip_h);
            let actions = notes_strip::show(ui, strip, &extras.rows, now);
            out.actions.extend(actions.into_iter().map(PaneAction::Strip));
            top += strip_h;
        }
        let body = Rect::from_min_max(pos2(rect.left(), rect.top() + top), rect.max);
        if view.loaded.is_none() {
            message(ui, body, view.error.as_deref(), "Loading history...");
            return;
        }
        out.notes_changed = show_body(ui, body, view);
    });
    out
}

fn show_header(ui: &mut Ui, rect: Rect, view: &mut TabView, chip: &TerminalChip) -> bool {
    let mut at = pos2(rect.left() + SIDE_PAD + 2.0, rect.center().y);
    let back = terminal_chip(ui, at, chip);
    at.x = back.rect.right() + 14.0;
    let painter = ui.painter_at(rect);
    let from = view.summary.as_ref().map_or_else(
        || "working tree".to_owned(),
        |(id, _)| id.to_hex_with_len(SHORT_ID).to_string(),
    );
    let sha = painter.layout_no_wrap(
        format!("in {from}"),
        FontId::proportional(12.0),
        theme::text_muted(),
    );
    let limit = rect.right() - SIDE_PAD - SWITCH_W - 10.0;
    let font = FontId::monospace(13.0);
    let path = view.file.as_deref().unwrap_or("");
    let room = (limit - at.x - sha.size().x - 10.0).max(40.0);
    let shown = tail_fit(ui, path, &font, room);
    let galley = painter.layout_no_wrap(shown, font, theme::text_strong());
    let path_w = galley.size().x;
    painter.galley(pos2(at.x, at.y - galley.size().y / 2.0), galley, theme::text_strong());
    painter.galley(pos2(at.x + path_w + 10.0, at.y - sha.size().y / 2.0), sha, theme::text_muted());

    let switch = Rect::from_min_max(
        pos2(rect.right() - SIDE_PAD - SWITCH_W, rect.top()),
        pos2(rect.right() - SIDE_PAD + 4.0, rect.bottom()),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("file-view-switch")
            .max_rect(switch)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            layout_switch(ui, view, true);
        },
    );
    back.clicked()
}

fn terminal_chip(ui: &mut Ui, left_center: Pos2, chip: &TerminalChip) -> Response {
    let font = FontId::proportional(13.0);
    let label = ui.painter().layout_no_wrap("Terminal".to_owned(), font.clone(), brand::SOFT);
    let state = chip
        .text()
        .map(|text| ui.painter().layout_no_wrap(text, font.clone(), theme::text_muted()));
    let dot_w = if chip.dot().is_some() { 15.0 } else { 0.0 };
    let state_w = state.as_ref().map_or(0.0, |galley| 10.0 + dot_w + galley.size().x);
    let width = 10.0 + 12.0 + 8.0 + label.size().x + state_w + 12.0;
    let rect =
        Rect::from_min_size(pos2(left_center.x, left_center.y - CHIP_H / 2.0), vec2(width, CHIP_H));
    let response = ui.interact(rect, egui::Id::new("terminal-chip"), Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() { brand::BUTTON } else { brand::CARD };
    painter.rect_filled(rect, 8, fill);
    let caret = pos2(rect.left() + 14.0, rect.center().y);
    let stroke = Stroke::new(1.6, brand::SOFT);
    painter.line_segment([caret + vec2(2.5, -4.0), caret + vec2(-1.5, 0.0)], stroke);
    painter.line_segment([caret + vec2(-1.5, 0.0), caret + vec2(2.5, 4.0)], stroke);
    let mut x = rect.left() + 10.0 + 12.0 + 8.0;
    painter.galley(pos2(x, rect.center().y - label.size().y / 2.0), label.clone(), brand::SOFT);
    x += label.size().x + 10.0;
    if let Some(state) = state {
        if let Some(color) = chip.dot() {
            painter.circle_filled(pos2(x + 3.5, rect.center().y), 3.5, color);
            x += dot_w;
        }
        painter.galley(pos2(x, rect.center().y - state.size().y / 2.0), state, theme::text_muted());
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::status::Reason;

    fn chip(profile: Option<&str>, status: Option<AgentStatus>) -> TerminalChip {
        TerminalChip { profile: profile.map(str::to_owned), status }
    }

    #[test]
    fn the_chip_names_the_profile_and_what_it_is_doing() {
        let table = [
            (AgentStatus::Working, "Builder working"),
            (AgentStatus::Done, "Builder ready"),
            (AgentStatus::NeedsYou(Reason::Permission), "Builder needs you"),
            (AgentStatus::SettingUp, "Builder starting"),
            (AgentStatus::Failed("x".into()), "Builder failed"),
        ];
        for (status, want) in table {
            assert_eq!(chip(Some("Builder"), Some(status)).text().as_deref(), Some(want));
        }
    }

    #[test]
    fn a_plain_shell_has_a_bare_chip() {
        assert_eq!(chip(None, Some(AgentStatus::Shell)).text(), None);
        assert_eq!(chip(None, None).text(), None);
        assert_eq!(chip(None, Some(AgentStatus::Working)).text().as_deref(), Some("working"));
    }
}
