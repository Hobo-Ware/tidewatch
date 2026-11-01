//! The status bar along the bottom of the window: how many agents work and need you, where this
//! agent tab stands, and the keys that matter on the screen. The logic is pure; `show` only draws.

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Pos2, Rect, Sense, Stroke, Ui,
    UiBuilder, pos2, vec2,
};

use crate::brand;
use tidewatch_core::pr_status::Tone as PrTone;

use crate::commands::{Command, Platform};
use crate::new_tab::Kind;
use crate::pr_strip::tone_ink;
use crate::screen_kit::{Size, button_width, quiet_button};
use crate::screen_kit::{TEXT_SECONDARY, TEXT_TAG};
use crate::screen_kit::{body, mono};
use crate::token_budget::{Mark, Meter};
use tidewatch_core::panel_view::Tone;

pub(crate) const HEIGHT: f32 = 30.0;
const PAD: f32 = crate::layout::EDGE;
const GAP: f32 = 18.0;
const DOT: f32 = 7.0;
const FONT: f32 = TEXT_SECONDARY;
const MOST_HINTS: usize = 4;
const CHIP_H: f32 = Size::Tag.height();
const AWAKE_R: f32 = 5.0;
const FETCH_FAILED: &str = "Fetch failed";
const CHIP_PAD: f32 = Size::Tag.pad();
const SWITCHING_TITLE_CHARS: usize = 36;
const METER_GAP: f32 = 14.0;
const METER_PART_GAP: f32 = 6.0;
const TRACK_W: f32 = 56.0;
const TRACK_H: f32 = 5.0;
const TRACK_RADIUS: u8 = 3;

pub(crate) mod slow;

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
            Tone::Needs | Tone::Blocked => counts.needs += 1,
            Tone::Setup
            | Tone::Waiting
            | Tone::Done
            | Tone::Failed
            | Tone::Stopped
            | Tone::Shell => {}
        }
        counts
    })
}

/// `agent/sunny-reef from main`, or just the branch when no base is known.
pub(crate) fn place(branch: &str, base: Option<&str>) -> String {
    base.map_or_else(|| branch.to_owned(), |base| format!("{branch} from {base}"))
}

/// What the bar says on the left of a new-tab form.
pub(crate) fn draft_place(kind: Kind) -> String {
    let start = match kind {
        Kind::Meeseeks => "Summon",
        Kind::Agent | Kind::Shell => "Start",
    };
    format!("New tab \u{b7} nothing starts until you press {start}")
}

/// What the bar says while a tab change waits for the agent's turn to end.
pub(crate) fn switching_text(title: &str) -> String {
    let mut chars = title.chars();
    let short: String = chars.by_ref().take(SWITCHING_TITLE_CHARS).collect();
    let cut = if chars.next().is_some() { "..." } else { "" };
    format!("Switching {short}{cut} when its turn ends")
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
    /// The new-tab form: nothing starts before Start.
    Form,
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
        Some(Input::Form) => {
            return vec![(Keys::CommandEnter, "start"), (Keys::Text("Esc"), "close tab")];
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
    /// The tooltip of the mark that shows while the computer is kept awake.
    pub(crate) awake: Option<String>,
    /// The tooltip of `Fetch failed`, which shows after a background fetch of the repo on show failed.
    pub(crate) fetch_failed: Option<String>,
    /// A tab change that waits for the agent's turn to end, with a Cancel.
    pub(crate) switching: Option<String>,
    /// The oldest git or gh call that has run longer than 10 s, with a Cancel.
    pub(crate) slow: Option<slow::Line>,
    /// The active tab's tokens.
    pub(crate) budget: Option<Mark>,
    /// The plan limits with a current reading, five-hour first.
    pub(crate) meters: Vec<Meter>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Clicks {
    pub(crate) open_review: bool,
    pub(crate) cancel_switch: bool,
    pub(crate) cancel_slow: Option<u64>,
}

#[derive(Clone, Copy)]
enum Cancelling {
    Switch,
    Slow(u64),
}

struct MeterLayout<'a> {
    meter: &'a Meter,
    label: std::sync::Arc<egui::Galley>,
    percent: std::sync::Arc<egui::Galley>,
}

impl MeterLayout<'_> {
    fn width(&self) -> f32 {
        self.label.size().x + METER_PART_GAP + TRACK_W + METER_PART_GAP + self.percent.size().x
    }
}

/// Draws the meters with their right edge at `right`, the way the board's option A shows them, and
/// returns where the room left of them ends. They give way to the left side of the bar: when all do
/// not fit between `left` and `right` the fullest one is drawn alone, and when that does not fit
/// none is.
fn draw_meters(ui: &Ui, meters: &[Meter], y: f32, left: f32, right: f32) -> f32 {
    let laid: Vec<MeterLayout<'_>> = meters
        .iter()
        .map(|meter| MeterLayout {
            meter,
            label: ui.painter().layout_no_wrap(
                meter.label.to_owned(),
                mono(TEXT_TAG),
                brand::MUTED,
            ),
            percent: ui.painter().layout_no_wrap(
                meter.percent_text(),
                mono(TEXT_TAG),
                brand::MUTED,
            ),
        })
        .collect();
    let total = |parts: &[&MeterLayout<'_>]| {
        parts.iter().map(|part| part.width()).sum::<f32>()
            + METER_GAP * parts.len().saturating_sub(1) as f32
    };
    let all: Vec<&MeterLayout<'_>> = laid.iter().collect();
    let fullest = laid.iter().max_by_key(|part| part.meter.percent);
    let shown: Vec<&MeterLayout<'_>> = if left + total(&all) <= right {
        all
    } else {
        fullest.into_iter().filter(|part| left + part.width() <= right).collect()
    };
    if shown.is_empty() {
        return right;
    }
    let mut x = right - total(&shown);
    let start = x;
    for part in shown {
        let meter = part.meter;
        let hit = Rect::from_min_size(pos2(x, y - HEIGHT / 2.0), vec2(part.width(), HEIGHT));
        ui.painter().galley(
            pos2(x, y - part.label.size().y / 2.0),
            part.label.clone(),
            brand::MUTED,
        );
        x += part.label.size().x + METER_PART_GAP;
        let track = Rect::from_min_size(pos2(x, y - TRACK_H / 2.0), vec2(TRACK_W, TRACK_H));
        ui.painter().rect_filled(track, CornerRadius::same(TRACK_RADIUS), brand::FIELD_LINE);
        let fill = Rect::from_min_size(track.min, vec2(TRACK_W * meter.fraction(), TRACK_H));
        let ink = if meter.hot { brand::ORANGE } else { brand::TEAL };
        ui.painter().rect_filled(fill, CornerRadius::same(TRACK_RADIUS), ink);
        x += TRACK_W + METER_PART_GAP;
        ui.painter().galley(
            pos2(x, y - part.percent.size().y / 2.0),
            part.percent.clone(),
            brand::MUTED,
        );
        x += part.percent.size().x + METER_GAP;
        ui.interact(hit, Id::new(("status-bar-meter", meter.label)), Sense::hover())
            .on_hover_text(meter.tip.as_str());
    }
    start - GAP
}

/// Draws the bar and says what was clicked.
pub(crate) fn show(ui: &mut Ui, rect: Rect, bar: &Bar) -> Clicks {
    let mut clicks = Clicks::default();
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
    if let Some(tip) = &bar.awake {
        let center = pos2(x + AWAKE_R + 1.0, y);
        painter.circle_stroke(center, AWAKE_R, Stroke::new(1.2, brand::SOFT));
        painter.circle_filled(center, 1.6, brand::SOFT);
        let mark = Rect::from_center_size(center, vec2(2.0 * AWAKE_R + 8.0, HEIGHT));
        ui.interact(mark, Id::new("status-bar-awake"), Sense::hover()).on_hover_text(tip.as_str());
        x += 2.0 * AWAKE_R + GAP;
    }
    if let Some(why) = &bar.fetch_failed {
        let width = put(pos2(x, y), Align2::LEFT_CENTER, FETCH_FAILED, body(FONT), brand::MUTED);
        let label = Rect::from_min_size(pos2(x, y - HEIGHT / 2.0), vec2(width, HEIGHT));
        ui.interact(label, Id::new("status-bar-fetch"), Sense::hover()).on_hover_text(why.as_str());
        x += width + GAP;
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
        ui.painter().rect_filled(chip, CornerRadius::same(Size::Tag.radius()), fill);
        ui.painter().galley(chip.center() - galley.size() / 2.0, galley, brand::TEAL_TEXT);
        clicks.open_review = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Open your review above the agent's chat box")
            .clicked();
        x += chip.width() + GAP;
    }
    let mut hints_left = rect.right() - PAD;
    for hint in bar.hints.iter().rev() {
        hints_left -=
            put(pos2(hints_left, y), Align2::RIGHT_CENTER, hint.label, body(FONT), brand::MUTED);
        hints_left -= 6.0;
        hints_left -=
            put(pos2(hints_left, y), Align2::RIGHT_CENTER, &hint.keys, mono(FONT), brand::SOFT);
        hints_left -= GAP;
    }
    hints_left = draw_meters(ui, &bar.meters, y, x, hints_left);
    if let Some(mark) = &bar.budget {
        let galley = ui.painter().layout_no_wrap(mark.text.clone(), body(FONT), brand::MUTED);
        if x + galley.size().x + GAP <= hints_left {
            let area =
                Rect::from_min_size(pos2(x, y - HEIGHT / 2.0), vec2(galley.size().x, HEIGHT));
            ui.painter().galley(pos2(x, y - galley.size().y / 2.0), galley, brand::MUTED);
            ui.interact(area, Id::new("status-bar-budget"), Sense::hover())
                .on_hover_text(mark.tip.as_str());
            x += area.width() + GAP;
        }
    }
    let cancel_w = button_width(ui, "Cancel", Size::Tag);
    let mut cancels = Vec::new();
    if let Some(text) = &bar.switching {
        x += put(pos2(x, y), Align2::LEFT_CENTER, text, body(FONT), brand::TEAL_TEXT) + 10.0;
        cancels.push((Cancelling::Switch, x));
        x += cancel_w + GAP;
    }
    if let Some(line) = &bar.slow {
        let galley = ui.painter().layout_no_wrap(line.text.clone(), body(FONT), brand::ORANGE);
        if slow::fits(x, galley.size().x, cancel_w, hints_left) {
            x += put(pos2(x, y), Align2::LEFT_CENTER, &line.text, body(FONT), brand::ORANGE) + 10.0;
            cancels.push((Cancelling::Slow(line.id), x));
            x += cancel_w + GAP;
        }
    }
    if let Some(hint) = bar.note_hint {
        put(pos2(x, y), Align2::LEFT_CENTER, hint, body(FONT), brand::MUTED);
    }
    for (which, at) in cancels {
        let slot = Rect::from_min_size(
            pos2(at, y - Size::Tag.height() / 2.0),
            vec2(cancel_w, Size::Tag.height()),
        );
        let cancel = ui.scope_builder(
            UiBuilder::new()
                .id_salt(("status-bar-cancel", y.to_bits(), at.to_bits()))
                .max_rect(slot)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| quiet_button(ui, "Cancel", true, Size::Tag),
        );
        if cancel.inner.clicked() {
            match which {
                Cancelling::Switch => clicks.cancel_switch = true,
                Cancelling::Slow(id) => clicks.cancel_slow = Some(id),
            }
        }
    }
    clicks
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
    fn a_new_tab_form_hints_start_and_close_and_says_nothing_starts_yet() {
        let form = Screen::Workspace(Workspace {
            focus: false,
            history: true,
            file_open: false,
            reader: None,
            input: Some(Input::Form),
        });
        assert_eq!(labels(form), ["start", "close tab"]);
        let caps = |platform| -> Vec<String> {
            hints(form, platform).into_iter().map(|hint| hint.keys).collect()
        };
        assert_eq!(caps(Platform::Mac), ["Cmd+Enter", "Esc"]);
        assert_eq!(caps(Platform::Linux), ["Ctrl+Enter", "Esc"]);
        assert_eq!(draft_place(Kind::Agent), "New tab \u{b7} nothing starts until you press Start");
        assert_eq!(
            draft_place(Kind::Meeseeks),
            "New tab \u{b7} nothing starts until you press Summon"
        );
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
            (
                vec![Tone::Blocked, Tone::Needs, Tone::Waiting, Tone::Working],
                Counts { working: 1, needs: 2 },
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

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn bar(switching: Option<&str>) -> Bar {
        bar_with(switching, None)
    }

    fn slow_line(secs: u64, more: usize) -> slow::Line {
        let text = slow::text("git worktree add", std::time::Duration::from_secs(secs), more);
        slow::Line { id: 5, text }
    }

    fn bar_with(switching: Option<&str>, slow: Option<slow::Line>) -> Bar {
        Bar {
            counts: Counts::default(),
            place: None,
            review: None,
            note_hint: None,
            hints: hints(Screen::Welcome, Platform::Mac),
            pr: None,
            awake: None,
            fetch_failed: None,
            switching: switching.map(switching_text),
            slow,
            budget: None,
            meters: Vec::new(),
        }
    }

    fn frame(ctx: &egui::Context, bar: &Bar, events: Vec<egui::Event>) -> Clicks {
        frame_in(ctx, bar, events, 900.0)
    }

    fn frame_in(ctx: &egui::Context, bar: &Bar, events: Vec<egui::Event>, width: f32) -> Clicks {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 120.0))),
            events,
            focused: true,
            ..egui::RawInput::default()
        };
        let mut clicks = Clicks::default();
        let _ = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT));
            clicks = show(ui, rect, bar);
        });
        clicks
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        ctx
    }

    fn galley_texts(ctx: &egui::Context, bar: &Bar) -> Vec<String> {
        galley_texts_in(ctx, bar, 900.0)
    }

    fn galley_texts_in(ctx: &egui::Context, bar: &Bar, width: f32) -> Vec<String> {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 120.0))),
            focused: true,
            ..egui::RawInput::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT));
            show(ui, rect, bar);
        });
        out.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_switch_line_names_the_tab_and_cuts_a_long_title() {
        assert_eq!(switching_text("fix-ci"), "Switching fix-ci when its turn ends");
        let long = "x".repeat(60);
        assert_eq!(
            switching_text(&long),
            format!("Switching {}... when its turn ends", "x".repeat(SWITCHING_TITLE_CHARS))
        );
    }

    #[test]
    fn a_waiting_switch_shows_its_line_with_a_cancel_that_reports_its_click() {
        let ctx = context();
        let waiting = bar(Some("fix-ci"));
        let texts = galley_texts(&ctx, &waiting);
        assert!(texts.iter().any(|t| t == "Switching fix-ci when its turn ends"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Cancel"), "{texts:?}");
        let quiet = galley_texts(&ctx, &bar(None));
        assert!(!quiet.iter().any(|t| t == "Cancel"), "{quiet:?}");

        assert_eq!(frame(&ctx, &waiting, vec![]), Clicks::default());
        let _ = frame(&ctx, &waiting, vec![key(egui::Key::Tab)]);
        let clicks = frame(&ctx, &waiting, vec![key(egui::Key::Enter)]);
        assert_eq!(clicks, Clicks { cancel_switch: true, ..Clicks::default() });
    }

    fn budget_bar(text: &str) -> Bar {
        Bar { budget: Some(Mark { text: text.into(), tip: "tip".into() }), ..bar(None) }
    }

    fn drawn(ctx: &egui::Context, bar: &Bar, width: f32) -> Vec<egui::epaint::ClippedShape> {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 120.0))),
                focused: true,
                ..egui::RawInput::default()
            },
            |ui| {
                show(ui, Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT)), bar);
            },
        )
        .shapes
    }

    #[test]
    fn the_tokens_mark_shows_on_the_left_in_the_muted_ink() {
        let ctx = context();
        let text = "1.2M tokens";
        let ink: Vec<_> = drawn(&ctx, &budget_bar(text), 1200.0)
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.fallback_color),
                _ => None,
            })
            .collect();
        assert_eq!(ink, [brand::MUTED]);
    }

    #[test]
    fn a_bar_without_a_tokens_mark_draws_none_and_a_narrow_one_leaves_it_out_instead_of_crowding() {
        let ctx = context();
        let text = "1.2M tokens";
        assert_eq!(count(&galley_texts(&ctx, &bar(None)), text), 0);
        assert_eq!(count(&galley_texts(&ctx, &budget_bar(text)), text), 1);
        assert_eq!(count(&galley_texts_in(&ctx, &budget_bar(text), 300.0), text), 0);
    }

    fn meter(label: &'static str, percent: u8, hot: bool) -> Meter {
        Meter { label, percent, hot, tip: format!("{label} tip") }
    }

    fn meter_bar(meters: Vec<Meter>) -> Bar {
        Bar { meters, ..bar(None) }
    }

    fn two() -> Bar {
        meter_bar(vec![meter("5h", 31, false), meter("week", 84, true)])
    }

    fn track_fills(shapes: &[egui::epaint::ClippedShape]) -> Vec<(Color32, f32)> {
        shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.rect.height() == TRACK_H => {
                    Some((rect.fill, rect.rect.width()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn two_meters_draw_a_label_a_track_and_a_percent_with_the_fill_hot_at_the_threshold() {
        let ctx = context();
        let shapes = drawn(&ctx, &two(), 1200.0);
        let want = [
            (brand::FIELD_LINE, TRACK_W),
            (brand::TEAL, TRACK_W * 0.31),
            (brand::FIELD_LINE, TRACK_W),
            (brand::ORANGE, TRACK_W * 0.84),
        ];
        let got = track_fills(&shapes);
        assert_eq!(got.len(), want.len());
        for ((got_ink, got_w), (want_ink, want_w)) in got.into_iter().zip(want) {
            assert_eq!(got_ink, want_ink);
            assert!((got_w - want_w).abs() < 0.1, "{got_w} {want_w}");
        }
        let texts = galley_texts(&ctx, &two());
        for want in ["5h", "31%", "week", "84%"] {
            assert_eq!(count(&texts, want), 1, "{want}: {texts:?}");
        }
    }

    #[test]
    fn the_meters_sit_left_of_the_hints_in_mono_text_and_keep_14_px_between_them() {
        let ctx = context();
        let bar = Bar { hints: hints(Screen::Welcome, Platform::Mac), ..two() };
        let shapes = drawn(&ctx, &bar, 1200.0);
        let spot = |text: &str| {
            shapes.iter().find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some((t.pos.x, t.pos.x + t.galley.size().x))
                }
                _ => None,
            })
        };
        let (week_label, _) = spot("week").unwrap();
        let (_, five_right) = spot("31%").unwrap();
        assert!((week_label - five_right - METER_GAP).abs() < 0.01, "{week_label} {five_right}");
        let (_, week_right) = spot("84%").unwrap();
        let first_hint = bar.hints.first().map(|hint| hint.keys.as_str()).unwrap();
        let (hint_left, _) = spot(first_hint).unwrap();
        assert!(week_right <= hint_left, "{week_right} {hint_left}");
        for clipped in &shapes {
            if let egui::Shape::Text(t) = &clipped.shape
                && ["5h", "31%", "week", "84%"].contains(&t.galley.text())
            {
                assert_eq!(
                    t.galley.job.sections[0].format.font_id.family,
                    egui::FontFamily::Monospace
                );
                assert_eq!(t.galley.job.sections[0].format.font_id.size, TEXT_TAG);
            }
        }
    }

    #[test]
    fn a_meter_shows_its_tooltip_on_hover() {
        let ctx = context();
        let bar = two();
        let at = drawn(&ctx, &bar, 1200.0)
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(t) if t.galley.text() == "week" => Some(t.pos + vec2(4.0, 6.0)),
                _ => None,
            })
            .unwrap();
        let texts = galley_texts_after_hover(&ctx, &bar, at);
        assert_eq!(count(&texts, "week tip"), 1, "{texts:?}");
        let away = galley_texts_after_hover(&context(), &bar, pos2(5.0, 5.0));
        assert_eq!(count(&away, "week tip"), 0, "{away:?}");
    }

    fn galley_texts_after_hover(ctx: &egui::Context, bar: &Bar, at: egui::Pos2) -> Vec<String> {
        let frame = |events: Vec<egui::Event>, time: f64| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 120.0))),
                    events,
                    focused: true,
                    time: Some(time),
                    ..egui::RawInput::default()
                },
                |ui| {
                    show(ui, Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, HEIGHT)), bar);
                },
            )
        };
        let _ = frame(vec![egui::Event::PointerMoved(at)], 0.0);
        let mut out = frame(Vec::new(), 0.1);
        for step in 2..14 {
            out = frame(Vec::new(), 0.1 * f64::from(step));
        }
        out.shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bar_with_no_meters_draws_no_track() {
        let ctx = context();
        assert_eq!(track_fills(&drawn(&ctx, &bar(None), 1200.0)), Vec::new());
    }

    #[test]
    fn a_narrow_bar_keeps_the_fullest_meter_and_a_narrower_one_none() {
        let ctx = context();

        let widths = [1200.0, 560.0, 460.0, 360.0];
        let counted: Vec<usize> = widths
            .iter()
            .map(|width| track_fills(&drawn(&ctx, &two(), *width)).len() / 2)
            .collect();
        assert_eq!(counted, [2, 2, 1, 0], "meters by width {widths:?}");
        let narrow = galley_texts_in(&ctx, &two(), 460.0);
        assert_eq!((count(&narrow, "week"), count(&narrow, "5h")), (1, 0));
    }

    #[test]
    fn the_meters_leave_the_slow_and_switch_lines_their_room() {
        let ctx = context();
        let busy = Bar { switching: Some(switching_text("fix-ci")), ..two() };
        let texts = galley_texts(&ctx, &busy);
        assert_eq!(count(&texts, "Switching fix-ci when its turn ends"), 1, "{texts:?}");
        assert_eq!(count(&texts, "31%"), 1);
    }

    fn count(texts: &[String], want: &str) -> usize {
        texts.iter().filter(|text| text.as_str() == want).count()
    }

    #[test]
    fn a_slow_call_shows_its_line_with_a_cancel_that_reports_its_id() {
        let ctx = context();
        let slow = bar_with(None, Some(slow_line(14, 0)));
        let texts = galley_texts(&ctx, &slow);
        let line = "git worktree add is taking long \u{b7} 14 s";
        assert_eq!(count(&texts, line), 1, "{texts:?}");
        assert_eq!(count(&texts, "Cancel"), 1, "{texts:?}");
        let quiet = galley_texts(&ctx, &bar_with(None, None));
        assert_eq!(count(&quiet, "Cancel"), 0, "{quiet:?}");

        assert_eq!(frame(&ctx, &slow, vec![]), Clicks::default());
        let _ = frame(&ctx, &slow, vec![key(egui::Key::Tab)]);
        let clicks = frame(&ctx, &slow, vec![key(egui::Key::Enter)]);
        assert_eq!(clicks, Clicks { cancel_slow: Some(5), ..Clicks::default() });
    }

    #[test]
    fn a_slow_call_with_others_behind_it_says_how_many() {
        let ctx = context();
        let texts = galley_texts(&ctx, &bar_with(None, Some(slow_line(61, 2))));
        let line = "git worktree add is taking long \u{b7} 1 min 1 s \u{b7} and 2 more";
        assert_eq!(count(&texts, line), 1, "{texts:?}");
    }

    #[test]
    fn both_lines_show_with_their_own_cancel_when_the_bar_is_wide() {
        let ctx = context();
        let both = bar_with(Some("fix-ci"), Some(slow_line(14, 0)));
        let texts = galley_texts_in(&ctx, &both, 1600.0);
        assert_eq!(count(&texts, "Switching fix-ci when its turn ends"), 1, "{texts:?}");
        assert_eq!(count(&texts, "git worktree add is taking long \u{b7} 14 s"), 1, "{texts:?}");
        assert_eq!(count(&texts, "Cancel"), 2, "{texts:?}");

        let _ = frame_in(&ctx, &both, vec![key(egui::Key::Tab)], 1600.0);
        let _ = frame_in(&ctx, &both, vec![key(egui::Key::Tab)], 1600.0);
        let clicks = frame_in(&ctx, &both, vec![key(egui::Key::Enter)], 1600.0);
        assert_eq!(clicks, Clicks { cancel_slow: Some(5), ..Clicks::default() });
    }

    #[test]
    fn on_a_narrow_bar_the_switch_line_wins_and_the_slow_line_waits() {
        let ctx = context();
        let both = bar_with(Some("fix-ci"), Some(slow_line(14, 0)));
        let texts = galley_texts_in(&ctx, &both, 800.0);
        assert_eq!(count(&texts, "Switching fix-ci when its turn ends"), 1, "{texts:?}");
        assert_eq!(count(&texts, "git worktree add is taking long \u{b7} 14 s"), 0, "{texts:?}");
        assert_eq!(count(&texts, "Cancel"), 1, "{texts:?}");
    }

    #[test]
    fn a_cancel_in_the_bar_is_a_tag_tall_button_that_fits_inside_it() {
        let ctx = context();
        let both = bar_with(Some("fix-ci"), Some(slow_line(14, 0)));
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 120.0))),
            focused: true,
            ..egui::RawInput::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, HEIGHT));
            show(ui, rect, &both);
        });
        let mut heights = Vec::new();
        for clipped in &out.shapes {
            if let egui::Shape::Rect(rect) = &clipped.shape
                && rect.rect.width() > 40.0
            {
                heights.push(rect.rect.height());
            }
        }
        assert_eq!(heights, vec![Size::Tag.height(); 2]);
        assert!(Size::Tag.height() < HEIGHT);
    }
}
