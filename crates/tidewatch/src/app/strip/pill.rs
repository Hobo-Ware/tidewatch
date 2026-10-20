//! The Parked pill at the strip's right end and the list it opens.

use egui::{
    Align2, Color32, CornerRadius, Id, Margin, Pos2, Rect, RectAlign, Sense, Stroke, StrokeKind,
    Ui, pos2, vec2,
};
use stdusk_egui::colors;

use crate::agent_avatar::{self, Face};
use crate::brand;
use crate::park::{Line, Tone};
use crate::remote_pill::Popover;
use crate::screen_kit::body;

pub(crate) const PILL_H: f32 = 28.0;
const FACE: f32 = 20.0;
const FACE_STEP: f32 = 14.0;
const PAD_LEFT: f32 = 8.0;
const PAD_RIGHT: f32 = 12.0;
const GAP: f32 = 10.0;
const CHEVRON_W: f32 = 12.0;
const CHEVRON_GAP: f32 = 8.0;
const LABEL_SIZE: f32 = 13.0;
const MOST_FACES: usize = 3;
const POPOVER_W: f32 = 420.0;
const ROW_H: f32 = 52.0;
const ROW_FACE: f32 = 28.0;
const ROW_PAD: f32 = 10.0;
const ROW_GAP: f32 = 12.0;
const PLACED: &str = "parked-pill-placed";
const HOT: &str = "parked-pill-hot";
const DRAGGING: &str = "parked-pill-dragging";
const GHOST_LABEL: &str = "Drop to park";

#[derive(Clone)]
pub(crate) struct Row {
    pub(crate) index: usize,
    pub(crate) name: String,
    pub(crate) face: Face,
    pub(crate) line: Line,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Placed {
    pub(crate) pill: Option<Rect>,
    pub(crate) rows: Vec<Rect>,
}

pub(crate) fn placed_id() -> Id {
    Id::new(PLACED)
}

pub(crate) fn rect(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|data| data.get_temp::<Placed>(placed_id())).and_then(|placed| placed.pill)
}

pub(crate) fn set_hot(ctx: &egui::Context, hot: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(HOT), hot));
}

pub(crate) fn set_dragging(ctx: &egui::Context, dragging: bool) {
    ctx.data_mut(|data| data.insert_temp(Id::new(DRAGGING), dragging));
}

pub(crate) fn wanted(ctx: &egui::Context, count: usize) -> bool {
    count > 0 || ctx.data(|data| data.get_temp::<bool>(Id::new(DRAGGING))).unwrap_or(false)
}

fn hot(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(Id::new(HOT))).unwrap_or(false)
}

pub(crate) fn label(count: usize) -> String {
    format!("Parked {count}")
}

pub(crate) fn width(ui: &Ui, count: usize) -> f32 {
    if count == 0 {
        let galley =
            ui.painter().layout_no_wrap(GHOST_LABEL.to_owned(), body(LABEL_SIZE), brand::SOFT);
        return PAD_RIGHT * 2.0 + galley.size().x;
    }
    let galley = ui.painter().layout_no_wrap(label(count), body(LABEL_SIZE), brand::SOFT);
    let stack = FACE + FACE_STEP * (count.min(MOST_FACES) - 1) as f32;
    PAD_LEFT + stack + GAP + galley.size().x + CHEVRON_GAP + CHEVRON_W + PAD_RIGHT
}

pub(crate) fn widest(ui: &Ui) -> f32 {
    width(ui, 99)
}

pub(crate) struct Output {
    pub(crate) picked: Option<usize>,
}

fn tone_ink(tone: Tone) -> Color32 {
    match tone {
        Tone::Teal => brand::TEAL,
        Tone::Blue => brand::BLUE_TEXT,
        Tone::Muted => brand::MUTED,
    }
}

fn paint_chevron(painter: &egui::Painter, center: Pos2, ink: Color32) {
    let stroke = Stroke::new(1.5, ink);
    let (left, middle, right) = (vec2(-3.0, -1.5), vec2(0.0, 1.5), vec2(3.0, -1.5));
    painter.line_segment([center + left, center + middle], stroke);
    painter.line_segment([center + middle, center + right], stroke);
}

pub(crate) fn show(ui: &mut Ui, rows: &[Row], popover: Popover) -> Output {
    let count = rows.len();
    let (rect, _) = ui.allocate_exact_size(vec2(width(ui, count), PILL_H), Sense::hover());
    let dropping = hot(ui.ctx());
    if count == 0 {
        paint_ghost(ui, rect, dropping);
        ui.ctx().data_mut(|data| {
            data.insert_temp(placed_id(), Placed { pill: Some(rect), rows: Vec::new() });
        });
        return Output { picked: None };
    }
    let response = ui.interact(rect, ui.id().with("parked-pill"), Sense::click());
    let painter = ui.painter().clone();
    let fill = if response.hovered() || dropping { brand::ITEM } else { brand::BAR };
    let line = if dropping { brand::TEAL } else { brand::FIELD_LINE };
    painter.rect_filled(rect, CornerRadius::same(255), fill);
    painter.rect_stroke(rect, CornerRadius::same(255), Stroke::new(1.0, line), StrokeKind::Inside);
    for (n, row) in rows.iter().take(MOST_FACES).enumerate() {
        let at = pos2(rect.left() + PAD_LEFT + FACE_STEP * n as f32, rect.center().y - FACE / 2.0);
        let face = Rect::from_min_size(at, vec2(FACE, FACE));
        painter.rect_filled(face.expand(1.5), CornerRadius::same(7), fill);
        agent_avatar::paint_face(ui.ctx(), &painter, face, &row.face);
    }
    let stack = FACE + FACE_STEP * (count.clamp(1, MOST_FACES) - 1) as f32;
    let text_at = pos2(rect.left() + PAD_LEFT + stack + GAP, rect.center().y);
    painter.text(text_at, Align2::LEFT_CENTER, label(count), body(LABEL_SIZE), brand::SOFT);
    paint_chevron(
        &painter,
        pos2(rect.right() - PAD_RIGHT - CHEVRON_W / 2.0, rect.center().y),
        brand::MUTED,
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Parked tabs keep running. Click one to bring it back.");
    if popover == Popover::Open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    let mut placed = Placed { pill: Some(rect), rows: Vec::new() };
    let picked = list(&response, rows, &mut placed);
    ui.ctx().data_mut(|data| data.insert_temp(placed_id(), placed));
    Output { picked }
}

fn paint_ghost(ui: &Ui, rect: Rect, dropping: bool) {
    let painter = ui.painter();
    let line = if dropping { brand::TEAL } else { brand::FIELD_LINE };
    let ink = if dropping { brand::TEAL_TEXT } else { brand::MUTED };
    painter.rect_filled(
        rect,
        CornerRadius::same(255),
        if dropping { brand::TEAL_NAV } else { brand::BAR },
    );
    painter.rect_stroke(rect, CornerRadius::same(255), Stroke::new(1.0, line), StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, GHOST_LABEL, body(LABEL_SIZE), ink);
}

fn list(response: &egui::Response, rows: &[Row], placed: &mut Placed) -> Option<usize> {
    let mut picked = None;
    let frame = egui::Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(12)
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(110),
        })
        .inner_margin(Margin::same(8));
    egui::Popup::menu(response)
        .align(RectAlign::BOTTOM_END)
        .gap(6.0)
        .frame(frame)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(POPOVER_W - 16.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (n, row) in rows.iter().enumerate() {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                placed.rows.push(rect);
                paint_row(ui, rect, row, resp.hovered());
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    picked = Some(n);
                    ui.close();
                }
            }
        });
    picked
}

fn paint_row(ui: &Ui, rect: Rect, row: &Row, hovered: bool) {
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, CornerRadius::same(8), colors::hover_elevated());
    }
    let face = Rect::from_min_size(
        pos2(rect.left() + ROW_PAD, rect.center().y - ROW_FACE / 2.0),
        vec2(ROW_FACE, ROW_FACE),
    );
    agent_avatar::paint_face(ui.ctx(), painter, face, &row.face);
    let left = face.right() + ROW_GAP;
    let room = (rect.right() - ROW_PAD - left).max(1.0);
    let name = painter.layout(row.name.clone(), body(14.0), brand::TEXT, room);
    let line = painter.layout(row.line.text.clone(), body(12.5), tone_ink(row.line.tone), room);
    let block = name.size().y + line.size().y;
    let top = rect.center().y - block / 2.0;
    let line_top = top + name.size().y;
    painter.galley(pos2(left, top), name, brand::TEXT);
    painter.galley(pos2(left, line_top), line, brand::MUTED);
}
