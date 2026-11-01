//! The toolbar under one of the owner's own bubbles in the Reader: Edit and send, Copy and Send
//! again as three icon buttons in one small card, or Send again and Dismiss under a message the
//! agent never showed. It floats on its own layer in the gap under the bubble, so the bubble, its
//! `You` line and the space to the next row stay as they are.

use egui::epaint::Shadow;
use egui::{
    Area, Color32, CornerRadius, Id, LayerId, Order, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, pos2, vec2,
};

use crate::brand;
use crate::code_block;
use crate::screen_kit::Size;

const PAD: f32 = 3.0;
const GAP: f32 = 2.0;
const BORDER: f32 = 1.0;
const BUTTON: f32 = Size::Compact.height();
const ICON: f32 = 16.0;
const RADIUS: u8 = 10;
const BUTTON_RADIUS: u8 = 7;
const BELOW: f32 = 2.0;
/// Space added under a bubble so the toolbar ends before the next row's first line.
pub(crate) const ROOM: f32 = 16.0;
const COPIED_SECS: f64 = 1.5;
const GLYPH_STROKE: f32 = 1.3;
const SHADOW: Shadow =
    Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(96) };

/// What a click on a bubble's toolbar asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BubbleAction {
    SendAgain,
    EditAndSend,
    Dismiss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    EditAndSend,
    Copy,
    SendAgain,
    Dismiss,
}

impl Tool {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::EditAndSend => "Edit and send",
            Self::Copy => "Copy",
            Self::SendAgain => "Send again",
            Self::Dismiss => "Dismiss",
        }
    }

    fn action(self) -> Option<BubbleAction> {
        match self {
            Self::EditAndSend => Some(BubbleAction::EditAndSend),
            Self::Copy => None,
            Self::SendAgain => Some(BubbleAction::SendAgain),
            Self::Dismiss => Some(BubbleAction::Dismiss),
        }
    }
}

/// Which message the toolbar belongs to: one the agent has, or one it never showed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Sent,
    NotSeen,
}

impl Kind {
    pub(crate) fn tools(self) -> &'static [Tool] {
        match self {
            Self::Sent => &[Tool::EditAndSend, Tool::Copy, Tool::SendAgain],
            Self::NotSeen => &[Tool::SendAgain, Tool::Dismiss],
        }
    }

    fn always(self) -> bool {
        self == Self::NotSeen
    }
}

/// Where the last frame put a toolbar and its bubble, for the headless tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    pub(crate) bubble: Rect,
    pub(crate) toolbar: Rect,
}

/// The id of a tool's button, the same in every frame.
pub(crate) fn tool_id(seq: u64, tool: Tool) -> Id {
    Id::new(("bubble-tool", seq, tool.name()))
}

#[cfg(test)]
pub(crate) fn placed_id(seq: u64) -> Id {
    Id::new(("bubble-toolbar-placed", seq))
}

fn size_of(count: usize) -> egui::Vec2 {
    let buttons = count as f32 * BUTTON + count.saturating_sub(1) as f32 * GAP;
    vec2(buttons + 2.0 * (PAD + BORDER), BUTTON + 2.0 * (PAD + BORDER))
}

/// The toolbar's rectangle: right-aligned with the bubble, a hair under it.
pub(crate) fn toolbar_rect(bubble: Rect, kind: Kind) -> Rect {
    let size = size_of(kind.tools().len());
    Rect::from_min_size(pos2(bubble.right() - size.x, bubble.bottom() + BELOW), size)
}

/// Draws the toolbar while the pointer is over the message or the toolbar, and always for a
/// message the agent did not show. The text is what Copy puts on the clipboard.
pub(crate) fn show(
    ui: &mut Ui,
    seq: u64,
    text: &str,
    kind: Kind,
    (bubble, whole): (Rect, Rect),
) -> Option<BubbleAction> {
    let toolbar = toolbar_rect(bubble, kind);
    let area_id = ui.id().with(("bubble-toolbar", seq));
    let layer = LayerId::new(Order::Foreground, area_id);
    let reach = [whole, toolbar.expand2(vec2(0.0, BELOW))];
    let over = ui.input(|input| input.pointer.hover_pos()).is_some_and(|at| {
        reach.iter().any(|rect| rect.contains(at))
            && ui.ctx().layer_id_at(at).is_none_or(|top| top == ui.layer_id() || top == layer)
    });
    if !(kind.always() || over) {
        return None;
    }
    #[cfg(test)]
    ui.ctx().data_mut(|data| data.insert_temp(placed_id(seq), Placed { bubble, toolbar }));
    let mut asked = None;
    Area::new(area_id)
        .order(Order::Foreground)
        .fixed_pos(toolbar.min)
        .movable(false)
        .sense(Sense::hover())
        .fade_in(false)
        .constrain(false)
        .show(ui.ctx(), |area| {
            area.set_min_size(toolbar.size());
            let painter = area.painter().clone();
            painter.add(SHADOW.as_shape(toolbar, CornerRadius::same(RADIUS)));
            painter.rect(
                toolbar,
                CornerRadius::same(RADIUS),
                brand::ITEM,
                Stroke::new(BORDER, brand::ITEM_LINE),
                StrokeKind::Inside,
            );
            let mut x = toolbar.left() + BORDER + PAD;
            for tool in kind.tools() {
                let rect = Rect::from_min_size(
                    pos2(x, toolbar.top() + BORDER + PAD),
                    vec2(BUTTON, BUTTON),
                );
                x += BUTTON + GAP;
                if let Some(action) = button(area, seq, *tool, rect, text) {
                    asked = Some(action);
                }
            }
        });
    asked
}

fn button(ui: &mut Ui, seq: u64, tool: Tool, rect: Rect, text: &str) -> Option<BubbleAction> {
    let response = ui
        .interact(rect, tool_id(seq, tool), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tool.name());
    let now = ui.input(|input| input.time);
    let copied_id = tool_id(seq, tool).with("copied");
    let until = ui.data(|data| data.get_temp::<f64>(copied_id)).unwrap_or(0.0);
    let copied = tool == Tool::Copy && now < until;
    if tool == Tool::Copy && response.clicked() {
        ui.ctx().copy_text(text.to_owned());
        ui.data_mut(|data| data.insert_temp(copied_id, now + COPIED_SECS));
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(COPIED_SECS));
    } else if copied {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(until - now));
    }
    let hovered = response.hovered() || response.has_focus();
    if hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(BUTTON_RADIUS),
            Color32::from_white_alpha(16),
        );
    }
    let ink = if hovered || copied { brand::TEAL_TEXT } else { brand::MUTED };
    let glyph = Rect::from_center_size(rect.center(), vec2(ICON, ICON));
    if copied {
        code_block::paint_check(ui.painter(), glyph, ink);
    } else {
        paint_glyph(ui.painter(), tool, glyph, ink);
    }
    response.clicked().then(|| tool.action()).flatten()
}

fn paint_glyph(painter: &Painter, tool: Tool, glyph: Rect, ink: Color32) {
    let stroke = Stroke::new(GLYPH_STROKE, ink);
    let at = |x: f32, y: f32| -> Pos2 { glyph.min + vec2(x, y) * (glyph.width() / 24.0) };
    match tool {
        Tool::Copy => code_block::paint_copy(painter, glyph, ink),
        Tool::EditAndSend => {
            painter.line_segment([at(12.0, 20.0), at(21.0, 20.0)], stroke);
            let tip = (0..=4).map(|step| {
                let angle = (-135.0_f32 + 45.0 * step as f32).to_radians();
                at(18.0 + 2.12 * angle.cos(), 5.0 + 2.12 * angle.sin())
            });
            let outline: Vec<Pos2> =
                tip.chain([at(7.0, 19.0), at(3.0, 20.0), at(4.0, 16.0)]).collect();
            painter.add(Shape::closed_line(outline, stroke));
        }
        Tool::SendAgain => {
            let arc = (0..=10).map(|step| {
                let angle = (312.0_f32 * step as f32 / 10.0).to_radians();
                at(12.0 + 9.0 * angle.cos(), 12.0 + 9.0 * angle.sin())
            });
            painter.add(Shape::line(arc.chain([at(21.0, 8.0)]).collect(), stroke));
            painter.add(Shape::line(vec![at(21.0, 3.0), at(21.0, 8.0), at(16.0, 8.0)], stroke));
        }
        Tool::Dismiss => {
            painter.line_segment([at(6.0, 6.0), at(18.0, 18.0)], stroke);
            painter.line_segment([at(18.0, 6.0), at(6.0, 18.0)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sent_message_gets_edit_copy_and_send_again_in_that_order() {
        let names: Vec<&str> = Kind::Sent.tools().iter().map(|tool| tool.name()).collect();
        assert_eq!(names, ["Edit and send", "Copy", "Send again"]);
    }

    #[test]
    fn a_message_the_agent_never_showed_gets_send_again_and_dismiss() {
        let names: Vec<&str> = Kind::NotSeen.tools().iter().map(|tool| tool.name()).collect();
        assert_eq!(names, ["Send again", "Dismiss"]);
        assert!(Kind::NotSeen.always());
        assert!(!Kind::Sent.always());
    }

    #[test]
    fn copy_asks_for_nothing_because_it_copies_on_the_spot() {
        assert_eq!(Tool::Copy.action(), None);
        assert_eq!(Tool::EditAndSend.action(), Some(BubbleAction::EditAndSend));
        assert_eq!(Tool::SendAgain.action(), Some(BubbleAction::SendAgain));
        assert_eq!(Tool::Dismiss.action(), Some(BubbleAction::Dismiss));
    }

    #[test]
    fn the_toolbar_is_right_aligned_under_the_bubble_and_never_over_it() {
        let bubble = Rect::from_min_max(pos2(100.0, 40.0), pos2(400.0, 120.0));
        for kind in [Kind::Sent, Kind::NotSeen] {
            let rect = toolbar_rect(bubble, kind);
            assert_eq!(rect.right(), bubble.right());
            assert_eq!(rect.top(), bubble.bottom() + BELOW);
            assert_eq!(rect.height(), BUTTON + 2.0 * (PAD + BORDER));
        }
        assert_eq!(toolbar_rect(bubble, Kind::Sent).width(), 3.0 * 28.0 + 2.0 * 2.0 + 8.0);
        assert_eq!(toolbar_rect(bubble, Kind::NotSeen).width(), 2.0 * 28.0 + 2.0 + 8.0);
    }
}
