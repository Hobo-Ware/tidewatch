//! Shared drawing pieces: headings, key caps, CLI badges, section labels.

use crate::brand;
use crate::commands::Platform;
use egui::{
    Align, Align2, Color32, Context, CornerRadius, FontId, Id, Layout, Margin, Modal,
    ModalResponse, Painter, Rect, Response, RichText, Sense, Shadow, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use kelp_ui::theme::semibold;

const BUTTON_RADIUS: u8 = 8;
const DISABLED_INK: f32 = 0.4;
const CAP_FONT: f32 = 11.5;
const CAP_PAD: f32 = 6.0;
const CAP_MIN_W: f32 = 22.0;
const CAP_GAP: f32 = 8.0;
const CAP_RADIUS: u8 = 5;
const FOOTER_GAP: f32 = 8.0;
pub(crate) const DIALOG_HEADER_H: f32 = 48.0;
const DIALOG_PAD: i8 = 16;
const ROW_RADIUS: u8 = 8;
const BAR_W: f32 = 2.0;
const BAR_INSET: f32 = 7.0;

pub(crate) fn heading(size: f32) -> FontId {
    FontId::new(size, semibold())
}

pub(crate) fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

pub(crate) fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub(crate) fn badge_colors(agent_id: &str) -> (Color32, Color32) {
    match agent_id {
        "claude" => brand::CLAUDE_BADGE,
        "codex" => brand::CODEX_BADGE,
        "gemini" => brand::GEMINI_BADGE,
        "copilot" => brand::COPILOT_BADGE,
        _ => (brand::BUTTON, brand::MUTED),
    }
}

pub(crate) fn badge_letter(agent_id: &str) -> String {
    match agent_id {
        "claude" => "C".to_owned(),
        "codex" => "X".to_owned(),
        "gemini" => "G".to_owned(),
        "copilot" => "P".to_owned(),
        other => other.chars().next().map_or("?".to_owned(), |c| c.to_uppercase().to_string()),
    }
}

pub(crate) fn cli_short_name(agent_id: &str) -> String {
    let mut chars = agent_id.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

pub(crate) fn paint_cli_badge(painter: &Painter, rect: Rect, agent_id: &str) {
    let (fill, ink) = badge_colors(agent_id);
    painter.rect_filled(rect, CornerRadius::same((rect.width() * 0.28) as u8), fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        badge_letter(agent_id),
        heading(rect.width() * 0.48),
        ink,
    );
}

pub(crate) fn clickable(
    ui: &mut Ui,
    rect: Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> egui::Response {
    ui.interact(rect, ui.id().with(salt), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Size {
    Regular,
    Compact,
}

impl Size {
    pub(crate) const fn height(self) -> f32 {
        match self {
            Self::Regular => 32.0,
            Self::Compact => 26.0,
        }
    }

    const fn pad(self) -> f32 {
        match self {
            Self::Regular => 14.0,
            Self::Compact => 12.0,
        }
    }

    const fn font(self) -> f32 {
        match self {
            Self::Regular => 13.5,
            Self::Compact => 13.0,
        }
    }

    const fn cap_h(self) -> f32 {
        match self {
            Self::Regular => 20.0,
            Self::Compact => 18.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    Quiet,
    Danger,
    Primary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Rest,
    Hover,
    Pressed,
    Disabled,
}

impl State {
    fn pick(enabled: bool, pressed: bool, hovered: bool) -> Self {
        match (enabled, pressed, hovered) {
            (false, _, _) => Self::Disabled,
            (true, true, _) => Self::Pressed,
            (true, false, true) => Self::Hover,
            (true, false, false) => Self::Rest,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Paint {
    fill: Color32,
    line: Color32,
    ink: Color32,
    cap: Color32,
}

fn teal_dark(amount: f32) -> Color32 {
    brand::TEAL.lerp_to_gamma(brand::KEY_EDGE, amount)
}

fn paint_of(look: Look, state: State) -> Paint {
    let clear = Color32::TRANSPARENT;
    let dim = |color: Color32| color.gamma_multiply(DISABLED_INK);
    match (look, state) {
        (Look::Primary, State::Rest) => Paint {
            fill: brand::TEAL,
            line: brand::TEAL,
            ink: brand::KEY_EDGE,
            cap: teal_dark(0.3),
        },
        (Look::Primary, State::Hover) => Paint {
            fill: brand::TEAL_TEXT,
            line: brand::TEAL_TEXT,
            ink: brand::KEY_EDGE,
            cap: teal_dark(0.3),
        },
        (Look::Primary, State::Pressed) => Paint {
            fill: teal_dark(0.15),
            line: teal_dark(0.15),
            ink: brand::KEY_EDGE,
            cap: teal_dark(0.4),
        },
        (Look::Primary, State::Disabled) => Paint {
            fill: brand::BUTTON,
            line: brand::SEGMENT_LINE,
            ink: dim(brand::TEXT),
            cap: brand::SEGMENT,
        },
        (Look::Quiet, State::Rest) => {
            Paint { fill: clear, line: brand::FIELD_LINE, ink: brand::TEXT, cap: brand::BUTTON }
        }
        (Look::Quiet, State::Hover) => {
            Paint { fill: brand::BUTTON, line: brand::FAINT, ink: brand::TEXT, cap: brand::SEGMENT }
        }
        (Look::Quiet, State::Pressed) => {
            Paint { fill: brand::SEGMENT, line: brand::FAINT, ink: brand::TEXT, cap: brand::BUTTON }
        }
        (Look::Quiet, State::Disabled) => Paint {
            fill: clear,
            line: brand::SEGMENT_LINE,
            ink: dim(brand::TEXT),
            cap: brand::BUTTON,
        },
        (Look::Danger, State::Rest) => {
            Paint { fill: clear, line: brand::RED_LINE, ink: brand::RED, cap: brand::RED_FILL }
        }
        (Look::Danger, State::Hover) => Paint {
            fill: brand::RED_FILL,
            line: brand::RED.gamma_multiply(0.5),
            ink: brand::RED,
            cap: brand::RED_FILL,
        },
        (Look::Danger, State::Pressed) => {
            Paint { fill: brand::RED_FILL, line: brand::RED, ink: brand::RED, cap: brand::RED_FILL }
        }
        (Look::Danger, State::Disabled) => Paint {
            fill: clear,
            line: brand::SEGMENT_LINE,
            ink: dim(brand::RED),
            cap: brand::RED_FILL,
        },
    }
}

fn cap_width(text_w: f32) -> f32 {
    (text_w + 2.0 * CAP_PAD).max(CAP_MIN_W)
}

/// The width of a button whose label is `label_w` wide, with a key cap of `key_w` text when it has one.
fn width_of(size: Size, label_w: f32, key_w: Option<f32>) -> f32 {
    let cap = key_w.map_or(0.0, |w| CAP_GAP + cap_width(w));
    2.0 * size.pad() + label_w + cap
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), font, brand::SOFT).size().x
}

pub(crate) fn button_width(ui: &Ui, label: &str, size: Size) -> f32 {
    width_of(size, text_width(ui, label, body(size.font())), None)
}

pub(crate) fn keyed_button_width(ui: &Ui, label: &str, key: &str, size: Size) -> f32 {
    let key_w = text_width(ui, key, mono(CAP_FONT));
    width_of(size, text_width(ui, label, body(size.font())), Some(key_w))
}

struct Spec<'a> {
    label: &'a str,
    key: Option<&'a str>,
    enabled: bool,
    size: Size,
    look: Look,
}

fn button(ui: &mut Ui, spec: &Spec) -> Response {
    let Spec { label, key, enabled, size, look } = *spec;
    let label_w = text_width(ui, label, body(size.font()));
    let key_w = key.map(|key| text_width(ui, key, mono(CAP_FONT)));
    let width = width_of(size, label_w, key_w);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, size.height()),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let paint = paint_of(
        look,
        State::pick(enabled, response.is_pointer_button_down_on(), response.hovered()),
    );
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(BUTTON_RADIUS),
        paint.fill,
        Stroke::new(1.0, paint.line),
        StrokeKind::Inside,
    );
    let mid = rect.center().y;
    painter.text(
        pos2(rect.left() + size.pad(), mid),
        Align2::LEFT_CENTER,
        label,
        body(size.font()),
        paint.ink,
    );
    if let (Some(key), Some(key_w)) = (key, key_w) {
        let cap = Rect::from_min_size(
            pos2(rect.right() - size.pad() - cap_width(key_w), mid - size.cap_h() / 2.0),
            vec2(cap_width(key_w), size.cap_h()),
        );
        painter.rect_filled(cap, CornerRadius::same(CAP_RADIUS), paint.cap);
        painter.text(cap.center(), Align2::CENTER_CENTER, key, mono(CAP_FONT), paint.ink);
    }
    kelp_ui::widgets::focus_ring(ui, &response, f32::from(BUTTON_RADIUS));
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

fn plain(ui: &mut Ui, label: &str, enabled: bool, size: Size, look: Look) -> Response {
    button(ui, &Spec { label, key: None, enabled, size, look })
}

/// A bordered button with no fill: the default for every action that is not the one main action.
pub(crate) fn quiet_button(ui: &mut Ui, label: &str, enabled: bool, size: Size) -> Response {
    plain(ui, label, enabled, size, Look::Quiet)
}

/// A quiet button in red for an action that destroys something.
pub(crate) fn danger_button(ui: &mut Ui, label: &str, size: Size) -> Response {
    plain(ui, label, true, size, Look::Danger)
}

/// The one filled button of a view or dialog.
pub(crate) fn primary_button(ui: &mut Ui, label: &str, enabled: bool, size: Size) -> Response {
    plain(ui, label, enabled, size, Look::Primary)
}

/// `primary_button` with the shortcut that does the same drawn as a key cap at its right.
pub(crate) fn primary_button_keyed(
    ui: &mut Ui,
    label: &str,
    key: &str,
    enabled: bool,
    size: Size,
) -> Response {
    button(ui, &Spec { label, key: Some(key), enabled, size, look: Look::Primary })
}

/// A dialog or form footer: `hint` muted at the left, the buttons `add` draws right-aligned (the
/// rightmost first) with 8 px between them, all on one centered row `Size::Regular` tall.
pub(crate) fn footer_row(ui: &mut Ui, hint: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(Size::Regular.height());
        if !hint.is_empty() {
            ui.label(RichText::new(hint).size(12.0).color(brand::MUTED));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = FOOTER_GAP;
            add(ui);
        });
    });
}

/// The shortcut that confirms a form or a dialog, as the text of a key cap.
#[derive(Clone, Copy, Default)]
struct Held {
    any: bool,
    not_text: bool,
}

fn held_id() -> egui::Id {
    egui::Id::new("focus-held-last-frame")
}

/// Records at the end of a frame whether a widget holds the keyboard. egui drops that focus on Esc
/// before any code of the next frame runs, so this is the only way to tell that Esc did it.
pub(crate) fn remember_focus(ctx: &egui::Context) {
    let focused = ctx
        .memory(egui::Memory::focused)
        .is_some_and(|id| !crate::terminal_focus::is_grid(ctx, id));
    let held = Held { any: focused, not_text: focused && !ctx.text_edit_focused() };
    ctx.data_mut(|data| data.insert_temp(held_id(), held));
}

/// Whether this frame's Esc only took the keyboard from a widget (any widget, or with `text` false
/// one that is not a text field), unless an open dropdown needs the key to close. When it did, the
/// key is used up.
pub(crate) fn esc_left_a_widget(ctx: &egui::Context, text: bool) -> bool {
    let held: Held = ctx.data(|data| data.get_temp(held_id())).unwrap_or_default();
    let held = if text { held.any } else { held.not_text };
    held && !egui::Popup::is_any_open(ctx)
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}

pub(crate) fn confirm_key() -> &'static str {
    match Platform::CURRENT {
        Platform::Mac => "\u{2318}\u{21b5}",
        Platform::Linux => "Ctrl \u{21b5}",
    }
}

/// A modal dialog: a card with a 48 px header (title, divider) over a padded body of `width`.
pub(crate) fn dialog_frame<R>(
    ctx: &Context,
    id: Id,
    title: &str,
    width: f32,
    content: impl FnOnce(&mut Ui) -> R,
) -> ModalResponse<R> {
    let frame = egui::Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::FIELD_LINE))
        .corner_radius(12)
        .shadow(Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(100),
        })
        .inner_margin(Margin::ZERO);
    Modal::new(id).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        let (header, _) = ui.allocate_exact_size(vec2(width, DIALOG_HEADER_H), Sense::hover());
        let painter = ui.painter();
        painter.text(
            header.left_center() + vec2(f32::from(DIALOG_PAD), 0.0),
            Align2::LEFT_CENTER,
            title,
            heading(15.0),
            brand::TEXT,
        );
        painter.hline(header.x_range(), header.bottom(), Stroke::new(1.0, brand::CARD_LINE));
        egui::Frame::new().inner_margin(Margin::same(DIALOG_PAD)).show(ui, content).inner
    })
}

/// The fill and border of a small floating card: a notice, a pill, a message over a pane.
pub(crate) fn paint_pill_card(painter: &Painter, rect: Rect) {
    painter.rect(
        rect,
        CornerRadius::same(ROW_RADIUS),
        brand::ITEM,
        Stroke::new(1.0, brand::ITEM_LINE),
        StrokeKind::Inside,
    );
}

/// Paints a selectable row: a teal tint with a short teal bar on the left when selected, a fainter tint when hovered.
pub(crate) fn selection_row(painter: &Painter, rect: Rect, selected: bool, hovered: bool) {
    let radius = CornerRadius::same(ROW_RADIUS);
    if selected {
        painter.rect_filled(rect, radius, brand::TEAL_NAV);
        let bar = Rect::from_min_max(
            rect.left_top() + vec2(0.0, BAR_INSET),
            rect.left_bottom() + vec2(BAR_W, -BAR_INSET),
        );
        painter.rect_filled(bar, CornerRadius::same(1), brand::TEAL);
    } else if hovered {
        painter.rect_filled(rect, radius, brand::TEAL_NAV.gamma_multiply(0.6));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut add: impl FnMut(&mut Ui)) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(600.0, 200.0))),
            ..egui::RawInput::default()
        };
        let _ = Context::default().run_ui(input, |ui| add(ui));
    }

    #[test]
    fn a_button_is_as_tall_as_its_size_whatever_its_look_and_label() {
        let mut sizes = Vec::new();
        run(|ui| {
            sizes.clear();
            for size in [Size::Regular, Size::Compact] {
                for label in ["Ok", "A much longer label"] {
                    sizes.push((size, quiet_button(ui, label, true, size).rect.size()));
                }
                sizes.push((size, primary_button(ui, "Go", false, size).rect.size()));
                sizes.push((size, danger_button(ui, "Delete", size).rect.size()));
                sizes.push((
                    size,
                    primary_button_keyed(ui, "Start", "\u{2318}\u{21b5}", true, size).rect.size(),
                ));
            }
        });
        for (size, rect) in &sizes {
            assert_eq!(rect.y, size.height(), "{size:?}");
        }
        assert!(sizes[1].1.x > sizes[0].1.x);
    }

    #[test]
    fn the_two_sizes_are_32_and_26_tall() {
        assert_eq!(Size::Regular.height(), 32.0);
        assert_eq!(Size::Compact.height(), 26.0);
    }

    #[test]
    fn padding_grows_a_button_and_a_key_cap_adds_its_width_and_a_gap() {
        let cases = [
            (Size::Regular, 40.0, None, 68.0),
            (Size::Compact, 40.0, None, 64.0),
            (Size::Regular, 40.0, Some(8.0), 68.0 + CAP_GAP + 22.0),
            (Size::Compact, 40.0, Some(30.0), 64.0 + CAP_GAP + 30.0 + 2.0 * CAP_PAD),
        ];
        for (size, label, key, want) in cases {
            assert_eq!(width_of(size, label, key), want, "{size:?} {key:?}");
        }
    }

    #[test]
    fn a_disabled_button_never_looks_hovered_or_pressed() {
        for (pressed, hovered) in [(false, false), (false, true), (true, true)] {
            assert_eq!(State::pick(false, pressed, hovered), State::Disabled);
        }
        let cases = [
            (false, false, State::Rest),
            (false, true, State::Hover),
            (true, true, State::Pressed),
            (true, false, State::Pressed),
        ];
        for (pressed, hovered, want) in cases {
            assert_eq!(State::pick(true, pressed, hovered), want, "{pressed} {hovered}");
        }
    }

    #[test]
    fn each_look_has_the_colors_it_promises() {
        let quiet = paint_of(Look::Quiet, State::Rest);
        assert_eq!(
            (quiet.fill, quiet.line, quiet.ink),
            (Color32::TRANSPARENT, brand::FIELD_LINE, brand::TEXT)
        );
        let primary = paint_of(Look::Primary, State::Rest);
        assert_eq!((primary.fill, primary.ink), (brand::TEAL, brand::KEY_EDGE));
        assert_ne!(primary.cap, primary.fill);
        let danger = paint_of(Look::Danger, State::Rest);
        assert_eq!((danger.ink, danger.line), (brand::RED, brand::RED_LINE));
        assert_eq!(paint_of(Look::Danger, State::Hover).fill, brand::RED_FILL);
        for look in [Look::Quiet, Look::Primary, Look::Danger] {
            assert_ne!(paint_of(look, State::Rest), paint_of(look, State::Hover), "{look:?} hover");
            assert_ne!(
                paint_of(look, State::Hover),
                paint_of(look, State::Pressed),
                "{look:?} pressed"
            );
            let off = paint_of(look, State::Disabled);
            assert!(off.ink.a() < 120, "{look:?} disabled ink is 40%: {}", off.ink.a());
        }
    }

    #[test]
    fn a_disabled_button_is_never_clicked() {
        let mut clicked = false;
        run(|ui| clicked |= quiet_button(ui, "Send", false, Size::Regular).clicked());
        assert!(!clicked);
    }

    #[test]
    fn a_footer_right_aligns_its_buttons_8_px_apart_on_one_baseline() {
        let mut rects = Vec::new();
        let mut footer = Rect::NOTHING;
        run(|ui| {
            rects.clear();
            footer = ui.max_rect();
            footer_row(ui, "Esc cancels", |ui| {
                rects.push(primary_button(ui, "Go", true, Size::Regular).rect);
                rects.push(quiet_button(ui, "Cancel", true, Size::Regular).rect);
            });
        });
        assert_eq!(rects[0].center().y, rects[1].center().y);
        assert_eq!(rects[1].right() + 8.0, rects[0].left());
        assert_eq!(rects[0].right(), footer.right());
    }

    #[test]
    fn badges_use_the_brand_hues_and_unknown_clis_fall_back_to_muted() {
        assert_eq!(badge_colors("claude"), brand::CLAUDE_BADGE);
        assert_eq!(badge_colors("gemini"), brand::GEMINI_BADGE);
        assert_eq!(badge_colors("other"), (brand::BUTTON, brand::MUTED));
        assert_eq!(badge_letter("copilot"), "P");
        assert_eq!(badge_letter("zed"), "Z");
    }

    fn esc_frame(ctx: &Context, text: bool) -> bool {
        let key = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let input = egui::RawInput { events: vec![key], ..egui::RawInput::default() };
        let mut taken = false;
        let _ = ctx.run_ui(input, |_| taken = esc_left_a_widget(ctx, text));
        taken
    }

    fn held(ctx: &Context, any: bool, not_text: bool) {
        ctx.data_mut(|data| data.insert_temp(held_id(), Held { any, not_text }));
    }

    #[test]
    fn esc_is_left_alone_when_no_widget_held_the_keyboard() {
        let ctx = Context::default();
        held(&ctx, false, false);
        assert!(!esc_left_a_widget(&ctx, true));
        assert!(!esc_left_a_widget(&ctx, false));
        assert!(!esc_frame(&ctx, true));
        assert!(!esc_frame(&ctx, false));
    }

    #[test]
    fn esc_is_used_up_by_a_widget_that_held_the_keyboard() {
        let ctx = Context::default();
        held(&ctx, true, true);
        assert!(esc_frame(&ctx, false));
        assert!(esc_frame(&ctx, true));
    }

    #[test]
    fn esc_in_a_text_field_is_left_to_its_own_owner_unless_asked() {
        let ctx = Context::default();
        held(&ctx, true, false);
        assert!(!esc_frame(&ctx, false));
        assert!(esc_frame(&ctx, true));
    }
}
