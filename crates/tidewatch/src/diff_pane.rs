//! The Diff pane: what the selected commit (or the working tree) changed, as Kelp's diff body
//! with a file list above it and a toolbar for unified or split, jumping between changes and the
//! notes count. Notes written in the body go through `note_actions` into the tab's notes.

use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};

use egui::text::LayoutJob;
use egui::{
    Align2, Color32, FontId, Id, Rect, Sense, Stroke, TextFormat, Ui, UiBuilder, pos2, vec2,
};
use kelp_core::commit::{self, ChangeKind, FileChange};
use kelp_ui::diff::{DiffInput, DiffLayout, DiffMode, DiffStyle, DiffTexts};
use tidewatch_core::notes::Notes;

use crate::brand;
use crate::commands::Platform;
use crate::commit_files::paint_path;
use crate::file_view::{ARROW_W, BUTTON_H, GAP, arrow, note_buttons};
use crate::flow::PaneExtras;
use crate::history_pane;
use crate::layout::CHROME_H;
use crate::note_actions;
use crate::notes_strip::{self, StripAction};
use crate::screen_kit::{TEXT_SECONDARY, TEXT_TAG};
use crate::screen_kit::{body, heading, mono, selection_row};
use crate::segmented;
use crate::tab_view::TabView;

const SIDE_PAD: f32 = 14.0;
const PAD_RIGHT: f32 = 10.0;
const FILE_ROW_H: f32 = 28.0;
const FILES_MAX_ROWS: f32 = 4.0;
const FILES_PAD: f32 = 8.0;
const ROW_GAP: f32 = 2.0;
const ROW_PAD: f32 = 10.0;
const LETTER_W: f32 = 22.0;
const CAPTION_H: f32 = 28.0;
const MIN_TITLE_W: f32 = 120.0;
const TITLE_GAP: f32 = 2.0;
const LAYOUTS: [&str; 2] = ["Unified", "Split"];
pub(crate) const SHORT_ID: usize = 7;

static MAC_TEXTS: DiffTexts = DiffTexts {
    saved_hint: Cow::Borrowed("Draft: this note joins your review, sent to the agent in one go"),
    reply_hint: Cow::Borrowed("Reply...  (Cmd Enter to save)"),
    comment_hint: Cow::Borrowed("Note for your review...  (Cmd Enter to save)"),
};

static LINUX_TEXTS: DiffTexts = DiffTexts {
    saved_hint: Cow::Borrowed("Draft: this note joins your review, sent to the agent in one go"),
    reply_hint: Cow::Borrowed("Reply...  (Ctrl Enter to save)"),
    comment_hint: Cow::Borrowed("Note for your review...  (Ctrl Enter to save)"),
};

fn texts(platform: Platform) -> &'static DiffTexts {
    match platform {
        Platform::Mac => &MAC_TEXTS,
        Platform::Linux => &LINUX_TEXTS,
    }
}

pub(crate) fn kind_letter(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "A",
        ChangeKind::Modified => "M",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
    }
}

pub(crate) fn kind_color(kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added => brand::GREEN,
        ChangeKind::Modified => brand::ORANGE,
        ChangeKind::Deleted => brand::RED,
        ChangeKind::Renamed => brand::TEAL_TEXT,
    }
}

/// `3 open`, `3 open · 1 resolved`, or the hint when there are no notes yet.
pub(crate) fn notes_summary(notes: &Notes) -> String {
    let open = notes.open_count();
    let resolved = notes.threads.len() - open;
    match (open, resolved) {
        (0, 0) => "Hover a line, click + to leave a note".to_owned(),
        (open, 0) => format!("{open} open"),
        (open, resolved) => format!("{open} open · {resolved} resolved"),
    }
}

/// What the user asked for in the pane beyond editing notes in the diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneAction {
    SendReview,
    OpenReview,
    PullPr,
    OpenUrl(String),
    Strip(StripAction),
}

#[derive(Default)]
pub(crate) struct PaneOut {
    pub(crate) back: bool,
    pub(crate) notes_changed: bool,
    pub(crate) actions: Vec<PaneAction>,
}

/// Draws the pane as a card: a 48 px header with the commit and the note buttons, the files, a line
/// for the notes, the strip and the diff. `extras` is what an agent tab adds: the notes strip, the
/// send buttons and the PR chip; a tab without an agent passes `None`. `now` is unix seconds.
pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    extras: Option<&PaneExtras>,
    now: i64,
) -> PaneOut {
    history_pane::paint_card(ui, rect);
    let mut out = PaneOut::default();
    ui.scope_builder(UiBuilder::new().id_salt("diff-pane").max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        if view.loaded.is_none() {
            message(ui, rect, view.error.as_deref(), "Loading history...");
            return;
        }
        let inner = rect.shrink2(vec2(1.0, 0.0));
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), CHROME_H));
        ui.painter().hline(inner.x_range(), header.bottom(), Stroke::new(1.0, brand::CARD_LINE));
        show_header(ui, header, view, extras, &mut out.actions);
        let mut top = header.bottom();
        let files_h = FILE_ROW_H * (view.files.len() as f32).min(FILES_MAX_ROWS);
        let files = Rect::from_min_size(pos2(inner.left(), top), vec2(inner.width(), files_h));
        show_files(ui, files, view);
        top += files_h;
        let caption = Rect::from_min_size(pos2(inner.left(), top), vec2(inner.width(), CAPTION_H));
        show_caption(ui, caption, view);
        top += CAPTION_H;
        let strip_h = extras.map_or(0.0, |e| notes_strip::height(e.rows.len()));
        if let Some(extras) = extras.filter(|_| strip_h > 0.0) {
            let strip = Rect::from_min_size(pos2(inner.left(), top), vec2(inner.width(), strip_h));
            let actions = notes_strip::show(ui, strip, &extras.rows, now);
            out.actions.extend(actions.into_iter().map(PaneAction::Strip));
            top += strip_h;
        }
        ui.painter().hline(inner.x_range(), top, Stroke::new(1.0, brand::CARD_LINE));
        let body = Rect::from_min_max(pos2(inner.left(), top + 1.0), rect.max - vec2(1.0, 1.0));
        out.notes_changed = show_body(ui, body, view, DiffStyle::embedded(brand::TEAL));
        history_pane::round_bottom(ui, rect);
    });
    out
}

fn show_header(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    extras: Option<&PaneExtras>,
    actions: &mut Vec<PaneAction>,
) {
    let center_y = rect.center().y;
    let active = usize::from(view.layout == DiffLayout::Split);
    let switch = segmented::show(
        ui,
        Id::new("diff-pane-layout"),
        rect.right() - PAD_RIGHT,
        center_y,
        &LAYOUTS,
        active,
    );
    if let Some(index) = switch.clicked {
        set_layout(view, if index == 0 { DiffLayout::Unified } else { DiffLayout::Split });
    }
    let mut right = switch.left - GAP;
    if view.open.is_some() {
        right = change_arrows(ui, right, center_y, view);
    }
    let floor = rect.left() + SIDE_PAD + MIN_TITLE_W + GAP;
    if let Some(extras) = extras {
        right = note_buttons(ui, right, floor, center_y, extras, actions);
    }
    show_title(ui, Rect::from_min_max(rect.min, pos2(right - GAP, rect.bottom())), view);
}

fn change_arrows(ui: &mut Ui, right: f32, center_y: f32, view: &mut TabView) -> f32 {
    let top = center_y - BUTTON_H / 2.0;
    let next = Rect::from_min_size(pos2(right - ARROW_W, top), vec2(ARROW_W, BUTTON_H));
    let prev = next.translate(vec2(-ARROW_W - 2.0, 0.0));
    if let Some(open) = &mut view.open {
        if arrow(ui, Id::new("diff-pane-prev"), prev, true, true, "Previous change (Alt+Up)") {
            open.body.jump_to_change(false);
        }
        if arrow(ui, Id::new("diff-pane-next"), next, false, true, "Next change (Alt+Down)") {
            open.body.jump_to_change(true);
        }
    }
    prev.left() - GAP
}

/// The commit's title over its id, author and counts, cut with an ellipsis where `area` ends.
fn show_title(ui: &Ui, area: Rect, view: &TabView) {
    let HeaderText { what, title, who } = header_text(view);
    let width = (area.width() - SIDE_PAD).max(0.0);
    let painter = ui.painter().with_clip_rect(area);
    let one_line = |mut job: LayoutJob| {
        job.wrap.max_width = width;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        painter.layout_job(job)
    };
    let plain = |font: FontId, color| TextFormat { font_id: font, color, ..TextFormat::default() };
    let heading_job = LayoutJob::simple_singleline(title, heading(14.0), brand::TEXT);
    let mut meta = LayoutJob::default();
    meta.append(&what, 0.0, plain(mono(TEXT_TAG), brand::SOFT));
    meta.append(&format!(" · {who}"), 0.0, plain(body(12.0), brand::MUTED));
    if let Some(open) = &view.open {
        meta.append(&format!("  +{}", open.diff.added), 0.0, plain(body(12.0), brand::GREEN));
        meta.append(&format!(" -{}", open.diff.removed), 0.0, plain(body(12.0), brand::RED));
    }
    let (title, meta) = (one_line(heading_job), one_line(meta));
    let block = title.size().y + TITLE_GAP + meta.size().y;
    let top = area.center().y - block / 2.0;
    let x = area.left() + SIDE_PAD;
    painter.galley(pos2(x, top), title.clone(), brand::TEXT);
    painter.galley(pos2(x, top + title.size().y + TITLE_GAP), meta, brand::MUTED);
}

fn show_caption(ui: &Ui, rect: Rect, view: &TabView) {
    ui.painter().text(
        pos2(rect.left() + SIDE_PAD, rect.center().y),
        Align2::LEFT_CENTER,
        notes_summary(&view.notes),
        body(12.0),
        brand::FAINT,
    );
}

pub(crate) fn message(ui: &mut Ui, rect: Rect, error: Option<&str>, idle: &str) {
    let (text, color) = match error {
        Some(error) => (error, brand::RED),
        None => (idle, brand::FAINT),
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::proportional(TEXT_SECONDARY),
        color,
    );
}

struct HeaderText {
    what: String,
    title: String,
    who: String,
}

fn header_text(view: &TabView) -> HeaderText {
    if let (Some((id, summary)), Some(_)) = (&view.summary, view.wanted_commit()) {
        let when = commit::relative_time(summary.time, now_secs());
        return HeaderText {
            what: format!("commit {}", id.to_hex_with_len(SHORT_ID)),
            title: summary.title.clone(),
            who: format!("{} · {when}", summary.author),
        };
    }
    let count = view.changes().len();
    let files = if count == 1 { "file" } else { "files" };
    HeaderText {
        what: "working tree".to_owned(),
        title: "Uncommitted changes".to_owned(),
        who: format!("Live · {count} {files} still changing"),
    }
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn show_files(ui: &mut Ui, rect: Rect, view: &mut TabView) {
    if view.files.is_empty() {
        return;
    }
    let mut picked = None;
    ui.scope_builder(UiBuilder::new().id_salt("diff-files").max_rect(rect), |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for change in &view.files {
                if file_row(ui, change, view.file.as_deref() == Some(change.path.as_str())) {
                    picked = Some(change.path.clone());
                }
            }
        });
    });
    if let Some(path) = picked {
        view.select_file(&path);
    }
}

fn file_row(ui: &mut Ui, change: &FileChange, selected: bool) -> bool {
    let (slot, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
    let row = slot.shrink2(vec2(FILES_PAD, ROW_GAP));
    let painter = ui.painter_at(slot);
    selection_row(&painter, row, selected, response.hovered());
    painter.text(
        pos2(row.left() + ROW_PAD, row.center().y),
        Align2::LEFT_CENTER,
        kind_letter(change.kind),
        mono(TEXT_TAG),
        kind_color(change.kind),
    );
    let ink = if selected { brand::TEAL_TEXT } else { brand::SOFT };
    let left = row.left() + ROW_PAD + LETTER_W;
    paint_path(ui, &painter, &change.path, (left, row.right() - ROW_PAD, row.center().y), ink);
    response.clicked()
}

pub(crate) fn set_layout(view: &mut TabView, layout: DiffLayout) {
    view.layout = layout;
    if let Some(open) = &mut view.open {
        open.body.set_layout(layout);
    }
}

pub(crate) fn show_body(ui: &mut Ui, rect: Rect, view: &mut TabView, style: DiffStyle) -> bool {
    let wanted_commit = view.wanted_commit().map(|id| id.to_string());
    let Some(loaded) = &view.loaded else { return false };
    if view.is_loading_source() {
        message(ui, rect, view.error.as_deref(), "Loading...");
        return false;
    }
    let Some(open) = &mut view.open else {
        let idle = if view.files.is_empty() { "Nothing changed here" } else { "Loading..." };
        message(ui, rect, view.error.as_deref(), idle);
        return false;
    };
    let path = open.diff.path.clone();
    let mut actions = Vec::new();
    ui.scope_builder(UiBuilder::new().id_salt("diff-body").max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        open.body.jump_with_keys(ui);
        actions = open.body.ui(
            ui,
            DiffInput {
                diff: &open.diff,
                threads: &view.notes.threads,
                author: &loaded.author,
                mode: DiffMode::Diff,
                stacked: false,
                hunk_buttons: &[],
                line_select_hint: None,
                texts: texts(Platform::CURRENT),
                style,
            },
        );
    });
    note_actions::apply(&mut view.notes, &path, wanted_commit.as_deref(), actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment, Side};

    #[test]
    fn the_save_hint_names_the_platforms_key() {
        assert!(texts(Platform::Mac).comment_hint.contains("Cmd Enter"));
        assert!(texts(Platform::Linux).comment_hint.contains("Ctrl Enter"));
        assert!(texts(Platform::Linux).reply_hint.contains("Ctrl Enter"));
    }

    fn notes_with(open: usize, resolved: usize) -> Notes {
        let mut notes = Notes::default();
        for n in 0..open + resolved {
            let anchor = Anchor {
                side: Side::New,
                line_hint: 1,
                text: String::new(),
                before: vec![],
                after: vec![],
                span: 0,
                block: vec![],
            };
            let comment = Comment { author: "You".into(), time: 0, body: "x".into() };
            let id = notes.add("a.rs", None, anchor, comment);
            if n >= open {
                notes.set_resolved(id, true);
            }
        }
        notes
    }

    #[test]
    fn the_notes_summary_counts_open_and_resolved_threads() {
        assert_eq!(notes_summary(&notes_with(0, 0)), "Hover a line, click + to leave a note");
        assert_eq!(notes_summary(&notes_with(3, 0)), "3 open");
        assert_eq!(notes_summary(&notes_with(2, 1)), "2 open · 1 resolved");
        assert_eq!(notes_summary(&notes_with(0, 2)), "0 open · 2 resolved");
    }

    #[test]
    fn each_change_kind_has_its_own_letter() {
        let letters: Vec<_> =
            [ChangeKind::Added, ChangeKind::Modified, ChangeKind::Deleted, ChangeKind::Renamed]
                .map(kind_letter)
                .into();
        assert_eq!(letters, ["A", "M", "D", "R"]);
    }
}
