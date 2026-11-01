//! A sub-agent's chat in the place of a tab's Reader: a `CHROME_H` header like the service Logs
//! view's (a `‹ Chat` back button, a dot, what the agent does, its model and effort, how long it
//! ran and what it spent) over the Reader's own rows, read-only. Esc goes back too.

use egui::{Align, Key, Layout, Modifiers, Rect, Ui, UiBuilder, pos2, vec2};

use crate::agent_avatar::Face;
use crate::brand;
use crate::layout;
use crate::reader_chat_view::Parent;
use crate::reader_view::{ChatFacts, ellipsized, show_chat};
use crate::screen_kit::{
    Size, TEXT_BODY, TEXT_TAG, TagFill, body, mono, paint_tag, quiet_button, tag_width,
};
use crate::subagent_chat::Open;
use crate::subagents_model::head;

pub(crate) const BACK_LABEL: &str = "\u{2039} Chat";
const GAP: f32 = 12.0;
const MIN_WHAT_W: f32 = 96.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Out {
    Back,
}

pub(crate) struct Facts<'a> {
    pub(crate) chat: &'a mut Open,
    /// The name and face of the agent that started this one.
    pub(crate) parent: (&'a str, &'a Face),
    pub(crate) now_ms: i64,
    pub(crate) keys_free: bool,
}

pub(crate) fn show(ui: &mut Ui, area: Rect, facts: Facts<'_>) -> Option<Out> {
    let Facts { chat, parent: (parent_name, parent_face), now_ms, keys_free } = facts;
    chat.mark_drawn();
    let top = layout::agent_grid(area);
    let header = Rect::from_min_size(
        top.min,
        vec2(top.width() - crate::reader_view::INSET, layout::CHROME_H),
    );
    let mut out = None;
    let line = head(&chat.agent, now_ms);
    let mid = header.center().y;
    let back = ui
        .scope_builder(
            UiBuilder::new().max_rect(header).layout(Layout::left_to_right(Align::Center)),
            |ui| quiet_button(ui, BACK_LABEL, true, Size::Compact),
        )
        .inner;
    let back_right = back.rect.right();
    if back.on_hover_text("Back to the Reader").clicked() {
        out = Some(Out::Back);
    }
    let painter = ui.painter();
    let (dot, halo) = if line.running { (brand::TEAL, true) } else { (brand::GREEN, false) };
    let centre = pos2(back_right + GAP + 4.0, mid);
    if halo {
        painter.circle_filled(centre, 7.0, dot.gamma_multiply(0.13));
    }
    painter.circle_filled(centre, 4.0, dot);
    let right = painter.layout_no_wrap(line.right.clone(), mono(TEXT_TAG), brand::MUTED);
    let right_left = header.right() - right.size().x;
    let what_left = centre.x + 4.0 + GAP;
    let tag_at = line.tag.as_deref().map(|tag| {
        let width = tag_width(painter, tag);
        (tag, right_left - GAP - width)
    });
    let what_right = match tag_at {
        Some((_, at)) if at >= what_left + MIN_WHAT_W => at - GAP,
        _ => right_left - GAP,
    };
    let what =
        ellipsized(painter, &line.what, body(TEXT_BODY), brand::TEXT, what_right - what_left);
    painter.galley(pos2(what_left, mid - what.size().y / 2.0), what, brand::TEXT);
    if let Some((tag, at)) = tag_at.filter(|(_, at)| *at >= what_left + MIN_WHAT_W) {
        paint_tag(painter, (at, mid), tag, brand::SOFT, TagFill::Line(brand::ITEM_LINE));
    }
    painter.galley(pos2(right_left, mid - right.size().y / 2.0), right, brand::MUTED);
    if line.running {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    }
    let body_rect = Rect::from_min_max(pos2(area.left(), header.bottom()), area.max);
    let facts = ChatFacts {
        key: chat.key(),
        agent: "Sub-agent",
        parent: Parent { name: parent_name, face: parent_face },
        running: chat.agent.is_running(),
    };
    show_chat(ui, body_rect, &chat.rows, &mut chat.view, &facts);
    if keys_free && ui.ctx().input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        out = Some(Out::Back);
    }
    out
}

#[cfg(test)]
mod tests;
