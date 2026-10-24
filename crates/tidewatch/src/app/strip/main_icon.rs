//! The main checkout of the repo on show, drawn in the strip as a square icon next to the repo chip instead of a tab.

use egui::{Align2, FontId, Rect, Sense, Shape, vec2};
use stdusk_core::terminal::CmdState;
use stdusk_egui::ui::TAB_H;

use super::{FrameIntent, TAB_INSET_Y, Tidewatch, nav_look, tab_signals};
use crate::brand;
use crate::commands::Command;
use crate::pane::Pane;
use crate::tabs::{Action, Tabs};

pub(super) const WIDTH: f32 = 32.0;
const GLYPH: &str = "\u{E47E}";
const GLYPH_SIZE: f32 = 17.0;
const DOT: f32 = 8.0;
const DOT_RING: f32 = 11.0;
const DOT_INSET: f32 = 2.0;

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
    egui::pos2(body.right() - DOT_INSET, body.top() + DOT_INSET)
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
    let (rect, resp) = ui.allocate_exact_size(vec2(WIDTH, TAB_H), Sense::click());
    let painter = ui.painter().clone();
    let under = painter.add(Shape::Noop);
    nav_look(&painter, under, rect, selected, resp.hovered());
    let ink = if selected {
        brand::TEAL
    } else if resp.hovered() {
        brand::TEXT
    } else {
        brand::MUTED
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        GLYPH,
        FontId::proportional(GLYPH_SIZE),
        ink,
    );
    if running {
        let center = dot_center(rect.shrink2(vec2(0.0, TAB_INSET_Y)));
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
