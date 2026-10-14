//! The files of the open commit, drawn as a card under its row in the History tree: kind letter,
//! path and line counts. Kelp's graph cannot grow a row, so the card is laid over the rows below
//! the selected one and follows it as the graph scrolls.

use egui::{
    Align2, CornerRadius, FontId, Id, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    pos2, vec2,
};
use kelp_core::commit::FileChange;
use kelp_ui::theme;

use crate::brand;
use crate::diff_pane::{kind_color, kind_letter};
use crate::pane_jobs::LineCounts;
use crate::tab_view::TabView;

const FILE_ROW_H: f32 = 28.0;
const META_H: f32 = 24.0;
const PAD_BOTTOM: f32 = 8.0;
const INSET: f32 = 6.0;
const TEXT_X: f32 = 30.0;
const MIN_SHOWN: f32 = 160.0;
const ROOM_MARGIN: f32 = 6.0;
const SHORT_ID: usize = 7;

/// Where the selected row is on screen and the strip of the pane the graph's rows scroll in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    pub(crate) row_top: f32,
    pub(crate) row_h: f32,
    pub(crate) viewport: Rect,
}

/// The totals of every file, only when every file was counted.
pub(crate) fn totals(counts: &[Option<LineCounts>], files: usize) -> Option<LineCounts> {
    if counts.len() != files {
        return None;
    }
    counts.iter().try_fold((0, 0), |(added, removed), c| {
        let (a, r) = (*c)?;
        Some((added + a, removed + r))
    })
}

pub(crate) fn meta_text(
    what: &str,
    author: Option<&str>,
    files: usize,
    sums: Option<LineCounts>,
) -> String {
    let mut parts = vec![what.to_owned()];
    parts.extend(author.map(str::to_owned));
    parts.push(format!("{files} {}", if files == 1 { "file" } else { "files" }));
    if let Some((added, removed)) = sums {
        parts.push(format!("+{added} -{removed}"));
    }
    parts.join(" · ")
}

pub(crate) fn band_height(rows: usize) -> f32 {
    META_H + rows.max(1) as f32 * FILE_ROW_H + PAD_BOTTOM
}

/// The height the card takes when `wanted` is its natural height and `room` what is left below the
/// row: a long list scrolls inside, but never gets smaller than a few rows.
pub(crate) fn fit(wanted: f32, room: f32) -> f32 {
    if wanted <= room { wanted } else { room.max(wanted.min(MIN_SHOWN)) }
}

fn short_id(view: &TabView) -> String {
    view.summary.as_ref().map_or_else(
        || "Uncommitted".to_owned(),
        |(id, _)| id.to_hex_with_len(SHORT_ID).to_string(),
    )
}

pub(crate) fn show(ui: &mut Ui, view: &mut TabView, geo: Geometry, body: Rect) {
    let band_top = geo.row_top + geo.row_h;
    if band_top > geo.viewport.bottom() || geo.row_top + geo.row_h < geo.viewport.top() {
        return;
    }
    let loading = view.is_loading_source();
    let rows = if loading { 0 } else { view.files.len() };
    let room = geo.viewport.bottom() - band_top - ROOM_MARGIN;
    let wanted = band_height(rows);
    if view.take_needs_room() && wanted.min(MIN_SHOWN) > room {
        view.graph.scroll_to = view.selection();
        ui.ctx().request_repaint();
    }
    let band = Rect::from_min_size(
        pos2(body.left() + INSET, band_top),
        vec2(body.width() - 2.0 * INSET, fit(wanted, room)),
    );
    let card = Rect::from_min_max(pos2(band.left(), geo.row_top), band.max);
    let clip = geo.viewport.intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    painter.rect_filled(band, CornerRadius { nw: 0, ne: 0, sw: 10, se: 10 }, brand::TEAL_FILL);
    painter.rect_stroke(card, 10, Stroke::new(1.0, brand::TEAL), StrokeKind::Inside);
    ui.interact(band, Id::new("commit-files-band"), Sense::click_and_drag());

    let author = view.summary.as_ref().map(|(_, summary)| summary.author.as_str());
    let sums = totals(&view.counts, view.files.len());
    let meta = meta_text(&short_id(view), author, view.files.len(), sums);
    painter.text(
        pos2(band.left() + TEXT_X, band.top() + META_H / 2.0),
        Align2::LEFT_CENTER,
        if loading { "Loading files...".to_owned() } else { meta },
        FontId::monospace(11.5),
        theme::text_muted(),
    );
    if rows == 0 {
        if !loading {
            painter.text(
                pos2(band.left() + TEXT_X, band.top() + META_H + FILE_ROW_H / 2.0),
                Align2::LEFT_CENTER,
                "Nothing changed here",
                FontId::proportional(12.5),
                theme::text_faint(),
            );
        }
        return;
    }
    let list = Rect::from_min_max(
        pos2(band.left() + INSET, band.top() + META_H),
        pos2(band.right() - INSET, band.bottom() - PAD_BOTTOM),
    );
    let mut child = ui.new_child(UiBuilder::new().id_salt("commit-files-list").max_rect(list));
    child.set_clip_rect(list.intersect(clip));
    let mut picked = None;
    ScrollArea::vertical().auto_shrink(false).show(&mut child, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, change) in view.files.iter().enumerate() {
            let current = view.file.as_deref() == Some(change.path.as_str());
            let counts = view.counts.get(index).copied().flatten();
            if file_row(ui, change, counts, current) {
                picked = Some(change.path.clone());
            }
        }
    });
    if let Some(path) = picked {
        view.open_file(&path);
    }
}

fn file_row(ui: &mut Ui, change: &FileChange, counts: Option<LineCounts>, current: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
    let painter = ui.painter_at(rect);
    if current {
        painter.rect_filled(rect, 8, brand::TEAL.gamma_multiply(0.16));
        let bar = Rect::from_min_max(rect.left_top(), pos2(rect.left() + 3.0, rect.bottom()));
        painter.rect_filled(bar.shrink2(vec2(0.0, 5.0)), 2, brand::TEAL);
    } else if response.hovered() {
        painter.rect_filled(rect, 8, theme::overlay(0x08));
    }
    painter.text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        kind_letter(change.kind),
        FontId::monospace(12.0),
        kind_color(change.kind),
    );
    let mut right = rect.right() - 10.0;
    if let Some((added, removed)) = counts {
        for (text, color, show) in [
            (format!("-{removed}"), brand::RED, removed > 0),
            (format!("+{added}"), brand::GREEN, added > 0),
        ] {
            if !show {
                continue;
            }
            let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), color);
            let width = galley.size().x;
            painter.galley(
                pos2(right - width, rect.center().y - galley.size().y / 2.0),
                galley,
                color,
            );
            right -= width + 8.0;
        }
    }
    let color = if current { theme::text_strong() } else { brand::SOFT };
    let font = FontId::monospace(12.5);
    let room = right - 8.0 - (rect.left() + TEXT_X);
    painter.text(
        pos2(rect.left() + TEXT_X, rect.center().y),
        Align2::LEFT_CENTER,
        tail_fit(ui, &change.path, &font, room),
        font,
        color,
    );
    response.on_hover_text(&change.path).clicked()
}

/// `path` cut from the front with an ellipsis until it fits `room`, so the file name stays.
pub(crate) fn tail_fit(ui: &Ui, path: &str, font: &FontId, room: f32) -> String {
    let width = |text: &str| {
        ui.painter().layout_no_wrap(text.to_owned(), font.clone(), brand::TEXT).size().x
    };
    if width(path) <= room {
        return path.to_owned();
    }
    let chars: Vec<char> = path.chars().collect();
    (1..chars.len())
        .map(|skip| format!("…{}", chars[skip..].iter().collect::<String>()))
        .find(|text| width(text) <= room)
        .unwrap_or_else(|| "…".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_need_every_file_counted() {
        assert_eq!(totals(&[Some((3, 1)), Some((3, 0))], 2), Some((6, 1)));
        assert_eq!(totals(&[Some((3, 1)), None], 2), None);
        assert_eq!(totals(&[Some((3, 1))], 2), None);
        assert_eq!(totals(&[], 0), Some((0, 0)));
    }

    #[test]
    fn the_meta_line_names_the_commit_author_files_and_lines() {
        assert_eq!(
            meta_text("8f100f4", Some("Builder"), 2, Some((6, 1))),
            "8f100f4 · Builder · 2 files · +6 -1"
        );
        assert_eq!(meta_text("Uncommitted", None, 1, None), "Uncommitted · 1 file");
    }

    #[test]
    fn the_card_scrolls_inside_when_the_room_is_short_but_keeps_a_few_rows() {
        assert_eq!(fit(100.0, 300.0), 100.0);
        assert_eq!(fit(400.0, 300.0), 300.0);
        assert_eq!(fit(400.0, 50.0), MIN_SHOWN);
        assert_eq!(fit(100.0, 50.0), 100.0);
        assert_eq!(band_height(0), band_height(1));
        assert_eq!(band_height(2), META_H + 2.0 * FILE_ROW_H + PAD_BOTTOM);
    }
}
