//! The message box of the Reader and of an agent tab's first message: one widget. A single line is a
//! 48 px box with the text centred, the placeholder where typed text starts and Send centred on the
//! line; it grows with its text up to 40% of the column, then scrolls inside. Pasted pictures are
//! chips above the text, `:` and two letters open the emoji list, a slash opens the command list.

use std::path::PathBuf;

use egui::epaint::RectShape;
use egui::text::CCursor;
use egui::{
    CornerRadius, EventFilter, FontId, Frame, Id, Key, KeyboardShortcut, Modifiers, Rect, RichText,
    ScrollArea, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2,
};

use crate::brand;
use crate::bullets;
use crate::emoji_pick::{self, Hit, MAX_ROWS, Query, Usage};
use crate::emoji_view;
use crate::pictures;
use crate::reader_chat_view::BODY_PX;
use crate::reader_view::INSET;
use crate::screen_kit::{Size, button_width, primary_button};
use crate::slash::Entry as SlashEntry;
use crate::slash_view;

pub(crate) const BOX_H: f32 = 48.0;
pub(crate) const GAP_Y: f32 = 8.0;
pub(crate) const SHARE: f32 = 0.4;
const BORDER: f32 = 1.0;
const RADIUS: u8 = 14;
const PAD_X: f32 = 18.0;
const SEND_GAP: f32 = 12.0;
const CHIP_TOP: f32 = 10.0;
const CHIP_TEXT_GAP: f32 = 8.0;
const SEND: &str = "Send";
const OPTICAL_LIFT: f32 = 0.4;
const FOCUS_RING: (f32, f32) = (3.0, 0.10);
const DROP_RING: (f32, f32) = (6.0, 0.45);
const DROP_FILL: f32 = 0.12;

pub(crate) fn text_font() -> FontId {
    FontId::proportional(BODY_PX)
}

fn send_edge() -> f32 {
    (BOX_H - Size::Regular.height()) / 2.0
}

/// The sizes of a box, worked out from the height of one text row so the space above and below
/// the text is the same by construction: `pad_y` is what one text row needs to fill `BOX_H`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fit {
    pub(crate) box_h: f32,
    pub(crate) pad_y: f32,
    pub(crate) chips_h: f32,
    pub(crate) scrolls: bool,
}

pub(crate) fn fit(row_h: f32, text_h: f32, chip_rows: usize, cap: f32) -> Fit {
    let pad_y = ((BOX_H - 2.0 * BORDER - row_h) / 2.0).max(0.0);
    let chips_h = if chip_rows == 0 {
        0.0
    } else {
        CHIP_TOP + pictures::chips_height(chip_rows) + CHIP_TEXT_GAP
    };
    let top_pad = if chip_rows == 0 { pad_y } else { 0.0 };
    let around = 2.0 * BORDER + chips_h + top_pad + pad_y;
    let wanted = around + text_h;
    let least = (around + row_h).max(BOX_H);
    let box_h = wanted.min(cap.max(BOX_H)).max(least);
    Fit { box_h, pad_y, chips_h, scrolls: wanted > box_h + 0.5 }
}

struct Measure {
    fit: Fit,
    send_w: f32,
}

fn row_height(ui: &Ui) -> f32 {
    ui.painter().layout_no_wrap(String::new(), text_font(), brand::TEXT).size().y
}

fn measure(ui: &Ui, text: &str, pictures: usize, box_w: f32, cap: f32) -> Measure {
    let send_w = button_width(ui, SEND, Size::Regular);
    let wrap = box_w - PAD_X - send_edge() - send_w - SEND_GAP;
    let typed = bullets::layout(ui, text, &text_font(), (brand::TEXT, brand::MUTED), wrap, &None);
    let rows = pictures::chip_rows(pictures, box_w - 2.0 * PAD_X, pictures::REMOVABLE_W);
    let fit = fit(row_height(ui), typed.size().y, rows, cap);
    Measure { fit, send_w }
}

/// The height a box takes in `column`, with the gap above and below it.
pub(crate) fn height(ui: &Ui, text: &str, pictures: usize, column: Rect) -> f32 {
    let cap = (column.height() * SHARE).max(BOX_H);
    measure(ui, text, pictures, column.width() - 2.0 * INSET, cap).fit.box_h + 2.0 * GAP_Y
}

pub(crate) struct Draft<'a> {
    pub(crate) text: &'a mut String,
    pub(crate) pictures: &'a mut Vec<PathBuf>,
    pub(crate) focus: &'a mut bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Spec<'a> {
    pub(crate) id: Id,
    pub(crate) hint: &'a str,
    pub(crate) enabled: bool,
    pub(crate) commands: &'a [SlashEntry],
    pub(crate) ghost: Option<&'a str>,
}

fn collapsed_cursor(ui: &Ui, id: Id) -> Option<usize> {
    let range = TextEdit::load_state(ui.ctx(), id)?.cursor.char_range()?;
    (range.primary == range.secondary).then_some(range.primary.index.0)
}

fn byte_of(text: &str, at: usize) -> usize {
    text.char_indices().nth(at).map_or(text.len(), |(byte, _)| byte)
}

fn consume(ui: &Ui, key: Key) -> bool {
    ui.input_mut(|input| input.consume_key(Modifiers::NONE, key))
}

fn usage_id() -> Id {
    Id::new("emoji-usage")
}

fn usage(ui: &Ui) -> Usage {
    ui.data(|data| data.get_temp(usage_id())).unwrap_or_default()
}

fn count_use(ui: &Ui, emoji: &'static emojis::Emoji) {
    ui.data_mut(|data| data.get_temp_mut_or_default::<Usage>(usage_id()).bump(emoji));
}

struct Typing {
    query: Query,
    cursor: usize,
}

fn typing_at(text: &str, cursor: Option<usize>) -> Option<Typing> {
    let cursor = cursor?;
    emoji_pick::query_at(text, byte_of(text, cursor)).map(|query| Typing { query, cursor })
}

fn picked_into(ui: &Ui, id: Id, text: &mut String, typing: &Typing, hit: &Hit) {
    let cursor = byte_of(text, typing.cursor);
    let after = emoji_pick::insert(text, &typing.query, cursor, hit.emoji);
    bullets::place_cursor(ui, id, text[..after].chars().count());
    count_use(ui, hit.emoji);
}

fn emoji_keys(
    ui: &Ui,
    id: Id,
    text: &mut String,
    menu: &mut emoji_pick::Menu,
    (hits, typing): (&[Hit], &Typing),
) -> bool {
    let pressed = if consume(ui, Key::Escape) {
        emoji_pick::Key::Close
    } else if consume(ui, Key::ArrowDown) {
        emoji_pick::Key::Down
    } else if consume(ui, Key::ArrowUp) {
        emoji_pick::Key::Up
    } else if consume(ui, Key::Enter) || consume(ui, Key::Tab) {
        emoji_pick::Key::Accept
    } else {
        return false;
    };
    if let emoji_pick::Pressed::Accepted(at) = menu.press(pressed, hits.len())
        && let Some(hit) = hits.get(at)
    {
        picked_into(ui, id, text, typing, hit);
    }
    true
}

fn emoji_hits(ui: &Ui, menu: &emoji_pick::Menu, typing: Option<&Typing>) -> Vec<Hit> {
    let query = typing.map(|typing| &typing.query);
    match typing {
        Some(typing) if menu.is_open(query) => {
            emoji_pick::find(&typing.query.term, &usage(ui), MAX_ROWS)
        }
        _ => Vec::new(),
    }
}

fn menu_id(id: Id) -> Id {
    id.with("emoji-state")
}

fn backspace_removes_a_picture(ui: &Ui, id: Id, text: &str, pictures: &mut Vec<PathBuf>) -> bool {
    let at_start = collapsed_cursor(ui, id).map_or(text.is_empty(), |at| at == 0);
    let took = at_start && !pictures.is_empty() && consume(ui, Key::Backspace);
    if took {
        pictures.pop();
    }
    took
}

/// Draws the box in the room `ui` has and works it. Enter sends a non-empty text or any pictures and
/// clears both, Shift+Enter starts a new line, and on a bullet line Enter starts the next bullet
/// instead. Asks for focus once, the first time it is drawn, and again after a send. Off while
/// `spec.enabled` is false.
pub(crate) fn show(ui: &mut Ui, draft: Draft<'_>, spec: &Spec<'_>) -> Option<String> {
    let Draft { text, pictures, focus } = draft;
    let Spec { id, hint, enabled, commands, ghost } = *spec;
    let room = ui.max_rect();
    let mut sent = None;
    let mut slash = slash_view::load(ui, id);
    let slash_keys_rows = if enabled { slash.rows(text, commands) } else { Vec::new() };
    if enabled {
        slash_view::keys(ui, id, text, &mut slash, &slash_keys_rows);
    }
    let mut emoji_menu: emoji_pick::Menu =
        ui.data(|data| data.get_temp(menu_id(id))).unwrap_or_default();
    let focused = ui.memory(|memory| memory.has_focus(id));
    let before = typing_at(text, collapsed_cursor(ui, id)).filter(|_| slash_keys_rows.is_empty());
    emoji_menu.follow(before.as_ref().map(|typing| &typing.query));
    if enabled && focused {
        let hits = emoji_hits(ui, &emoji_menu, before.as_ref());
        if let Some(typing) = before.as_ref().filter(|_| !hits.is_empty()) {
            emoji_keys(ui, id, text, &mut emoji_menu, (&hits, typing));
        }
        backspace_removes_a_picture(ui, id, text, pictures);
    }
    let tint: bullets::Tint = before
        .as_ref()
        .filter(|_| enabled && focused && !emoji_hits(ui, &emoji_menu, before.as_ref()).is_empty())
        .map(|typing| {
            let end = byte_of(text, typing.cursor);
            (typing.query.start..end, brand::TEAL_TEXT)
        });
    let ghost = ghost.filter(|_| enabled && text.is_empty());
    if let Some(ghost) = ghost
        && ui.memory(|memory| memory.has_focus(id))
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab))
    {
        *text = ghost.to_owned();
        bullets::place_cursor(ui, id, text.chars().count());
    }
    let ghost = ghost.filter(|_| text.is_empty());
    let listing = enabled && (ui.memory(|memory| memory.has_focus(id)) || !ui.input(|i| i.focused));
    let slash_rows = if listing { slash.rows(text, commands) } else { Vec::new() };
    slash.open = !slash_rows.is_empty();
    let mut enter = enabled
        && ui.memory(|memory| memory.has_focus(id))
        && ui.input(|input| input.events.iter().any(crate::agent_ready::plain_enter));
    if enter && let Some(cursor) = collapsed_cursor(ui, id) {
        let (outcome, at) = bullets::enter(text, cursor);
        if outcome != bullets::Enter::Plain {
            bullets::place_cursor(ui, id, at);
            enter = false;
        }
    }
    ui.add_enabled_ui(enabled, |ui| {
        let measured = measure(ui, text, pictures.len(), room.width(), room.height());
        let Fit { box_h, pad_y, chips_h, .. } = measured.fit;
        let boxed = Rect::from_min_size(room.min, vec2(room.width(), box_h));
        let background = ui.painter().add(Shape::Noop);
        let ring = ui.painter().add(Shape::Noop);
        let whole = ui.interact(boxed, id.with("box"), Sense::click());
        let edge = send_edge();
        let send = Rect::from_min_size(
            pos2(
                boxed.right() - edge - measured.send_w,
                boxed.bottom() - edge - Size::Regular.height(),
            ),
            vec2(measured.send_w, Size::Regular.height()),
        );
        let chips_origin = pos2(boxed.left() + PAD_X, boxed.top() + BORDER + CHIP_TOP);
        let removed =
            pictures::paint_block(ui, chips_origin, boxed.width() - 2.0 * PAD_X, pictures, true);
        if let Some(at) = removed {
            pictures.remove(at);
        }
        let view = Rect::from_min_max(
            pos2(boxed.left() + PAD_X, boxed.top() + BORDER + chips_h),
            pos2(send.left() - SEND_GAP, boxed.bottom() - BORDER),
        );
        let shown = view.height();
        let height_id = id.with("height");
        let before = ui.data(|data| data.get_temp::<f32>(height_id));
        ui.data_mut(|data| data.insert_temp(height_id, shown));
        let resized = before.is_some_and(|before| (before - shown).abs() > 0.5);
        let lift = if chips_h == 0.0 { OPTICAL_LIFT } else { 0.0 };
        let top_pad = if chips_h == 0.0 { pad_y - lift } else { 0.0 };
        let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            bullets::layout(
                ui,
                buf.as_str(),
                &text_font(),
                (brand::TEXT, brand::MUTED),
                wrap,
                &tint,
            )
        };
        let placeholder = if ghost.is_some() { brand::MUTED } else { brand::FAINT };
        ui.visuals_mut().weak_text_color = Some(placeholder);
        let mut output = None;
        ui.scope_builder(UiBuilder::new().max_rect(view), |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.style_mut().scroll_animation = egui::style::ScrollAnimation::none();
            ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .max_height(shown)
                .min_scrolled_height(shown)
                .auto_shrink([false, false])
                .scroll_bar_visibility(if measured.fit.scrolls {
                    egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
                } else {
                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden
                })
                .show(ui, |ui| {
                    ui.add_space(top_pad);
                    let edit = TextEdit::multiline(text)
                        .id(id)
                        .frame(Frame::NONE)
                        .hint_text(RichText::new(ghost.unwrap_or(hint)).size(BODY_PX))
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .font(text_font())
                        .text_color(brand::TEXT)
                        .layouter(&mut layouter)
                        .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                        .show(ui);
                    ui.add_space(pad_y + lift);
                    if resized && let Some(range) = edit.cursor_range {
                        let at = edit.galley_pos.to_vec2() - vec2(edit.galley.rect.left(), 0.0);
                        ui.scroll_to_rect(
                            edit.galley.pos_from_cursor(range.primary).translate(at),
                            None,
                        );
                    }
                    output = Some(edit);
                });
        });
        let can_send = !text.trim().is_empty() || !pictures.is_empty();
        let mut clicked = false;
        ui.scope_builder(UiBuilder::new().max_rect(send), |ui| {
            clicked = primary_button(ui, SEND, can_send, Size::Regular).clicked();
        });
        if whole.clicked() || std::mem::take(focus) {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        let has_focus = ui.memory(|memory| memory.has_focus(id));
        let dropping = crate::file_drop::hovering(ui.ctx());
        let line = if has_focus || dropping { brand::TEAL } else { brand::FIELD_LINE };
        let radius = CornerRadius::same(RADIUS);
        ui.painter().set(
            background,
            RectShape::new(
                boxed,
                radius,
                if dropping {
                    brand::CARD.lerp_to_gamma(brand::TEAL, DROP_FILL)
                } else {
                    brand::CARD
                },
                Stroke::new(BORDER, line),
                StrokeKind::Inside,
            ),
        );
        if has_focus || dropping {
            let (width, alpha) = if dropping { DROP_RING } else { FOCUS_RING };
            ui.painter().set(
                ring,
                RectShape::stroke(
                    boxed.expand(1.5),
                    CornerRadius::same(RADIUS + 1),
                    Stroke::new(width, brand::TEAL.gamma_multiply(alpha)),
                    StrokeKind::Middle,
                ),
            );
        }
        if ghost.is_some() && has_focus {
            let filter = EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                ..EventFilter::default()
            };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
        }
        if let Some(edit) = output {
            let cursor =
                edit.cursor_range.filter(|r| r.primary == r.secondary).map(|r| r.primary.index.0);
            if edit.response.changed()
                && let Some(at) = cursor
                && let Some(closed) = emoji_pick::closed_before(text, byte_of(text, at))
            {
                let cursor_byte = byte_of(text, at);
                text.replace_range(closed.start..closed.end, closed.emoji.as_str());
                let moved = cursor_byte - (closed.end - closed.start) + closed.emoji.as_str().len();
                bullets::place_cursor(ui, id, text[..moved].chars().count());
                count_use(ui, closed.emoji);
            }
            let after =
                typing_at(text, cursor).filter(|_| slash_rows.is_empty() && enabled && has_focus);
            emoji_menu.follow(after.as_ref().map(|typing| &typing.query));
            let hits = emoji_hits(ui, &emoji_menu, after.as_ref());
            if let Some(typing) = after.as_ref().filter(|_| !hits.is_empty()) {
                emoji_view::hold_keys(ui, id);
                let colon = edit
                    .galley
                    .pos_from_cursor(CCursor::new(text[..typing.query.start].chars().count()));
                let x = (edit.galley_pos.x + colon.min.x)
                    .min(boxed.right() - emoji_view::WIDTH)
                    .max(boxed.left());
                let picked = emoji_menu.picked_in(hits.len());
                if let Some(at) = emoji_view::show(ui, id, pos2(x, boxed.top()), &hits, picked) {
                    picked_into(ui, id, text, typing, &hits[at]);
                    ui.memory_mut(|memory| memory.request_focus(id));
                }
            }
        }
        if !slash_rows.is_empty() {
            if ui.memory(|memory| memory.has_focus(id)) {
                slash_view::hold_keys(ui, id);
            }
            let picked = slash.picked_in(slash_rows.len());
            if let Some(at) = slash_view::show(ui, id, boxed, &slash_rows, picked) {
                slash_view::complete_with_click(ui, id, text, slash_rows[at]);
            }
        }
        if (enter || clicked) && can_send {
            sent = Some(crate::pictures::message(text, pictures));
            text.clear();
            pictures.clear();
            *focus = true;
        }
    });
    slash_view::store(ui, id, slash);
    ui.data_mut(|data| data.insert_temp(menu_id(id), emoji_menu));
    sent
}

#[cfg(test)]
mod tests {
    use egui::epaint::ClippedShape;
    use egui::{Context, Event, RawInput};

    use super::*;

    const COLUMN_W: f32 = 600.0;
    const COLUMN_H: f32 = 700.0;
    const HINT: &str = "Reply to the agent";

    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn typed(text: &str) -> Event {
        Event::Text(text.into())
    }

    fn flat(shape: &Shape, out: &mut Vec<Shape>) {
        match shape {
            Shape::Vec(inner) => inner.iter().for_each(|shape| flat(shape, out)),
            other => out.push(other.clone()),
        }
    }

    struct Seen {
        sent: Option<String>,
        shapes: Vec<Shape>,
        clips: Vec<(Shape, Rect)>,
        box_rect: Rect,
    }

    impl Seen {
        fn text_rows(&self, wanted: &str) -> Vec<Rect> {
            self.shapes
                .iter()
                .filter_map(|shape| match shape {
                    Shape::Text(t) if t.galley.text() == wanted => {
                        Some(t.galley.rows.iter().map(|row| row.rect().translate(t.pos.to_vec2())))
                    }
                    _ => None,
                })
                .flatten()
                .collect()
        }

        fn last_row_and_clip(&self, wanted: &str) -> (Rect, Rect) {
            self.clips
                .iter()
                .find_map(|(shape, clip)| match shape {
                    Shape::Text(t) if t.galley.text() == wanted => {
                        let row = t.galley.rows.last()?.rect().translate(t.pos.to_vec2());
                        Some((row, *clip))
                    }
                    _ => None,
                })
                .expect("the text was painted")
        }

        fn rects(&self, fill: egui::Color32) -> Vec<RectShape> {
            self.shapes
                .iter()
                .filter_map(|shape| match shape {
                    Shape::Rect(rect) if rect.fill == fill => Some(rect.clone()),
                    _ => None,
                })
                .collect()
        }

        fn gaps(&self, wanted: &str) -> (f32, f32) {
            let rows = self.text_rows(wanted);
            let (first, last) = (rows.first().unwrap(), rows.last().unwrap());
            (first.top() - self.box_rect.top(), self.box_rect.bottom() - last.bottom())
        }
    }

    struct Rig {
        ctx: Context,
        text: String,
        pictures: Vec<PathBuf>,
        focus: bool,
        ghost: Option<&'static str>,
        hovered: bool,
    }

    impl Rig {
        fn new() -> Self {
            let ctx = Context::default();
            crate::fonts::install(&ctx);
            let mut rig = Self {
                ctx,
                text: String::new(),
                pictures: Vec::new(),
                focus: true,
                ghost: None,
                hovered: false,
            };
            rig.frame(vec![]);
            rig.frame(vec![]);
            rig
        }

        fn frame(&mut self, events: Vec<Event>) -> Seen {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(COLUMN_W, COLUMN_H))),
                events,
                focused: true,
                hovered_files: if self.hovered {
                    vec![egui::HoveredFile { path: Some("/x/a.png".into()), ..Default::default() }]
                } else {
                    Vec::new()
                },
                ..RawInput::default()
            };
            let (mut sent, mut input) = (None, Rect::NOTHING);
            let out = self.ctx.run_ui(raw, |ui| {
                let column = Rect::from_min_size(pos2(0.0, 0.0), vec2(COLUMN_W, COLUMN_H));
                let h = height(ui, &self.text, self.pictures.len(), column);
                input = Rect::from_min_max(pos2(0.0, COLUMN_H - h), column.max);
                let room = input.shrink2(vec2(INSET, GAP_Y));
                let mut child = ui.new_child(UiBuilder::new().max_rect(room));
                let draft = Draft {
                    text: &mut self.text,
                    pictures: &mut self.pictures,
                    focus: &mut self.focus,
                };
                let spec = Spec {
                    id: Id::new("reply-box"),
                    hint: HINT,
                    enabled: true,
                    commands: &[],
                    ghost: self.ghost,
                };
                sent = show(&mut child, draft, &spec);
            });
            let mut shapes = Vec::new();
            let mut clips = Vec::new();
            out.shapes.iter().for_each(|clipped: &ClippedShape| {
                let before = shapes.len();
                flat(&clipped.shape, &mut shapes);
                clips.extend(shapes[before..].iter().cloned().map(|s| (s, clipped.clip_rect)));
            });
            let box_rect = input.shrink2(vec2(INSET, GAP_Y));
            let boxed = shapes
                .iter()
                .find_map(|shape| match shape {
                    Shape::Rect(r) if r.fill == brand::CARD => Some(r.rect),
                    _ => None,
                })
                .unwrap_or(box_rect);
            Seen { sent, shapes, clips, box_rect: boxed }
        }

        fn settle(&mut self, events: Vec<Event>) -> Seen {
            self.frame(events);
            self.frame(vec![])
        }

        fn type_text(&mut self, text: &str) -> Seen {
            self.settle(vec![typed(text)])
        }

        fn press(&mut self, pressed: Key) -> Seen {
            self.settle(vec![key(pressed)])
        }

        fn click(&mut self, at: egui::Pos2) -> Seen {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(vec![])
        }
    }

    fn rings(seen: &Seen) -> Vec<f32> {
        seen.shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Rect(rect)
                    if rect.fill == egui::Color32::TRANSPARENT && rect.stroke.width >= 3.0 =>
                {
                    Some(rect.stroke.width)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn files_over_the_window_light_the_box_with_a_stronger_ring_than_focus_does() {
        let mut rig = Rig::new();
        assert_eq!(rings(&rig.frame(vec![])), [FOCUS_RING.0]);
        rig.hovered = true;
        assert_eq!(rings(&rig.frame(vec![])), [DROP_RING.0]);
        rig.focus = false;
        rig.ctx.memory_mut(|memory| memory.surrender_focus(Id::new("reply-box")));
        assert_eq!(rings(&rig.frame(vec![])), [DROP_RING.0], "lit even when the box has no focus");
        rig.hovered = false;
        assert_eq!(rings(&rig.frame(vec![])).len(), 0, "dark once the drag leaves");
    }

    #[test]
    fn a_box_is_48_tall_for_one_row_and_taller_by_whole_rows_up_to_its_cap() {
        let row = 18.0;
        let cases = [
            (row, row, 0, 280.0, BOX_H),
            (row, 3.0 * row, 0, 280.0, BOX_H + 2.0 * row),
            (row, 100.0 * row, 0, 280.0, 280.0),
            (row, 100.0 * row, 0, 20.0, BOX_H),
            (row, row, 1, 280.0, 2.0 + 10.0 + 48.0 + 8.0 + row + 14.0),
        ];
        for (row_h, text_h, chips, cap, want) in cases {
            let got = fit(row_h, text_h, chips, cap);
            assert_eq!(got.box_h, want, "{row_h} {text_h} {chips} {cap}");
        }
        assert_eq!(fit(18.0, 18.0, 0, 280.0).pad_y, 14.0);
        assert!(fit(18.0, 3600.0, 0, 280.0).scrolls);
        assert!(!fit(18.0, 36.0, 0, 280.0).scrolls);
    }

    #[test]
    fn one_row_is_a_48_px_box_with_the_same_gap_above_and_below_the_text() {
        let mut rig = Rig::new();
        let seen = rig.type_text("hello");
        assert_eq!(seen.box_rect.height(), BOX_H);
        let (above, below) = seen.gaps("hello");
        assert!((above - below).abs() <= 1.0, "above {above}, below {below}");
        assert!(above > 10.0, "{above}");
    }

    #[test]
    fn three_rows_have_the_same_gap_above_and_below_the_text() {
        let mut rig = Rig::new();
        let seen = rig.type_text("one\ntwo\nthree");
        let (above, below) = seen.gaps("one\ntwo\nthree");
        assert!((above - below).abs() <= 1.0, "above {above}, below {below}");
        let rows = seen.text_rows("one\ntwo\nthree").len();
        assert_eq!(rows, 3);
        assert!(seen.box_rect.height() > BOX_H + 30.0);
    }

    #[test]
    fn the_placeholder_starts_exactly_where_typed_text_starts() {
        let mut rig = Rig::new();
        let empty = rig.frame(vec![]);
        let hint = empty.text_rows(HINT)[0];
        let seen = rig.type_text("x");
        let first = seen.text_rows("x")[0];
        assert!((hint.min.x - first.min.x).abs() < 0.5, "x {hint:?} {first:?}");
        assert!((hint.min.y - first.min.y).abs() < 0.5, "y {hint:?} {first:?}");
        assert!((hint.height() - first.height()).abs() < 0.5, "same size {hint:?} {first:?}");
        let (above, below) = empty.gaps(HINT);
        assert!((above - below).abs() <= 1.0, "above {above}, below {below}");
    }

    #[test]
    fn send_is_centred_on_a_single_line_and_at_the_bottom_right_of_several() {
        let mut rig = Rig::new();
        let one = rig.type_text("hello");
        let send = one.text_rows("Send")[0];
        assert!((send.center().y - one.box_rect.center().y).abs() < 0.6, "{send:?}");
        let gap_right = one.box_rect.right() - send.right();
        assert!(gap_right > 10.0 && gap_right < 30.0, "{gap_right}");
        let several = rig.type_text("\nmore\nlines");
        let send = several.text_rows("Send")[0];
        let last = several.text_rows("hello\nmore\nlines").pop().unwrap();
        assert!((send.center().y - last.center().y).abs() < 1.0, "{send:?} {last:?}");
        assert!(several.box_rect.bottom() - send.bottom() < 20.0);
    }

    #[test]
    fn the_box_has_a_1_px_border_14_px_corners_and_a_teal_border_only_while_focused() {
        let mut rig = Rig::new();
        let focused = rig.type_text("hello");
        let shape = focused.rects(brand::CARD).remove(0);
        assert_eq!(shape.stroke.width, 1.0);
        assert_eq!(shape.stroke.color, brand::TEAL);
        assert_eq!(shape.corner_radius, CornerRadius::same(14));
        rig.focus = false;
        rig.ctx.memory_mut(|memory| memory.surrender_focus(Id::new("reply-box")));
        let blurred = rig.frame(vec![]);
        assert_eq!(blurred.rects(brand::CARD).remove(0).stroke.color, brand::FIELD_LINE);
    }

    #[test]
    fn sixty_rows_grow_the_box_to_40_percent_of_the_column_and_then_scroll_inside_it() {
        let mut rig = Rig::new();
        let mut tallest: f32 = 0.0;
        for line in 1..=60 {
            let seen = rig.settle(vec![
                typed(&format!("line {line}")),
                Event::Key {
                    key: Key::Enter,
                    physical_key: Some(Key::Enter),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::SHIFT,
                },
            ]);
            tallest = tallest.max(seen.box_rect.height());
            assert!(seen.box_rect.height() <= COLUMN_H * SHARE + 0.5, "line {line}");
        }
        assert!((tallest - COLUMN_H * SHARE).abs() < 0.5, "{tallest}");
        assert!(rig.text.ends_with("line 60\n"));
        let seen = rig.type_text("line 61");
        let (row, clip) = seen.last_row_and_clip(&rig.text);
        assert!(clip.contains_rect(row.shrink(0.5)), "cursor row {row:?} outside visible {clip:?}");
        assert!(seen.box_rect.contains_rect(clip), "{clip:?} outside {:?}", seen.box_rect);
    }

    fn rig_with_two_pictures() -> Rig {
        let mut rig = Rig::new();
        rig.pictures = vec![
            PathBuf::from("/nope/shot-1793011211356.png"),
            PathBuf::from("/nope/shot-1793011218705.png"),
        ];
        rig.frame(vec![]);
        rig
    }

    #[test]
    fn pictures_are_chips_8_px_above_the_text_and_the_text_stays_clean() {
        let mut rig = rig_with_two_pictures();
        let seen = rig.type_text("look at these");
        let chips = seen.rects(brand::ITEM);
        assert_eq!(chips.len(), 2);
        assert_eq!(chips[0].rect.size(), vec2(pictures::REMOVABLE_W, pictures::CHIP_H));
        let text = seen.text_rows("look at these")[0];
        assert!((text.top() - chips[0].rect.bottom() - 8.0).abs() < 1.0, "{text:?}");
        assert_eq!(rig.text, "look at these");
        let plain = Rig::new().type_text("look at these");
        assert!(seen.box_rect.height() > plain.box_rect.height() + 40.0);
    }

    #[test]
    fn backspace_at_the_start_of_the_text_removes_the_last_chip_and_elsewhere_deletes_a_letter() {
        let mut rig = rig_with_two_pictures();
        rig.type_text("ab");
        rig.press(Key::Backspace);
        assert_eq!((rig.text.as_str(), rig.pictures.len()), ("a", 2));
        rig.press(Key::Backspace);
        assert_eq!((rig.text.as_str(), rig.pictures.len()), ("", 2));
        rig.press(Key::Backspace);
        assert_eq!(rig.pictures, [PathBuf::from("/nope/shot-1793011211356.png")]);
        rig.press(Key::Backspace);
        assert_eq!(rig.pictures, Vec::<PathBuf>::new());
        rig.press(Key::Backspace);
        assert_eq!(rig.text, "");
    }

    #[test]
    fn sending_appends_the_paths_after_the_text_and_clears_the_chips() {
        let mut rig = rig_with_two_pictures();
        rig.type_text("look at these");
        let sent = rig.frame(vec![key(Key::Enter)]).sent;
        assert_eq!(
            sent.as_deref(),
            Some("look at these\n/nope/shot-1793011211356.png\n/nope/shot-1793011218705.png")
        );
        assert!(rig.pictures.is_empty() && rig.text.is_empty());
    }

    #[test]
    fn a_picture_alone_can_be_sent_and_an_empty_box_cannot() {
        let mut rig = Rig::new();
        assert_eq!(rig.frame(vec![key(Key::Enter)]).sent, None);
        rig.pictures = vec![PathBuf::from("/nope/shot-1.png")];
        rig.frame(vec![]);
        let sent = rig.frame(vec![key(Key::Enter)]).sent;
        assert_eq!(sent.as_deref(), Some("/nope/shot-1.png"));
    }

    #[test]
    fn the_remove_button_of_a_chip_takes_that_chip_away() {
        let mut rig = rig_with_two_pictures();
        let seen = rig.frame(vec![]);
        let first = seen.rects(brand::ITEM)[0].rect;
        let button = egui::pos2(first.right() - 8.0 - 13.0, first.center().y);
        rig.click(button);
        assert_eq!(rig.pictures, [PathBuf::from("/nope/shot-1793011218705.png")]);
    }

    fn emoji_box(typed_text: &str) -> Rig {
        let mut rig = Rig::new();
        rig.type_text(typed_text);
        rig
    }

    #[test]
    fn tab_puts_the_best_match_in_the_text_and_sends_nothing() {
        let mut rig = emoji_box("Nice work :smi");
        let seen = rig.frame(vec![key(Key::Tab)]);
        assert_eq!(seen.sent, None);
        assert_eq!(rig.text, "Nice work \u{1f604}");
        rig.frame(vec![]);
        assert!(rig.ctx.memory(|memory| memory.has_focus(Id::new("reply-box"))));
    }

    #[test]
    fn enter_with_the_list_open_picks_and_does_not_send() {
        let mut rig = emoji_box(":smi");
        let seen = rig.frame(vec![key(Key::Enter)]);
        assert_eq!(seen.sent, None);
        assert_eq!(rig.text, "\u{1f604}");
        let sent = rig.frame(vec![key(Key::Enter)]).sent;
        assert_eq!(sent.as_deref(), Some("\u{1f604}"), "the next Enter sends");
    }

    #[test]
    fn down_moves_the_pick_and_up_moves_it_back() {
        let mut rig = emoji_box(":smi");
        rig.press(Key::ArrowDown);
        rig.press(Key::Tab);
        assert_eq!(rig.text, "\u{1f60f}");
        let mut rig = emoji_box(":smi");
        rig.press(Key::ArrowUp);
        rig.press(Key::ArrowDown);
        rig.press(Key::Tab);
        assert_eq!(rig.text, "\u{1f604}");
    }

    #[test]
    fn esc_closes_the_list_keeps_the_text_and_the_keyboard_and_enter_then_sends() {
        let mut rig = emoji_box(":smi");
        rig.press(Key::Escape);
        assert_eq!(rig.text, ":smi");
        assert!(rig.ctx.memory(|memory| memory.has_focus(Id::new("reply-box"))));
        let sent = rig.frame(vec![key(Key::Enter)]).sent;
        assert_eq!(sent.as_deref(), Some(":smi"));
    }

    #[test]
    fn a_whole_name_and_a_space_turn_into_the_emoji_without_the_list() {
        let mut rig = Rig::new();
        rig.type_text(":tada:");
        rig.type_text(" ");
        assert_eq!(rig.text, "\u{1f389} ");
        rig.type_text("ok");
        assert_eq!(rig.text, "\u{1f389} ok");
    }

    #[test]
    fn a_picked_emoji_rises_in_the_next_list() {
        let mut rig = emoji_box(":smi");
        rig.press(Key::ArrowDown);
        rig.press(Key::ArrowDown);
        rig.press(Key::Tab);
        assert_eq!(rig.text, "\u{1f603}");
        rig.text.clear();
        rig.type_text(":smi");
        rig.press(Key::Tab);
        assert_eq!(rig.text, "\u{1f603}");
    }

    #[test]
    fn tab_takes_the_suggestion_only_in_an_empty_box_with_the_list_closed() {
        let mut rig = Rig::new();
        rig.ghost = Some("run the tests");
        rig.frame(vec![]);
        rig.press(Key::Tab);
        assert_eq!(rig.text, "run the tests");
        let mut rig = Rig::new();
        rig.ghost = Some("run the tests");
        rig.type_text(":smi");
        rig.press(Key::Tab);
        assert_eq!(rig.text, "\u{1f604}");
    }

    fn teal_run(seen: &Seen, typed: &str) -> Vec<String> {
        let galleys = seen.shapes.iter().filter_map(|shape| match shape {
            Shape::Text(t) if t.galley.text() == typed => Some(t.galley.clone()),
            _ => None,
        });
        galleys
            .flat_map(|galley| {
                let job = &galley.job;
                job.sections
                    .iter()
                    .filter(|section| section.format.color == brand::TEAL_TEXT)
                    .map(|section| {
                        job.text[section.byte_range.start.0..section.byte_range.end.0].to_owned()
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn the_name_being_typed_is_teal_while_the_list_is_open_and_plain_after_esc() {
        let mut rig = emoji_box("Nice work :smi");
        assert_eq!(teal_run(&rig.frame(vec![]), "Nice work :smi"), [":smi"]);
        rig.press(Key::Escape);
        assert_eq!(teal_run(&rig.frame(vec![]), "Nice work :smi"), Vec::<String>::new());
        let mut plain = emoji_box("see 10:30");
        assert_eq!(teal_run(&plain.frame(vec![]), "see 10:30"), Vec::<String>::new());
    }

    #[test]
    fn a_colon_in_ordinary_text_opens_nothing_and_enter_still_sends() {
        let mut rig = emoji_box("see 10:30 or http://x");
        let sent = rig.frame(vec![key(Key::Enter)]).sent;
        assert_eq!(sent.as_deref(), Some("see 10:30 or http://x"));
    }
}
