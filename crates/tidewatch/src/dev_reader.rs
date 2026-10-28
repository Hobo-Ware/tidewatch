//! Dev-only switches for the Reader's checks, read once at startup.
//!
//! - `TIDEWATCH_DEV_READER=raw`: agent tabs start on the raw terminal instead of the Reader.
//! - `TIDEWATCH_DEV_READER_KEY=<seconds>`: press the Reader chord once, that long after startup, through
//!   the same key path a real press takes.
//! - `TIDEWATCH_DEV_ANSWER=allow|deny`: click that permission button once, when the Reader shows one.
//! - `TIDEWATCH_DEV_REPLY=<text>` after `TIDEWATCH_DEV_REPLY_AFTER=<seconds>` (default 0): send a
//!   reply from the Reader's box once, when it is enabled.
//! - `TIDEWATCH_DEV_PICTURES=<path>:<path>`: attach those pictures to the message box once, as a paste of
//!   screenshots does.
//! - `TIDEWATCH_DEV_TURNS=open`: open the list of calls and the folded updates under every agent turn
//!   once, as a click on a chip does; `notes` opens only the folded updates.
//! - `TIDEWATCH_DEV_READER_SCROLL=<pixels>@<seconds>`: once, that long after startup, scroll the
//!   transcript up from its bottom by that many pixels, as a wheel does (the jump pill shows).
//! - `TIDEWATCH_DEV_PICK=<digit>[@<seconds>]`: press that digit once, that long after startup, to pick a ready answer.
//! - `TIDEWATCH_DEV_QUESTION=<steps>`: drive the live question card, one step every 0.4 s once it
//!   shows. Steps, comma separated: `pick<N>` (the number key), `next`, `back`, `space`, `down`,
//!   `all` (Go with your picks), `type:<text>` ("Something else" with that text), `send`, `skip`.
//! - `TIDEWATCH_DEV_AUTOPICK_SECS=<seconds>`: how long a question waits in a tab that runs on its own
//!   before Tidewatch answers it (default five minutes).

use std::collections::VecDeque;
use std::path::PathBuf;

use egui::Context;
use tidewatch_core::question::Reply;

use crate::commands::{self, Command};
use crate::question_card::{CardKey, Effect, Row};
use crate::question_view::Live;
use crate::reader_keys::Answer;
use crate::reader_view::{Action, Prompt};

const QUESTION_STEP_S: f64 = 0.4;

/// How long an unanswered question waits in a tab that runs on its own, read once.
pub(crate) fn autopick_wait() -> f64 {
    static WAIT: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *WAIT.get_or_init(|| {
        std::env::var("TIDEWATCH_DEV_AUTOPICK_SECS")
            .ok()
            .and_then(|secs| secs.parse::<f64>().ok())
            .filter(|secs| *secs > 0.0)
            .unwrap_or(crate::question_auto::WAIT_SECS)
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CardStep {
    Key(CardKey),
    Back,
    Type(String),
    Send,
    Skip,
}

fn scroll_of(text: &str) -> Option<(f32, f64)> {
    let (pixels, after) = text.split_once('@').unwrap_or((text, "0"));
    Some((pixels.parse().ok()?, after.parse().ok()?))
}

fn card_steps(text: &str) -> VecDeque<CardStep> {
    text.split(',')
        .filter_map(|step| {
            let step = step.trim();
            if let Some(number) = step.strip_prefix("pick") {
                return number.parse().ok().map(|n| CardStep::Key(CardKey::Digit(n)));
            }
            match step {
                "next" => Some(CardStep::Key(CardKey::Enter)),
                "space" => Some(CardStep::Key(CardKey::Space)),
                "down" => Some(CardStep::Key(CardKey::Down)),
                "all" => Some(CardStep::Key(CardKey::Picks)),
                "back" => Some(CardStep::Back),
                "send" => Some(CardStep::Send),
                "skip" => Some(CardStep::Skip),
                other => other.strip_prefix("type:").map(|text| CardStep::Type(text.to_owned())),
            }
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevReader {
    pub(crate) raw: bool,
    key_after: Option<f64>,
    answer: Option<Answer>,
    reply: Option<(String, f64)>,
    pictures: Vec<PathBuf>,
    question: VecDeque<CardStep>,
    question_at: f64,
    open_turns: bool,
    notes_only: bool,
    scroll_up: Option<(f32, f64)>,
    digit: Option<(egui::Key, char, f64)>,
}

fn digit_of(text: &str) -> Option<(egui::Key, char, f64)> {
    let (digit, after) = text.split_once('@').unwrap_or((text, "0"));
    let digit = digit.chars().next().filter(char::is_ascii_digit)?;
    let key = egui::Key::from_name(&digit.to_string())?;
    Some((key, digit, after.parse().ok()?))
}

impl DevReader {
    pub(crate) fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        let turns = var("TIDEWATCH_DEV_TURNS");
        let open_turns = matches!(turns.as_deref(), Some("open" | "notes"));
        let notes_only = turns.as_deref() == Some("notes");
        let parsed = Self::parse(
            var("TIDEWATCH_DEV_READER").as_deref(),
            var("TIDEWATCH_DEV_READER_KEY").as_deref(),
            var("TIDEWATCH_DEV_ANSWER").as_deref(),
            var("TIDEWATCH_DEV_REPLY").as_deref(),
            var("TIDEWATCH_DEV_REPLY_AFTER").as_deref(),
            var("TIDEWATCH_DEV_QUESTION").as_deref(),
        );
        let pictures = var("TIDEWATCH_DEV_PICTURES")
            .map(|paths| paths.split(':').filter(|p| !p.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default();
        let scroll_up = var("TIDEWATCH_DEV_READER_SCROLL").and_then(|text| scroll_of(&text));
        let digit = var("TIDEWATCH_DEV_PICK").and_then(|text| digit_of(&text));
        Self { pictures, open_turns, notes_only, scroll_up, digit, ..parsed }
    }

    fn parse(
        reader: Option<&str>,
        key: Option<&str>,
        answer: Option<&str>,
        reply: Option<&str>,
        reply_after: Option<&str>,
        question: Option<&str>,
    ) -> Self {
        Self {
            question: question.map(card_steps).unwrap_or_default(),
            pictures: Vec::new(),
            question_at: 0.0,
            open_turns: false,
            notes_only: false,
            scroll_up: None,
            digit: None,
            raw: reader == Some("raw"),
            key_after: key.and_then(|s| s.parse().ok()),
            answer: match answer {
                Some("allow") => Some(Answer::Allow),
                Some("deny") => Some(Answer::Deny),
                _ => None,
            },
            reply: reply
                .filter(|r| !r.is_empty())
                .map(|r| (r.to_owned(), reply_after.and_then(|s| s.parse().ok()).unwrap_or(0.0))),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.key_after.is_none()
            && self.answer.is_none()
            && self.reply.is_none()
            && self.pictures.is_empty()
            && self.question.is_empty()
            && !self.open_turns
            && self.scroll_up.is_none()
            && self.digit.is_none()
    }

    pub(crate) fn take_pictures(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.pictures)
    }

    /// Whether the turns open only their folded updates, not their calls.
    pub(crate) fn notes_only(&self) -> bool {
        self.notes_only
    }

    /// True once, when turns should be opened and there are rows to open them on.
    pub(crate) fn take_open_turns(&mut self, rows: usize) -> bool {
        let now = self.open_turns && rows > 0;
        if now {
            self.open_turns = false;
        }
        now
    }

    /// The pixels to scroll up from the bottom, once, when their time has come.
    pub(crate) fn take_scroll_up(&mut self, now: f64) -> Option<f32> {
        let (pixels, after) = self.scroll_up?;
        (now >= after).then(|| {
            self.scroll_up = None;
            pixels
        })
    }

    /// Queues the Reader chord's key event for this frame once its time has come.
    pub(crate) fn press_toggle(&mut self, ctx: &Context, now: f64) {
        if self.key_after.is_none_or(|after| now < after) {
            return;
        }
        self.key_after = None;
        if let Some(event) = commands::press_event(Command::Reader) {
            ctx.input_mut(|i| i.events.push(event));
        }
    }

    /// Queues the digit key's events for this frame once its time has come.
    pub(crate) fn press_digit(&mut self, ctx: &Context, now: f64) {
        let Some((key, digit, _)) = self.digit.filter(|(_, _, after)| now >= *after) else {
            return;
        };
        self.digit = None;
        ctx.input_mut(|i| {
            i.events.push(egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            i.events.push(egui::Event::Text(digit.to_string()));
        });
    }

    /// The next scripted step on the live question card, when there is one and its time has come.
    pub(crate) fn drive_card(&mut self, now: f64, live: Option<&mut Live>) -> Option<Action> {
        let live = live?;
        if now < self.question_at {
            return None;
        }
        let step = self.question.pop_front()?;
        self.question_at = now + QUESTION_STEP_S;
        let seq = live.seq;
        let effect = match step {
            CardStep::Key(key) => live.card.key(key),
            CardStep::Back => {
                live.card.back();
                Effect::Nothing
            }
            CardStep::Type(text) => {
                live.card.choose(Row::Other);
                live.card.typed_mut().push_str(&text);
                Effect::Nothing
            }
            CardStep::Send => live.card.send(),
            CardStep::Skip => Effect::Skip,
        };
        match effect {
            Effect::Send(given) => Some(Action::Ask(seq, Reply::Answered(given))),
            Effect::Skip => Some(Action::Ask(seq, Reply::Skipped)),
            Effect::GoWithPicks if live.picks > 0 => Some(Action::GoWithPicks),
            Effect::Nothing | Effect::TypeOther | Effect::GoWithPicks => None,
        }
    }

    /// What a click or a send would do now, for a Reader that is on screen.
    pub(crate) fn actions(&mut self, now: f64, prompt: Option<Prompt>) -> Vec<Action> {
        let mut actions = Vec::new();
        if prompt == Some(Prompt::Permission)
            && let Some(answer) = self.answer.take()
        {
            actions.push(Action::Answer(answer));
        }
        if prompt.is_none()
            && let Some((_, after)) = &self.reply
            && now >= *after
            && let Some((text, _)) = self.reply.take()
        {
            actions.push(Action::Reply(text));
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_switches_means_idle() {
        assert!(DevReader::parse(None, None, None, None, None, None).is_idle());
    }

    #[test]
    fn the_switches_are_read() {
        let dev = DevReader::parse(
            Some("raw"),
            Some("12"),
            Some("allow"),
            Some("hi"),
            Some("5"),
            Some("pick2,next,all,type:both,send,bogus"),
        );
        assert!(dev.raw);
        assert_eq!(
            dev.question,
            [
                CardStep::Key(CardKey::Digit(2)),
                CardStep::Key(CardKey::Enter),
                CardStep::Key(CardKey::Picks),
                CardStep::Type("both".into()),
                CardStep::Send,
            ]
        );
        assert_eq!(dev.key_after, Some(12.0));
        assert_eq!(dev.answer, Some(Answer::Allow));
        assert_eq!(dev.reply, Some(("hi".into(), 5.0)));
    }

    #[test]
    fn an_answer_waits_for_a_permission_prompt_and_a_reply_for_a_free_box() {
        let mut dev = DevReader::parse(None, None, Some("deny"), Some("go"), None, None);
        assert_eq!(dev.actions(1.0, Some(Prompt::Question)), Vec::<Action>::new());
        assert_eq!(dev.actions(1.0, Some(Prompt::Permission)), [Action::Answer(Answer::Deny)]);
        assert_eq!(dev.actions(1.0, Some(Prompt::Permission)), Vec::<Action>::new());
        assert_eq!(dev.actions(1.0, None), [Action::Reply("go".into())]);
        assert!(dev.is_idle());
    }

    #[test]
    fn the_scroll_switch_reads_pixels_and_a_delay_and_fires_once() {
        assert_eq!(scroll_of("40@6"), Some((40.0, 6.0)));
        assert_eq!(scroll_of("40"), Some((40.0, 0.0)));
        assert_eq!(scroll_of("up"), None);
        let mut dev = DevReader { scroll_up: scroll_of("40@6"), ..DevReader::default() };
        assert_eq!(dev.take_scroll_up(5.0), None);
        assert_eq!(dev.take_scroll_up(6.0), Some(40.0));
        assert_eq!(dev.take_scroll_up(7.0), None);
    }

    #[test]
    fn open_turns_fires_once_and_only_when_there_are_rows() {
        let mut dev = DevReader { open_turns: true, ..DevReader::default() };
        assert!(!dev.take_open_turns(0));
        assert!(dev.take_open_turns(3));
        assert!(!dev.take_open_turns(3));
    }
}
