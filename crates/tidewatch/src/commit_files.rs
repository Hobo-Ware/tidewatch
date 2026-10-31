//! The files of the open commit, drawn as a card around its row in the History list: the gap
//! kelp-ui opens under the row holds a meta line and one line per file.

use egui::text::LayoutJob;
use std::f32::consts::FRAC_PI_2;

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Mesh, Painter, Pos2, Rect, ScrollArea, Sense,
    TextFormat, Ui, UiBuilder, pos2, vec2,
};
use kelp_core::commit::FileChange;
use kelp_ui::graph::GraphStyle;
use kelp_ui::icons::{self, Icon};

use crate::brand;
use crate::diff_pane::{kind_color, kind_letter};
use crate::pane_jobs::LineCounts;
use crate::screen_kit::TEXT_TAG;
use crate::screen_kit::{body, mono, selected_ink, selection_row};
use crate::selectable;
use crate::tab_view::TabView;
use crate::waiting_notes::Waiting;

const FILE_ROW_H: f32 = 30.0;
const META_H: f32 = 24.0;
const PAD_BOTTOM: f32 = 10.0;
const MARGIN_BELOW: f32 = 4.0;
const MIN_SHOWN: f32 = 160.0;
const RADIUS: u8 = 9;
const SIDE_PAD: f32 = 12.0;
const ROW_PAD: f32 = 8.0;
const LETTER_W: f32 = 22.0;
const ADD_W: f32 = 36.0;
const DEL_W: f32 = 28.0;
const COUNT_GAP: f32 = 10.0;
const FILL_ALPHA: u8 = 0x1a;
const ROW_GAP: f32 = 2.0;
const SHORT_ID: usize = 7;
const MAX_LANES: u16 = 14;
const SELECTION_RADIUS: f32 = 7.0;
const CORNER_STEPS: u32 = 6;
const NOTES_ICON: f32 = 12.0;
const NOTES_ICON_GAP: f32 = 3.0;
const NOTES_MOST: usize = 99;

/// Where the card goes: the gap under the open row, the row's height, the x its text starts at and
/// the part of the graph that is on screen.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Spot {
    pub(crate) gap: Rect,
    pub(crate) row_h: f32,
    pub(crate) inset: f32,
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

/// The ink of a removed-lines count: red, or muted for a zero, which removes nothing.
pub(crate) fn removed_ink(removed: usize) -> egui::Color32 {
    if removed == 0 { brand::MUTED } else { brand::RED }
}

/// What follows the short id in the meta line: the author and how many files.
pub(crate) fn meta_tail(author: Option<&str>, files: usize) -> String {
    let mut parts = Vec::with_capacity(2);
    parts.extend(author);
    let count = if files == 1 { "file" } else { "files" };
    let files = format!("{files} {count}");
    parts.push(&files);
    parts.join(" · ")
}

/// The height of the gap under the open row for `files` files, with `room` left in the pane below
/// the row: a long list scrolls inside, but never gets smaller than a few rows.
pub(crate) fn gap_height(files: usize, room: f32) -> f32 {
    let wanted = META_H + files.max(1) as f32 * FILE_ROW_H + PAD_BOTTOM + MARGIN_BELOW;
    wanted.min(room.max(MIN_SHOWN))
}

/// Where a row's text starts, counted from the graph's left edge: after the last lane and the gutter
/// the uncommitted changes' lane takes.
pub(crate) fn message_inset(style: &GraphStyle, lanes: u16, pinned_wip: bool) -> f32 {
    let lane_w = style.lane_w.max(style.row_h / 2.0);
    let gutter = if pinned_wip { lane_w } else { 0.0 };
    let lanes = lanes.clamp(1, MAX_LANES);
    style.graph_lead + style.graph_tail + f32::from(lanes - 1) * lane_w + gutter
}

fn short_id(view: &TabView) -> Option<String> {
    let (id, _) = view.summary.as_ref()?;
    Some(id.to_hex_with_len(SHORT_ID).to_string())
}

fn meta_job(view: &TabView, files: usize) -> LayoutJob {
    let mut job = LayoutJob::default();
    let plain = |font: FontId, color| TextFormat { font_id: font, color, ..TextFormat::default() };
    if let Some(id) = short_id(view) {
        job.append(&id, 0.0, plain(mono(TEXT_TAG), brand::SOFT));
    }
    let author = view.summary.as_ref().map(|(_, summary)| summary.author.as_str());
    let tail = format!(" · {}", meta_tail(author, files));
    job.append(&tail, 0.0, plain(body(12.0), brand::MUTED));
    if let Some((added, removed)) = totals(&view.counts, files) {
        job.append(" · ", 0.0, plain(body(12.0), brand::MUTED));
        job.append(&format!("+{added}"), 0.0, plain(body(12.0), brand::GREEN));
        job.append(&format!(" -{removed}"), 0.0, plain(body(12.0), brand::RED));
    }
    job
}

/// Paints the card for the open commit over its row and the gap under it, clipped to the part of
/// the graph on screen.
pub(crate) fn show(ui: &mut Ui, view: &mut TabView, spot: Spot, waiting: &Waiting) {
    let Spot { gap, row_h, inset, viewport } = spot;
    let loading = view.is_loading_source();
    let files = if loading { 0 } else { view.files.len() };
    let card = Rect::from_min_max(
        pos2(gap.left(), gap.top() - row_h),
        pos2(gap.right(), gap.bottom() - MARGIN_BELOW),
    );
    let below = Rect::from_min_max(pos2(card.left(), gap.top()), card.max);
    let clip = viewport.intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    let fill = brand::TEAL.gamma_multiply_u8(FILL_ALPHA);
    painter.rect_filled(below, CornerRadius { nw: 0, ne: 0, sw: RADIUS, se: RADIUS }, fill);
    for left_side in [true, false] {
        painter.add(row_corner(
            pos2(if left_side { below.left() } else { below.right() }, gap.top()),
            left_side,
            fill,
        ));
    }
    ui.interact(below, Id::new("commit-files-card"), Sense::click_and_drag());

    let text_x = card.left() + inset;
    let meta_y = gap.top() + META_H / 2.0;
    if loading {
        selectable::text(
            ui,
            pos2(text_x, meta_y),
            Align2::LEFT_CENTER,
            "Loading files...",
            body(12.0),
            brand::MUTED,
        );
        return;
    }
    let galley = painter.layout_job(meta_job(view, files));
    selectable::galley(ui, pos2(text_x, meta_y - galley.size().y / 2.0), galley, brand::MUTED);
    if files == 0 {
        painter.text(
            pos2(text_x + ROW_PAD, gap.top() + META_H + FILE_ROW_H / 2.0),
            Align2::LEFT_CENTER,
            "Nothing changed here",
            body(13.0),
            brand::FAINT,
        );
        return;
    }
    let list = Rect::from_min_max(
        pos2(text_x, gap.top() + META_H),
        pos2(card.right() - SIDE_PAD, card.bottom() - PAD_BOTTOM),
    );
    let mut child = ui.new_child(UiBuilder::new().id_salt("commit-files-list").max_rect(list));
    child.set_clip_rect(list.intersect(clip));
    child.spacing_mut().item_spacing.y = 0.0;
    let commit = view.wanted_commit();
    let mut picked = None;
    ScrollArea::vertical().auto_shrink(false).show_rows(
        &mut child,
        FILE_ROW_H,
        files,
        |ui, rows| {
            for index in rows {
                let change = &view.files[index];
                let current = view.file.as_deref() == Some(change.path.as_str());
                let mark = Mark::of(current, view.file_open());
                let counts = view.counts.get(index).copied().flatten();
                let notes = waiting.file(commit, &change.path);
                if file_row(ui, change, counts, notes, mark) {
                    picked = Some(change.path.clone());
                }
            }
        },
    );
    if let Some(path) = picked {
        view.open_file(&path);
    }
}

/// What the graph's rounded selection leaves bare at a bottom corner of the open row, filled so the
/// row and the gap read as one card.
fn row_corner(corner: Pos2, left_side: bool, fill: Color32) -> Mesh {
    let sign = if left_side { 1.0 } else { -1.0 };
    let center = pos2(corner.x + sign * SELECTION_RADIUS, corner.y - SELECTION_RADIUS);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(corner, fill);
    for step in 0..=CORNER_STEPS {
        let angle = FRAC_PI_2 * step as f32 / CORNER_STEPS as f32;
        let at =
            center + vec2(-sign * SELECTION_RADIUS * angle.sin(), SELECTION_RADIUS * angle.cos());
        mesh.colored_vertex(at, fill);
    }
    for step in 0..CORNER_STEPS {
        mesh.add_triangle(0, step + 1, step + 2);
    }
    mesh
}

/// How a file row stands against the file the view has selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    None,
    Selected,
    /// The selected file is the one the file view shows.
    Shown,
}

impl Mark {
    fn of(current: bool, file_view: bool) -> Self {
        match (current, file_view) {
            (false, _) => Self::None,
            (true, false) => Self::Selected,
            (true, true) => Self::Shown,
        }
    }
}

fn file_row(
    ui: &mut Ui,
    change: &FileChange,
    counts: Option<LineCounts>,
    notes: usize,
    mark: Mark,
) -> bool {
    let (slot, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
    let painter = ui.painter_at(slot);
    let rect = slot.shrink2(vec2(0.0, ROW_GAP / 2.0));
    selection_row(
        &painter,
        rect,
        mark == Mark::Shown,
        mark == Mark::Selected || response.hovered(),
    );
    let center_y = rect.center().y;
    painter.text(
        pos2(rect.left() + ROW_PAD, center_y),
        Align2::LEFT_CENTER,
        kind_letter(change.kind),
        mono(TEXT_TAG),
        kind_color(change.kind),
    );
    let mut right = rect.right() - ROW_PAD;
    if let Some((added, removed)) = counts {
        painter.text(
            pos2(right, center_y),
            Align2::RIGHT_CENTER,
            format!("-{removed}"),
            mono(12.0),
            removed_ink(removed),
        );
        right -= DEL_W + COUNT_GAP;
        painter.text(
            pos2(right, center_y),
            Align2::RIGHT_CENTER,
            format!("+{added}"),
            mono(12.0),
            brand::GREEN,
        );
        right -= ADD_W;
    }
    if notes > 0 {
        right -= paint_notes_badge(&painter, right, center_y, notes) + COUNT_GAP;
    }
    let left = rect.left() + ROW_PAD + LETTER_W;
    let ink = selected_ink(mark == Mark::Shown, brand::TEXT);
    paint_path(ui, &painter, &change.path, (left, right - COUNT_GAP, center_y), ink);
    response.on_hover_text(&change.path).clicked()
}

fn notes_label(count: usize) -> String {
    if count > NOTES_MOST { format!("{NOTES_MOST}+") } else { count.to_string() }
}

/// A speech bubble and `count` (`99+` past 99) in teal with the badge's right edge at `right`.
/// Returns the badge's width.
fn paint_notes_badge(painter: &Painter, right: f32, mid: f32, count: usize) -> f32 {
    let galley = painter.layout_no_wrap(notes_label(count), body(TEXT_TAG), brand::TEAL);
    let width = NOTES_ICON + NOTES_ICON_GAP + galley.size().x;
    let left = right - width;
    let spot =
        Rect::from_center_size(pos2(left + NOTES_ICON / 2.0, mid), vec2(NOTES_ICON, NOTES_ICON));
    icons::paint(painter, spot, Icon::Comment, brand::TEAL);
    painter.galley(
        pos2(left + NOTES_ICON + NOTES_ICON_GAP, mid - galley.size().y / 2.0),
        galley,
        brand::TEAL,
    );
    width
}

pub(crate) fn paint_path(
    ui: &Ui,
    painter: &egui::Painter,
    path: &str,
    spot: (f32, f32, f32),
    ink: Color32,
) {
    let (left, right, y) = spot;
    let font = body(13.0);
    let (dir, name) = split_dir(path);
    let room = (right - left).max(0.0);
    let name_galley = painter.layout_no_wrap(name.to_owned(), font.clone(), ink);
    if name_galley.size().x >= room {
        let text = tail_fit(ui, name, &font, room);
        painter.text(pos2(left, y), Align2::LEFT_CENTER, text, font, ink);
        return;
    }
    let dir_text = tail_fit(ui, dir, &font, room - name_galley.size().x);
    let dir_galley = painter.layout_no_wrap(dir_text, font, brand::FAINT);
    let name_x = left + dir_galley.size().x;
    painter.galley(pos2(left, y - dir_galley.size().y / 2.0), dir_galley, brand::FAINT);
    painter.galley(pos2(name_x, y - name_galley.size().y / 2.0), name_galley, ink);
}

/// The folder with its trailing slash, then the file name.
pub(crate) fn split_dir(path: &str) -> (&str, &str) {
    path.rsplit_once('/').map_or(("", path), |(dir, name)| (&path[..=dir.len()], name))
}

/// `path` cut from the front with an ellipsis until it fits `room`, so the file name stays.
pub(crate) fn tail_fit(ui: &Ui, path: &str, font: &FontId, room: f32) -> String {
    let width = |text: &str| {
        ui.painter().layout_no_wrap(text.to_owned(), font.clone(), brand::TEXT).size().x
    };
    if path.is_empty() || width(path) <= room {
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
    fn a_file_with_many_notes_shows_99_plus() {
        let cases = [(1, "1"), (12, "12"), (99, "99"), (100, "99+"), (5000, "99+")];
        for (count, want) in cases {
            assert_eq!(notes_label(count), want, "{count}");
        }
    }

    #[test]
    fn the_badge_is_a_bubble_and_the_count_and_widens_with_it() {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut widths = Vec::new();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            for count in [1, 10, 100] {
                widths.push(paint_notes_badge(ui.painter(), 200.0, 10.0, count));
            }
        });
        assert!(widths[0] > NOTES_ICON + NOTES_ICON_GAP);
        assert!(widths[1] > widths[0]);
        assert!(widths[2] > widths[1]);
    }

    #[test]
    fn a_zero_removed_count_is_muted_and_a_real_one_is_red() {
        assert_eq!(removed_ink(0), brand::MUTED);
        assert_eq!(removed_ink(1), brand::RED);
    }

    #[test]
    fn totals_need_every_file_counted() {
        assert_eq!(totals(&[Some((3, 1)), Some((3, 0))], 2), Some((6, 1)));
        assert_eq!(totals(&[Some((3, 1)), None], 2), None);
        assert_eq!(totals(&[Some((3, 1))], 2), None);
        assert_eq!(totals(&[], 0), Some((0, 0)));
    }

    #[test]
    fn a_path_splits_into_its_folder_with_the_slash_and_the_file_name() {
        assert_eq!(split_dir("src/models/Image.ts"), ("src/models/", "Image.ts"));
        assert_eq!(split_dir("deno.json"), ("", "deno.json"));
        assert_eq!(split_dir("…ls/Image.ts"), ("…ls/", "Image.ts"));
    }

    #[test]
    fn only_the_selected_file_of_an_open_file_view_is_marked_as_shown() {
        assert_eq!(Mark::of(true, true), Mark::Shown);
        assert_eq!(Mark::of(true, false), Mark::Selected);
        assert_eq!(Mark::of(false, true), Mark::None);
        assert_eq!(Mark::of(false, false), Mark::None);
    }

    #[test]
    fn the_meta_line_names_the_author_and_how_many_files() {
        assert_eq!(meta_tail(Some("Builder"), 2), "Builder · 2 files");
        assert_eq!(meta_tail(None, 1), "1 file");
    }

    #[test]
    fn the_gap_grows_with_the_files_up_to_the_room_but_keeps_a_few_rows() {
        let one = META_H + FILE_ROW_H + PAD_BOTTOM + MARGIN_BELOW;
        assert_eq!(gap_height(1, 1000.0), one);
        assert_eq!(gap_height(0, 1000.0), one, "an empty list still has its message row");
        assert_eq!(gap_height(3, 1000.0), one + 2.0 * FILE_ROW_H);
        assert_eq!(gap_height(100, 500.0), 500.0);
        assert_eq!(gap_height(100, 50.0), MIN_SHOWN);
        assert_eq!(gap_height(1, 50.0), one);
    }

    #[test]
    fn the_text_starts_after_the_lanes_and_the_gutter_of_the_uncommitted_lane() {
        let style = GraphStyle::embedded(brand::TEAL);
        assert_eq!(message_inset(&style, 0, false), 34.0);
        assert_eq!(message_inset(&style, 1, false), 34.0);
        assert_eq!(message_inset(&style, 3, false), 34.0 + 2.0 * 18.0);
        assert_eq!(message_inset(&style, 1, true), 34.0 + 18.0);
        assert_eq!(message_inset(&style, 200, false), message_inset(&style, MAX_LANES, false));
    }
}
