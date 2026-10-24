//! The Sync card of Settings, General: the switch, the repo field, the last sync with Sync now,
//! and the quiet note a sync leaves. It hands back what the user asked for; the app does it.

use egui::{Margin, RichText, Ui};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use tidewatch_core::settings_sync::remote_url;

use crate::screen_kit::Size;
use crate::settings_ui::{
    self, Control, MUTED, ORANGE, TEXT, Tone, button, row, row_with, section, switch_row,
    text_input,
};

pub(crate) const FOOTNOTE: &str =
    "Folder trust and app state stay on this Mac. Uses your own git login.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Note {
    None,
    KeptLocal,
    Updated { repo: String },
    MissingRepo { slug: Option<String> },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct View {
    pub(crate) enabled: bool,
    pub(crate) has_repo: bool,
    pub(crate) busy: bool,
    pub(crate) last_synced: Option<i64>,
    pub(crate) now: i64,
    pub(crate) note: Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Enabled(bool),
    Repo(String),
    SyncNow,
    CreateRepo,
}

#[derive(Default)]
pub(crate) struct Editor {
    pub(crate) repo: String,
}

impl Editor {
    pub(crate) fn new(repo: &str) -> Self {
        Self { repo: repo.to_owned() }
    }

    fn problem(&self) -> bool {
        !self.repo.trim().is_empty() && remote_url(&self.repo).is_none()
    }
}

pub(crate) fn stamp_label(synced: Option<i64>, now: i64, zone: &TimeZone) -> String {
    let Some(at) = synced.and_then(|secs| Timestamp::from_second(secs).ok()) else {
        return "Never".to_owned();
    };
    let then = at.to_zoned(zone.clone());
    let today =
        Timestamp::from_second(now).map_or(then.date(), |t| t.to_zoned(zone.clone()).date());
    let clock = then.strftime("%H:%M");
    if then.date() == today {
        format!("Today, {clock}")
    } else if today.yesterday().is_ok_and(|yesterday| yesterday == then.date()) {
        format!("Yesterday, {clock}")
    } else {
        format!("{}, {clock}", then.strftime("%b %-d"))
    }
}

pub(crate) fn note_text(note: &Note) -> Option<(String, bool)> {
    match note {
        Note::None | Note::MissingRepo { .. } => None,
        Note::KeptLocal => Some((
            "The repo changed the same settings. Kept the ones on this Mac.".to_owned(),
            false,
        )),
        Note::Updated { repo } => Some((format!("Settings updated from {repo}."), false)),
        Note::Failed(why) => Some((format!("Sync failed: {why}"), true)),
    }
}

pub(crate) fn card(ui: &mut Ui, editor: &mut Editor, view: &View) -> Option<Change> {
    let mut change = None;
    if std::env::var_os("TIDEWATCH_DEV_SCROLL_SYNC").is_some() {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    section(ui, "Sync", |ui| {
        if let Some(on) = switch_row(
            ui,
            "Sync settings to a GitHub repo",
            "Pull on launch, push when a setting changes.",
            view.enabled,
        ) {
            change = Some(Change::Enabled(on));
        }
        if let Some(text) = repo_row(ui, editor) {
            change = Some(Change::Repo(text));
        }
        if let Note::MissingRepo { slug } = &view.note
            && create_row(ui, slug.as_deref(), view.busy)
        {
            change = Some(Change::CreateRepo);
        }
        if last_row(ui, view) {
            change = Some(Change::SyncNow);
        }
        if let Some((text, problem)) = note_text(&view.note) {
            let ink = if problem { ORANGE } else { MUTED };
            row_with(
                ui,
                Control::NONE,
                |ui| {
                    ui.label(RichText::new(&text).size(13.0).color(ink));
                },
                |_| {},
            );
        }
        row_with(
            ui,
            Control::NONE,
            |ui| {
                ui.label(RichText::new(FOOTNOTE).size(13.0).color(MUTED));
            },
            |_| {},
        );
    });
    change
}

fn repo_row(ui: &mut Ui, editor: &mut Editor) -> Option<String> {
    settings_ui::divider(ui);
    egui::Frame::new()
        .inner_margin(Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new("Repo").size(15.0).color(TEXT));
            let hint = "A private repo you own, such as you/tidewatch-settings.";
            ui.label(RichText::new(hint).size(13.0).color(MUTED));
            let response = text_input(ui, &mut editor.repo, "you/tidewatch-settings");
            if editor.problem() {
                settings_ui::note(ui, "Use owner/name, or the full address of a git repo.", ORANGE);
            }
            (response.lost_focus() && !editor.problem()).then(|| editor.repo.trim().to_owned())
        })
        .inner
}

fn create_row(ui: &mut Ui, slug: Option<&str>, busy: bool) -> bool {
    let (title, desc) = match slug {
        Some(slug) => (
            "That repo does not exist yet".to_owned(),
            format!("Create {slug} as a private repo on your GitHub account? Uses the gh CLI."),
        ),
        None => (
            "That repo does not exist".to_owned(),
            "Only a GitHub owner/name can be created from here.".to_owned(),
        ),
    };
    let label = "Create private repo";
    let control = Control::button(ui, label, Size::Compact);
    row(ui, &title, &desc, control, |ui| {
        ui.add_enabled_ui(slug.is_some() && !busy, |ui| {
            button(ui, label, Tone::Primary, Size::Compact).clicked()
        })
        .inner
    })
}

fn last_row(ui: &mut Ui, view: &View) -> bool {
    let zone = TimeZone::system();
    let when = if view.busy {
        "Syncing...".to_owned()
    } else {
        stamp_label(view.last_synced, view.now, &zone)
    };
    let label = "Sync now";
    let control = Control::button(ui, label, Size::Compact);
    row(ui, "Last synced", &when, control, |ui| {
        ui.add_enabled_ui(view.has_repo && !view.busy, |ui| {
            button(ui, label, Tone::Quiet, Size::Compact).clicked()
        })
        .inner
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

    const UTC: TimeZone = TimeZone::UTC;

    fn view() -> View {
        View {
            enabled: true,
            has_repo: true,
            busy: false,
            last_synced: Some(1_700_000_000),
            now: 1_700_000_000 + 180,
            note: Note::None,
        }
    }

    #[test]
    fn the_last_sync_reads_as_a_clock_time_a_day_word_or_a_date() {
        let day = 86_400;
        let noon = 1_700_000_000 - 1_700_000_000 % day + 12 * 3600;
        let cases = [
            (None, noon, "Never"),
            (Some(noon), noon + 60, "Today, 12:00"),
            (Some(noon), noon + day, "Yesterday, 12:00"),
            (Some(noon), noon + 5 * day, "Nov 14, 12:00"),
        ];
        for (synced, now, want) in cases {
            assert_eq!(stamp_label(synced, now, &UTC), want, "{synced:?} at {now}");
        }
    }

    #[test]
    fn a_conflict_note_is_quiet_and_a_failure_is_not() {
        assert_eq!(note_text(&Note::None), None);
        assert_eq!(note_text(&Note::MissingRepo { slug: None }), None);
        let kept = note_text(&Note::KeptLocal).unwrap();
        assert!(!kept.1 && kept.0.contains("Kept the ones on this Mac"));
        assert_eq!(
            note_text(&Note::Updated { repo: "you/x".into() }),
            Some(("Settings updated from you/x.".to_owned(), false))
        );
        assert_eq!(
            note_text(&Note::Failed("no network".into())),
            Some(("Sync failed: no network".to_owned(), true))
        );
    }

    #[test]
    fn repo_text_that_is_not_a_repo_is_a_problem_and_empty_text_is_not() {
        for (text, problem) in
            [("", false), ("you/x", false), ("/tmp/r.git", false), ("nope", true), ("a b/c", true)]
        {
            assert_eq!(Editor::new(text).problem(), problem, "{text:?}");
        }
    }

    fn frame(
        ctx: &Context,
        events: Vec<Event>,
        editor: &mut Editor,
        view: &View,
    ) -> Option<Change> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 900.0))),
            focused: true,
            events,
            ..RawInput::default()
        };
        let mut out = None;
        let _ = ctx.run_ui(raw, |ui| out = card(ui, editor, view));
        out
    }

    #[test]
    fn the_card_draws_in_every_state_and_asks_for_nothing_by_itself() {
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        let notes = [
            Note::None,
            Note::KeptLocal,
            Note::Updated { repo: "you/x".into() },
            Note::MissingRepo { slug: Some("you/x".into()) },
            Note::MissingRepo { slug: None },
            Note::Failed("boom".into()),
        ];
        for note in notes {
            for busy in [false, true] {
                let mut editor = Editor::new("you/x");
                let view = View { note: note.clone(), busy, ..view() };
                assert_eq!(frame(&ctx, Vec::new(), &mut editor, &view), None);
            }
        }
    }

    fn click_at(ctx: &Context, editor: &mut Editor, view: &View, label_y: f32) -> Option<Change> {
        let at = pos2(850.0, label_y);
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(ctx, vec![Event::PointerMoved(at)], editor, view);
        frame(ctx, vec![press(true)], editor, view);
        frame(ctx, vec![press(false)], editor, view)
    }

    #[test]
    fn clicking_the_switch_row_control_asks_to_turn_sync_off() {
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        let mut editor = Editor::new("you/x");
        let view = view();
        frame(&ctx, Vec::new(), &mut editor, &view);
        let mut found = None;
        for y in (40..120).step_by(4) {
            if let Some(change) = click_at(&ctx, &mut editor, &view, y as f32) {
                found = Some(change);
                break;
            }
        }
        assert_eq!(found, Some(Change::Enabled(false)));
    }

    #[test]
    fn sync_now_is_not_offered_while_a_sync_runs_or_without_a_repo() {
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        for view in [View { busy: true, ..view() }, View { has_repo: false, ..view() }] {
            let mut editor = Editor::new("");
            frame(&ctx, Vec::new(), &mut editor, &view);
            let mut got = Vec::new();
            for y in (40..400).step_by(6) {
                got.extend(click_at(&ctx, &mut editor, &view, y as f32));
            }
            assert!(!got.contains(&Change::SyncNow), "{view:?}");
        }
    }
}
