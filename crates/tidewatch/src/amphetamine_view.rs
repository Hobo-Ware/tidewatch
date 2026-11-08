//! The Amphetamine button at the right end of the tab strip: the tablet, the time pill that grows to
//! its left, the menu a click opens, the tooltip, the toast lines in the Lovecraftian voice and the
//! safety notice card.

use egui::{
    Align2, Color32, CornerRadius, Id, Margin, Order, Rect, RectAlign, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, pos2, vec2,
};
use tidewatch_core::amphetamine::{Choice, Omen, State};
use tidewatch_core::amphetamine_system::Route;

use stdusk_egui::colors;

use crate::amphetamine_icon;
use crate::amphetamine_menu;
use crate::amphetamine_motion::{Look, STANDARD, bezier};
use crate::amphetamine_pill::{
    self as pill, Fit, ICON_W, INFINITY, Pips, ROLL_S, Slide, clock_texts, left_text, line_fraction,
};
use crate::app::strip::ICON_BOX;
use crate::away_model;
use crate::brand;
use crate::mascot_motion;
use crate::screen_kit::{Size, body, mono, quiet_button};
use crate::screen_kit::{TEXT_BODY, TEXT_SECONDARY, TEXT_TAG, TEXT_TITLE};

pub(crate) const SLOT_W: f32 = ICON_W;
const SLOT_H: f32 = ICON_BOX;
pub(crate) const PILL_H: f32 = 32.0;
const LABEL_SIZE: f32 = 13.0;
const LABEL_PAD_LEFT: f32 = 10.0;
const LABEL_PAD_RIGHT: f32 = 4.0;
const LINE_H: f32 = 2.0;
const LINE_INSET: f32 = 12.0;
const LINE_INSET_BARE: f32 = 8.0;
const MENU_W: f32 = 340.0;
const MENU_MARGIN: i8 = 6;
const STILL_AGE: f64 = 30.5;
const NOTICE_W: f32 = 450.0;
const TOAST_BOTTOM: f32 = 34.0;
const TOAST_PAD: egui::Vec2 = vec2(14.0, 8.0);
const BREATH_S: f64 = 2.4;
const GRADIENT_END: Color32 = Color32::from_rgb(0x15, 0x39, 0x3a);
const PIP_W: f32 = 3.0;
const PIP_GAP: f32 = 2.5;
const PIP_H: f32 = 10.0;
const MARK_GAP: f32 = 8.0;
const AWAY_WORD: &str = "Away";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Pick(Choice),
    Retime(Choice),
    TurnOff,
    Away,
}

/// Everything the button reads from the app for one frame.
#[allow(clippy::struct_excessive_bools)] // independent facts of one frame, not a mode
pub(crate) struct Input {
    pub(crate) look: Look,
    pub(crate) session: State,
    pub(crate) wall: f64,
    pub(crate) total: Option<f64>,
    pub(crate) lid: bool,
    pub(crate) route: Route,
    pub(crate) gap: f32,
    pub(crate) force_short: bool,
    pub(crate) pinned: bool,
    pub(crate) away: bool,
}

#[derive(Default)]
pub(crate) struct Memory {
    force_menu: bool,
    slide: Option<Slide>,
    open_width: f32,
    shown: String,
    rolled: Option<(String, f64)>,
    last_pill: Option<Rect>,
    draft: Option<Choice>,
}

impl Memory {
    /// `TIDEWATCH_DEV_AMPHETAMINE_MENU=1` keeps the menu open, for captures.
    pub(crate) fn from_env() -> Self {
        let force_menu = std::env::var("TIDEWATCH_DEV_AMPHETAMINE_MENU")
            .is_ok_and(|v| !v.is_empty() && v != "0");
        Self { force_menu, ..Self::default() }
    }
}

#[cfg(test)]
impl Memory {
    fn pill_left(&self) -> Option<f32> {
        self.last_pill.map(|rect| rect.left())
    }
}

pub(crate) struct Output {
    pub(crate) action: Option<Action>,
}

fn menu_open(ctx: &egui::Context, response: &Response) -> bool {
    egui::Popup::is_id_open(ctx, egui::Popup::default_response_id(response))
}

/// The tablet seen when the window is not animating: the steady look, or nothing in motion.
fn still(look: Look) -> Look {
    match look {
        Look::On { .. } => Look::On { age: STILL_AGE },
        Look::Stopping { .. } => Look::Off,
        other => other,
    }
}

fn label_texts(session: State, wall: f64) -> (String, String) {
    match session.left(wall) {
        Some(left) => clock_texts(left),
        None => (INFINITY.to_owned(), INFINITY.to_owned()),
    }
}

fn text_width(ui: &Ui, text: &str) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), mono(LABEL_SIZE), brand::TEAL_TEXT).size().x
}

fn pill_for(ui: &Ui, text: &str, marks: f32) -> f32 {
    ICON_W + LABEL_PAD_LEFT + marks + text_width(ui, text) + LABEL_PAD_RIGHT
}

/// What sits left of the time in the pill: the hour pips and, while away, the violet word.
struct Marks {
    pips: Option<Pips>,
    away: bool,
}

impl Marks {
    fn of(input: &Input) -> Self {
        let pips = match (input.session.left(input.wall), input.total) {
            (Some(left), Some(total)) => pill::pips(left, total),
            _ => None,
        };
        Self { pips, away: input.away }
    }

    fn width(&self, painter: &egui::Painter) -> f32 {
        let pips = self.pips.map_or(0.0, |pips| pips_width(pips.count) + MARK_GAP);
        let away = if self.away { away_word_width(painter) + MARK_GAP * 2.0 } else { 0.0 };
        pips + away
    }
}

fn pips_width(count: u32) -> f32 {
    count as f32 * PIP_W + count.saturating_sub(1) as f32 * PIP_GAP
}

fn away_word_width(painter: &egui::Painter) -> f32 {
    painter.layout_no_wrap(AWAY_WORD.to_owned(), body(TEXT_TAG), brand::VIOLET_TEXT).size().x
}

/// Draws the button where the strip's cursor is and returns what the menu asked for.
pub(crate) fn show(ui: &mut Ui, input: &Input, memory: &mut Memory, now: f64) -> Output {
    let ctx = ui.ctx().clone();
    let (slot, _) = ui.allocate_exact_size(vec2(SLOT_W, SLOT_H), Sense::hover());
    let on = input.session.is_on();
    let (full, short) = label_texts(input.session, input.wall);
    let marks = Marks::of(input);
    let marks_w = marks.width(ui.painter());
    let (full_pill, short_pill) = (pill_for(ui, &full, marks_w), pill_for(ui, &short, marks_w));
    let fit = if input.force_short && on && input.gap >= 0.0 {
        Fit::Short
    } else if on {
        pill::fit(input.gap, full_pill, short_pill)
    } else {
        Fit::Hidden
    };
    let text = if fit == Fit::Short { &short } else { &full };
    let slide = memory.slide.get_or_insert_with(|| Slide::new(false));
    slide.set(fit != Fit::Hidden && on, now);
    if fit != Fit::Hidden {
        memory.open_width = pill::pill_width(fit, full_pill, short_pill);
    }
    let amount = slide.amount(now);
    let sliding = slide.moving(now);
    let width = ICON_W + (memory.open_width.max(ICON_W) - ICON_W) * amount as f32;
    let pill_rect = Rect::from_min_max(
        pos2(slot.right() - width, slot.center().y - PILL_H / 2.0),
        pos2(slot.right(), slot.center().y + PILL_H / 2.0),
    );
    let id = ui.id().with("amphetamine");
    let response = ui.interact(pill_rect.union(slot), id, Sense::click());
    let open = menu_open(&ctx, &response);
    let hovered = response.hovered() || open || memory.force_menu;
    let hover = f64::from(ctx.animate_bool_with_time(id.with("hover"), hovered, 0.25));
    let (focused, minimized) = ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
    let animating = mascot_motion::should_animate(
        ui.is_rect_visible(slot),
        focused.unwrap_or(false),
        minimized != Some(true),
    );
    if memory.shown != *text {
        if !memory.shown.is_empty() && fit != Fit::Hidden && on {
            memory.rolled = Some((std::mem::take(&mut memory.shown), now));
        }
        memory.shown.clone_from(text);
    }
    let rolling = memory.rolled.as_ref().is_some_and(|(_, since)| now - since < ROLL_S);
    if !rolling {
        memory.rolled = None;
    }
    let look = if animating || input.pinned { input.look } else { still(input.look) };
    let painter = ui.painter().clone();
    memory.last_pill = (amount > 0.01).then_some(pill_rect);
    paint_button_frame(&painter, slot, hovered, open, amount > 0.01);
    if amount > 0.01 {
        paint_pill(&painter, pill_rect, text, memory, amount, now);
        paint_marks(&painter, pill_rect, text, &marks, amount);
    }
    if on && (fit == Fit::Hidden || marks.pips.is_none()) {
        paint_line(&painter, pill_rect, slot, fit, input, amount);
    }
    amphetamine_icon::paint(&painter, slot, look, hover);
    if input.away {
        paint_away_badge(&painter, slot);
    }
    if animating
        && (look.animating() || sliding || rolling)
        && let Some(after) = mascot_motion::next_repaint(true)
    {
        ctx.request_repaint_after(after);
    }
    let response = if open { response } else { response.on_hover_ui(|ui| tooltip(ui, input)) };
    let mut draft = memory.draft.unwrap_or(amphetamine_menu::FIRST_DRAFT);
    let action = menu(&response, input, memory.force_menu, ui.ctx(), &mut draft);
    memory.draft = Some(draft);
    Output { action }
}

fn paint_button_frame(painter: &egui::Painter, slot: Rect, hovered: bool, open: bool, pill: bool) {
    if pill {
        return;
    }
    if hovered || open {
        let tint = Rect::from_center_size(slot.center(), vec2(ICON_BOX, ICON_BOX));
        painter.rect_filled(tint, CornerRadius::same(8), colors::hover());
    }
}

fn paint_pill(
    painter: &egui::Painter,
    rect: Rect,
    text: &str,
    memory: &Memory,
    amount: f64,
    now: f64,
) {
    let radius = CornerRadius::same(16);
    let fade = pill::label_alpha(amount);
    let breath = 0.75 + 0.25 * (now * std::f64::consts::TAU / BREATH_S).sin() as f32;
    let glow = egui::epaint::Shadow {
        offset: [0, 0],
        blur: 18,
        spread: 0,
        color: brand::TEAL.gamma_multiply(0.22 * breath * amount as f32),
    };
    painter.add(glow.as_shape(rect, radius));
    painter.add(gradient(rect, brand::TEAL_NAV, GRADIENT_END));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, brand::TEAL.gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
    let clip = painter.with_clip_rect(rect.shrink2(vec2(2.0, 0.0)));
    let at = pos2(rect.right() - ICON_W - LABEL_PAD_RIGHT, rect.center().y);
    let slide_in = (1.0 - fade) * 6.0;
    let ink = brand::TEAL_TEXT;
    match &memory.rolled {
        Some((old, since)) => {
            let p = bezier(STANDARD, (now - since) / ROLL_S) as f32;
            let travel = 16.0;
            clip.text(
                at + vec2(slide_in, -travel * p),
                Align2::RIGHT_CENTER,
                old,
                mono(LABEL_SIZE),
                ink.gamma_multiply(fade * (1.0 - p)),
            );
            clip.text(
                at + vec2(slide_in, travel * (1.0 - p)),
                Align2::RIGHT_CENTER,
                text,
                mono(LABEL_SIZE),
                ink.gamma_multiply(fade * p),
            );
        }
        None => {
            clip.text(
                at + vec2(slide_in, 0.0),
                Align2::RIGHT_CENTER,
                text,
                mono(LABEL_SIZE),
                ink.gamma_multiply(fade),
            );
        }
    }
}

fn paint_marks(painter: &egui::Painter, pill: Rect, text: &str, marks: &Marks, amount: f64) {
    let fade = pill::label_alpha(amount);
    let clip = painter.with_clip_rect(pill.shrink2(vec2(2.0, 0.0)));
    let label_w =
        painter.layout_no_wrap(text.to_owned(), mono(LABEL_SIZE), brand::TEAL_TEXT).size().x;
    let mut right = pill.right() - ICON_W - LABEL_PAD_RIGHT - label_w - MARK_GAP;
    let y = pill.center().y;
    if let Some(pips) = marks.pips {
        for index in 0..pips.count {
            let x = right - (pips.count - index) as f32 * (PIP_W + PIP_GAP) + PIP_GAP;
            let pip = Rect::from_min_size(pos2(x, y - PIP_H / 2.0), vec2(PIP_W, PIP_H));
            let glowing = index >= pips.count - pips.lit;
            if glowing {
                clip.rect_filled(
                    pip.expand(1.5),
                    CornerRadius::same(3),
                    brand::TEAL.gamma_multiply(0.25 * fade),
                );
            }
            let ink = if glowing { brand::TEAL_TEXT } else { brand::ITEM_LINE };
            clip.rect_filled(pip, CornerRadius::same(2), ink.gamma_multiply(fade));
        }
        right -= pips_width(pips.count) + MARK_GAP;
    }
    if marks.away {
        let rule = right;
        clip.line_segment(
            [pos2(rule, y - 7.0), pos2(rule, y + 7.0)],
            Stroke::new(1.0, brand::VIOLET_LINE.gamma_multiply(fade)),
        );
        clip.text(
            pos2(rule - MARK_GAP, y),
            Align2::RIGHT_CENTER,
            AWAY_WORD,
            body(TEXT_TAG),
            brand::VIOLET_TEXT.gamma_multiply(fade),
        );
    }
}

fn paint_line(
    painter: &egui::Painter,
    pill: Rect,
    slot: Rect,
    fit: Fit,
    input: &Input,
    amount: f64,
) {
    let (Some(left), Some(total)) = (input.session.left(input.wall), input.total) else { return };
    let (span, inset) =
        if fit == Fit::Hidden { (slot, LINE_INSET_BARE) } else { (pill, LINE_INSET) };
    let y = span.bottom() - 1.0;
    let track =
        Rect::from_min_max(pos2(span.left() + inset, y - LINE_H), pos2(span.right() - inset, y));
    let radius = CornerRadius::same(1);
    let shown = if fit == Fit::Hidden { 1.0 } else { amount.clamp(0.0, 1.0) as f32 };
    painter.rect_filled(track, radius, brand::ITEM_LINE.gamma_multiply(shown));
    let fill = Rect::from_min_size(
        track.min,
        vec2(track.width() * line_fraction(left, total), track.height()),
    );
    let halo = fill.expand2(vec2(1.0, 1.5));
    painter.rect_filled(halo, CornerRadius::same(2), brand::TEAL.gamma_multiply(0.25 * shown));
    painter.rect_filled(fill, radius, brand::TEAL.gamma_multiply(0.95 * shown));
}

/// A left-to-right fill inside the pill's rounded outline, as one fan of triangles.
fn gradient(rect: Rect, from: Color32, to: Color32) -> egui::Shape {
    let mut outline = Vec::new();
    egui::epaint::tessellator::path::rounded_rectangle(
        &mut outline,
        rect.shrink(0.5),
        CornerRadius::same(16).into(),
    );
    let ink = |x: f32| from.lerp_to_gamma(to, ((x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), ink(rect.center().x));
    for point in &outline {
        mesh.colored_vertex(*point, ink(point.x));
    }
    let count = outline.len() as u32;
    for index in 0..count {
        mesh.add_triangle(0, 1 + index, 1 + (index + 1) % count);
    }
    egui::Shape::mesh(mesh)
}

/// The crescent on the tablet's lower right while Away is on.
fn paint_away_badge(painter: &egui::Painter, slot: Rect) {
    let center = slot.center() + vec2(11.0, 10.0);
    painter.circle(center, 5.5, brand::VIOLET_FILL, Stroke::new(1.5, brand::GROUND));
    amphetamine_menu::paint_moon(painter, center, 3.2, brand::VIOLET_TEXT, brand::VIOLET_FILL);
}

fn tooltip(ui: &mut Ui, input: &Input) {
    ui.set_max_width(270.0);
    let (title, body_text) = tip_text(input);
    ui.label(RichText::new(title).color(brand::TEXT));
    ui.label(RichText::new(body_text).color(brand::SOFT));
    if input.away {
        ui.label(RichText::new(away_model::TIP).color(brand::VIOLET_TEXT));
    }
}

pub(crate) fn tip_text(input: &Input) -> (&'static str, String) {
    let machine = if input.lid { "Mac" } else { "computer" };
    match input.session {
        State::Off if input.lid => (
            "Amphetamine",
            format!(
                "Keep this Mac awake, even with the lid closed on battery. Pick a timer.{}",
                match input.route {
                    Route::SetUp => " Asks for your password once to set up Touch ID.",
                    Route::Fingerprint => " Starts with Touch ID.",
                    Route::Outdated => " Asks for your password once to update Touch ID.",
                    Route::Password => "",
                }
            ),
        ),
        State::Off => (
            "Amphetamine",
            "Keep this computer awake. Pick a timer. There is no lid mode on Linux.".to_owned(),
        ),
        State::Starting { .. } if input.route == Route::Fingerprint => {
            ("Amphetamine", "Waiting for Touch ID.".to_owned())
        }
        State::Starting { .. } => ("Amphetamine", "Waiting for the password prompt.".to_owned()),
        State::Stopping => ("Amphetamine", "Turning off.".to_owned()),
        State::On { until, .. } => {
            let stays = if input.lid {
                format!("The {machine} stays awake with the lid closed on battery.")
            } else {
                format!("The {machine} stays awake.")
            };
            let first = until.map_or_else(
                || format!("No timer. The {machine} stays awake until you turn it off."),
                |until| {
                    format!(
                        "{} {stays} Click to change or stop.",
                        left_text((until - input.wall).max(0.0)) + "."
                    )
                },
            );
            ("Amphetamine is on", first)
        }
    }
}

fn menu_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(12)
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(128),
        })
        .inner_margin(Margin::same(MENU_MARGIN))
}

fn menu(
    response: &Response,
    input: &Input,
    force: bool,
    ctx: &egui::Context,
    draft: &mut Choice,
) -> Option<Action> {
    let menu_session = matches!(input.session, State::Off | State::On { .. });
    if force {
        egui::Popup::open_id(ctx, egui::Popup::default_response_id(response));
    }
    if !menu_session {
        egui::Popup::close_id(ctx, egui::Popup::default_response_id(response));
        return None;
    }
    let mut action = None;
    egui::Popup::menu(response)
        .align(RectAlign::BOTTOM_END)
        .gap(8.0)
        .frame(menu_frame())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(MENU_W - f32::from(MENU_MARGIN) * 2.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            action = amphetamine_menu::body_ui(ui, input, draft);
            if action.is_some_and(|action| action != Action::Away) {
                ui.close();
            }
        });
    action
}

/// An amber triangle with a bar and a dot, drawn so no font has to have the glyph.
fn paint_warning(painter: &egui::Painter, center: egui::Pos2, size: f32) {
    let unit = size / 24.0;
    let ink = Stroke::new(1.6 * unit * 1.5, brand::AMBER);
    let tip = center + vec2(0.0, -9.0 * unit);
    let left = center + vec2(-10.0 * unit, 8.0 * unit);
    let right = center + vec2(10.0 * unit, 8.0 * unit);
    painter.add(egui::Shape::closed_line(vec![tip, left, right], ink));
    painter.line_segment([center + vec2(0.0, -3.0 * unit), center + vec2(0.0, 2.0 * unit)], ink);
    painter.circle_filled(center + vec2(0.0, 5.0 * unit), 1.0 * unit * 1.5, brand::AMBER);
}

fn omen_ink(omen: Omen) -> Color32 {
    match omen {
        Omen::StarsRight => brand::BLUE_TEXT,
        Omen::Shoggoth => brand::TEAL_TEXT,
        Omen::Hours => brand::AMBER,
        Omen::ElderSign => brand::ORANGE,
        Omen::Rlyeh => brand::RED,
        Omen::Plain => brand::TEXT,
    }
}

/// The toast pill with an omen tag in front of each line.
pub(crate) fn toast(ui: &Ui, column: Rect, lines: &[(Omen, String)], fade: f32) {
    let alpha = |color: Color32| color.gamma_multiply(fade);
    let painter = ui.painter();
    let tag_font = mono(TEXT_TAG);
    let rows: Vec<_> = lines
        .iter()
        .map(|(omen, text)| {
            let tag =
                painter.layout_no_wrap(omen.tag().to_owned(), tag_font.clone(), omen_ink(*omen));
            let wrap =
                (column.width() - 2.0 * (TOAST_PAD.x + 16.0) - tag.size().x - 10.0).max(120.0);
            let line = painter.layout(text.clone(), body(TEXT_BODY), brand::TEXT, wrap);
            (*omen, tag, line)
        })
        .collect();
    let tag_w = rows.iter().map(|(_, tag, _)| tag.size().x).fold(0.0, f32::max);
    let gap = if tag_w > 0.0 { 10.0 } else { 0.0 };
    let text_w = rows.iter().map(|(_, _, line)| line.size().x).fold(0.0, f32::max);
    let height: f32 = rows.iter().map(|(_, _, line)| line.size().y + 3.0).sum::<f32>() - 3.0;
    let size = vec2(tag_w + gap + text_w, height) + TOAST_PAD * 2.0;
    let center =
        pos2(column.center().x, column.bottom() - TOAST_BOTTOM - (size.y - 32.0).max(0.0) / 2.0);
    let rect = Rect::from_center_size(center, size);
    painter.rect_filled(rect, 10.0, alpha(brand::ITEM));
    painter.rect_stroke(rect, 10.0, Stroke::new(1.0, alpha(brand::ITEM_LINE)), StrokeKind::Inside);
    let mut y = rect.top() + TOAST_PAD.y;
    for (omen, tag, line) in rows {
        let tag_at =
            pos2(rect.left() + TOAST_PAD.x, y + (line.size().y - tag.size().y) / 2.0 + 1.0);
        let line_height = line.size().y;
        painter.galley_with_override_text_color(tag_at, tag, alpha(omen_ink(omen)));
        painter.galley_with_override_text_color(
            pos2(rect.left() + TOAST_PAD.x + tag_w + gap, y),
            line,
            alpha(brand::TEXT),
        );
        y += line_height + 3.0;
    }
}

/// The card shown once at launch when the last run left sleep off. True when Got it was clicked.
pub(crate) fn notice(
    ctx: &egui::Context,
    notice: &crate::amphetamine_glue::Notice,
    column: Rect,
) -> bool {
    let mut dismissed = false;
    egui::Area::new(Id::new("amphetamine_notice"))
        .order(Order::Foreground)
        .fixed_pos(pos2(column.center().x - NOTICE_W / 2.0, column.top() + 78.0))
        .show(ctx, |ui| {
            let frame = egui::Frame::new()
                .fill(brand::ITEM)
                .stroke(Stroke::new(1.0, brand::ITEM_LINE))
                .corner_radius(12)
                .inner_margin(Margin::same(16))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha(110),
                });
            frame.show(ui, |ui| {
                ui.set_width(NOTICE_W - 32.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (glyph, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                    ui.painter().rect(
                        glyph,
                        CornerRadius::same(8),
                        Color32::from_rgb(0x14, 0x19, 0x1f),
                        Stroke::new(1.0, brand::ITEM_LINE),
                        StrokeKind::Inside,
                    );
                    paint_warning(ui.painter(), glyph.center() + vec2(0.0, 0.5), 18.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.label(RichText::new(notice.title).color(brand::TEXT).size(TEXT_TITLE));
                        ui.add(
                            egui::Label::new(
                                RichText::new(&notice.body).color(brand::SOFT).size(TEXT_BODY),
                            )
                            .wrap(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(notice.foot).color(brand::FAINT).size(TEXT_SECONDARY),
                            )
                            .wrap(),
                        );
                        ui.add_space(4.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            dismissed = quiet_button(ui, "Got it", true, Size::Compact).clicked();
                        });
                    });
                });
            });
        });
    dismissed
}

#[cfg(test)]
mod tests;
