//! D14 folder trust on screen: the wording per CLI (pure), the first-launch consent dialog and the
//! settings page. Both name every file that would change and the line it gets.

use std::path::Path;

use egui::{Align, Align2, Color32, Id, Key, Layout, Modifiers, RichText, Sense, pos2, vec2};
use tidewatch_core::consent::{Consent, Event};
use tidewatch_core::folder_repos::tilde;
use tidewatch_core::trust::{CliTrust, Status, TrustCli, TrustState};

use crate::brand;
use crate::screen_kit::TEXT_BODY;
use crate::screen_kit::TEXT_TAG;
use crate::screen_kit::{
    DialogWidth, Size, TagFill, body, dialog_pinned, fit_path, footer_row, mono, paint_pill_card,
    paint_tag, primary_button, quiet_button, tag_width,
};
use crate::settings_ui::{
    self, Control, FAINT, MUTED, TEXT, button, button_width, disclosure, divider, footer_note,
    note, page_header, path_line, row_with, section,
};

const LINE_H: f32 = 36.0;
const LINE_PAD: f32 = 12.0;
const LINE_GAP: f32 = 12.0;
const CONTROL_GAP: f32 = 8.0;
const NEW_WORKTREE: &str = "<each new worktree>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Change,
    Fine,
    Warn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) name: &'static str,
    pub(crate) file: Option<String>,
    pub(crate) place: Option<&'static str>,
    pub(crate) line: Option<String>,
    pub(crate) note: String,
    pub(crate) tone: Tone,
}

/// What a page or dialog needs to draw, copied out of the host so drawing never locks anything.
pub(crate) struct Page<'a> {
    pub(crate) consent: Consent,
    pub(crate) state: Option<&'a TrustState>,
    pub(crate) root: &'a str,
    pub(crate) home: Option<&'a Path>,
    pub(crate) added: usize,
    pub(crate) busy: bool,
}

pub(crate) fn row(trust: &CliTrust, root: &str) -> Row {
    let cli = trust.cli;
    let file = trust.file.as_ref().map(|f| f.display().to_string());
    let (line, note, tone) = match &trust.status {
        Status::Needs => (
            Some(cli.added_line(root)),
            "Added once, for the Tidewatch folder. Covers every worktree under it.".to_owned(),
            Tone::Change,
        ),
        Status::PerWorktree => (
            Some(cli.added_line(NEW_WORKTREE)),
            "Its trust does not cover subfolders of a git repo, so each new worktree gets its own \
             entry just before its agent starts. Removed when the worktree is deleted."
                .to_owned(),
            Tone::Change,
        ),
        Status::Trusted => (None, "Already trusted. Nothing to change.".to_owned(), Tone::Fine),
        Status::LaunchEnv => (
            None,
            "No file changed: Tidewatch sets GEMINI_CLI_TRUST_WORKSPACE=true for its Gemini tabs."
                .to_owned(),
            Tone::Fine,
        ),
        Status::Blocked(why) => (
            None,
            format!("Cannot be edited: {why}. Its prompt will still appear in each new worktree."),
            Tone::Warn,
        ),
    };
    let note = if cli == TrustCli::Codex && matches!(trust.status, Status::PerWorktree) {
        format!("{note} Not tested with a login yet.")
    } else {
        note
    };
    let place = matches!(trust.status, Status::LaunchEnv).then_some("env variable");
    Row { name: cli.name(), file, place, line, note, tone }
}

pub(crate) fn rows(state: &TrustState, root: &str) -> Vec<Row> {
    state.clis.iter().map(|c| row(c, root)).collect()
}

pub(crate) fn consent_text(consent: Consent) -> &'static str {
    match consent {
        Consent::Undecided => "Not asked yet.",
        Consent::Granted => "You said yes. Tidewatch keeps the folders it creates trusted.",
        Consent::Declined => "You said no. Each CLI asks about each new worktree itself.",
    }
}

fn pill_of(tone: Tone) -> (&'static str, Color32) {
    match tone {
        Tone::Change => ("edits file", brand::TEAL_TEXT),
        Tone::Fine => ("no file change", brand::GREEN),
        Tone::Warn => ("cannot edit", brand::ORANGE),
    }
}

fn tilde_of(path: &str, home: Option<&Path>) -> String {
    tilde(Path::new(path), home)
}

/// One CLI on one line: its name at the left, the file it edits (or how it is told) and the
/// tag at the right. `boxed` draws the dialog's card around it; the page puts it between dividers.
fn cli_line(ui: &mut egui::Ui, row: &Row, home: Option<&Path>, boxed: bool) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), LINE_H), Sense::hover());
    let painter = ui.painter();
    if boxed {
        paint_pill_card(painter, rect);
    }
    let mid = rect.center().y;
    let (text, ink) = pill_of(row.tone);
    let tag_left = rect.right() - LINE_PAD - tag_width(painter, text);
    paint_tag(painter, (tag_left, mid), text, ink, TagFill::Tint(ink.gamma_multiply(0.14)));
    let name = painter.text(
        pos2(rect.left() + LINE_PAD, mid),
        Align2::LEFT_CENTER,
        row.name,
        body(TEXT_BODY),
        brand::TEXT,
    );
    let right = tag_left - LINE_GAP;
    let room = right - name.right() - LINE_GAP;
    if let Some(file) = &row.file {
        let shown = fit_path(painter, "", &tilde_of(file, home), &mono(TEXT_TAG), room);
        painter.text(pos2(right, mid), Align2::RIGHT_CENTER, shown, mono(TEXT_TAG), brand::MUTED);
        response.on_hover_text(file);
    } else if let Some(place) = row.place {
        painter.text(pos2(right, mid), Align2::RIGHT_CENTER, place, body(TEXT_TAG), brand::MUTED);
    }
}

/// What each CLI's file gets, line by line, and the backup that comes with it.
fn exact_lines(ui: &mut egui::Ui, rows: &[Row]) {
    ui.spacing_mut().item_spacing.y = 4.0;
    for row in rows {
        ui.label(RichText::new(row.name).size(TEXT_BODY).color(brand::TEXT));
        if let Some(file) = &row.file {
            ui.label(RichText::new(file).monospace().size(TEXT_TAG).color(brand::FAINT));
        }
        if let Some(line) = &row.line {
            ui.label(
                RichText::new(format!("adds  {line}"))
                    .monospace()
                    .size(TEXT_TAG)
                    .color(brand::SOFT),
            );
        }
        ui.label(RichText::new(&row.note).size(TEXT_TAG).color(brand::MUTED));
        ui.add_space(6.0);
    }
    ui.label(
        RichText::new(
            "Each file gets a copy named <file>.tidewatch-bak before its first change. \
             Settings, Folder trust, undoes it. No keeps today's behavior: the prompt \
             appears in each new worktree.",
        )
        .size(TEXT_TAG)
        .color(brand::MUTED),
    );
}

/// `Claude Code, Copilot and Codex`: the CLIs whose prompt Tidewatch would answer.
pub(crate) fn asking(rows: &[Row]) -> String {
    let names: Vec<&str> =
        rows.iter().filter(|row| row.tone != Tone::Fine).map(|row| row.name).collect();
    match names.split_last() {
        None => "some agents".to_owned(),
        Some((only, [])) => (*only).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// The first-launch question. `None` until the user picks; Esc counts as No.
pub(crate) fn dialog(ctx: &egui::Context, page: &Page) -> Option<Event> {
    let mut answer = None;
    let rows = page.state.map(|s| rows(s, page.root)).unwrap_or_default();
    let modal = dialog_pinned(
        ctx,
        Id::new("folder_trust"),
        "Skip the folder trust prompts?",
        DialogWidth::Medium,
        &mut answer,
        |ui, _| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                RichText::new(format!(
                    "New worktrees make {} ask \"do you trust this folder?\". Tidewatch can \
                     answer once for the folders it creates under {}.",
                    asking(&rows),
                    tilde_of(page.root, page.home)
                ))
                .size(TEXT_BODY)
                .color(brand::SOFT),
            );
            ui.add_space(6.0);
            for row in &rows {
                cli_line(ui, row, page.home, true);
            }
            ui.add_space(2.0);
            disclosure(ui, "trust_exact_lines", "Show the exact lines added", |ui| {
                ui.add_space(4.0);
                exact_lines(ui, &rows);
            });
        },
        |ui, answer| {
            footer_row(ui, &[("Esc", "counts as No")], |ui| {
                if primary_button(ui, "Yes, trust the Tidewatch folder", true, Size::Regular)
                    .clicked()
                {
                    *answer = Some(Event::Yes);
                }
                if quiet_button(ui, "No, keep asking me", true, Size::Regular).clicked() {
                    *answer = Some(Event::No);
                }
            });
        },
    );
    if answer.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        answer = Some(Event::No);
    }
    if answer.is_none() && modal.should_close() {
        answer = Some(Event::No);
    }
    answer
}

/// The settings page: where each CLI stands, and the buttons that apply or undo consent.
pub(crate) fn settings_page(ui: &mut egui::Ui, page: &Page, file: &str) -> Option<Event> {
    let mut event = None;
    page_header(ui, "Folder trust", consent_text(page.consent));
    section(ui, "Tidewatch folder", |ui| {
        let width = button_width(ui, "Trust the Tidewatch folder", Size::Compact).max(
            button_width(ui, "Undo", Size::Compact)
                + button_width(ui, "Apply again", Size::Compact)
                + CONTROL_GAP,
        );
        row_with(
            ui,
            Control::new(width, 34.0),
            |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new("Skip the trust prompts").size(TEXT_BODY).color(TEXT));
                note(
                    ui,
                    "Answer \"do you trust this folder?\" once for the folders Tidewatch creates.",
                    MUTED,
                );
                path_line(ui, page.root, (TEXT_TAG, FAINT), ui.available_width());
                if page.added > 0 {
                    note(ui, &format!("{} entries added by Tidewatch", page.added), MUTED);
                }
            },
            |ui| {
                ui.add_enabled_ui(!page.busy, |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = CONTROL_GAP;
                        if page.consent == Consent::Granted {
                            if button(ui, "Apply again", settings_ui::Tone::Quiet, Size::Compact)
                                .clicked()
                            {
                                event = Some(Event::Apply);
                            }
                            if button(ui, "Undo", settings_ui::Tone::Quiet, Size::Compact).clicked()
                            {
                                event = Some(Event::Undo);
                            }
                        } else if button(
                            ui,
                            "Trust the Tidewatch folder",
                            settings_ui::Tone::Primary,
                            Size::Compact,
                        )
                        .clicked()
                        {
                            event = Some(Event::Apply);
                        }
                    });
                });
            },
        );
    });
    section(ui, "Agent CLIs", |ui| match page.state {
        None => {
            settings_ui::row(ui, "Looking at the CLIs...", "", Control::NONE, |_| {});
        }
        Some(state) if state.clis.is_empty() => {
            settings_ui::row(ui, "No agent CLI found on PATH.", "", Control::NONE, |_| {});
        }
        Some(state) => {
            for cli in rows(state, page.root) {
                divider(ui);
                cli_line(ui, &cli, page.home, false);
            }
        }
    });
    if let Some(state) = page.state {
        ui.add_space(8.0);
        disclosure(ui, "trust_page_exact_lines", "Show the exact lines added", |ui| {
            ui.add_space(4.0);
            exact_lines(ui, &rows(state, page.root));
        });
    }
    footer_note(ui, file);
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{RawInput, Rect, Vec2, pos2};
    use std::path::Path;
    use tidewatch_core::trust::assess;

    const ROOT: &str = "/home/u/.tidewatch";

    fn state() -> TrustState {
        let file = Path::new("/home/u/.claude.json");
        TrustState {
            clis: vec![
                assess(TrustCli::Claude, Some(file), None, ROOT),
                assess(
                    TrustCli::Copilot,
                    Some(Path::new("/home/u/.copilot/config.json")),
                    None,
                    ROOT,
                ),
                assess(TrustCli::Gemini, None, None, ROOT),
                assess(
                    TrustCli::Codex,
                    Some(Path::new("/home/u/.codex/config.toml")),
                    Some("["),
                    ROOT,
                ),
            ],
        }
    }

    #[test]
    fn every_cli_gets_the_wording_of_what_happens_to_it() {
        let rows = rows(&state(), ROOT);
        let [claude, copilot, gemini, codex] = &rows[..] else { panic!("four rows") };
        assert_eq!(claude.tone, Tone::Change);
        assert_eq!(claude.file.as_deref(), Some("/home/u/.claude.json"));
        assert!(claude.line.as_deref().unwrap().contains("<each new worktree>"));
        assert!(claude.note.contains("each new worktree gets its own entry"));
        assert_eq!(
            copilot.line.as_deref(),
            Some("\"trustedFolders\": [ ..., \"/home/u/.tidewatch\" ]")
        );
        assert_eq!(
            (gemini.tone, gemini.line.as_deref(), gemini.file.as_deref()),
            (Tone::Fine, None, None)
        );
        assert!(gemini.note.contains("GEMINI_CLI_TRUST_WORKSPACE"));
        assert_eq!(codex.tone, Tone::Warn);
        assert!(codex.note.contains("Its prompt will still appear"));
    }

    #[test]
    fn an_unverified_codex_says_so_and_a_trusted_cli_asks_for_nothing() {
        let codex = assess(TrustCli::Codex, None, None, ROOT);
        assert!(row(&codex, ROOT).note.ends_with("Not tested with a login yet."));
        let trusted =
            assess(TrustCli::Copilot, None, Some("{\"trustedFolders\": [\"/home/u\"]}"), ROOT);
        let row = row(&trusted, ROOT);
        assert_eq!((row.tone, row.line), (Tone::Fine, None));
    }

    #[test]
    fn the_consent_line_says_where_things_stand() {
        assert!(consent_text(Consent::Undecided).contains("Not asked"));
        assert!(consent_text(Consent::Granted).contains("yes"));
        assert!(consent_text(Consent::Declined).contains("no"));
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, draw: impl FnMut(&mut egui::Ui)) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1000.0, 760.0))),
            focused: true,
            events,
            ..RawInput::default()
        };
        let _ = ctx.run_ui(raw, draw);
    }

    #[test]
    fn the_dialog_and_the_page_render_and_stay_quiet_until_clicked() {
        let state = state();
        let page = Page {
            consent: Consent::Undecided,
            state: Some(&state),
            root: ROOT,
            home: None,
            added: 0,
            busy: false,
        };
        let ctx = crate::fonts::test_context();
        for _ in 0..2 {
            let mut answer = None;
            let mut event = None;
            frame(&ctx, Vec::new(), |ui| {
                answer = dialog(ui.ctx(), &page);
                event = settings_page(ui, &page, "trust.toml");
            });
            assert_eq!((answer, event), (None, None));
        }
    }

    #[test]
    fn escape_in_the_dialog_is_a_no() {
        let state = state();
        let page = Page {
            consent: Consent::Undecided,
            state: Some(&state),
            root: ROOT,
            home: None,
            added: 0,
            busy: false,
        };
        let ctx = crate::fonts::test_context();
        let esc = egui::Event::Key {
            key: Key::Escape,
            physical_key: Some(Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let mut answer = None;
        frame(&ctx, vec![esc], |ui| answer = dialog(ui.ctx(), &page));
        assert_eq!(answer, Some(Event::No));
    }

    fn painted(width: f32, height: f32) -> Vec<(String, Rect)> {
        let state = state();
        let page = Page {
            consent: Consent::Undecided,
            state: Some(&state),
            root: ROOT,
            home: Some(Path::new("/home/u")),
            added: 0,
            busy: false,
        };
        let ctx = crate::fonts::test_context();
        let mut out = None;
        for _ in 0..3 {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(width, height))),
                focused: true,
                ..RawInput::default()
            };
            out = Some(ctx.run_ui(raw, |ui| {
                let _ = dialog(ui.ctx(), &page);
            }));
        }
        crate::screen_kit::painted_texts(&out.unwrap())
    }

    #[test]
    fn the_question_keeps_its_title_and_buttons_on_screen_in_a_short_window() {
        let texts = painted(900.0, 360.0);
        let at = |name: &str| texts.iter().find(|(text, _)| text == name).map(|(_, rect)| *rect);
        let title = at("Skip the folder trust prompts?").unwrap();
        let yes = at("Yes, trust the Tidewatch folder").unwrap();
        let no = at("No, keep asking me").unwrap();
        assert!(title.top() >= 0.0, "{title:?}");
        assert!(yes.bottom() <= 360.0 && no.bottom() <= 360.0, "{yes:?} {no:?}");
    }

    #[test]
    fn each_cli_is_one_line_and_the_exact_lines_wait_behind_a_disclosure() {
        let texts = painted(1000.0, 760.0);
        let at = |name: &str| texts.iter().find(|(text, _)| text == name).map(|(_, rect)| *rect);
        let claude = at("Claude Code").unwrap();
        let file = at("~/.claude.json").unwrap();
        let tag = at("edits file").unwrap();
        assert!((claude.center().y - file.center().y).abs() < 2.0, "{claude:?} {file:?}");
        assert!((claude.center().y - tag.center().y).abs() < 2.0, "{claude:?} {tag:?}");
        assert!(file.right() < tag.left());
        assert!(at("Show the exact lines added").is_some());
        let adds = texts.iter().filter(|(text, _)| text.starts_with("adds  ")).count();
        assert_eq!(adds, 0);
    }

    #[test]
    fn the_question_names_the_clis_that_would_ask() {
        let rows = rows(&state(), ROOT);
        assert_eq!(asking(&rows), "Claude Code, GitHub Copilot and Codex");
        assert_eq!(asking(&rows[2..3]), "some agents");
        assert_eq!(asking(&rows[..1]), "Claude Code");
    }
}
