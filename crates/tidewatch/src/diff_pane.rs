//! The Diff pane: what the selected commit (or the working tree) changed, as Kelp's diff body
//! with a file list above it and a toolbar for unified or split, jumping between changes and the
//! notes count. Notes written in the body go through `note_actions` into the tab's notes.

use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{
    Align, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};
use kelp_core::commit::{self, ChangeKind, FileChange};
use kelp_ui::diff::{DiffInput, DiffLayout, DiffMode, DiffTexts};
use kelp_ui::theme;
use stdusk_egui::ui::icons;
use stdusk_egui::widgets::{chip, icon_button};
use tidewatch_core::notes::Notes;

use crate::brand;
use crate::commands::Platform;
use crate::flow::PaneExtras;
use crate::note_actions;
use crate::notes_strip::{self, StripAction};
use crate::tab_view::TabView;

const SIDE_PAD: f32 = 14.0;
const FILE_ROW_H: f32 = 22.0;
const FILES_MAX_ROWS: f32 = 4.0;
const TOOLBAR_H: f32 = 34.0;
const HEADER_H: f32 = 64.0;
const SHORT_ID: usize = 7;

static MAC_TEXTS: DiffTexts = DiffTexts {
    saved_hint: Cow::Borrowed("Notes go to this tab's agent when you send them"),
    reply_hint: Cow::Borrowed("Reply...  (Cmd Enter to save)"),
    comment_hint: Cow::Borrowed("Note for the agent...  (Cmd Enter to save)"),
};

static LINUX_TEXTS: DiffTexts = DiffTexts {
    saved_hint: Cow::Borrowed("Notes go to this tab's agent when you send them"),
    reply_hint: Cow::Borrowed("Reply...  (Ctrl Enter to save)"),
    comment_hint: Cow::Borrowed("Note for the agent...  (Ctrl Enter to save)"),
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

fn kind_color(kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added => theme::added(),
        ChangeKind::Modified => theme::modified(),
        ChangeKind::Deleted => theme::deleted(),
        ChangeKind::Renamed => theme::hunk_text(),
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
    SendAll,
    ToggleAuto,
    PullPr,
    OpenUrl(String),
    Strip(StripAction),
}

#[derive(Default)]
pub(crate) struct PaneOut {
    pub(crate) notes_changed: bool,
    pub(crate) actions: Vec<PaneAction>,
}

/// Draws the pane. `extras` is what an agent tab adds: the notes strip, the send buttons and the PR
/// chip; a tab without an agent passes `None`. `now` is unix seconds.
pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    extras: Option<&PaneExtras>,
    now: i64,
) -> PaneOut {
    ui.painter().rect_filled(rect, 0.0, theme::bg());
    let mut out = PaneOut::default();
    ui.scope_builder(UiBuilder::new().id_salt("diff-pane").max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        if view.loaded.is_none() {
            message(ui, rect, view.error.as_deref(), "Loading history...");
            return;
        }
        let header = take_row(ui, rect, 0.0, HEADER_H);
        show_header(ui, header, view);
        if let Some(extras) = extras {
            header_actions(ui, header, extras, &mut out.actions);
        }
        let top = HEADER_H;
        let files_h = FILE_ROW_H * (view.files.len() as f32).min(FILES_MAX_ROWS);
        let files = take_row(ui, rect, top, files_h);
        show_files(ui, files, view);
        let toolbar = take_row(ui, rect, top + files_h, TOOLBAR_H);
        show_toolbar(ui, toolbar, view);
        if let Some(extras) = extras {
            toolbar_pr(ui, toolbar, extras, &mut out.actions);
        }
        let strip_h = extras.map_or(0.0, |e| notes_strip::height(e.rows.len()));
        if let Some(extras) = extras.filter(|_| strip_h > 0.0) {
            let strip = take_row(ui, rect, top + files_h + TOOLBAR_H, strip_h);
            let actions = notes_strip::show(ui, strip, &extras.rows, now);
            out.actions.extend(actions.into_iter().map(PaneAction::Strip));
        }
        let body = Rect::from_min_max(pos2(rect.left(), toolbar.bottom() + strip_h), rect.max);
        out.notes_changed = show_body(ui, body, view);
    });
    out
}

fn small_button(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let text = RichText::new(label).size(11.5);
    ui.add(egui::Button::new(text).selected(selected).corner_radius(5))
}

/// Send all open and the deliver-when-idle switch, on the header's last line.
fn header_actions(ui: &mut Ui, header: Rect, extras: &PaneExtras, actions: &mut Vec<PaneAction>) {
    let area = Rect::from_min_max(
        pos2(header.left() + SIDE_PAD, header.top() + 40.0),
        pos2(header.right() - SIDE_PAD, header.bottom() - 4.0),
    );
    ui.scope_builder(
        UiBuilder::new()
            .id_salt("diff-header-actions")
            .max_rect(area)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let label = format!("Send all open ({})", extras.pending);
            let ready = extras.send_all.is_ok() && extras.pending > 0;
            let send = ui.add_enabled(
                ready,
                egui::Button::new(RichText::new(label).size(11.5)).corner_radius(5),
            );
            let send = match extras.send_all {
                Err(reason) => send.on_disabled_hover_text(reason),
                Ok(()) if extras.pending == 0 => send.on_disabled_hover_text("No unsent open notes"),
                Ok(()) => send.on_hover_text("Paste every unsent open note into the agent's terminal"),
            };
            if send.clicked() {
                actions.push(PaneAction::SendAll);
            }
            let switch = small_button(ui, "Deliver when idle", extras.auto).on_hover_text(
                "Send queued notes by themselves when the agent is done (never while it works or waits on you)",
            );
            if switch.clicked() {
                actions.push(PaneAction::ToggleAuto);
            }
        },
    );
}

/// The PR chip and "Pull PR comments" on the toolbar's right.
fn toolbar_pr(ui: &mut Ui, toolbar: Rect, extras: &PaneExtras, actions: &mut Vec<PaneAction>) {
    let Some(chip) = &extras.chip else { return };
    let area = toolbar.shrink2(vec2(SIDE_PAD - 4.0, 0.0));
    ui.scope_builder(
        UiBuilder::new()
            .id_salt("diff-toolbar-pr")
            .max_rect(area)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let pull = ui.add_enabled(
                !extras.pulling,
                egui::Button::new(
                    RichText::new(if extras.pulling { "Pulling..." } else { "Pull PR comments" })
                        .size(11.5),
                )
                .corner_radius(5),
            );
            if pull.on_hover_text("Import the pull request's review comments as notes").clicked() {
                actions.push(PaneAction::PullPr);
            }
            let tag = ui
                .add(
                    egui::Button::new(RichText::new(&chip.text).size(11.5).color(theme::added()))
                        .corner_radius(10),
                )
                .on_hover_text(format!("{} - click to open in the browser", chip.url));
            if tag.clicked() {
                actions.push(PaneAction::OpenUrl(chip.url.clone()));
            }
        },
    );
}

fn take_row(ui: &Ui, rect: Rect, top: f32, height: f32) -> Rect {
    let row = Rect::from_min_size(pos2(rect.left(), rect.top() + top), vec2(rect.width(), height));
    ui.painter().hline(rect.x_range(), row.bottom(), Stroke::new(1.0, theme::border()));
    row
}

fn message(ui: &mut Ui, rect: Rect, error: Option<&str>, idle: &str) {
    let (text, color) = match error {
        Some(error) => (error, theme::deleted()),
        None => (idle, theme::text_faint()),
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::proportional(12.5),
        color,
    );
}

fn show_header(ui: &mut Ui, rect: Rect, view: &TabView) {
    let painter = ui.painter_at(rect);
    let x = rect.left() + SIDE_PAD;
    let branch = view.loaded.as_ref().and_then(|l| l.branch.as_deref()).unwrap_or("detached HEAD");
    let HeaderText { what, title, who } = header_text(view);
    let meta = painter.text(
        pos2(x, rect.top() + 8.0),
        egui::Align2::LEFT_TOP,
        &what,
        FontId::monospace(11.5),
        theme::text_strong(),
    );
    painter.text(
        pos2(meta.right() + 8.0, rect.top() + 8.0),
        egui::Align2::LEFT_TOP,
        format!("· {branch}"),
        FontId::monospace(11.5),
        brand::TEAL_TEXT,
    );
    if let Some(open) = &view.open {
        let stats = format!("+{}  -{}", open.diff.added, open.diff.removed);
        painter.text(
            pos2(rect.right() - SIDE_PAD, rect.top() + 8.0),
            egui::Align2::RIGHT_TOP,
            stats,
            FontId::monospace(11.5),
            theme::text_muted(),
        );
    }
    painter.text(
        pos2(x, rect.top() + 24.0),
        egui::Align2::LEFT_TOP,
        title,
        FontId::new(15.0, theme::semibold()),
        theme::text_strong(),
    );
    painter.text(
        pos2(x, rect.top() + 45.0),
        egui::Align2::LEFT_TOP,
        who,
        FontId::proportional(12.0),
        theme::text_muted(),
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
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 0.0, theme::selected_row());
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, theme::menu_hover());
    }
    painter.text(
        pos2(rect.left() + SIDE_PAD, rect.center().y),
        egui::Align2::LEFT_CENTER,
        kind_letter(change.kind),
        FontId::monospace(11.5),
        kind_color(change.kind),
    );
    let color = if selected { theme::text_strong() } else { theme::text_body() };
    painter.text(
        pos2(rect.left() + SIDE_PAD + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &change.path,
        FontId::monospace(11.5),
        color,
    );
    response.clicked()
}

fn show_toolbar(ui: &mut Ui, rect: Rect, view: &mut TabView) {
    let summary = notes_summary(&view.notes);
    ui.scope_builder(
        UiBuilder::new()
            .id_salt("diff-toolbar")
            .max_rect(rect.shrink2(vec2(SIDE_PAD - 4.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let unified = view.layout == DiffLayout::Unified;
            if chip(ui, "Unified", unified).clicked() {
                set_layout(view, DiffLayout::Unified);
            }
            if chip(ui, "Split", !unified).clicked() {
                set_layout(view, DiffLayout::Split);
            }
            if let Some(open) = &mut view.open {
                if icon_button(ui, icons::CARET_UP, "Previous change (Alt+Up)").clicked() {
                    open.body.jump_to_change(false);
                }
                if icon_button(ui, icons::CARET_DOWN, "Next change (Alt+Down)").clicked() {
                    open.body.jump_to_change(true);
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new(summary).size(12.0).color(theme::text_faint()));
        },
    );
}

fn set_layout(view: &mut TabView, layout: DiffLayout) {
    view.layout = layout;
    if let Some(open) = &mut view.open {
        open.body.set_layout(layout);
    }
}

fn show_body(ui: &mut Ui, rect: Rect, view: &mut TabView) -> bool {
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
