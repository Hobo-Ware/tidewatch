//! The Changes side of the History card: the uncommitted files in three groups (Staged, Changed,
//! New files), one fixed-height line each, and a footer with the totals. It only lists; a click
//! opens the file's diff in the main area and nothing here stages, discards or commits.

use std::borrow::Cow;
use std::collections::HashMap;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2,
};
use kelp_core::commit::{ChangeKind, FileChange};
use kelp_core::status::is_folder;
use tidewatch_core::changes::Groups;
use tidewatch_core::history::Source;

use crate::brand;
use crate::commit_files::{split_dir, tail_fit, totals};
use crate::diff_pane::{kind_color, kind_letter};
use crate::pane_jobs::LineCounts;
use crate::reader_view::ellipsized;
use crate::screen_kit::{TEXT_SECONDARY, TEXT_TAG};
use crate::screen_kit::{body, mono};
use crate::selectable;
use crate::tab_view::TabView;

pub(crate) const ROW_H: f32 = 30.0;
pub(crate) const FOOTER_H: f32 = 36.0;
const SIDE_PAD: f32 = 8.0;
const ROW_PAD: f32 = 10.0;
const LETTER_W: f32 = 22.0;
const ADD_W: f32 = 44.0;
const DEL_W: f32 = 36.0;
const COUNT_GAP: f32 = 8.0;
const DIR_GAP: f32 = 8.0;
const RADIUS: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Staged,
    Changed,
    New,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Self::Staged => "Staged",
            Self::Changed => "Changed",
            Self::New => "New files",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row<'a> {
    Header { group: Group, count: usize },
    File { group: Group, change: &'a FileChange },
}

pub(crate) fn rows(groups: &Groups) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    for (group, files) in [
        (Group::Staged, &groups.staged),
        (Group::Changed, &groups.changed),
        (Group::New, &groups.new_files),
    ] {
        if files.is_empty() {
            continue;
        }
        rows.push(Row::Header { group, count: files.len() });
        rows.extend(files.iter().map(|change| Row::File { group, change }));
    }
    rows
}

pub(crate) fn letter(group: Group, kind: ChangeKind) -> (&'static str, Color32) {
    match group {
        Group::New => ("?", brand::BLUE),
        Group::Staged | Group::Changed => (kind_letter(kind), kind_color(kind)),
    }
}

pub(crate) fn footer_text(files: usize) -> String {
    match files {
        0 => "Nothing changed since the last commit".to_owned(),
        1 => "1 file changed since the last commit".to_owned(),
        n => format!("{n} files changed since the last commit"),
    }
}

fn counts_by_path(view: &TabView) -> HashMap<&str, LineCounts> {
    if view.shown_source != Some(Source::WorkTree) {
        return HashMap::new();
    }
    view.files
        .iter()
        .zip(&view.counts)
        .filter_map(|(change, counts)| Some((change.path.as_str(), (*counts)?)))
        .collect()
}

fn work_tree_totals(view: &TabView, files: usize) -> Option<LineCounts> {
    (view.shown_source == Some(Source::WorkTree) && view.files.len() == files)
        .then(|| totals(&view.counts, files))
        .flatten()
}

pub(crate) fn show(ui: &mut Ui, rect: Rect, view: &mut TabView, side: bool) {
    view.show_work_tree();
    let footer = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOTER_H), rect.max);
    let list = Rect::from_min_max(rect.min, pos2(rect.right(), footer.top()));
    let Some(groups) = view.groups.take() else {
        message(ui, list, "Loading changes...");
        paint_footer(ui, footer, "", None);
        return;
    };
    let files = groups.files();
    let diffable = files.saturating_sub(groups.folders.len());
    let picked = if files == 0 {
        message(ui, list, "No uncommitted changes");
        None
    } else {
        list_files(ui, list, view, &groups, side)
    };
    view.groups = Some(groups);
    match picked {
        Some(path) if side => view.open_file(&path),
        Some(path) => view.select_file(&path),
        None => {}
    }
    paint_footer(ui, footer, &footer_text(files), work_tree_totals(view, diffable));
}

fn message(ui: &Ui, area: Rect, text: &str) {
    selectable::text(
        ui,
        pos2(area.center().x, area.top() + 40.0),
        Align2::CENTER_CENTER,
        text,
        body(TEXT_SECONDARY),
        brand::FAINT,
    );
}

fn paint_footer(ui: &Ui, footer: Rect, text: &str, counts: Option<LineCounts>) {
    let painter = ui.painter();
    painter.hline(footer.x_range(), footer.top(), egui::Stroke::new(1.0, brand::CARD_LINE));
    let mut job = LayoutJob::default();
    let plain = |color| TextFormat { font_id: body(12.0), color, ..TextFormat::default() };
    job.append(text, 0.0, plain(brand::MUTED));
    if let Some((added, removed)) = counts {
        job.append(" · ", 0.0, plain(brand::MUTED));
        job.append(&format!("+{added}"), 0.0, plain(brand::GREEN));
        job.append(&format!(" -{removed}"), 0.0, plain(brand::RED));
    }
    let galley = painter.layout_job(job);
    let at = pos2(footer.left() + SIDE_PAD + ROW_PAD, footer.center().y - galley.size().y / 2.0);
    selectable::galley(ui, at, galley, brand::MUTED);
}

fn list_files(
    ui: &mut Ui,
    area: Rect,
    view: &TabView,
    groups: &Groups,
    side: bool,
) -> Option<String> {
    let rows = rows(groups);
    let counts = counts_by_path(view);
    let shown = view.file.as_deref().filter(|_| view.file_open() || !side);
    let mut picked = None;
    let mut child = ui.new_child(UiBuilder::new().id_salt("changes-list").max_rect(area));
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing.y = 0.0;
    ScrollArea::vertical().auto_shrink(false).show_rows(
        &mut child,
        ROW_H,
        rows.len(),
        |ui, span| {
            for row in &rows[span] {
                match *row {
                    Row::Header { group, count } => header_row(ui, group, count),
                    Row::File { group, change } => {
                        let current = shown == Some(change.path.as_str());
                        let counted = counts.get(change.path.as_str()).copied();
                        let folder = groups.folder_files(&change.path);
                        if file_row(ui, (group, change), (counted, folder), current) {
                            picked = Some(change.path.clone());
                        }
                    }
                }
            }
        },
    );
    picked
}

fn header_row(ui: &mut Ui, group: Group, count: usize) {
    let (slot, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let painter = ui.painter_at(slot);
    let left = slot.left() + SIDE_PAD + ROW_PAD;
    let title = painter.layout_no_wrap(group.title().to_owned(), body(12.0), brand::SOFT);
    let width = title.size().x;
    selectable::galley(
        ui,
        pos2(left, slot.center().y - title.size().y / 2.0 + 1.0),
        title,
        brand::SOFT,
    );
    selectable::text(
        ui,
        pos2(left + width + 6.0, slot.center().y + 1.0),
        Align2::LEFT_CENTER,
        &count.to_string(),
        mono(TEXT_TAG),
        brand::FAINT,
    );
}

fn file_row(
    ui: &mut Ui,
    (group, change): (Group, &FileChange),
    (counts, folder): (Option<LineCounts>, Option<usize>),
    current: bool,
) -> bool {
    let sense = if folder.is_some() { Sense::hover() } else { Sense::click() };
    let (slot, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
    let painter = ui.painter_at(slot);
    let rect = slot.shrink2(vec2(SIDE_PAD, 1.0));
    if current {
        painter.rect_filled(rect, CornerRadius::same(RADIUS), brand::SEGMENT_ON);
    } else if response.hovered() && folder.is_none() {
        painter.rect_filled(rect, CornerRadius::same(RADIUS), brand::ITEM);
    }
    let center_y = rect.center().y;
    let (mark, ink) = letter(group, change.kind);
    painter.text(
        pos2(rect.left() + ROW_PAD, center_y),
        Align2::LEFT_CENTER,
        mark,
        mono(TEXT_TAG),
        ink,
    );
    let mut right = rect.right() - ROW_PAD;
    if let Some((added, removed)) = counts {
        painter.text(
            pos2(right, center_y),
            Align2::RIGHT_CENTER,
            format!("-{removed}"),
            mono(12.0),
            brand::RED,
        );
        painter.text(
            pos2(right - DEL_W - COUNT_GAP, center_y),
            Align2::RIGHT_CENTER,
            format!("+{added}"),
            mono(12.0),
            brand::GREEN,
        );
    }
    if let Some(files) = folder {
        painter.text(
            pos2(right, center_y),
            Align2::RIGHT_CENTER,
            folder_count(files),
            mono(12.0),
            brand::FAINT,
        );
    }
    right -= DEL_W + COUNT_GAP + ADD_W + COUNT_GAP;
    paint_name(ui, &painter, &change.path, (rect.left() + ROW_PAD + LETTER_W, right), center_y);
    let tip = folder.map_or_else(
        || change.path.clone(),
        |files| format!("{} - {} untracked", change.path, folder_count(files)),
    );
    response.on_hover_text(tip).clicked()
}

fn folder_count(files: usize) -> String {
    match files {
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    }
}

fn paint_name(ui: &Ui, painter: &egui::Painter, path: &str, (left, right): (f32, f32), y: f32) {
    let (dir, name) = split_dir(path.trim_end_matches('/'));
    let name = if is_folder(path) { Cow::Owned(format!("{name}/")) } else { Cow::Borrowed(name) };
    let font: FontId = body(13.0);
    let room = (right - left).max(0.0);
    let name_galley = ellipsized(painter, &name, font.clone(), brand::TEXT, room);
    let name_w = name_galley.size().x;
    painter.galley(pos2(left, y - name_galley.size().y / 2.0), name_galley, brand::TEXT);
    let dir_room = room - name_w - DIR_GAP;
    if dir.is_empty() || dir_room < 24.0 {
        return;
    }
    let text = tail_fit(ui, dir.trim_end_matches('/'), &font, dir_room);
    let galley = painter.layout_no_wrap(text, body(12.0), brand::FAINT);
    painter.galley(pos2(left + name_w + DIR_GAP, y - galley.size().y / 2.0), galley, brand::FAINT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::status::parse_porcelain;

    fn groups() -> Groups {
        Groups::of(&parse_porcelain("M  a.rs\0MM b.rs\0 D c.rs\0?? d.md\0?? e/f.md\0"))
    }

    fn describe(rows: &[Row<'_>]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Header { group, count } => format!("{} {count}", group.title()),
                Row::File { group, change } => format!("{group:?}:{}", change.path),
            })
            .collect()
    }

    #[test]
    fn the_list_is_a_header_and_its_files_for_each_group_that_has_files() {
        assert_eq!(
            describe(&rows(&groups())),
            [
                "Staged 2",
                "Staged:a.rs",
                "Staged:b.rs",
                "Changed 2",
                "Changed:b.rs",
                "Changed:c.rs",
                "New files 2",
                "New:d.md",
                "New:e/f.md",
            ]
        );
    }

    #[test]
    fn a_group_with_no_files_has_no_header() {
        let only_new = Groups::of(&parse_porcelain("?? a.txt\0"));
        assert_eq!(describe(&rows(&only_new)), ["New files 1", "New:a.txt"]);
        assert_eq!(rows(&Groups::default()).len(), 0);
    }

    #[test]
    fn each_status_has_its_letter_and_color() {
        let cases = [
            (Group::Changed, ChangeKind::Modified, "M", brand::ORANGE),
            (Group::Staged, ChangeKind::Added, "A", brand::GREEN),
            (Group::Changed, ChangeKind::Deleted, "D", brand::RED),
            (Group::New, ChangeKind::Added, "?", brand::BLUE),
        ];
        for (group, kind, want, color) in cases {
            assert_eq!(letter(group, kind), (want, color), "{group:?} {kind:?}");
        }
    }

    #[test]
    fn the_footer_counts_files_and_says_so_for_none_and_one() {
        assert_eq!(footer_text(5), "5 files changed since the last commit");
        assert_eq!(footer_text(1), "1 file changed since the last commit");
        assert_eq!(footer_text(0), "Nothing changed since the last commit");
    }

    use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Shape};

    struct Panel {
        ctx: Context,
        view: TabView,
        side: bool,
        texts: Vec<(String, Rect)>,
    }

    fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Text(text) => out.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
            _ => {}
        }
    }

    impl Panel {
        fn new(porcelain: &str, side: bool) -> Self {
            let mut view = TabView::new(1, "/work/web".into(), false, None);
            view.groups = Some(Groups::of(&parse_porcelain(porcelain)));
            let mut panel = Self { ctx: crate::fonts::test_context(), view, side, texts: vec![] };
            panel.frame(Vec::new());
            panel.frame(Vec::new());
            panel
        }

        fn frame(&mut self, events: Vec<Event>) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 600.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let (view, side) = (&mut self.view, self.side);
            let output = self.ctx.run_ui(raw, |ui| {
                let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 600.0));
                show(ui, rect, view, side);
            });
            self.texts.clear();
            for clipped in &output.shapes {
                collect(&clipped.shape, &mut self.texts);
            }
        }

        fn at(&self, text: &str) -> Option<Pos2> {
            self.texts.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| rect.center())
        }

        fn click(&mut self, at: Pos2) {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(Vec::new());
        }
    }

    const FEW: &str = "M  a.rs\0";
    const MANY: &str = "M  a.rs\0MM b.rs\0 D c.rs\0?? d.md\0?? e/f.md\0 M g.rs\0";

    #[test]
    fn the_rows_and_the_footer_do_not_move_when_files_come_and_go() {
        let few = Panel::new(FEW, true);
        let many = Panel::new(MANY, true);
        let top = |panel: &Panel, text: &str| {
            panel.texts.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| rect.top())
        };
        assert_eq!(top(&few, "Staged"), top(&many, "Staged"));
        assert_eq!(top(&few, "a.rs"), top(&many, "a.rs"));
        let footer = |panel: &Panel| {
            panel
                .texts
                .iter()
                .find(|(text, _)| text.contains("changed since the last commit"))
                .map(|(_, rect)| rect.top())
        };
        assert!(footer(&few).is_some());
        assert_eq!(footer(&few), footer(&many));
    }

    #[test]
    fn the_footer_counts_the_distinct_files() {
        let panel = Panel::new(MANY, true);
        let says = panel
            .texts
            .iter()
            .any(|(text, _)| text.starts_with("6 files changed since the last commit"));
        assert!(says, "{:?}", panel.texts);
    }

    #[test]
    fn a_click_on_a_file_opens_its_diff_in_place_of_the_terminal_in_the_side_layout() {
        let mut panel = Panel::new(MANY, true);
        let row = panel.at("d.md").unwrap();
        panel.click(row);
        assert_eq!(panel.view.file.as_deref(), Some("d.md"));
        assert!(panel.view.file_open());
    }

    #[test]
    fn a_click_on_a_file_picks_it_for_the_diff_pane_below_in_the_other_layout() {
        let mut panel = Panel::new(MANY, false);
        let row = panel.at("g.rs").unwrap();
        panel.click(row);
        assert_eq!(panel.view.file.as_deref(), Some("g.rs"));
        assert!(!panel.view.file_open());
    }

    const WITH_FOLDER: &str = "M  a.rs\0?? notes/\0?? deep/er/\0?? z.md\0";

    fn folder_panel(side: bool) -> Panel {
        let mut status = parse_porcelain(WITH_FOLDER);
        status.folders[0].files = 300;
        status.folders[1].files = 1;
        let mut panel = Panel::new("", side);
        panel.view.groups = Some(Groups::of(&status));
        panel.frame(Vec::new());
        panel.frame(Vec::new());
        panel
    }

    #[test]
    fn an_untracked_folder_is_one_row_among_the_new_files() {
        let groups = Groups::of(&parse_porcelain(WITH_FOLDER));
        assert_eq!(
            describe(&rows(&groups)),
            ["Staged 1", "Staged:a.rs", "New files 3", "New:deep/er/", "New:notes/", "New:z.md"]
        );
    }

    #[test]
    fn a_folder_row_shows_its_name_with_the_slash_and_how_many_files_it_holds() {
        let panel = folder_panel(true);
        assert!(panel.at("notes/").is_some(), "{:?}", panel.texts);
        assert!(panel.at("er/").is_some(), "{:?}", panel.texts);
        assert!(panel.at("deep").is_some(), "{:?}", panel.texts);
        assert!(panel.at("300 files").is_some(), "{:?}", panel.texts);
        assert!(panel.at("1 file").is_some(), "{:?}", panel.texts);
    }

    #[test]
    fn the_footer_counts_a_folder_as_one_entry_like_git_status() {
        let panel = folder_panel(true);
        let says = panel
            .texts
            .iter()
            .any(|(text, _)| text.starts_with("4 files changed since the last commit"));
        assert!(says, "{:?}", panel.texts);
    }

    #[test]
    fn the_footer_keeps_the_line_totals_of_the_real_files_beside_a_folder() {
        let mut panel = folder_panel(true);
        panel.view.files = ["a.rs", "z.md"]
            .map(|path| FileChange { path: path.into(), kind: ChangeKind::Modified })
            .to_vec();
        panel.view.counts = vec![Some((2, 1)), Some((1, 0))];
        panel.view.shown_source = Some(Source::WorkTree);
        panel.frame(Vec::new());
        let says = panel.texts.iter().any(|(text, _)| {
            text.starts_with("4 files changed since the last commit") && text.ends_with("+3 -1")
        });
        assert!(says, "{:?}", panel.texts);
    }

    #[test]
    fn a_click_on_a_folder_row_opens_nothing_in_either_layout() {
        for side in [true, false] {
            let mut panel = folder_panel(side);
            let row = panel.at("notes/").unwrap();
            panel.click(row);
            assert_eq!(panel.view.file, None, "side {side}");
            assert!(!panel.view.file_open(), "side {side}");
        }
    }

    #[test]
    fn the_tab_views_and_never_changes_anything() {
        let panel = Panel::new(MANY, true);
        for word in ["Stage", "Unstage", "Discard", "Commit", "Revert", "Reset"] {
            assert!(
                !panel.texts.iter().any(|(text, _)| text == word),
                "{word} is on the Changes tab"
            );
        }
    }

    #[test]
    fn a_clean_tree_says_so_and_a_status_that_has_not_loaded_says_loading() {
        let clean = Panel::new("", true);
        assert!(clean.at("No uncommitted changes").is_some());
        assert!(clean.at("Nothing changed since the last commit").is_some());
        let mut loading = Panel::new("", true);
        loading.view.groups = None;
        loading.frame(Vec::new());
        assert!(loading.at("Loading changes...").is_some());
    }
}
