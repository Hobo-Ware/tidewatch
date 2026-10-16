//! The status bar along the bottom of the window: how many agents work and need you, where this
//! agent tab stands, and the keys that matter on the screen. The logic is pure; `show` only draws.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Ui, pos2, vec2};

use crate::brand;
use tidewatch_core::pr_status::Tone as PrTone;

use crate::commands::{Command, Platform};
use crate::pr_strip::tone_ink;
use crate::screen_kit::{body, mono};
use crate::status_view::Tone;

pub(crate) const HEIGHT: f32 = 30.0;
const PAD: f32 = 16.0;
const GAP: f32 = 18.0;
const DOT: f32 = 7.0;
const FONT: f32 = 12.0;
const MOST_HINTS: usize = 4;
const CHIP_H: f32 = 20.0;
const CHIP_PAD: f32 = 9.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) working: usize,
    pub(crate) needs: usize,
}

/// Agents working and agents that need you, from the tones of every agent tab of the window.
pub(crate) fn counts(tones: &[Tone]) -> Counts {
    tones.iter().fold(Counts::default(), |mut counts, tone| {
        match tone {
            Tone::Working => counts.working += 1,
            Tone::Needs => counts.needs += 1,
            Tone::Setup | Tone::Done | Tone::Failed | Tone::Shell => {}
        }
        counts
    })
}

/// `agent/sunny-reef from main`, or just the branch when no base is known.
pub(crate) fn place(branch: &str, base: Option<&str>) -> String {
    base.map_or_else(|| branch.to_owned(), |base| format!("{branch} from {base}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Welcome,
    Settings,
    Overview,
    Workspace(Workspace),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Workspace {
    pub(crate) focus: bool,
    /// The tab has History beside it: the side layout with panes showing.
    pub(crate) history: bool,
    pub(crate) file_open: bool,
    /// `Some(true)` while an agent tab shows its Reader, `Some(false)` its raw terminal.
    pub(crate) reader: Option<bool>,
    pub(crate) input: Option<Input>,
}

/// A text box that owns the keyboard in the tab: where its own keys are worth a hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Prompt,
    Task,
}

const NOTE_HINT: &str = "Hover a line and press + to leave a note";

/// What to say about notes while a file's diff is on screen.
pub(crate) fn note_hint(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::Workspace(Workspace { focus: false, file_open: true, .. }) => Some(NOTE_HINT),
        Screen::Workspace(_) | Screen::Welcome | Screen::Settings | Screen::Overview => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hint {
    pub(crate) keys: String,
    pub(crate) label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keys {
    Command(Command),
    Text(&'static str),
    CommandEnter,
}

impl Keys {
    fn text(self, platform: Platform) -> String {
        match self {
            Self::Command(command) => command.keys_on(platform),
            Self::Text(text) => text.to_owned(),
            Self::CommandEnter => match platform {
                Platform::Mac => "Cmd+Enter".to_owned(),
                Platform::Linux => "Ctrl+Enter".to_owned(),
            },
        }
    }
}

fn of(command: Command, label: &'static str) -> (Keys, &'static str) {
    (Keys::Command(command), label)
}

fn commands(screen: Screen) -> Vec<(Keys, &'static str)> {
    match screen {
        Screen::Welcome => {
            vec![of(Command::OpenRepo, "open repo"), of(Command::Settings, "settings")]
        }
        Screen::Settings => vec![
            of(Command::Settings, "close settings"),
            of(Command::Overview, "overview"),
            of(Command::Palette, "commands"),
        ],
        Screen::Overview => vec![
            of(Command::Overview, "close overview"),
            of(Command::NewTab, "new tab"),
            of(Command::Settings, "settings"),
            of(Command::Palette, "commands"),
        ],
        Screen::Workspace(view) => workspace_commands(view),
    }
}

fn workspace_commands(view: Workspace) -> Vec<(Keys, &'static str)> {
    let reader = view.reader.map(|on| of(Command::Reader, if on { "terminal" } else { "reader" }));
    if view.focus {
        return [
            Some(of(Command::Focus, "leave focus")),
            reader,
            Some(of(Command::Overview, "overview")),
            Some(of(Command::Palette, "commands")),
        ]
        .into_iter()
        .flatten()
        .collect();
    }
    if view.file_open {
        return vec![
            of(Command::BackToTerminal, "terminal"),
            of(Command::NextFile, "next file"),
            of(Command::Overview, "overview"),
            of(Command::Focus, "focus"),
        ];
    }
    match view.input {
        Some(Input::Prompt) => {
            return vec![
                (Keys::Text("Enter"), "send"),
                (Keys::Text("Shift+Enter"), "new line"),
                of(Command::Reader, "terminal"),
                of(Command::Overview, "overview"),
            ];
        }
        Some(Input::Task) => {
            return vec![
                (Keys::CommandEnter, "start"),
                of(Command::Settings, "settings"),
                of(Command::Overview, "overview"),
            ];
        }
        None => {}
    }
    [
        reader,
        view.history.then_some(of(Command::FileView, "last file")),
        Some(of(Command::Overview, "overview")),
        Some(of(Command::Focus, "focus")),
        Some(of(Command::NewTab, "new tab")),
        Some(of(Command::Palette, "commands")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The key hints of a screen, with the chords of `platform` read from the command table.
pub(crate) fn hints(screen: Screen, platform: Platform) -> Vec<Hint> {
    commands(screen)
        .into_iter()
        .take(MOST_HINTS)
        .map(|(keys, label)| Hint { keys: keys.text(platform), label })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bar {
    pub(crate) counts: Counts,
    pub(crate) place: Option<String>,
    /// `Review: 3 notes` while the tab has a draft review.
    pub(crate) review: Option<String>,
    pub(crate) note_hint: Option<&'static str>,
    pub(crate) hints: Vec<Hint>,
    /// The active tab's pull request: `PR #42 checks running`, with the tone of its dot.
    pub(crate) pr: Option<(PrTone, String)>,
}

/// Draws the bar. Returns whether the review chip was clicked.
pub(crate) fn show(ui: &mut Ui, rect: Rect, bar: &Bar) -> bool {
    let mut open_review = false;
    let painter = ui.painter();
    let y = rect.center().y;
    let put = |at: Pos2, anchor: Align2, text: &str, font: FontId, color: Color32| {
        painter.text(at, anchor, text, font, color).width()
    };
    let mut x = rect.left() + PAD;
    let dots = [
        (bar.counts.working, brand::WORKING, "working"),
        (bar.counts.needs, brand::NEEDS, "needs you"),
    ];
    for (count, color, word) in dots.into_iter().filter(|(count, ..)| *count > 0) {
        painter.circle_filled(pos2(x + DOT / 2.0, y), DOT / 2.0, color);
        x += DOT + 6.0;
        let label = format!("{count} {word}");
        x += put(pos2(x, y), Align2::LEFT_CENTER, &label, body(FONT), brand::MUTED) + GAP;
    }
    if let Some((tone, text)) = &bar.pr {
        let ink = tone_ink(*tone);
        painter.circle_filled(pos2(x + DOT / 2.0, y), DOT / 2.0, ink);
        x += DOT + 6.0;
        x += put(pos2(x, y), Align2::LEFT_CENTER, text, body(FONT), brand::MUTED) + GAP;
    }
    if let Some(place) = &bar.place {
        x += put(pos2(x, y), Align2::LEFT_CENTER, place, body(FONT), brand::MUTED) + GAP;
    }
    if let Some(review) = &bar.review {
        let galley = ui.painter().layout_no_wrap(review.clone(), body(FONT), brand::TEAL_TEXT);
        let chip = Rect::from_min_size(
            pos2(x, y - CHIP_H / 2.0),
            vec2(galley.size().x + 2.0 * CHIP_PAD, CHIP_H),
        );
        let response = ui.interact(chip, Id::new("status-bar-review"), Sense::click());
        let fill = brand::TEAL.gamma_multiply(if response.hovered() { 0.2 } else { 0.1 });
        ui.painter().rect_filled(chip, CornerRadius::same(255), fill);
        ui.painter().galley(chip.center() - galley.size() / 2.0, galley, brand::TEAL_TEXT);
        open_review = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Open your review above the agent's chat box")
            .clicked();
        x += chip.width() + GAP;
    }
    if let Some(hint) = bar.note_hint {
        put(pos2(x, y), Align2::LEFT_CENTER, hint, body(FONT), brand::MUTED);
    }
    let mut right = rect.right() - PAD;
    for hint in bar.hints.iter().rev() {
        right -= put(pos2(right, y), Align2::RIGHT_CENTER, hint.label, body(FONT), brand::MUTED);
        right -= 6.0;
        right -= put(pos2(right, y), Align2::RIGHT_CENTER, &hint.keys, mono(FONT), brand::SOFT);
        right -= GAP;
    }
    open_review
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(focus: bool, history: bool, file_open: bool, reader: Option<bool>) -> Screen {
        Screen::Workspace(Workspace { focus, history, file_open, reader, input: None })
    }

    fn labels(screen: Screen) -> Vec<&'static str> {
        hints(screen, Platform::Mac).into_iter().map(|hint| hint.label).collect()
    }

    #[test]
    fn only_working_and_needs_you_are_counted() {
        let cases = [
            (vec![], Counts { working: 0, needs: 0 }),
            (vec![Tone::Shell, Tone::Done, Tone::Failed, Tone::Setup], Counts::default()),
            (
                vec![Tone::Working, Tone::Working, Tone::Needs, Tone::Done],
                Counts { working: 2, needs: 1 },
            ),
        ];
        for (tones, expected) in cases {
            assert_eq!(counts(&tones), expected, "{tones:?}");
        }
    }

    #[test]
    fn the_place_names_the_branch_and_where_it_forked_from() {
        assert_eq!(place("agent/sunny-reef", Some("main")), "agent/sunny-reef from main");
        assert_eq!(place("agent/sunny-reef", None), "agent/sunny-reef");
    }

    #[test]
    fn each_screen_shows_only_the_hints_that_still_apply() {
        let cases = [
            (Screen::Welcome, vec!["open repo", "settings"]),
            (Screen::Settings, vec!["close settings", "overview", "commands"]),
            (Screen::Overview, vec!["close overview", "new tab", "settings", "commands"]),
            (
                workspace(false, true, false, Some(true)),
                vec!["terminal", "last file", "overview", "focus"],
            ),
            (
                workspace(false, true, false, Some(false)),
                vec!["reader", "last file", "overview", "focus"],
            ),
            (
                workspace(false, true, false, None),
                vec!["last file", "overview", "focus", "new tab"],
            ),
            (
                workspace(false, false, false, None),
                vec!["overview", "focus", "new tab", "commands"],
            ),
            (
                workspace(false, true, true, Some(true)),
                vec!["terminal", "next file", "overview", "focus"],
            ),
            (
                workspace(true, false, false, Some(true)),
                vec!["leave focus", "terminal", "overview", "commands"],
            ),
            (workspace(true, false, false, None), vec!["leave focus", "overview", "commands"]),
        ];
        for (screen, expected) in cases {
            assert_eq!(labels(screen), expected, "{screen:?}");
        }
    }

    fn typing(input: Input) -> Screen {
        Screen::Workspace(Workspace {
            focus: false,
            history: false,
            file_open: false,
            reader: Some(true),
            input: Some(input),
        })
    }

    #[test]
    fn a_text_box_that_owns_the_keyboard_shows_its_own_keys() {
        let keys = |screen, platform| -> Vec<String> {
            hints(screen, platform).into_iter().map(|hint| hint.keys).collect()
        };
        assert_eq!(labels(typing(Input::Prompt)), ["send", "new line", "terminal", "overview"]);
        assert_eq!(keys(typing(Input::Prompt), Platform::Mac)[..2], ["Enter", "Shift+Enter"]);
        assert_eq!(labels(typing(Input::Task)), ["start", "settings", "overview"]);
        assert_eq!(keys(typing(Input::Task), Platform::Mac)[0], "Cmd+Enter");
        assert_eq!(keys(typing(Input::Task), Platform::Linux)[0], "Ctrl+Enter");
    }

    #[test]
    fn the_note_hint_shows_with_a_file_open_and_nowhere_else() {
        let hint = Some("Hover a line and press + to leave a note");
        assert_eq!(note_hint(workspace(false, true, true, Some(true))), hint);
        assert_eq!(note_hint(workspace(false, true, false, Some(true))), None);
        assert_eq!(note_hint(workspace(true, true, true, None)), None);
        assert_eq!(note_hint(Screen::Overview), None);
        assert_eq!(note_hint(Screen::Welcome), None);
        assert_eq!(note_hint(Screen::Settings), None);
    }

    #[test]
    fn the_chords_come_from_the_command_table_of_the_platform() {
        let screen = workspace(false, true, false, Some(true));
        let keys = |platform| -> Vec<String> {
            hints(screen, platform).into_iter().map(|hint| hint.keys).collect()
        };
        assert_eq!(keys(Platform::Mac), ["Cmd+Shift+T", "Cmd+J", "Cmd+0", "Cmd+."]);
        assert_eq!(keys(Platform::Linux), ["Ctrl+Shift+R", "Ctrl+Shift+J", "Alt+0", "Ctrl+."]);
    }

    #[test]
    fn no_screen_has_more_than_four_hints_or_fewer_than_two() {
        let screens = [
            Screen::Welcome,
            typing(Input::Prompt),
            typing(Input::Task),
            Screen::Settings,
            Screen::Overview,
            workspace(false, true, false, Some(true)),
            workspace(false, false, false, None),
            workspace(false, true, true, None),
            workspace(true, true, false, Some(false)),
            workspace(true, false, false, None),
        ];
        for screen in screens {
            let count = hints(screen, Platform::CURRENT).len();
            assert!((2..=4).contains(&count), "{screen:?}: {count}");
        }
    }
}
