//! Dev-only switches for the screenshot runs, read once at startup.
//!
//! - `TIDEWATCH_DEV_SCOPE=whole`: start the graph on Whole repo.
//! - `TIDEWATCH_DEV_PICK=<row>`: select that graph row (0 is the newest commit) once history loads.
//! - `TIDEWATCH_DEV_OPEN=commit|file|file:<n>`: open the selected row's files in the tree, or also
//!   open file `n` (0 is the first) in the file view, once the files have loaded.
//! - `TIDEWATCH_DEV_PRESS=<command id>@<seconds>,...`: press the chord of those table commands
//!   (`file-view`, `back-to-terminal`, `next-file`...) through the real key path after that long. A command
//!   with no chord (`build-workflow`) runs directly.
//! - `TIDEWATCH_DEV_NOTE=<text>`: write a note on the first added line of the first diff shown, once
//!   per run, through the same notes store the diff widget uses.
//! - `TIDEWATCH_DEV_NOTE_SPAN=<n>`: make that note cover `n` more lines below the first added line.
//! - `TIDEWATCH_DEV_NOTES=[<row>@]<path>:<line>[-<last>]:<text>|...`: add those notes to the draft
//!   review once the tab's notes have loaded, so a capture shows a review across several files. A
//!   `row@` puts the note on that graph row's commit, a range makes it a block note.
//! - `TIDEWATCH_DEV_PANEL=progress|changes|history`: show that side of the History card.
//! - `TIDEWATCH_DEV_COMPOSER=<line>|<first>-<last>`: open the note composer on those lines of the
//!   file view once its diff has loaded.

use kelp_core::diff::{Body, FileDiff, LineKind};
use kelp_core::review::{Anchor, Comment, Side};
use kelp_ui::diff::DiffAction;
use tidewatch_core::history::Scope;

use crate::commands::{self, Command};
use crate::progress_view::Side as Panel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevOpen {
    Commit,
    File(usize),
}

pub(crate) struct DevPanes {
    pub(crate) scope: Option<Scope>,
    pub(crate) pick_row: Option<usize>,
    pub(crate) open: Option<DevOpen>,
    pub(crate) presses: Vec<(f64, Command)>,
    pub(crate) note: Option<String>,
    pub(crate) note_span: u32,
    pub(crate) notes: Vec<DevNote>,
    pub(crate) composer: Option<(u32, u32)>,
    pub(crate) panel: Option<Panel>,
}

/// A note for the draft review: `src/a.rs:12:rename this`, `2@src/a.rs:12-14:extract these`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevNote {
    /// The graph row of the commit the note is on, `None` for the working tree.
    pub(crate) row: Option<usize>,
    pub(crate) path: String,
    pub(crate) line: u32,
    pub(crate) span: u32,
    pub(crate) text: String,
}

fn parse_lines(text: &str) -> Option<(u32, u32)> {
    let (first, last) = text.split_once('-').unwrap_or((text, text));
    Some((first.parse().ok()?, last.parse().ok()?))
}

fn parse_notes(text: &str) -> Vec<DevNote> {
    text.split('|')
        .filter_map(|part| {
            let (row, part) = match part.split_once('@') {
                Some((row, rest)) if row.parse::<usize>().is_ok() => (row.parse().ok(), rest),
                _ => (None, part),
            };
            let mut fields = part.splitn(3, ':');
            let (path, lines, text) = (fields.next()?, fields.next()?, fields.next()?);
            let (first, last) = parse_lines(lines)?;
            Some(DevNote {
                row,
                path: path.to_owned(),
                line: first,
                span: last.saturating_sub(first),
                text: text.to_owned(),
            })
        })
        .collect()
}

fn parse_presses(text: &str) -> Vec<(f64, Command)> {
    let mut presses: Vec<(f64, Command)> = text
        .split(',')
        .filter_map(|part| {
            let (id, after) = part.split_once('@')?;
            Some((after.parse().ok()?, commands::command_of(id)?))
        })
        .collect();
    presses.sort_by(|a, b| a.0.total_cmp(&b.0));
    presses
}

fn parse_open(text: &str) -> Option<DevOpen> {
    match text {
        "commit" => Some(DevOpen::Commit),
        "file" => Some(DevOpen::File(0)),
        other => other.strip_prefix("file:")?.parse().ok().map(DevOpen::File),
    }
}

impl DevPanes {
    pub(crate) fn from_env() -> Self {
        Self::parse(
            std::env::var("TIDEWATCH_DEV_SCOPE").ok().as_deref(),
            std::env::var("TIDEWATCH_DEV_PICK").ok().as_deref(),
            std::env::var("TIDEWATCH_DEV_OPEN").ok().as_deref(),
            std::env::var("TIDEWATCH_DEV_PRESS").ok().as_deref(),
            std::env::var("TIDEWATCH_DEV_NOTE").ok(),
        )
        .with_notes(std::env::var("TIDEWATCH_DEV_NOTES").ok().as_deref())
        .with_panel(std::env::var("TIDEWATCH_DEV_PANEL").ok().as_deref())
        .with_block(
            std::env::var("TIDEWATCH_DEV_COMPOSER").ok().as_deref(),
            std::env::var("TIDEWATCH_DEV_NOTE_SPAN").ok().as_deref(),
        )
    }

    fn with_block(self, composer: Option<&str>, span: Option<&str>) -> Self {
        Self {
            composer: composer.and_then(parse_lines),
            note_span: span.and_then(|span| span.parse().ok()).unwrap_or(0),
            ..self
        }
    }

    fn with_panel(self, panel: Option<&str>) -> Self {
        let panel = match panel {
            Some("progress") => Some(Panel::Progress),
            Some("changes") => Some(Panel::Changes),
            Some("history") => Some(Panel::History),
            _ => None,
        };
        Self { panel, ..self }
    }

    fn with_notes(self, notes: Option<&str>) -> Self {
        Self { notes: notes.map(parse_notes).unwrap_or_default(), ..self }
    }

    fn parse(
        scope: Option<&str>,
        pick: Option<&str>,
        open: Option<&str>,
        press: Option<&str>,
        note: Option<String>,
    ) -> Self {
        Self {
            scope: (scope == Some("whole")).then_some(Scope::Whole),
            pick_row: pick.and_then(|row| row.parse().ok()),
            open: open.and_then(parse_open),
            presses: press.map(parse_presses).unwrap_or_default(),
            note: note.filter(|text| !text.is_empty()),
            note_span: 0,
            notes: Vec::new(),
            composer: None,
            panel: None,
        }
    }
}

impl DevPanes {
    pub(crate) fn pending(&self) -> bool {
        self.pick_row.is_some() || self.open.is_some() || self.panel.is_some()
    }

    /// Presses the next command that is due. A command with no chord (`build-workflow`) cannot be
    /// pressed, so it comes back for the app to run.
    pub(crate) fn press(&mut self, ctx: &egui::Context, now: f64) -> Option<Command> {
        let (after, _) = self.presses.first()?;
        if now < *after {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(*after - now));
            return None;
        }
        let (_, command) = self.presses.remove(0);
        let Some(event) = commands::press_event(command) else { return Some(command) };
        ctx.input_mut(|input| input.events.push(event));
        ctx.request_repaint();
        None
    }
}

/// The action the diff widget would report for a note on the first added line of `diff`.
pub(crate) fn note_on_first_added_line(
    diff: &FileDiff,
    text: &str,
    span: u32,
    now: i64,
) -> Option<DiffAction> {
    let new_text = diff.new_text.as_deref()?;
    let lines: Vec<&str> = new_text.lines().collect();
    let Body::Text(hunks) = &diff.body else { return None };
    let number = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .find(|line| line.kind == LineKind::Added)
        .and_then(|line| line.new)?;
    let index = usize::try_from(number).ok()?.checked_sub(1).filter(|i| *i < lines.len())?;
    Some(DiffAction::AddThread {
        anchor: Anchor::over(&lines, index, span, Side::New),
        comment: Comment { author: "You".into(), time: now, body: text.to_owned() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::diff;

    #[test]
    fn review_notes_split_into_path_line_and_text_and_junk_is_skipped() {
        assert_eq!(
            parse_notes(
                "src/a.rs:12:rename: this|b.rs:3:log it|nonsense|c.rs:x:bad line|\
                 2@d.rs:5-8:extract|a@e.rs:1:at sign"
            ),
            [
                DevNote {
                    row: None,
                    path: "src/a.rs".into(),
                    line: 12,
                    span: 0,
                    text: "rename: this".into()
                },
                DevNote { row: None, path: "b.rs".into(), line: 3, span: 0, text: "log it".into() },
                DevNote {
                    row: Some(2),
                    path: "d.rs".into(),
                    line: 5,
                    span: 3,
                    text: "extract".into()
                },
                DevNote {
                    row: None,
                    path: "a@e.rs".into(),
                    line: 1,
                    span: 0,
                    text: "at sign".into()
                },
            ]
        );
        assert_eq!(parse_notes(""), []);
    }

    #[test]
    fn parses_scope_pick_and_note() {
        let dev =
            DevPanes::parse(Some("whole"), Some("2"), None, None, Some("use the helper".into()));
        assert_eq!(dev.scope, Some(Scope::Whole));
        assert_eq!(dev.pick_row, Some(2));
        assert_eq!(dev.note.as_deref(), Some("use the helper"));
    }

    #[test]
    fn anything_else_leaves_the_defaults() {
        let dev = DevPanes::parse(
            Some("current"),
            Some("two"),
            Some("later"),
            Some("x@1"),
            Some(String::new()),
        );
        assert_eq!((dev.scope, dev.pick_row, dev.open, dev.note), (None, None, None, None));
        let dev = DevPanes::parse(None, None, None, None, None);
        assert_eq!((dev.scope, dev.pick_row, dev.open, dev.note), (None, None, None, None));
    }

    #[test]
    fn presses_read_command_ids_with_their_delay_in_order() {
        let got = parse_presses("back-to-terminal@5,file-view@2.5,nope@1,next-file@x");
        assert_eq!(got, [(2.5, Command::FileView), (5.0, Command::BackToTerminal)]);
    }

    #[test]
    fn a_press_with_no_chord_comes_back_to_be_run_and_one_with_a_chord_is_pushed_as_a_key() {
        let ctx = egui::Context::default();
        let mut dev = DevPanes::parse(None, None, None, Some("build-workflow@1,file-view@2"), None);
        assert_eq!(dev.press(&ctx, 0.5), None, "not due yet");
        assert_eq!(dev.press(&ctx, 1.0), Some(Command::BuildWorkflow));
        assert_eq!(dev.press(&ctx, 2.0), None, "a chord goes in as a key event");
        assert_eq!(dev.presses, []);
    }

    #[test]
    fn open_reads_a_commit_or_a_file_by_position() {
        assert_eq!(parse_open("commit"), Some(DevOpen::Commit));
        assert_eq!(parse_open("file"), Some(DevOpen::File(0)));
        assert_eq!(parse_open("file:2"), Some(DevOpen::File(2)));
        assert_eq!(parse_open("file:x"), None);
    }

    #[test]
    fn the_note_anchors_on_the_first_added_line_with_its_neighbors() {
        let diff = diff::build("a.rs", Some(b"one\ntwo\nthree\n"), Some(b"one\ntwo\nNEW\nthree\n"));
        let Some(DiffAction::AddThread { anchor, comment }) =
            note_on_first_added_line(&diff, "rename this", 0, 7)
        else {
            panic!("expected a new thread")
        };
        assert_eq!((anchor.side, anchor.line_hint, anchor.text.as_str()), (Side::New, 3, "NEW"));
        assert_eq!(anchor.before, ["one", "two"]);
        assert_eq!(anchor.after, ["three"]);
        assert_eq!(
            (comment.author.as_str(), comment.time, comment.body.as_str()),
            ("You", 7, "rename this")
        );
    }

    #[test]
    fn a_span_makes_the_note_cover_the_lines_below_the_first_added_one() {
        let diff = diff::build("a.rs", Some(b"one\n"), Some(b"one\nNEW\ntwo\nthree\nfour\n"));
        let Some(DiffAction::AddThread { anchor, .. }) =
            note_on_first_added_line(&diff, "extract", 2, 0)
        else {
            panic!("expected a new thread")
        };
        assert_eq!((anchor.line_hint, anchor.span, anchor.text.as_str()), (2, 2, "NEW"));
        assert_eq!(anchor.block, ["two", "three"]);
        assert_eq!(anchor.after, ["four"]);
    }

    #[test]
    fn the_composer_and_span_switches_read_a_line_or_a_range() {
        let cases = [
            (Some("7"), Some("3"), Some((7, 7)), 3),
            (Some("4-9"), None, Some((4, 9)), 0),
            (Some("x"), Some("y"), None, 0),
            (None, None, None, 0),
        ];
        for (composer, span, want, want_span) in cases {
            let dev = DevPanes::parse(None, None, None, None, None).with_block(composer, span);
            assert_eq!((dev.composer, dev.note_span), (want, want_span), "{composer:?}");
        }
    }

    #[test]
    fn a_diff_with_nothing_added_or_no_text_gets_no_note() {
        let removed = diff::build("a.rs", Some(b"one\ntwo\n"), Some(b"one\n"));
        assert!(note_on_first_added_line(&removed, "x", 0, 0).is_none());
        let binary = diff::build("a.bin", Some(&[0, 159, 146]), Some(&[0, 1, 2]));
        assert!(note_on_first_added_line(&binary, "x", 0, 0).is_none());
    }
}
