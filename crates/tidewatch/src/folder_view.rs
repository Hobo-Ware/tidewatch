//! What a folder tab's panel draws that a repo tab's does not: the repo picker chip in the card
//! header (a dropdown in the look of the History scope switch, each repo with its branch), and, for
//! a folder with no git repo, the plain folder view: its path, one line saying so, and the files
//! changed in the last hour.

use std::path::{Path, PathBuf};

use egui::{Align2, CornerRadius, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use tidewatch_core::folder_repos::{self, FolderRepo, MAX_RECENT, Scan};

use crate::brand;
use crate::changes_view::{self, ROW_H};
use crate::history_pane::CARD_PAD;
use crate::screen_kit::{TEXT_SECONDARY, TEXT_TAG, body, mono};
use crate::search_bar::{self, CHIP_HEIGHT};
use crate::selectable;

pub(crate) const NO_REPO: &str = "No git repo in this folder";
pub(crate) const NOTHING_RECENT: &str = "Nothing changed in the last hour";
const MENU_W: f32 = 300.0;
const MENU_MAX_H: f32 = 320.0;
const MENU_ROW_H: f32 = 28.0;
const LABEL_CHARS: [usize; 3] = [24, 14, 8];
const CHIP_GAP: f32 = 10.0;
const SIDE_PAD: f32 = 8.0;
const ROW_PAD: f32 = 10.0;
const AGE_W: f32 = 64.0;
const INTRO_H: f32 = 64.0;

/// The repos of the folder and the one the panel reads.
pub(crate) struct Head<'a> {
    pub(crate) repos: &'a [FolderRepo],
    pub(crate) picked: &'a Path,
    /// Keeps the list open, for `TIDEWATCH_DEV_FOLDER_PICKER=1` captures.
    pub(crate) open: bool,
}

pub(crate) fn short_name(name: &str, chars: usize) -> String {
    if name.chars().count() <= chars {
        return name.to_owned();
    }
    let kept: String = name.chars().take(chars.saturating_sub(3)).collect();
    format!("{kept}...")
}

pub(crate) fn age_text(secs: i64) -> String {
    match secs {
        ..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", secs / 60),
        _ => format!("{}h ago", secs / 3600),
    }
}

fn picked_label(head: &Head<'_>) -> String {
    head.repos
        .iter()
        .find(|repo| repo.root == head.picked)
        .map(|repo| repo.name.clone())
        .or_else(|| head.picked.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

fn fitted_label(ui: &Ui, name: &str, room: f32) -> String {
    let labels: Vec<String> = LABEL_CHARS.iter().map(|chars| short_name(name, *chars)).collect();
    labels
        .iter()
        .find(|label| search_bar::chip_width(ui, label) <= room)
        .or(labels.last())
        .cloned()
        .unwrap_or_default()
}

/// Draws the picker chip with its left edge at `left` and returns its right edge and the root the
/// owner chose in the list.
pub(crate) fn picker(
    ui: &mut Ui,
    (left, right): (f32, f32),
    center_y: f32,
    head: &Head<'_>,
) -> (f32, Option<PathBuf>) {
    let label = fitted_label(ui, &picked_label(head), (right - left - 80.0).max(0.0));
    let width = search_bar::chip_width(ui, &label);
    let chip =
        Rect::from_min_size(pos2(left, center_y - CHIP_HEIGHT / 2.0), vec2(width, CHIP_HEIGHT));
    let response = ui
        .interact(chip, ui.id().with("folder-repo-picker"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if head.open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    search_bar::paint_chip(ui, chip, &label, response.hovered() || open);
    let mut chosen = None;
    egui::Popup::menu(&response).show(|ui| {
        ui.set_width(MENU_W);
        ScrollArea::vertical().max_height(MENU_MAX_H).show(ui, |ui| {
            for repo in head.repos {
                if repo_row(ui, repo, repo.root == head.picked) {
                    chosen = Some(repo.root.clone());
                }
            }
        });
    });
    (chip.right() + CHIP_GAP, chosen)
}

fn repo_row(ui: &mut Ui, repo: &FolderRepo, picked: bool) -> bool {
    let (slot, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), MENU_ROW_H), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter_at(slot);
    if response.hovered() {
        painter.rect_filled(slot, CornerRadius::same(6), brand::ITEM);
    }
    let branch = repo.branch.as_deref().unwrap_or("detached HEAD");
    let branch = painter.layout_no_wrap(short_name(branch, 28), mono(TEXT_TAG), brand::MUTED);
    let branch_w = branch.size().x;
    painter.galley(
        pos2(slot.right() - ROW_PAD - branch_w, slot.center().y - branch.size().y / 2.0),
        branch,
        brand::MUTED,
    );
    let ink = if picked { brand::TEAL_TEXT } else { brand::TEXT };
    let room = (slot.width() - ROW_PAD * 2.0 - branch_w - 12.0).max(0.0);
    let name =
        crate::reader_view::ellipsized(&painter, &repo.name, body(TEXT_SECONDARY), ink, room);
    painter.galley(pos2(slot.left() + ROW_PAD, slot.center().y - name.size().y / 2.0), name, ink);
    response.clicked()
}

pub(crate) fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map_or_else(|| folder.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The folder's path with the home folder as `~`.
pub(crate) fn folder_title(folder: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    folder_repos::tilde(folder, home.as_deref())
}

/// The plain folder view under the header.
pub(crate) fn show(ui: &mut Ui, rect: Rect, scan: &Scan, now_ms: i64) {
    let left = rect.left() + CARD_PAD;
    selectable::text(
        ui,
        pos2(left, rect.top() + 22.0),
        Align2::LEFT_CENTER,
        &folder_title(&scan.folder),
        mono(TEXT_SECONDARY),
        brand::TEXT,
    );
    selectable::text(
        ui,
        pos2(left, rect.top() + 44.0),
        Align2::LEFT_CENTER,
        NO_REPO,
        body(TEXT_SECONDARY),
        brand::MUTED,
    );
    let list = Rect::from_min_max(pos2(rect.left(), rect.top() + INTRO_H), rect.max);
    ui.painter().hline(list.x_range(), list.top(), egui::Stroke::new(1.0, brand::CARD_LINE));
    let (header, body_area) = {
        let head = Rect::from_min_size(list.min, vec2(list.width(), ROW_H));
        (head, Rect::from_min_max(pos2(list.left(), head.bottom()), list.max))
    };
    section_row(ui, header, scan.recent.len());
    if scan.recent.is_empty() {
        selectable::text(
            ui,
            pos2(body_area.center().x, body_area.top() + 40.0),
            Align2::CENTER_CENTER,
            NOTHING_RECENT,
            body(TEXT_SECONDARY),
            brand::FAINT,
        );
        return;
    }
    let mut child = ui.new_child(UiBuilder::new().id_salt("folder-recent").max_rect(body_area));
    child.set_clip_rect(body_area.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing.y = 0.0;
    ScrollArea::vertical().auto_shrink(false).show_rows(
        &mut child,
        ROW_H,
        scan.recent.len(),
        |ui, span| {
            for file in &scan.recent[span] {
                file_row(ui, &file.path, (now_ms - file.modified_ms) / 1000);
            }
        },
    );
}

fn section_row(ui: &Ui, slot: Rect, count: usize) {
    let left = slot.left() + SIDE_PAD + ROW_PAD;
    let title =
        ui.painter().layout_no_wrap("Changed in the last hour".to_owned(), body(12.0), brand::SOFT);
    let width = title.size().x;
    selectable::galley(ui, pos2(left, slot.center().y - title.size().y / 2.0), title, brand::SOFT);
    let shown = if count >= MAX_RECENT { format!("{MAX_RECENT}+") } else { count.to_string() };
    selectable::text(
        ui,
        pos2(left + width + 6.0, slot.center().y),
        Align2::LEFT_CENTER,
        &shown,
        mono(TEXT_TAG),
        brand::FAINT,
    );
}

fn file_row(ui: &mut Ui, path: &str, age_secs: i64) {
    let (slot, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let painter = ui.painter_at(slot);
    let rect = slot.shrink2(vec2(SIDE_PAD, 1.0));
    let right = rect.right() - ROW_PAD;
    painter.text(
        pos2(right, rect.center().y),
        Align2::RIGHT_CENTER,
        age_text(age_secs),
        mono(12.0),
        brand::FAINT,
    );
    changes_view::paint_name(
        ui,
        &painter,
        path,
        (rect.left() + ROW_PAD, right - AGE_W - 8.0),
        rect.center().y,
    );
}

#[cfg(test)]
mod tests;
