//! The main checkout of the repo on show, drawn in the strip as a round `main` entry after the repo name instead of a tab.

use super::{FrameIntent, TAB_FONT_PX, TAB_H, TAB_RADIUS, Tidewatch, tab_signals};
use crate::brand;
use crate::commands::Command;
use crate::pane::Pane;
use crate::tabs::{Action, Tabs};
use egui::{Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use stdusk_core::terminal::CmdState;

const GLYPH: &str = "\u{E47E}";
const GLYPH_SIZE: f32 = 20.0;
const LABEL: &str = "main";
const PAD: f32 = 12.0;
const GAP: f32 = 7.0;
const DOT: f32 = 8.0;
const DOT_RING: f32 = 11.0;

pub(super) fn tip(label: &str, branch: Option<&str>, running: bool) -> String {
    let keys = Command::SelectTab(0).keys();
    let mut parts = vec!["main", label];
    parts.extend(branch);
    let mut tip = format!("{} \u{b7} {keys}", parts.join(" \u{b7} "));
    if running {
        tip.push_str("\nRunning a command");
    }
    tip
}

pub(super) fn dot_center(body: Rect) -> egui::Pos2 {
    pos2(body.left() + PAD + GLYPH_SIZE - 1.0, body.center().y - GLYPH_SIZE / 2.0 + 1.0)
}

/// What the icon shows of the main shell of the repo on show: whether a command runs and its tooltip.
fn describe(label: &str, branch: Option<&str>, cmd: CmdState) -> (bool, String) {
    let running = cmd == CmdState::Running;
    (running, tip(label, branch, running))
}

/// Draws the icon of the active group's main tab and asks to select it on a click. `None` when
/// the group has no main tab.
pub(super) fn draw<T>(
    ui: &mut egui::Ui,
    tabs: &Tabs<Pane<T>>,
    describe: impl FnOnce(&Pane<T>) -> (bool, String),
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let (index, tab) = tabs.in_group(tabs.active_group()).find(|(_, tab)| tab.pinned)?;
    let selected = index == tabs.active();
    let (running, tip) = describe(&tab.pane);
    let font = if selected {
        crate::screen_kit::heading(TAB_FONT_PX)
    } else {
        FontId::proportional(TAB_FONT_PX)
    };
    let ink = if selected { brand::TEAL_TEXT } else { brand::SOFT };
    let galley = ui.painter().layout_no_wrap(LABEL.to_owned(), font, ink);
    let width = PAD + GLYPH_SIZE + GAP + galley.size().x + PAD;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, TAB_H), Sense::click());
    let painter = ui.painter().clone();
    let line = if selected {
        Some(brand::TEAL)
    } else if resp.hovered() {
        Some(brand::FIELD_LINE)
    } else {
        None
    };
    if let Some(line) = line {
        painter.rect_stroke(
            rect,
            CornerRadius::same(TAB_RADIUS),
            Stroke::new(1.0, line),
            StrokeKind::Inside,
        );
    }
    let glyph = Rect::from_min_size(
        pos2(rect.left() + PAD, rect.center().y - GLYPH_SIZE / 2.0),
        vec2(GLYPH_SIZE, GLYPH_SIZE),
    );
    painter.text(
        glyph.center(),
        Align2::CENTER_CENTER,
        GLYPH,
        FontId::proportional(GLYPH_SIZE),
        ink,
    );
    painter.galley(pos2(glyph.right() + GAP, rect.center().y - galley.size().y / 2.0), galley, ink);
    if running {
        let center = dot_center(rect);
        painter.circle_filled(center, DOT_RING / 2.0, brand::SIDEBAR);
        painter.circle_filled(center, DOT / 2.0, brand::TEAL);
    }
    let rect = resp.rect;
    if resp.on_hover_text(tip).clicked() {
        actions.push(Action::Select(index));
    }
    Some(rect)
}

impl Tidewatch {
    pub(super) fn main_icon(&self, ui: &mut egui::Ui, frame: &mut FrameIntent) {
        let workspace = &self.workspaces[self.tabs.active_group()];
        let (label, branch) = (workspace.chip_label(), self.bases.get(workspace.dir()));
        draw(
            ui,
            &self.tabs,
            |pane| describe(&label, branch, tab_signals(pane).1),
            &mut frame.actions,
        );
    }
}
