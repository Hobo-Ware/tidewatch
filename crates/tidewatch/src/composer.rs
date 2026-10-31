//! The message box of the Reader, of an agent tab's first message and of the new-tab form: one widget.
//! A single line is a 48 px box with the text centred, the placeholder where typed text starts and Send
//! centred on the line; it grows with its text up to 40% of the column, then scrolls inside. A picture or
//! file is a chip inside the sentence, `@` opens the repo's files, `:` and two letters open the emoji
//! list, a slash opens the command list. In the form it is a field of a fixed height with no Send,
//! where Enter is a new line.

use egui::epaint::RectShape;
use egui::text::{CCursor, CCursorRange};
use egui::{
    CornerRadius, EventFilter, FontId, Frame, Id, Key, KeyboardShortcut, Modifiers, Rect, RichText,
    ScrollArea, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2,
};

use crate::at_files;
use crate::at_files_view;
use crate::brand;
use crate::bullets;
use crate::emoji_paint;
use crate::emoji_pick::{self, Hit, MAX_ROWS, Query, Usage};
use crate::emoji_view;
use crate::reader_chat_view::BODY_PX;
use crate::reader_view::INSET;
use crate::recall::Recall;
use crate::screen_kit::{Size, button_width, primary_button, quiet_button};
use crate::slash::Entry as SlashEntry;
use crate::slash_view;
use crate::token_view;
use crate::tokens::{self, Target};

pub(crate) const BOX_H: f32 = 48.0;
pub(crate) const GAP_Y: f32 = 8.0;
pub(crate) const SHARE: f32 = 0.4;
const BORDER: f32 = 1.0;
const RADIUS: u8 = 14;
const PAD_X: f32 = 18.0;
const SEND_GAP: f32 = 12.0;
const SEND: &str = "Send";
const STOP: &str = "Stop";
const STOP_GAP: f32 = 8.0;
const OPTICAL_LIFT: f32 = 0.4;
const FOCUS_RING: (f32, f32) = (3.0, 0.10);
const DROP_RING: (f32, f32) = (6.0, 0.45);
const DROP_FILL: f32 = 0.12;
const CARET_W: f32 = 2.0;

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
    pub(crate) scrolls: bool,
}

pub(crate) fn fit(row_h: f32, text_h: f32, cap: f32) -> Fit {
    let pad_y = ((BOX_H - 2.0 * BORDER - row_h) / 2.0).max(0.0);
    let around = 2.0 * BORDER + 2.0 * pad_y;
    let wanted = around + text_h;
    let least = (around + row_h).max(BOX_H);
    let box_h = wanted.min(cap.max(BOX_H)).max(least);
    Fit { box_h, pad_y, scrolls: wanted > box_h + 0.5 }
}

/// What the box is for. A reply sends on Enter and grows with its text; a field belongs to a form
/// that sends on its own key, so it has no Send, Enter is a new line and the box is `height` tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mode {
    Reply,
    Field { height: f32 },
}

impl Mode {
    fn sends(self) -> bool {
        self == Self::Reply
    }
}

struct Measure {
    fit: Fit,
    send_w: f32,
    stop_w: f32,
}

fn row_height(ui: &Ui) -> f32 {
    ui.painter().layout_no_wrap(String::new(), text_font(), brand::TEXT).size().y
}

fn measure(ui: &Ui, text: &str, box_w: f32, (cap, mode, stop): (f32, Mode, bool)) -> Measure {
    let send_w = if mode.sends() { button_width(ui, SEND, Size::Regular) } else { 0.0 };
    let stop_w =
        if mode.sends() && stop { button_width(ui, STOP, Size::Regular) + STOP_GAP } else { 0.0 };
    let right = if mode.sends() { send_edge() + send_w + stop_w + SEND_GAP } else { PAD_X };
    let wrap = box_w - PAD_X - right;
    let typed = bullets::layout(ui, text, &text_font(), brand::TEXT, (wrap, &None));
    let cap = match mode {
        Mode::Reply => cap,
        Mode::Field { height } => height,
    };
    let mut fit = fit(row_height(ui), typed.size().y, cap);
    if let Mode::Field { height } = mode {
        fit.box_h = fit.box_h.max(height);
    }
    Measure { fit, send_w, stop_w }
}

/// The height a box takes in `column`, with the gap above and below it.
pub(crate) fn height(ui: &Ui, text: &str, column: Rect) -> f32 {
    height_with_stop(ui, text, column, false)
}

/// [`height`] for a box that also holds a Stop button, which narrows its text.
pub(crate) fn height_with_stop(ui: &Ui, text: &str, column: Rect, stop: bool) -> f32 {
    let cap = (column.height() * SHARE).max(BOX_H);
    measure(ui, text, column.width() - 2.0 * INSET, (cap, Mode::Reply, stop)).fit.box_h
        + 2.0 * GAP_Y
}

/// The height a field takes in a column `width` wide when it is asked for `height`.
pub(crate) fn field_height(ui: &Ui, text: &str, (width, height): (f32, f32)) -> f32 {
    measure(ui, text, width, (height, Mode::Field { height }, false)).fit.box_h
}

pub(crate) struct Draft<'a> {
    pub(crate) text: &'a mut String,
    pub(crate) focus: &'a mut bool,
    pub(crate) recall: Option<&'a mut Recall>,
}

/// What the box asked for this frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) sent: Option<String>,
    /// The Stop button, or Esc in an empty box, while the agent works.
    pub(crate) stop: bool,
    /// Up in an empty box while messages are held: the newest one is to be edited.
    pub(crate) edit_held: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Spec<'a> {
    pub(crate) id: Id,
    pub(crate) hint: &'a str,
    pub(crate) enabled: bool,
    pub(crate) commands: &'a [SlashEntry],
    pub(crate) ghost: Option<&'a str>,
    pub(crate) mode: Mode,
    /// A Stop button sits beside Send: the agent is working.
    pub(crate) stop: bool,
    /// How many of the owner's messages Tidewatch holds for this agent.
    pub(crate) held: usize,
    /// The files an `@` offers; none in a box with no folder behind it.
    pub(crate) files: Option<at_files::Source<'a>>,
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

fn list_tab(ui: &Ui, id: Id, text: &mut String, enabled: bool) {
    let focused = enabled && ui.memory(|memory| memory.has_focus(id));
    let Some(cursor) =
        collapsed_cursor(ui, id).filter(|at| focused && bullets::on_list_line(text, *at))
    else {
        return;
    };
    let way = if ui.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::Tab)) {
        bullets::Shift::Out
    } else if consume(ui, Key::Tab) {
        bullets::Shift::In
    } else {
        return;
    };
    if let Some(at) = bullets::shift(text, cursor, way) {
        bullets::place_cursor(ui, id, at);
    }
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

fn at_typing(text: &str, cursor: Option<usize>) -> Option<Typing> {
    let cursor = cursor?;
    at_files::query_at(text, byte_of(text, cursor)).map(|query| Typing { query, cursor })
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

fn token_keys(ui: &Ui, id: Id, text: &mut String) {
    let Some(cursor) = collapsed_cursor(ui, id).filter(|_| tokens::has_tokens(text)) else {
        return;
    };
    let tokens = tokens::find(text);
    let behind = tokens.iter().any(|token| token.chars.end == cursor);
    let ahead = tokens.iter().any(|token| token.chars.start == cursor);
    let moved = if behind && consume(ui, Key::Backspace) {
        tokens::remove_before(text, cursor)
    } else if ahead && consume(ui, Key::Delete) {
        tokens::remove_after(text, cursor)
    } else {
        None
    };
    if let Some(at) = moved {
        bullets::place_cursor(ui, id, at);
    }
}

fn keep_cursor_out_of_tokens(ui: &Ui, id: Id, text: &str, (range, was): (CCursorRange, usize)) {
    if !tokens::has_tokens(text) {
        return;
    }
    let primary = range.primary.index.0;
    let snapped = tokens::snap(text, primary, was);
    if snapped == primary {
        return;
    }
    let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(range.secondary, CCursor::new(snapped))));
    state.store(ui.ctx(), id);
}

/// Puts `targets` into `text` at the cursor of the box `id` when it has the keyboard (or no window
/// does), else at the end, one after another, and leaves the cursor behind the last one.
pub(crate) fn insert_targets(ctx: &egui::Context, id: Id, text: &mut String, targets: &[Target]) {
    let focused = ctx.memory(|memory| memory.has_focus(id)) || !ctx.input(|input| input.focused);
    let end = text.chars().count();
    let stored = TextEdit::load_state(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index.0);
    let mut at = stored.filter(|_| focused).map_or(end, |at| at.min(end));
    for target in targets {
        at = tokens::insert(text, at, target);
    }
    let mut state = TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
    state.store(ctx, id);
    ctx.request_repaint();
}

/// Draws the box in the room `ui` has and works it. Enter sends a non-empty text and clears it, Cmd+Enter sends from anywhere in the box, Shift+Enter starts a new line, and on a
/// list line Enter starts the next item instead while Tab and Shift+Tab move the item a level. Asks for focus once, the first time it is drawn, and again after a send. Off while
/// `spec.enabled` is false. A [`Mode::Field`] box has no Send and sends nothing: Enter is a new line and
/// Cmd+Enter is left to the form.
pub(crate) fn show(ui: &mut Ui, draft: Draft<'_>, spec: &Spec<'_>) -> Outcome {
    let Draft { text, focus, mut recall } = draft;
    let Spec { id, hint, enabled, commands, ghost, mode, stop, held, files } = *spec;
    let room = ui.max_rect();
    let mut outcome = Outcome::default();
    let mut slash = slash_view::load(ui, id);
    let slash_keys_rows = if enabled { slash.rows(text, commands) } else { Vec::new() };
    if enabled {
        slash_view::keys(ui, id, text, &mut slash, &slash_keys_rows);
    }
    let mut emoji_menu: emoji_pick::Menu =
        ui.data(|data| data.get_temp(menu_id(id))).unwrap_or_default();
    let focused = ui.memory(|memory| memory.has_focus(id));
    let cursor_was = collapsed_cursor(ui, id);
    let mut at_menu = at_files_view::load(ui, id);
    let at_before = files
        .and_then(|_| at_typing(text, collapsed_cursor(ui, id)))
        .filter(|_| slash_keys_rows.is_empty());
    at_menu.follow(at_before.as_ref().map(|typing| &typing.query));
    let before = typing_at(text, collapsed_cursor(ui, id))
        .filter(|_| slash_keys_rows.is_empty() && at_before.is_none());
    emoji_menu.follow(before.as_ref().map(|typing| &typing.query));
    let at_rows_before = at_files_view::hits(
        ui,
        id,
        &at_menu,
        (at_before.as_ref().map(|typing| &typing.query), files),
    );
    if enabled && focused {
        let hits = emoji_hits(ui, &emoji_menu, before.as_ref());
        if let Some(typing) = before.as_ref().filter(|_| !hits.is_empty()) {
            emoji_keys(ui, id, text, &mut emoji_menu, (&hits, typing));
        }
        if let (Some(typing), Some(files)) = (at_before.as_ref(), files)
            && !at_rows_before.is_empty()
            && let Some(at) = at_files_view::keys(
                ui,
                text,
                &mut at_menu,
                (&at_rows_before, &typing.query, files.root),
                typing.cursor,
            )
        {
            bullets::place_cursor(ui, id, at);
        }
        token_keys(ui, id, text);
    }
    let at_tint = at_before.as_ref().filter(|_| enabled && focused && !at_rows_before.is_empty());
    let tint: bullets::Tint = at_tint
        .or(before.as_ref().filter(|_| {
            enabled && focused && !emoji_hits(ui, &emoji_menu, before.as_ref()).is_empty()
        }))
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
    list_tab(ui, id, text, enabled);
    let menus_open = !slash_keys_rows.is_empty()
        || (focused
            && (!emoji_hits(ui, &emoji_menu, before.as_ref()).is_empty()
                || !at_rows_before.is_empty()));
    if enabled && focused && mode.sends() && !menus_open {
        history_keys(ui, id, text, recall.as_deref_mut(), &mut outcome, (stop, held));
    }
    let listing = enabled && (ui.memory(|memory| memory.has_focus(id)) || !ui.input(|i| i.focused));
    let slash_rows = if listing { slash.rows(text, commands) } else { Vec::new() };
    slash.open = !slash_rows.is_empty();
    let command_enter = enabled
        && mode.sends()
        && ui.memory(|memory| memory.has_focus(id))
        && ui.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Enter));
    let mut enter = enabled
        && mode.sends()
        && ui.memory(|memory| memory.has_focus(id))
        && ui.input(|input| input.events.iter().any(crate::agent_ready::plain_enter));
    if enter && let Some(cursor) = collapsed_cursor(ui, id) {
        let (outcome, at) = bullets::enter(text, cursor);
        if outcome != bullets::Enter::Plain {
            bullets::place_cursor(ui, id, at);
            enter = false;
        }
    }
    let enter = enter || command_enter;
    ui.add_enabled_ui(enabled, |ui| {
        let measured = measure(ui, text, room.width(), (room.height(), mode, stop));
        let Fit { box_h, pad_y, .. } = measured.fit;
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
        let stop_button = Rect::from_min_size(
            pos2(send.left() - measured.stop_w, send.top()),
            vec2(measured.stop_w - STOP_GAP, Size::Regular.height()),
        );
        let view = Rect::from_min_max(
            pos2(boxed.left() + PAD_X, boxed.top() + BORDER),
            pos2(
                if mode.sends() {
                    send.left() - measured.stop_w - SEND_GAP
                } else {
                    boxed.right() - PAD_X
                },
                boxed.bottom() - BORDER,
            ),
        );
        let shown = view.height();
        let height_id = id.with("height");
        let before = ui.data(|data| data.get_temp::<f32>(height_id));
        ui.data_mut(|data| data.insert_temp(height_id, shown));
        let resized = before.is_some_and(|before| (before - shown).abs() > 0.5);
        let top_pad = pad_y - OPTICAL_LIFT;
        let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            bullets::layout(ui, buf.as_str(), &text_font(), brand::TEXT, (wrap, &tint))
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
                        .return_key(KeyboardShortcut::new(
                            if mode.sends() { Modifiers::SHIFT } else { Modifiers::NONE },
                            Key::Enter,
                        ))
                        .show(ui);
                    bullets::paint_markers(ui, &edit.galley, edit.galley_pos, &text_font());
                    ui.add_space(pad_y + OPTICAL_LIFT);
                    emoji_paint::paint(ui, id.with("emoji"), edit.galley_pos, &edit.galley);
                    token_view::paint(
                        ui,
                        id.with("tokens"),
                        (edit.galley_pos, &edit.galley),
                        false,
                    );
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
        let can_send = mode.sends() && !text.trim().is_empty();
        let mut clicked = false;
        if mode.sends() {
            ui.scope_builder(UiBuilder::new().max_rect(send), |ui| {
                clicked = primary_button(ui, SEND, can_send, Size::Regular).clicked();
            });
        }
        if mode.sends() && stop {
            ui.scope_builder(UiBuilder::new().max_rect(stop_button), |ui| {
                if quiet_button(ui, STOP, true, Size::Regular)
                    .on_hover_text("Stop the agent's turn (Esc in an empty box)")
                    .clicked()
                {
                    outcome.stop = true;
                }
            });
        }
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
        let on_list = output
            .as_ref()
            .and_then(|edit| edit.cursor_range)
            .filter(|range| range.primary == range.secondary)
            .is_some_and(|range| bullets::on_list_line(text, range.primary.index.0));
        let walking = recall.as_deref().is_some_and(Recall::walking);
        let wants_escape = mode.sends() && (walking || (stop && text.is_empty()));
        if (ghost.is_some() || on_list || walking || wants_escape) && has_focus {
            let filter = EventFilter {
                tab: ghost.is_some() || on_list || walking,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: wants_escape,
            };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
        }
        if let Some(edit) = output {
            if dropping
                && has_focus
                && let Some(range) = edit.cursor_range
            {
                let at = edit.galley.pos_from_cursor(range.primary);
                let x = edit.galley_pos.x + at.min.x;
                let top = edit.galley_pos.y + at.min.y;
                ui.painter().line_segment(
                    [pos2(x, top), pos2(x, top + at.height())],
                    Stroke::new(CARET_W, brand::TEAL),
                );
            }
            if let Some(range) = edit.cursor_range {
                keep_cursor_out_of_tokens(
                    ui,
                    id,
                    text,
                    (range, cursor_was.unwrap_or(range.primary.index.0)),
                );
            }
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
            let listing_now = slash_rows.is_empty() && enabled && has_focus;
            let at_after = files.and_then(|_| at_typing(text, cursor)).filter(|_| listing_now);
            at_menu.follow(at_after.as_ref().map(|typing| &typing.query));
            let at_rows = at_files_view::hits(
                ui,
                id,
                &at_menu,
                (at_after.as_ref().map(|typing| &typing.query), files),
            );
            if let (Some(typing), Some(files)) = (at_after.as_ref(), files)
                && !at_rows.is_empty()
            {
                emoji_view::hold_keys(ui, id);
                let mark = edit
                    .galley
                    .pos_from_cursor(CCursor::new(text[..typing.query.start].chars().count()));
                let x = (edit.galley_pos.x + mark.min.x)
                    .min(boxed.right() - at_files_view::WIDTH)
                    .max(boxed.left());
                let picked = at_menu.picked_in(at_rows.len());
                if let Some(at) =
                    at_files_view::show(ui, id, pos2(x, boxed.top()), &at_rows, picked)
                {
                    let moved = at_files_view::pick(
                        text,
                        &typing.query,
                        typing.cursor,
                        files.root,
                        &at_rows[at],
                    );
                    bullets::place_cursor(ui, id, moved);
                    ui.memory_mut(|memory| memory.request_focus(id));
                }
            }
            let after = typing_at(text, cursor).filter(|_| listing_now && at_after.is_none());
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
            outcome.sent = Some(tokens::sendable(text));
            text.clear();
            if let Some(recall) = recall {
                recall.sent(text);
            }
            *focus = true;
        }
    });
    slash_view::store(ui, id, slash);
    at_files_view::store(ui, id, at_menu);
    ui.data_mut(|data| data.insert_temp(menu_id(id), emoji_menu));
    outcome
}

fn on_first_row(text: &str, cursor: Option<usize>) -> bool {
    cursor.is_none_or(|at| !text[..byte_of(text, at)].contains('\n'))
}

fn on_last_row(text: &str, cursor: Option<usize>) -> bool {
    cursor.is_none_or(|at| !text[byte_of(text, at)..].contains('\n'))
}

/// Up, Down and Esc of a focused reply box: Up in an empty box edits the newest held message, or
/// else recalls the messages sent before; Down and Esc walk back; Esc in an empty box stops a
/// working agent. A key is used up only when one of these takes it, so every other press reaches
/// the text.
fn history_keys(
    ui: &Ui,
    id: Id,
    text: &mut String,
    recall: Option<&mut Recall>,
    outcome: &mut Outcome,
    (stop, held): (bool, usize),
) {
    let cursor = collapsed_cursor(ui, id);
    let blank = text.is_empty();
    let walking = recall.as_deref().is_some_and(Recall::walking);
    if blank && held > 0 && !walking && consume(ui, Key::ArrowUp) {
        outcome.edit_held = true;
        return;
    }
    if let Some(recall) = recall {
        let walked =
            if recall.claims_up(text) && on_first_row(text, cursor) && consume(ui, Key::ArrowUp) {
                recall.up(text)
            } else if recall.claims_down(text)
                && on_last_row(text, cursor)
                && consume(ui, Key::ArrowDown)
            {
                recall.down(text)
            } else {
                walking && consume(ui, Key::Escape) && recall.escape(text)
            };
        if walked {
            bullets::place_cursor(ui, id, text.chars().count());
            return;
        }
    }
    if blank && stop && consume(ui, Key::Escape) {
        outcome.stop = true;
    }
}

#[cfg(test)]
mod tests {
    use egui::epaint::ClippedShape;
    use egui::{Context, Event, RawInput};
    use std::path::PathBuf;

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
        stop: bool,
        edit_held: bool,
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
        files: Option<(std::path::PathBuf, Vec<String>)>,
        focus: bool,
        ghost: Option<&'static str>,
        hovered: bool,
        mode: Mode,
        recall: Option<Recall>,
        stop: bool,
        held: usize,
    }

    impl Rig {
        fn new() -> Self {
            let ctx = Context::default();
            crate::fonts::install(&ctx);
            let mut rig = Self {
                ctx,
                text: String::new(),
                files: None,
                focus: true,
                ghost: None,
                hovered: false,
                mode: Mode::Reply,
                recall: None,
                stop: false,
                held: 0,
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
            let (mut outcome, mut input) = (Outcome::default(), Rect::NOTHING);
            let out = self.ctx.run_ui(raw, |ui| {
                let column = Rect::from_min_size(pos2(0.0, 0.0), vec2(COLUMN_W, COLUMN_H));
                let h = height(ui, &self.text, column);
                input = Rect::from_min_max(pos2(0.0, COLUMN_H - h), column.max);
                let room = input.shrink2(vec2(INSET, GAP_Y));
                let mut child = ui.new_child(UiBuilder::new().max_rect(room));
                let draft = Draft {
                    text: &mut self.text,
                    focus: &mut self.focus,
                    recall: self.recall.as_mut(),
                };
                let spec = Spec {
                    id: Id::new("reply-box"),
                    hint: HINT,
                    enabled: true,
                    commands: &[],
                    ghost: self.ghost,
                    mode: self.mode,
                    stop: self.stop,
                    held: self.held,
                    files: self
                        .files
                        .as_ref()
                        .map(|(root, paths)| at_files::Source { root, paths }),
                };
                outcome = show(&mut child, draft, &spec);
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
            Seen {
                sent: outcome.sent,
                stop: outcome.stop,
                edit_held: outcome.edit_held,
                shapes,
                clips,
                box_rect: boxed,
            }
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
            (row, row, 280.0, BOX_H),
            (row, 3.0 * row, 280.0, BOX_H + 2.0 * row),
            (row, 100.0 * row, 280.0, 280.0),
            (row, 100.0 * row, 20.0, BOX_H),
        ];
        for (row_h, text_h, cap, want) in cases {
            let got = fit(row_h, text_h, cap);
            assert_eq!(got.box_h, want, "{row_h} {text_h} {cap}");
        }
        assert_eq!(fit(18.0, 18.0, 280.0).pad_y, 14.0);
        assert!(fit(18.0, 3600.0, 280.0).scrolls);
        assert!(!fit(18.0, 36.0, 280.0).scrolls);
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

    fn with_history(entries: &[&str]) -> Rig {
        let mut rig = Rig::new();
        rig.recall = Some(Recall::with_entries(entries.iter().map(|e| (*e).to_owned()).collect()));
        rig.frame(vec![]);
        rig
    }

    #[test]
    fn up_in_an_empty_box_walks_back_through_what_was_sent_and_down_walks_forward_to_empty() {
        let mut rig = with_history(&["first", "second", "third"]);
        let mut shown = Vec::new();
        for _ in 0..3 {
            rig.press(Key::ArrowUp);
            shown.push(rig.text.clone());
        }
        rig.press(Key::ArrowDown);
        shown.push(rig.text.clone());
        rig.press(Key::ArrowDown);
        rig.press(Key::ArrowDown);
        shown.push(rig.text.clone());
        assert_eq!(shown, ["third", "second", "first", "second", ""]);
    }

    #[test]
    fn esc_during_a_walk_gives_the_box_back_and_keeps_the_keyboard() {
        let mut rig = with_history(&["first", "second"]);
        rig.press(Key::ArrowUp);
        assert_eq!(rig.text, "second");
        rig.press(Key::Escape);
        assert_eq!(rig.text, "");
        assert!(rig.ctx.memory(|memory| memory.has_focus(Id::new("reply-box"))));
    }

    #[test]
    fn up_in_a_box_with_text_is_left_to_the_cursor() {
        let mut rig = with_history(&["first"]);
        rig.type_text("my draft");
        rig.press(Key::ArrowUp);
        assert_eq!(rig.text, "my draft");
    }

    #[test]
    fn a_recalled_message_goes_out_on_enter_like_a_typed_one() {
        let mut rig = with_history(&["run the tests"]);
        rig.press(Key::ArrowUp);
        let seen = rig.frame(vec![key(Key::Enter)]);
        assert_eq!(seen.sent.as_deref(), Some("run the tests"));
        assert_eq!(rig.text, "");
    }

    #[test]
    fn edit_and_send_gives_the_draft_back_once_the_message_is_sent() {
        let mut rig = with_history(&["old"]);
        rig.type_text("my draft");
        let mut recall = rig.recall.take().unwrap();
        recall.load_message("an earlier one", &mut rig.text);
        rig.recall = Some(recall);
        rig.frame(vec![]);
        assert_eq!(rig.text, "an earlier one");
        let seen = rig.frame(vec![key(Key::Enter)]);
        assert_eq!(seen.sent.as_deref(), Some("an earlier one"));
        assert_eq!(rig.text, "my draft");
    }

    #[test]
    fn up_in_an_empty_box_asks_to_edit_the_newest_held_message_before_it_recalls_history() {
        let mut rig = with_history(&["first"]);
        rig.held = 2;
        let seen = rig.frame(vec![key(Key::ArrowUp)]);
        assert!(seen.edit_held);
        assert_eq!(rig.text, "", "history waits while a held message is open for editing");
        rig.held = 0;
        let seen = rig.press(Key::ArrowUp);
        assert!(!seen.edit_held);
        assert_eq!(rig.text, "first");
    }

    #[test]
    fn esc_in_an_empty_box_stops_a_working_agent_and_in_a_box_with_text_it_does_not() {
        let mut rig = Rig::new();
        rig.stop = true;
        rig.frame(vec![]);
        assert!(rig.frame(vec![key(Key::Escape)]).stop);
        let mut typed = Rig::new();
        typed.stop = true;
        typed.type_text("half");
        assert!(!typed.frame(vec![key(Key::Escape)]).stop);
        let mut idle = Rig::new();
        idle.frame(vec![]);
        assert!(!idle.frame(vec![key(Key::Escape)]).stop, "nothing to stop");
    }

    #[test]
    fn the_stop_button_sits_beside_send_only_while_the_agent_works_and_a_click_stops() {
        let mut rig = Rig::new();
        let idle = rig.frame(vec![]);
        assert_eq!(idle.text_rows("Stop").len(), 0);
        rig.stop = true;
        let seen = rig.frame(vec![]);
        let stop = seen.text_rows("Stop")[0];
        let send = seen.text_rows("Send")[0];
        assert!(stop.right() < send.left(), "{stop:?} {send:?}");
        assert!((stop.center().y - send.center().y).abs() < 0.6);
        assert!(seen.box_rect.contains_rect(stop));
        let at = stop.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        rig.frame(vec![Event::PointerMoved(at)]);
        rig.frame(vec![button(true)]);
        assert!(rig.frame(vec![button(false)]).stop);
    }

    #[test]
    fn typed_text_wraps_before_the_stop_button_not_under_it() {
        let mut rig = Rig::new();
        rig.stop = true;
        let text = "word ".repeat(40);
        let seen = rig.type_text(&text);
        let stop = seen.text_rows("Stop")[0];
        let rows = seen.text_rows(&text);
        assert!(rows.iter().all(|row| row.right() <= stop.left()), "{rows:?} {stop:?}");
    }

    const SHOT: &str = "/s/shot-1793011211356.png";
    const REPORT: &str = "/s/q3-report.pdf";

    fn cursor_after(rig: &Rig, words: &str) {
        let at = rig
            .text
            .find(words)
            .map(|byte| rig.text[..byte + words.len()].chars().count())
            .unwrap();
        let mut state = TextEdit::load_state(&rig.ctx, Id::new("reply-box")).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
        state.store(&rig.ctx, Id::new("reply-box"));
    }

    fn put(rig: &mut Rig, path: &str) {
        let target = Target::File(PathBuf::from(path));
        insert_targets(&rig.ctx.clone(), Id::new("reply-box"), &mut rig.text, &[target]);
        rig.frame(vec![]);
        rig.frame(vec![]);
    }

    fn chip_rects(seen: &Seen) -> Vec<Rect> {
        seen.shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Text(t) if tokens::has_tokens(t.galley.text()) => {
                    Some(token_view::chips(t.pos, &t.galley).into_iter().map(|(_, rect)| rect))
                }
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn word_edges(seen: &Seen, before: &str, after: &str) -> (f32, f32) {
        seen.shapes
            .iter()
            .find_map(|shape| match shape {
                Shape::Text(t) if tokens::has_tokens(t.galley.text()) => {
                    let text = t.galley.text();
                    let end = |words: &str| {
                        let at = text.find(words).unwrap();
                        text[..at].chars().count()
                    };
                    let left = t
                        .galley
                        .pos_from_cursor(CCursor::new(end(before) + before.chars().count()));
                    let right = t.galley.pos_from_cursor(CCursor::new(end(after)));
                    Some((t.pos.x + left.min.x, t.pos.x + right.min.x))
                }
                _ => None,
            })
            .expect("the text with a token was painted")
    }

    fn mid_sentence(rig: &mut Rig) {
        rig.type_text("The numbers in  don't match");
        cursor_after(rig, "The numbers in ");
        rig.frame(vec![]);
    }

    fn carets(seen: &Seen) -> usize {
        seen.shapes
            .iter()
            .filter(|shape| {
                matches!(shape, Shape::LineSegment { stroke, .. }
                    if stroke.width == CARET_W && stroke.color == brand::TEAL)
            })
            .count()
    }

    #[test]
    fn while_files_are_dragged_over_the_box_a_solid_caret_shows_where_they_will_go() {
        let mut rig = Rig::new();
        mid_sentence(&mut rig);
        assert_eq!(carets(&rig.frame(vec![])), 0);
        rig.hovered = true;
        assert_eq!(carets(&rig.frame(vec![])), 1);
        rig.hovered = false;
        assert_eq!(carets(&rig.frame(vec![])), 0);
    }

    #[test]
    fn a_file_put_in_while_the_box_has_no_keyboard_goes_at_the_end_not_at_a_stale_cursor() {
        let mut rig = Rig::new();
        mid_sentence(&mut rig);
        let id = Id::new("reply-box");
        rig.ctx.memory_mut(|memory| memory.surrender_focus(id));
        rig.focus = false;
        rig.frame(vec![]);
        put(&mut rig, REPORT);
        assert_eq!(tokens::sendable(&rig.text), format!("The numbers in  don't match {REPORT}"));
    }

    #[test]
    fn a_picture_pasted_mid_sentence_sits_between_the_words_around_the_cursor() {
        let mut rig = Rig::new();
        mid_sentence(&mut rig);
        put(&mut rig, SHOT);
        assert_eq!(tokens::sendable(&rig.text), format!("The numbers in {SHOT} don't match"));
        let seen = rig.frame(vec![]);
        let (left, right) = word_edges(&seen, "The numbers in ", "don't match");
        let chips = chip_rects(&seen);
        assert_eq!(chips.len(), 1);
        assert!(left <= chips[0].left() && chips[0].right() <= right, "{left} {chips:?} {right}");
        assert_eq!(seen.rects(brand::SUNKEN).len(), 1);
    }

    #[test]
    fn a_file_dropped_mid_sentence_sits_between_the_words_and_a_second_one_follows_it() {
        let mut rig = Rig::new();
        mid_sentence(&mut rig);
        let ctx = rig.ctx.clone();
        let both = [Target::File(REPORT.into()), Target::File(SHOT.into())];
        insert_targets(&ctx, Id::new("reply-box"), &mut rig.text, &both);
        let seen = rig.frame(vec![]);
        assert_eq!(
            tokens::sendable(&rig.text),
            format!("The numbers in {REPORT} {SHOT} don't match")
        );
        let chips = chip_rects(&seen);
        let (left, right) = word_edges(&seen, "The numbers in ", "don't match");
        assert_eq!(chips.len(), 2);
        assert!(left <= chips[0].left() && chips[0].right() <= chips[1].left());
        assert!(chips[1].right() <= right);
    }

    fn with_files(rig: &mut Rig) {
        rig.files = Some((
            "/repo".into(),
            ["src/main.rs", "src/composer.rs", "README.md"].map(String::from).to_vec(),
        ));
    }

    #[test]
    fn an_at_pick_mid_sentence_becomes_a_token_between_the_words_around_it() {
        let mut rig = Rig::new();
        with_files(&mut rig);
        rig.type_text("Check  and run");
        cursor_after(&rig, "Check ");
        rig.frame(vec![]);
        rig.type_text("@comp");
        assert_eq!(rig.frame(vec![]).text_rows("src/composer.rs").len(), 1);
        rig.press(Key::Enter);
        assert_eq!(tokens::sendable(&rig.text), "Check /repo/src/composer.rs and run");
        let seen = rig.frame(vec![]);
        let (left, right) = word_edges(&seen, "Check ", "and run");
        let chips = chip_rects(&seen);
        assert_eq!(chips.len(), 1);
        assert!(left <= chips[0].left() && chips[0].right() <= right);
        assert_eq!(seen.sent, None);
    }

    #[test]
    fn the_at_list_shows_the_best_matching_files_and_only_while_a_word_with_an_at_is_typed() {
        let mut rig = Rig::new();
        with_files(&mut rig);
        let seen = rig.type_text("see @main");
        assert_eq!(seen.text_rows("src/main.rs").len(), 1);
        assert_eq!(seen.text_rows("src/composer.rs").len(), 0);
        let seen = rig.type_text(" and then more words");
        assert_eq!(seen.text_rows("src/main.rs").len(), 0);
    }

    #[test]
    fn an_at_in_a_box_with_no_folder_opens_nothing_and_enter_sends() {
        let mut rig = Rig::new();
        rig.type_text("mail @team");
        assert_eq!(rig.frame(vec![key(Key::Enter)]).sent.as_deref(), Some("mail @team"));
    }

    #[test]
    fn backspace_after_a_token_takes_it_whole_and_delete_before_it_does_too() {
        let mut rig = Rig::new();
        mid_sentence(&mut rig);
        put(&mut rig, SHOT);
        rig.press(Key::Backspace);
        assert_eq!(rig.text, "The numbers in  don't match");
        put(&mut rig, SHOT);
        let before =
            rig.text.find(tokens::OPEN).map(|byte| rig.text[..byte].chars().count()).unwrap();
        let mut state = TextEdit::load_state(&rig.ctx, Id::new("reply-box")).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(before))));
        state.store(&rig.ctx, Id::new("reply-box"));
        rig.press(Key::Delete);
        assert_eq!(rig.text, "The numbers in  don't match");
    }

    #[test]
    fn the_cursor_steps_over_a_token_and_never_stops_inside_it() {
        let mut rig = Rig::new();
        mid_sentence(&mut rig);
        put(&mut rig, SHOT);
        let token = tokens::find(&rig.text).remove(0);
        for _ in 0..3 {
            rig.press(Key::ArrowLeft);
            let at = collapsed_at(&rig);
            assert!(at <= token.chars.start || at >= token.chars.end, "{at} in {token:?}");
        }
        for _ in 0..6 {
            rig.press(Key::ArrowRight);
            let at = collapsed_at(&rig);
            assert!(at <= token.chars.start || at >= token.chars.end, "{at} in {token:?}");
        }
    }

    fn collapsed_at(rig: &Rig) -> usize {
        TextEdit::load_state(&rig.ctx, Id::new("reply-box"))
            .and_then(|state| state.cursor.char_range())
            .map(|range| range.primary.index.0)
            .unwrap()
    }

    #[test]
    fn sending_keeps_each_path_where_the_token_stood_and_a_token_alone_can_be_sent() {
        let mut rig = Rig::new();
        mid_sentence(&mut rig);
        put(&mut rig, "/s/My Report.pdf");
        let sent = rig.frame(vec![key(Key::Enter)]).sent;
        assert_eq!(sent.as_deref(), Some("The numbers in '/s/My Report.pdf' don't match"));
        assert_eq!(rig.text, "");
        put(&mut rig, SHOT);
        assert_eq!(rig.frame(vec![key(Key::Enter)]).sent.as_deref(), Some(SHOT));
    }

    #[test]
    fn a_token_is_one_chip_tall_enough_for_its_row_and_wraps_with_the_words() {
        let mut rig = Rig::new();
        rig.type_text(&"word ".repeat(10));
        let flat = rig.frame(vec![]).box_rect.height();
        put(&mut rig, "/s/a-very-long-file-name-for-wrapping-checks-in-the-sentence.pdf");
        let seen = rig.type_text(&" tail".repeat(30));
        let chips = chip_rects(&seen);
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].height(), crate::screen_kit::Size::Tag.height());
        assert!(seen.box_rect.height() > flat);
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

    fn chord(pressed: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key: pressed,
            physical_key: Some(pressed),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn has_focus(rig: &Rig) -> bool {
        rig.ctx.memory(|memory| memory.has_focus(Id::new("reply-box")))
    }

    struct Marks {
        dots: Vec<f32>,
        rings: Vec<f32>,
        squares: Vec<f32>,
        numbers: Vec<(String, f32)>,
    }

    fn marks(seen: &Seen) -> Marks {
        let mut found = Marks { dots: vec![], rings: vec![], squares: vec![], numbers: vec![] };
        for shape in &seen.shapes {
            match shape {
                Shape::Circle(circle) if circle.fill == brand::SOFT => {
                    found.dots.push(circle.center.x);
                }
                Shape::Circle(circle) if circle.stroke.color == brand::SOFT => {
                    found.rings.push(circle.center.x);
                }
                Shape::Rect(rect) if rect.fill == brand::SOFT => {
                    found.squares.push(rect.rect.center().x);
                }
                Shape::Text(text)
                    if text.galley.text().ends_with('.') && text.galley.text().len() < 4 =>
                {
                    found
                        .numbers
                        .push((text.galley.text().to_owned(), text.pos.x + text.galley.size().x));
                }
                _ => {}
            }
        }
        found
    }

    const LIST: &str = "List:\n- item\n    - sub\n        - deep\nSteps:\n1. first\n    1. nested";

    #[test]
    fn list_text_starts_at_0_30_and_60_and_the_markers_sit_14_and_8_pt_left_of_it() {
        let mut rig = Rig::new();
        rig.text = LIST.to_owned();
        rig.frame(vec![]);
        let seen = rig.frame(vec![]);
        let rows = seen.text_rows(LIST);
        assert_eq!(rows.len(), 7);
        let edge = rows[0].min.x;
        let offsets: Vec<f32> = rows.iter().map(|row| row.min.x - edge).collect();
        assert_eq!(offsets, [0.0, 30.0, 60.0, 90.0, 0.0, 30.0, 60.0]);
        let found = marks(&seen);
        assert_eq!(found.dots, [edge + 30.0 - 14.0]);
        assert_eq!(found.rings, [edge + 60.0 - 14.0]);
        assert_eq!(found.squares, [edge + 90.0 - 14.0]);
        assert_eq!(
            found.numbers,
            [("1.".to_owned(), edge + 30.0 - 8.0), ("1.".to_owned(), edge + 60.0 - 8.0)]
        );
        assert_eq!(rig.text, LIST, "drawing never edits the text");
    }

    #[test]
    fn a_bullet_sits_inside_the_height_of_its_row() {
        let mut rig = Rig::new();
        rig.text = "- item".to_owned();
        rig.frame(vec![]);
        let seen = rig.frame(vec![]);
        let row = seen.text_rows("- item")[0];
        let dot = seen
            .shapes
            .iter()
            .find_map(|shape| match shape {
                Shape::Circle(circle) if circle.fill == brand::SOFT => Some(circle.center),
                _ => None,
            })
            .expect("a dot");
        assert!(dot.y > row.top() && dot.y < row.bottom(), "{dot:?} {row:?}");
        assert!((dot.y - row.center().y).abs() < 4.0, "{dot:?} {row:?}");
    }

    #[test]
    fn every_wrapped_row_of_an_item_starts_at_its_text_and_a_plain_line_goes_back_to_the_edge() {
        let long = "one two three four five six seven eight nine ten eleven twelve thirteen \
                    fourteen fifteen sixteen seventeen eighteen nineteen twenty";
        let text = format!("plain\n- {long}\n    - {long}\nplain again");
        let mut rig = Rig::new();
        rig.text = text.clone();
        rig.frame(vec![]);
        let seen = rig.frame(vec![]);
        let rows = seen.text_rows(&text);
        let edge = rows[0].min.x;
        let offsets: Vec<f32> = rows.iter().map(|row| row.min.x - edge).collect();
        assert!(offsets.len() >= 7, "both items wrap: {offsets:?}");
        let (first, rest) = offsets.split_first().unwrap();
        let (last, middle) = rest.split_last().unwrap();
        assert_eq!((*first, *last), (0.0, 0.0));
        let split = middle.iter().position(|x| *x == 60.0).expect("the nested item");
        assert!(middle[..split].iter().all(|x| *x == 30.0), "{offsets:?}");
        assert!(middle[split..].iter().all(|x| *x == 60.0), "{offsets:?}");
        let right = rows.iter().map(|row| row.max.x).fold(0.0, f32::max);
        assert!(right <= seen.box_rect.right() - 18.0, "a wrapped row stays inside the box");
        assert_eq!(rig.text, text);
    }

    #[test]
    fn a_list_does_not_change_the_box_height_a_plain_box_of_the_same_rows_has() {
        let mut list = Rig::new();
        list.text = "- a\n    - b\n1. c".to_owned();
        let mut plain = Rig::new();
        plain.text = "a\nb\nc".to_owned();
        list.frame(vec![]);
        plain.frame(vec![]);
        assert_eq!(list.frame(vec![]).box_rect.height(), plain.frame(vec![]).box_rect.height());
    }

    #[test]
    fn enter_tab_and_shift_tab_work_a_list_line_by_line() {
        let mut rig = Rig::new();
        rig.type_text("- one");
        assert_eq!(rig.press(Key::Enter).sent, None);
        assert_eq!(rig.text, "- one\n- ");
        rig.type_text("two");
        rig.settle(vec![chord(Key::Tab, Modifiers::NONE)]);
        assert_eq!(rig.text, "- one\n  - two");
        assert!(has_focus(&rig), "Tab on a list line keeps the keyboard");
        rig.settle(vec![chord(Key::Tab, Modifiers::NONE)]);
        assert_eq!(rig.text, "- one\n  - two", "no deeper than under the item above");
        assert_eq!(rig.press(Key::Enter).sent, None);
        assert_eq!(rig.text, "- one\n  - two\n  - ");
        rig.settle(vec![chord(Key::Tab, Modifiers::SHIFT)]);
        assert_eq!(rig.text, "- one\n  - two\n- ");
        assert!(has_focus(&rig));
        rig.type_text("three");
        assert_eq!(rig.text, "- one\n  - two\n- three");
        rig.press(Key::Enter);
        rig.press(Key::Enter);
        assert_eq!(rig.text, "- one\n  - two\n- three\n");
        let sent = rig.frame(vec![key(Key::Enter)]).sent;
        assert_eq!(sent.as_deref(), Some("- one\n  - two\n- three\n"));
    }

    #[test]
    fn enter_on_a_numbered_line_counts_up() {
        let mut rig = Rig::new();
        rig.type_text("1. first");
        rig.press(Key::Enter);
        rig.type_text("second");
        rig.press(Key::Enter);
        assert_eq!(rig.text, "1. first\n2. second\n3. ");
        rig.press(Key::Enter);
        assert_eq!(rig.text, "1. first\n2. second\n");
    }

    #[test]
    fn tab_off_a_list_line_keeps_the_text_and_moves_the_keyboard_on() {
        let mut rig = Rig::new();
        rig.type_text("hello");
        rig.settle(vec![chord(Key::Tab, Modifiers::NONE)]);
        assert_eq!(rig.text, "hello");
        assert!(!has_focus(&rig), "focus moved on");
    }

    #[test]
    fn cmd_enter_sends_from_a_list_line_and_from_a_plain_one() {
        let mut rig = Rig::new();
        rig.type_text("- one");
        rig.press(Key::Enter);
        rig.type_text("two");
        let sent = rig.frame(vec![chord(Key::Enter, Modifiers::COMMAND)]).sent;
        assert_eq!(sent.as_deref(), Some("- one\n- two"));
        assert_eq!(rig.text, "");
        rig.type_text("plain");
        let sent = rig.frame(vec![chord(Key::Enter, Modifiers::COMMAND)]).sent;
        assert_eq!(sent.as_deref(), Some("plain"));
        assert_eq!(rig.frame(vec![chord(Key::Enter, Modifiers::COMMAND)]).sent, None);
    }

    #[test]
    fn enter_on_a_plain_line_sends_and_shift_enter_adds_a_line() {
        let mut rig = Rig::new();
        rig.type_text("hello");
        rig.settle(vec![chord(Key::Enter, Modifiers::SHIFT)]);
        assert_eq!(rig.text, "hello\n");
        rig.type_text("world");
        let sent = rig.frame(vec![key(Key::Enter)]).sent;
        assert_eq!(sent.as_deref(), Some("hello\nworld"));
    }

    #[test]
    fn shift_enter_on_a_list_line_adds_a_plain_line() {
        let mut rig = Rig::new();
        rig.type_text("- one");
        rig.settle(vec![chord(Key::Enter, Modifiers::SHIFT)]);
        assert_eq!(rig.text, "- one\n");
    }

    fn field(height: f32) -> Rig {
        let mut rig = Rig::new();
        rig.mode = Mode::Field { height };
        rig.frame(vec![]);
        rig
    }

    #[test]
    fn a_field_is_as_tall_as_it_is_asked_and_has_no_send() {
        let mut rig = field(160.0);
        let empty = rig.frame(vec![]);
        assert_eq!(empty.box_rect.height(), 160.0);
        assert_eq!(empty.text_rows("Send").len(), 0);
        let full = rig.type_text(&"line\n".repeat(30));
        assert_eq!(full.box_rect.height(), 160.0, "long text scrolls inside instead of growing");
    }

    #[test]
    fn a_field_is_as_tall_as_it_is_asked_with_a_token_in_its_text() {
        let mut rig = field(100.0);
        put(&mut rig, SHOT);
        let seen = rig.frame(vec![]);
        assert_eq!(seen.box_rect.height(), 100.0);
        let asked = {
            let mut got = 0.0;
            let text = rig.text.clone();
            let _ = rig.ctx.run_ui(RawInput::default(), |ui| {
                got = field_height(ui, &text, (COLUMN_W - 2.0 * INSET, 100.0));
            });
            got
        };
        assert_eq!(asked, seen.box_rect.height(), "the form allocates what the box takes");
    }

    #[test]
    fn enter_in_a_field_is_a_new_line_that_sends_nothing_and_cmd_enter_adds_none() {
        let mut rig = field(160.0);
        rig.type_text("one");
        let seen = rig.settle(vec![key(Key::Enter)]);
        assert_eq!((seen.sent, rig.text.as_str()), (None, "one\n"));
        rig.type_text("two");
        let seen = rig.settle(vec![chord(Key::Enter, Modifiers::SHIFT)]);
        assert_eq!((seen.sent, rig.text.as_str()), (None, "one\ntwo\n"));
        let seen = rig.frame(vec![chord(Key::Enter, Modifiers::COMMAND)]);
        assert_eq!((seen.sent, rig.text.as_str()), (None, "one\ntwo\n"));
    }

    #[test]
    fn a_list_line_in_a_field_still_takes_a_plain_enter_as_one_new_line() {
        let mut rig = field(160.0);
        rig.type_text("- one");
        rig.settle(vec![key(Key::Enter)]);
        assert_eq!(rig.text, "- one\n");
    }

    #[test]
    fn a_field_wraps_at_its_own_right_padding_not_where_send_would_be() {
        let mut rig = field(160.0);
        let text = "word ".repeat(60);
        let seen = rig.type_text(&text);
        let widest = seen.text_rows(&text).iter().map(egui::Rect::right).fold(0.0, f32::max);
        assert!(widest > seen.box_rect.right() - 90.0, "{widest} in {:?}", seen.box_rect);
        assert!(widest <= seen.box_rect.right() - PAD_X, "{widest} in {:?}", seen.box_rect);
    }
}
