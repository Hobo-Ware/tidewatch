//! Painted text with clickable web links: the links of `links::plain` in teal, an underline and a
//! pointing hand under the pointer, the full address as a tooltip, and a click that opens it in
//! the browser. The Progress panel lays its lines out through `Text`.

use std::ops::Range;
use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::text_selection::LabelSelectionState;
use egui::{
    Color32, CursorIcon, FontId, Galley, Id, Label, OpenUrl, Painter, Pos2, Rect, Response, Sense,
    Stroke, TextWrapMode, Ui, Vec2, pos2,
};

use crate::brand;
use crate::emoji_paint;
use crate::emoji_text;
use crate::links;

fn link_format(format: &TextFormat) -> TextFormat {
    TextFormat {
        color: brand::TEAL_TEXT,
        underline: Stroke::new(1.0, brand::link_rest()),
        ..format.clone()
    }
}

struct Span {
    bytes: Range<usize>,
    url: String,
}

/// A line under construction: runs of text with their formats, links found in each run.
pub(crate) struct Text {
    job: LayoutJob,
    spans: Vec<Span>,
}

impl Text {
    pub(crate) fn new() -> Self {
        Self { job: LayoutJob::default(), spans: Vec::new() }
    }

    /// One run in `font` and `ink`, cut with an ellipsis at `max_w`.
    pub(crate) fn line(text: &str, font: FontId, ink: Color32, max_w: f32) -> Self {
        let mut line = Self::new();
        line.run(text, &TextFormat::simple(font, ink));
        line.cut(max_w)
    }

    /// Cuts the line with an ellipsis at `max_w`.
    pub(crate) fn cut(mut self, max_w: f32) -> Self {
        self.job.wrap = TextWrapping::truncate_at_width(max_w.max(1.0));
        self
    }

    pub(crate) fn run(&mut self, text: &str, format: &TextFormat) {
        let found = links::plain(text);
        let mut last = 0;
        for link in found.links {
            self.push(&found.text[last..link.range.start], format);
            let linked = link_format(format);
            let start = self.job.text.len();
            self.push(&found.text[link.range.clone()], &linked);
            self.spans.push(Span { bytes: start..self.job.text.len(), url: link.url });
            last = link.range.end;
        }
        self.push(&found.text[last..], format);
    }

    pub(crate) fn raw(&mut self, text: &str, format: &TextFormat) {
        self.push(text, format);
    }

    pub(crate) fn linked(&mut self, text: &str, format: &TextFormat, url: &str) {
        let start = self.job.text.len();
        self.push(text, format);
        self.spans.push(Span { bytes: start..self.job.text.len(), url: url.to_owned() });
    }

    pub(crate) fn show(self, ui: &mut Ui, id: Id, ink: Color32, wrap: TextWrapMode) -> Response {
        let Self { job, spans } = self;
        let label = Label::new(emoji_text::slot_job(ui.ctx(), job.clone()))
            .selectable(true)
            .wrap_mode(wrap);
        let (pos, galley, response) = label.layout_in_ui(ui);
        if ui.is_rect_visible(response.rect) {
            LabelSelectionState::label_text_selection(
                ui,
                &response,
                pos,
                galley.clone(),
                ink,
                Stroke::NONE,
            );
            emoji_paint::paint(ui, id, pos, &galley);
        }
        Laid { galley, job, spans }.interact(ui, id, pos);
        response
    }

    fn push(&mut self, text: &str, format: &TextFormat) {
        if !text.is_empty() {
            self.job.append(text, 0.0, format.clone());
        }
    }

    /// Wraps at `max_w`, keeping at most `max_rows` rows.
    pub(crate) fn wrapped(mut self, max_w: f32, max_rows: usize) -> Self {
        self.job.wrap.max_width = max_w;
        self.job.wrap.max_rows = max_rows;
        self
    }

    pub(crate) fn lay_out(self, painter: &Painter) -> Laid {
        let galley = painter.layout_job(self.job.clone());
        Laid { galley, job: self.job, spans: self.spans }
    }
}

/// A laid out `Text`: its size is known before it is painted.
pub(crate) struct Laid {
    galley: Arc<Galley>,
    job: LayoutJob,
    spans: Vec<Span>,
}

impl Laid {
    pub(crate) fn size(&self) -> Vec2 {
        self.galley.size()
    }

    /// Paints at `pos` and answers the pointer over its links; `id` must differ per line.
    pub(crate) fn paint(&self, ui: &Ui, id: Id, pos: Pos2, fallback: Color32) {
        let hovered = self.interact(ui, id, pos);
        ui.painter().galley(pos, self.galley_for(ui, hovered), fallback);
    }

    /// `paint`, and the text can be selected and copied; a click on a link still opens it.
    pub(crate) fn paint_selectable(
        &self,
        ui: &Ui,
        id: Id,
        pos: Pos2,
        fallback: Color32,
    ) -> Response {
        let rect = Rect::from_min_size(pos, self.size());
        let selection = ui.interact(rect, id, Sense::click_and_drag());
        let hovered = self.interact(ui, id, pos);
        let galley = self.galley_for(ui, hovered);
        LabelSelectionState::label_text_selection(
            ui,
            &selection,
            pos,
            galley,
            fallback,
            Stroke::NONE,
        );
        if hovered.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        selection
    }

    fn galley_for(&self, ui: &Ui, hovered: Option<usize>) -> Arc<Galley> {
        match hovered {
            Some(index) => ui.painter().layout_job(self.underlined(index)),
            None => self.galley.clone(),
        }
    }

    fn interact(&self, ui: &Ui, id: Id, pos: Pos2) -> Option<usize> {
        let mut hovered = None;
        for (n, (index, rect)) in self.link_rects(pos).into_iter().enumerate() {
            let url = &self.spans[index].url;
            let response = ui.interact(rect, id.with(n), Sense::click());
            if response.clicked() {
                ui.ctx().open_url(OpenUrl::new_tab(url));
            }
            if response.hovered() {
                hovered = Some(index);
            }
            response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(url);
        }
        hovered
    }

    fn underlined(&self, index: usize) -> LayoutJob {
        let bytes = &self.spans[index].bytes;
        let mut job = self.job.clone();
        for section in &mut job.sections {
            if bytes.contains(&usize::from(section.byte_range.start)) {
                section.format.underline = Stroke::new(1.0, brand::TEAL_TEXT);
            }
        }
        job
    }

    fn link_rects(&self, pos: Pos2) -> Vec<(usize, Rect)> {
        let chars: Vec<Range<usize>> = self
            .spans
            .iter()
            .map(|span| {
                let start = self.job.text[..span.bytes.start].chars().count();
                start..start + self.job.text[span.bytes.clone()].chars().count()
            })
            .collect();
        let mut rects = Vec::new();
        let mut base = 0;
        for placed in &self.galley.rows {
            let glyphs = &placed.row.glyphs;
            let band = placed.rect();
            for (index, span) in chars.iter().enumerate() {
                let first = span.start.saturating_sub(base);
                let end = span.end.saturating_sub(base).min(glyphs.len());
                if first >= end {
                    continue;
                }
                let left = placed.pos.x + glyphs[first].pos.x;
                let last = &glyphs[end - 1];
                let right = placed.pos.x + last.pos.x + last.advance_width;
                let min = pos + egui::vec2(left, band.min.y);
                rects.push((
                    index,
                    Rect::from_min_max(min, pos2(pos.x + right, pos.y + band.max.y)),
                ));
            }
            base += glyphs.len() + usize::from(placed.ends_with_newline);
        }
        rects
    }
}

#[cfg(test)]
mod tests;
