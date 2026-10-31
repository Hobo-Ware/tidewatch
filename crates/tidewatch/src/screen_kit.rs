//! Shared drawing pieces: headings, key caps, CLI badges, section labels.

use crate::brand;
use crate::commands::Platform;
use crate::reader_view::ellipsized;
use crate::selectable;
use egui::{
    Align, Align2, Color32, Context, CornerRadius, FontId, Id, Layout, Margin, Modal,
    ModalResponse, Painter, Rect, Response, RichText, Sense, Shadow, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use kelp_ui::theme::semibold;

const BUTTON_RADIUS: u8 = 8;
const DISABLED_INK: f32 = 0.4;
const CAP_FONT: f32 = TEXT_TAG;
const CAP_PAD: f32 = 6.0;
const CAP_MIN_W: f32 = 22.0;
const CAP_GAP: f32 = 8.0;
const CAP_RADIUS: u8 = 5;
const FOOTER_GAP: f32 = 8.0;
const HINT_GAP: f32 = 16.0;
pub(crate) const DIALOG_HEADER_H: f32 = 48.0;
pub(crate) const LABEL_H: f32 = 16.0;
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

pub(crate) const TEXT_TAG: f32 = 12.0;
pub(crate) const TEXT_SECONDARY: f32 = 13.0;
pub(crate) const TEXT_BODY: f32 = 14.0;
pub(crate) const TEXT_TITLE: f32 = 16.0;
pub(crate) const TEXT_PAGE: f32 = 22.0;

/// The three control sizes, one per job: `Tag` for a label you read, `Compact` for a button inside
/// a card, row, strip or panel, `Regular` for forms, dialogs and Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Size {
    Tag,
    Compact,
    Regular,
}

impl Size {
    pub(crate) const fn height(self) -> f32 {
        match self {
            Self::Tag => 20.0,
            Self::Compact => 28.0,
            Self::Regular => 32.0,
        }
    }

    pub(crate) const fn pad(self) -> f32 {
        match self {
            Self::Tag => 8.0,
            Self::Compact => 12.0,
            Self::Regular => 14.0,
        }
    }

    pub(crate) const fn font(self) -> f32 {
        match self {
            Self::Tag => TEXT_TAG,
            Self::Compact => TEXT_SECONDARY,
            Self::Regular => TEXT_BODY,
        }
    }

    pub(crate) const fn radius(self) -> u8 {
        match self {
            Self::Tag => (self.height() / 2.0) as u8,
            Self::Compact | Self::Regular => BUTTON_RADIUS,
        }
    }

    const fn cap_h(self) -> f32 {
        match self {
            Self::Tag => 16.0,
            Self::Compact => 18.0,
            Self::Regular => 20.0,
        }
    }
}

/// How a tag is drawn: an outline in a color, or a tint of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TagFill {
    Line(Color32),
    Tint(Color32),
}

pub(crate) fn tag_width(painter: &Painter, text: &str) -> f32 {
    painter.layout_no_wrap(text.to_owned(), body(Size::Tag.font()), brand::SOFT).size().x
        + 2.0 * Size::Tag.pad()
}

/// A read-only label: `Size::Tag` tall, fully round, its left edge at `left` and centered on
/// `mid`. Returns the rect it drew.
pub(crate) fn paint_tag(
    painter: &Painter,
    (left, mid): (f32, f32),
    text: &str,
    ink: Color32,
    fill: TagFill,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), body(Size::Tag.font()), ink);
    let size = vec2(galley.size().x + 2.0 * Size::Tag.pad(), Size::Tag.height());
    let rect = Rect::from_min_size(pos2(left, mid - size.y / 2.0), size);
    let radius = CornerRadius::same(Size::Tag.radius());
    match fill {
        TagFill::Line(line) => {
            painter.rect_stroke(rect, radius, Stroke::new(1.0, line), StrokeKind::Inside);
        }
        TagFill::Tint(tint) => {
            painter.rect_filled(rect, radius, tint);
        }
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
    rect
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

fn teal(alpha: f32) -> Color32 {
    brand::TEAL.gamma_multiply(alpha)
}

fn paint_of(look: Look, state: State) -> Paint {
    let clear = Color32::TRANSPARENT;
    let dim = |color: Color32| color.gamma_multiply(DISABLED_INK);
    match (look, state) {
        (Look::Primary, State::Rest) => {
            Paint { fill: clear, line: brand::TEAL, ink: brand::TEAL_TEXT, cap: teal(0.16) }
        }
        (Look::Primary, State::Hover) => {
            Paint { fill: teal(0.1), line: brand::TEAL, ink: brand::TEAL_TEXT, cap: teal(0.24) }
        }
        (Look::Primary, State::Pressed) => {
            Paint { fill: teal(0.2), line: brand::TEAL, ink: brand::TEAL_TEXT, cap: teal(0.3) }
        }
        (Look::Primary, State::Disabled) => Paint {
            fill: clear,
            line: dim(brand::TEAL),
            ink: dim(brand::TEAL_TEXT),
            cap: teal(0.08),
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
        (Look::Danger, State::Rest) => Paint {
            fill: clear,
            line: brand::RED.gamma_multiply(0.6),
            ink: brand::RED,
            cap: brand::RED_FILL,
        },
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
        CornerRadius::same(size.radius()),
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
    kelp_ui::widgets::focus_ring(ui, &response, f32::from(size.radius()));
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}

fn plain(ui: &mut Ui, label: &str, enabled: bool, size: Size, look: Look) -> Response {
    button(ui, &Spec { label, key: None, enabled, size, look })
}

/// A bordered button with no fill: the default for every action that is not the one main action.
pub(crate) fn quiet_button(ui: &mut Ui, label: &str, enabled: bool, size: Size) -> Response {
    plain(ui, label, enabled, size, Look::Quiet)
}

/// A quiet button that holds one glyph instead of a label: as tall as `size` and as wide, no fill
/// at rest.
pub(crate) fn icon_button(ui: &mut Ui, glyph: &str, tip: &str, size: Size) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(size.height(), size.height()), Sense::click());
    let paint = paint_of(
        Look::Quiet,
        State::pick(true, response.is_pointer_button_down_on(), response.hovered()),
    );
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(size.radius()),
        paint.fill,
        Stroke::new(1.0, paint.line),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(size.font() + 2.0),
        paint.ink,
    );
    kelp_ui::widgets::focus_ring(ui, &response, f32::from(size.radius()));
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// A quiet button in red for an action that destroys something.
pub(crate) fn danger_button(ui: &mut Ui, label: &str, size: Size) -> Response {
    plain(ui, label, true, size, Look::Danger)
}

/// The button of the main action of a view or dialog: a teal line and teal text, no fill.
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

/// The label over a field, a control row or a switch group in a form, dialog or popover: 13 px
/// semibold in the soft ink, sentence case, at the left edge of the control under it. Settings
/// pages keep their own section labels.
pub(crate) fn field_label(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), LABEL_H), Sense::hover());
    let galley = ellipsized(ui.painter(), text, heading(TEXT_SECONDARY), brand::SOFT, rect.width());
    let at = rect.left_center() - vec2(0.0, galley.size().y / 2.0);
    selectable::galley(ui, at, galley, brand::SOFT);
}

/// The width `field_label` gives `text`.
pub(crate) fn field_label_width(painter: &Painter, text: &str) -> f32 {
    painter.layout_no_wrap(text.to_owned(), heading(TEXT_SECONDARY), brand::SOFT).size().x
}

/// A dialog or form footer: `hint` muted at the left, the buttons `add` draws right-aligned (the
/// rightmost first) with 8 px between them, all on one centered row `Size::Regular` tall. A hint
/// the buttons leave no room for goes under the row as `notice::key_hint` does.
pub(crate) fn footer_row(ui: &mut Ui, hint: &str, add: impl FnOnce(&mut Ui)) {
    let mut below = false;
    ui.horizontal(|ui| {
        ui.set_min_height(Size::Regular.height());
        let left = ui.cursor().left();
        let centre = ui.min_rect().center().y;
        let buttons = ui
            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = FOOTER_GAP;
                add(ui);
            })
            .response
            .rect;
        if hint.is_empty() {
            return;
        }
        let text = RichText::new(hint).size(TEXT_TAG).color(brand::MUTED);
        let width = ui
            .painter()
            .layout_no_wrap(hint.to_owned(), FontId::proportional(TEXT_TAG), brand::MUTED)
            .size()
            .x;
        if hint_fits(left, width, buttons.left()) {
            let at = Rect::from_min_size(
                pos2(left, centre - Size::Regular.height() / 2.0),
                vec2(width, Size::Regular.height()),
            );
            ui.put(at, egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
        } else {
            below = true;
        }
    });
    if below {
        crate::notice::key_hint(ui, hint);
    }
}

fn hint_fits(left: f32, hint_width: f32, buttons_left: f32) -> bool {
    left + hint_width + HINT_GAP <= buttons_left
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
    let focused =
        ctx.memory(egui::Memory::focused).is_some_and(|id| !stdusk_egui::focus::is_grid(ctx, id));
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
    dialog_frame_in(ctx, (id, Modal::default_area(id)), title, width, content)
}

/// [`dialog_frame`] in an area of the caller's: a popover that sits under the tab strip.
pub(crate) fn dialog_frame_in<R>(
    ctx: &Context,
    (id, area): (Id, egui::Area),
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
    Modal::new(id).area(area).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        let (header, _) = ui.allocate_exact_size(vec2(width, DIALOG_HEADER_H), Sense::hover());
        let painter = ui.painter();
        painter.text(
            header.left_center() + vec2(f32::from(DIALOG_PAD), 0.0),
            Align2::LEFT_CENTER,
            title,
            heading(TEXT_TITLE),
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

pub(crate) const MENU_ROW_H: f32 = 32.0;
pub(crate) const MENU_ROW_PAD: f32 = 10.0;
const MENU_LABEL_H: f32 = 28.0;
const MENU_LABEL_PAD: f32 = 12.0;
const CAP_H: f32 = 20.0;
const CAP_STEP: f32 = 3.0;

/// The small upper-case label over a group of rows in a menu or popover, with an optional muted
/// note at its right edge. Returns the rect it took.
pub(crate) fn menu_label(ui: &mut Ui, text: &str, note: &str) -> Rect {
    menu_label_with(ui, text, "", note)
}

/// `menu_label` with `plain` after the upper-case `text`, in the same ink and case as given:
/// `SERVICES · acme-workers`.
pub(crate) fn menu_label_with(ui: &mut Ui, text: &str, plain: &str, note: &str) -> Rect {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), MENU_LABEL_H), Sense::hover());
    let painter = ui.painter();
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::text::TextFormat::simple(heading(TEXT_TAG), brand::FAINT),
    );
    job.append(plain, 0.0, egui::text::TextFormat::simple(heading(TEXT_TAG), brand::FAINT));
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(rect.left() + MENU_LABEL_PAD, rect.center().y + 2.0 - galley.size().y / 2.0),
        galley,
        brand::FAINT,
    );
    if !note.is_empty() {
        painter.text(
            pos2(rect.right() - MENU_LABEL_PAD, rect.center().y + 2.0),
            Align2::RIGHT_CENTER,
            note,
            body(TEXT_TAG),
            brand::FAINT,
        );
    }
    rect
}

/// A 1 px line across a menu, between two groups of rows.
pub(crate) fn menu_divider(ui: &mut Ui) {
    ui.add_space(3.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.left() + 4.0..=rect.right() - 4.0,
        rect.center().y,
        Stroke::new(1.0, brand::CARD_LINE),
    );
    ui.add_space(3.0);
}

/// A menu row as wide as the menu and `height` tall: the picked look when `selected`, a fainter
/// tint under the pointer. A disabled row has no tint and takes no click.
pub(crate) fn menu_row(
    ui: &mut Ui,
    height: f32,
    selected: bool,
    enabled: bool,
) -> (Rect, Response) {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    selection_row(ui.painter(), rect, selected, enabled && response.hovered());
    if enabled {
        (rect, response.on_hover_cursor(egui::CursorIcon::PointingHand))
    } else {
        (rect, response)
    }
}

/// One cap per key of a chord such as `Cmd+Shift+P`; a range such as `Cmd+1 to 9` keeps its `to`.
pub(crate) fn chord_keys(keys: &str) -> Vec<&str> {
    let mut caps = Vec::new();
    for (index, span) in keys.split(" to ").enumerate() {
        if index > 0 {
            caps.push("to");
        }
        caps.extend(span.split('+').filter(|key| !key.is_empty()));
    }
    caps
}

fn cap_w(painter: &Painter, key: &str) -> f32 {
    let text = painter.layout_no_wrap(key.to_owned(), mono(TEXT_TAG), brand::SOFT).size().x;
    (text + 2.0 * CAP_PAD).max(CAP_MIN_W)
}

/// The width `paint_caps` takes for `keys`.
pub(crate) fn caps_width(painter: &Painter, keys: &str) -> f32 {
    let keys = chord_keys(keys);
    let parts: f32 = keys
        .iter()
        .map(|key| {
            if *key == "to" {
                painter.layout_no_wrap((*key).to_owned(), body(TEXT_TAG), brand::FAINT).size().x
                    + CAP_STEP
                    + 2.0
            } else {
                cap_w(painter, key) + CAP_STEP
            }
        })
        .sum();
    (parts - CAP_STEP).max(0.0)
}

/// Key caps for `keys` ending at `right`, centered on `mid`, like the Shortcuts page. Returns the
/// left edge of the first one, or `right` for no keys.
pub(crate) fn paint_caps(painter: &Painter, right: f32, mid: f32, keys: &str) -> f32 {
    let mut x = right;
    for key in chord_keys(keys).into_iter().rev() {
        if key == "to" {
            let at = pos2(x - CAP_STEP - 2.0, mid);
            painter.text(at, Align2::RIGHT_CENTER, key, body(TEXT_TAG), brand::FAINT);
            x = at.x
                - painter.layout_no_wrap(key.to_owned(), body(TEXT_TAG), brand::FAINT).size().x;
            continue;
        }
        let width = cap_w(painter, key);
        let cap = Rect::from_min_size(pos2(x - width, mid - CAP_H / 2.0), vec2(width, CAP_H));
        let radius = CornerRadius::same(CAP_RADIUS);
        painter.rect_filled(cap, radius, brand::KEY_EDGE);
        let face = Rect::from_min_max(cap.min, cap.max - vec2(0.0, 1.0));
        painter.rect_filled(face, radius, brand::BUTTON);
        painter.text(face.center(), Align2::CENTER_CENTER, key, mono(TEXT_TAG), brand::SOFT);
        x -= width + CAP_STEP;
    }
    if x == right { right } else { x + CAP_STEP }
}

/// The glyphs menu rows and hover actions draw themselves, so no font has to have them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    Restart,
    More,
    Clock,
    Infinity,
}

/// Paints `glyph` in a `size` square centered on `center`.
pub(crate) fn paint_glyph(
    painter: &Painter,
    center: egui::Pos2,
    size: f32,
    glyph: Glyph,
    ink: Color32,
) {
    let unit = size / 16.0;
    let at = |x: f32, y: f32| center + vec2((x - 8.0) * unit, (y - 8.0) * unit);
    let stroke = Stroke::new(1.5 * unit.max(0.75), ink);
    match glyph {
        Glyph::Restart => {
            let arc: Vec<egui::Pos2> = (0..=21)
                .map(|step| {
                    let angle = (step as f32 * 15.0).to_radians();
                    at(8.0 + 5.0 * angle.cos(), 8.0 + 5.0 * angle.sin())
                })
                .collect();
            painter.add(egui::Shape::line(arc, stroke));
            painter
                .add(egui::Shape::line(vec![at(13.0, 3.0), at(13.0, 5.5), at(10.5, 5.5)], stroke));
        }
        Glyph::More => {
            for x in [3.5, 8.0, 12.5] {
                painter.circle_filled(at(x, 8.0), 1.3 * unit, ink);
            }
        }
        Glyph::Clock => {
            painter.circle_stroke(at(8.0, 8.0), 5.5 * unit, stroke);
            painter.add(egui::Shape::line(vec![at(8.0, 5.0), at(8.0, 8.0), at(10.0, 9.5)], stroke));
        }
        Glyph::Infinity => {
            let curve: Vec<egui::Pos2> = (0..=40)
                .map(|step| {
                    let angle = (step as f32 * 9.0).to_radians();
                    let shrink = 1.0 + angle.sin() * angle.sin();
                    at(
                        8.0 + 6.0 * angle.cos() / shrink,
                        8.0 + 6.0 * angle.sin() * angle.cos() / shrink,
                    )
                })
                .collect();
            painter.add(egui::Shape::line(curve, stroke));
        }
    }
}

/// `icon_button` with a painted glyph: as tall as `size` and as wide, a line and no fill.
pub(crate) fn glyph_button(ui: &mut Ui, glyph: Glyph, tip: &str, size: Size) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(size.height(), size.height()), Sense::click());
    let paint = paint_of(
        Look::Quiet,
        State::pick(true, response.is_pointer_button_down_on(), response.hovered()),
    );
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(size.radius()),
        paint.fill,
        Stroke::new(1.0, paint.line),
        StrokeKind::Inside,
    );
    paint_glyph(painter, rect.center(), 16.0, glyph, paint.ink);
    kelp_ui::widgets::focus_ring(ui, &response, f32::from(size.radius()));
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
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

    fn painted(out: &egui::FullOutput) -> Vec<(String, FontId, Color32, egui::Pos2)> {
        out.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => {
                    let format = &text.galley.job.sections.first()?.format;
                    Some((
                        text.galley.text().to_owned(),
                        format.font_id.clone(),
                        format.color,
                        text.pos,
                    ))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_field_label_is_sentence_case_13_px_semibold_soft_at_the_left_edge() {
        let ctx = crate::fonts::test_context();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(600.0, 200.0))),
            ..egui::RawInput::default()
        };
        let (mut left, mut height) = (0.0, 0.0);
        let out = ctx.run_ui(input, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            left = ui.max_rect().left();
            let top = ui.cursor().top();
            field_label(ui, "Worktree name");
            height = ui.cursor().top() - top;
        });
        let labels = painted(&out);
        let [(text, font, ink, at)] = &labels[..] else { panic!("one label: {labels:?}") };
        assert_eq!(text, "Worktree name");
        assert_eq!((font.clone(), *ink), (heading(TEXT_SECONDARY), brand::SOFT));
        assert!((at.x - left).abs() < 0.5, "{} vs {left}", at.x);
        assert_eq!(height, LABEL_H);
    }

    #[test]
    fn a_button_is_as_tall_as_its_size_whatever_its_look_and_label() {
        let mut sizes = Vec::new();
        run(|ui| {
            sizes.clear();
            for size in [Size::Tag, Size::Compact, Size::Regular] {
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
    fn an_icon_button_is_a_square_as_tall_as_its_size_with_no_fill_at_rest() {
        let mut rects = Vec::new();
        let mut fills = 0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(200.0, 100.0))),
            ..egui::RawInput::default()
        };
        let output = Context::default().run_ui(input, |ui| {
            rects.clear();
            for size in [Size::Tag, Size::Compact, Size::Regular] {
                rects.push((size, icon_button(ui, "x", "tip", size).rect.size()));
            }
        });
        for (size, rect) in &rects {
            assert_eq!((rect.x, rect.y), (size.height(), size.height()), "{size:?}");
        }
        for clipped in &output.shapes {
            if let egui::Shape::Rect(rect) = &clipped.shape {
                fills += usize::from(rect.fill != Color32::TRANSPARENT);
            }
        }
        assert_eq!(fills, 0, "an icon button rests without a fill");
    }

    #[test]
    fn each_size_is_the_height_text_and_corner_of_the_scale() {
        let table = [
            (Size::Tag, 20.0, 12.0, 10),
            (Size::Compact, 28.0, 13.0, 8),
            (Size::Regular, 32.0, 14.0, 8),
        ];
        for (size, height, font, radius) in table {
            assert_eq!(size.height(), height, "{size:?} height");
            assert_eq!(size.font(), font, "{size:?} font");
            assert_eq!(size.radius(), radius, "{size:?} radius");
        }
    }

    #[test]
    fn a_tag_is_drawn_as_tall_as_the_tag_size_and_a_button_as_tall_as_its_own() {
        let mut tag = Rect::NOTHING;
        let mut drawn = Vec::new();
        run(|ui| {
            drawn.clear();
            let (rect, _) = ui.allocate_exact_size(vec2(100.0, Size::Tag.height()), Sense::hover());
            tag = paint_tag(
                ui.painter(),
                (rect.left(), rect.center().y),
                "Claude",
                brand::TEAL,
                TagFill::Line(brand::TEAL),
            );
            for size in [Size::Tag, Size::Compact, Size::Regular] {
                drawn.push((size, quiet_button(ui, "Go", true, size).rect.height()));
            }
        });
        assert_eq!(tag.height(), 20.0);
        for (size, height) in drawn {
            assert_eq!(height, size.height(), "{size:?}");
        }
    }

    #[test]
    fn a_tag_is_as_wide_as_its_text_and_two_pads() {
        run(|ui| {
            let painter = ui.painter();
            let one = tag_width(painter, "3");
            let more = tag_width(painter, "Checks running");
            assert!(more > one);
            assert!(one >= 2.0 * Size::Tag.pad());
        });
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
        assert_eq!(
            (primary.fill, primary.line, primary.ink),
            (Color32::TRANSPARENT, brand::TEAL, brand::TEAL_TEXT)
        );
        assert_ne!(primary.cap, primary.fill);
        assert_eq!(paint_of(Look::Danger, State::Rest).fill, Color32::TRANSPARENT);
        let danger = paint_of(Look::Danger, State::Rest);
        assert_eq!((danger.ink, danger.line), (brand::RED, brand::RED.gamma_multiply(0.6)));
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

    fn footer_rects(width: f32, hint: &str) -> (Rect, Rect, f32) {
        let mut buttons = Rect::NOTHING;
        let mut left = 0.0;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 200.0))),
            ..egui::RawInput::default()
        };
        let out = Context::default().run_ui(input, |ui| {
            left = ui.max_rect().left();
            footer_row(ui, hint, |ui| {
                buttons = primary_button(ui, "Go", true, Size::Regular).rect;
                buttons = buttons.union(quiet_button(ui, "Cancel", true, Size::Regular).rect);
            });
        });
        let hint_rect = out
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == hint => {
                    Some(text.visual_bounding_rect())
                }
                _ => None,
            })
            .expect("the hint is drawn");
        (buttons, hint_rect, left)
    }

    #[test]
    fn a_footer_hint_sits_left_of_the_buttons_on_their_row() {
        let (buttons, hint, left) = footer_rects(600.0, "Esc cancels");
        assert!(hint.right() < buttons.left());
        assert!((hint.center().y - buttons.center().y).abs() < 1.0);
        assert!((hint.left() - left).abs() < 3.0, "{} vs {left}", hint.left());
    }

    #[test]
    fn a_footer_hint_the_buttons_leave_no_room_for_goes_under_the_row() {
        let long = "Enter deletes the branch  -  Esc cancels  -  nothing happens until you pick";
        let (buttons, hint, _) = footer_rects(260.0, long);
        assert!(hint.top() >= buttons.bottom());
    }

    #[test]
    fn a_hint_needs_its_width_and_a_gap_before_the_buttons() {
        let cases = [
            (0.0, 100.0, 116.0, true),
            (0.0, 100.0, 115.0, false),
            (8.0, 100.0, 124.0, true),
            (8.0, 100.0, 50.0, false),
        ];
        for (left, width, buttons_left, want) in cases {
            assert_eq!(hint_fits(left, width, buttons_left), want, "{left} {width} {buttons_left}");
        }
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

    #[test]
    fn a_chord_is_one_cap_per_key_and_a_range_keeps_its_to() {
        let cases: [(&str, Vec<&str>); 4] = [
            ("Cmd+Shift+P", vec!["Cmd", "Shift", "P"]),
            ("Cmd+1 to 9", vec!["Cmd", "1", "to", "9"]),
            ("Ctrl+,", vec!["Ctrl", ","]),
            ("", vec![]),
        ];
        for (keys, want) in cases {
            assert_eq!(chord_keys(keys), want, "{keys:?}");
        }
    }

    #[test]
    fn caps_end_at_the_right_edge_and_take_the_width_caps_width_says() {
        let ctx = crate::fonts::test_context();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(600.0, 200.0))),
            ..egui::RawInput::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            for keys in ["Cmd+O", "Cmd+Shift+Left", "Cmd+1 to 9", ""] {
                let width = caps_width(ui.painter(), keys);
                let left = paint_caps(ui.painter(), 500.0, 20.0, keys);
                assert!((500.0 - width - left).abs() < 0.01, "{keys:?}: {width} {left}");
            }
            assert_eq!(caps_width(ui.painter(), ""), 0.0);
        });
    }

    #[test]
    fn a_glyph_button_is_a_square_as_tall_as_its_size() {
        let mut sizes = Vec::new();
        run(|ui| {
            sizes.clear();
            for size in [Size::Tag, Size::Compact, Size::Regular] {
                sizes.push((size, glyph_button(ui, Glyph::More, "more", size).rect.size()));
            }
        });
        for (size, rect) in sizes {
            assert_eq!((rect.x, rect.y), (size.height(), size.height()), "{size:?}");
        }
    }

    #[test]
    fn a_menu_row_is_as_wide_as_the_menu_and_a_disabled_one_takes_no_click() {
        let mut rows = Vec::new();
        run(|ui| {
            rows.clear();
            ui.set_width(300.0);
            rows.push(menu_row(ui, MENU_ROW_H, false, true));
            rows.push(menu_row(ui, MENU_ROW_H, true, false));
        });
        for (rect, _) in &rows {
            assert_eq!((rect.width(), rect.height()), (300.0, MENU_ROW_H));
        }
        assert!(rows[0].1.sense.senses_click());
        assert!(!rows[1].1.sense.senses_click());
    }
}
