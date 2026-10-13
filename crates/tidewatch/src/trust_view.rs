//! D14 folder trust on screen: the wording per CLI (pure), the first-launch consent dialog and the
//! settings page. Both name every file that would change and the line it gets.

use egui::{Color32, Id, Key, Modifiers, RichText};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, overlay_frame};
use tidewatch_core::consent::{Consent, Event};
use tidewatch_core::trust::{CliTrust, Status, TrustCli, TrustState};

const DIALOG_WIDTH: f32 = 560.0;
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
    pub(crate) line: Option<String>,
    pub(crate) note: String,
    pub(crate) tone: Tone,
}

/// What a page or dialog needs to draw, copied out of the host so drawing never locks anything.
pub(crate) struct Page<'a> {
    pub(crate) consent: Consent,
    pub(crate) state: Option<&'a TrustState>,
    pub(crate) root: &'a str,
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
    Row { name: cli.name(), file, line, note, tone }
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

fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Change => colors::fg(),
        Tone::Fine => colors::green(),
        Tone::Warn => colors::yellow(),
    }
}

fn draw_rows(ui: &mut egui::Ui, rows: &[Row]) {
    for row in rows {
        ui.add_space(4.0);
        ui.label(RichText::new(row.name).strong().color(tone_color(row.tone)));
        if let Some(file) = &row.file {
            ui.label(RichText::new(file).monospace().size(11.5).color(colors::dim()));
        }
        if let Some(line) = &row.line {
            ui.label(
                RichText::new(format!("adds  {line}")).monospace().size(11.5).color(colors::fg()),
            );
        }
        ui.label(RichText::new(&row.note).size(12.0).color(colors::dim()));
    }
}

/// The first-launch question. `None` until the user picks; Esc counts as No.
pub(crate) fn dialog(ctx: &egui::Context, page: &Page) -> Option<Event> {
    let mut answer = None;
    let rows = page.state.map(|s| rows(s, page.root)).unwrap_or_default();
    let modal = egui::Modal::new(Id::new("folder_trust")).frame(overlay_frame()).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new("Skip the folder trust prompts?").strong().size(15.0).color(colors::fg()));
        ui.label(
            RichText::new(format!(
                "Every new worktree makes some agents ask \"do you trust this folder?\". Tidewatch \
                 can answer once for the folders it creates under {}. It edits these files only if \
                 you say yes:",
                page.root
            ))
            .color(colors::dim()),
        );
        draw_rows(ui, &rows);
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Each file gets a copy named <file>.tidewatch-bak before its first change. Settings, \
                 Folder trust, undoes it. No keeps today's behavior: the prompt appears in each new worktree.",
            )
            .size(12.0)
            .color(colors::dim()),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if action_button(ui, "No, keep asking me", false).clicked() {
                answer = Some(Event::No);
            }
            if action_button(ui, "Yes, trust the Tidewatch folder", true).clicked() {
                answer = Some(Event::Yes);
            }
        });
    });
    if answer.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        answer = Some(Event::No);
    }
    if answer.is_none() && modal.should_close() {
        answer = Some(Event::No);
    }
    answer
}

/// The settings page: where each CLI stands, and the button that applies or undoes consent.
pub(crate) fn settings_page(ui: &mut egui::Ui, page: &Page) -> Option<Event> {
    let mut event = None;
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.label(
        RichText::new(format!("Tidewatch folder: {}", page.root))
            .monospace()
            .size(11.5)
            .color(colors::dim()),
    );
    ui.label(RichText::new(consent_text(page.consent)).color(colors::fg()));
    match page.state {
        None => {
            ui.label(RichText::new("Looking at the CLIs...").color(colors::dim()));
        }
        Some(state) if state.clis.is_empty() => {
            ui.label(RichText::new("No agent CLI found on PATH.").color(colors::dim()));
        }
        Some(state) => draw_rows(ui, &rows(state, page.root)),
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_enabled_ui(!page.busy, |ui| {
            if page.consent == Consent::Granted {
                if action_button(ui, "Undo", false).clicked() {
                    event = Some(Event::Undo);
                }
                if action_button(ui, "Apply again", false).clicked() {
                    event = Some(Event::Apply);
                }
            } else if action_button(ui, "Trust the Tidewatch folder", true).clicked() {
                event = Some(Event::Apply);
            }
        });
        if page.added > 0 {
            ui.label(
                RichText::new(format!("{} entries added by Tidewatch", page.added))
                    .size(12.0)
                    .color(colors::dim()),
            );
        }
    });
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
            added: 0,
            busy: false,
        };
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let mut answer = None;
            let mut event = None;
            frame(&ctx, Vec::new(), |ui| {
                answer = dialog(ui.ctx(), &page);
                event = settings_page(ui, &page);
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
            added: 0,
            busy: false,
        };
        let ctx = egui::Context::default();
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
}
