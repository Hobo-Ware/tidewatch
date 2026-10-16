//! Dev-only switches for the Reader's checks, read once at startup.
//!
//! - `TIDEWATCH_DEV_READER=raw`: agent tabs start on the raw terminal instead of the Reader.
//! - `TIDEWATCH_DEV_READER_KEY=<seconds>`: press the Reader chord once, that long after startup, through
//!   the same key path a real press takes.
//! - `TIDEWATCH_DEV_ANSWER=allow|deny`: click that permission button once, when the Reader shows one.
//! - `TIDEWATCH_DEV_REPLY=<text>` after `TIDEWATCH_DEV_REPLY_AFTER=<seconds>` (default 0): send a
//!   reply from the Reader's box once, when it is enabled.
//! - `TIDEWATCH_DEV_QUESTION=<steps>`: drive the live question card, one step every 0.4 s once it
//!   shows. Steps, comma separated: `pick<N>` (the number key), `next`, `back`, `space`, `down`,
//!   `type:<text>` ("Something else" with that text), `send`, `skip`.

use std::collections::VecDeque;

use egui::Context;
use tidewatch_core::question::Reply;

use crate::commands::{self, Command};
use crate::question_card::{CardKey, Effect, Row};
use crate::question_view::Live;
use crate::reader_keys::Answer;
use crate::reader_view::{Action, Prompt};

const QUESTION_STEP_S: f64 = 0.4;

#[derive(Clone, Debug, PartialEq, Eq)]
enum CardStep {
    Key(CardKey),
    Back,
    Type(String),
    Send,
    Skip,
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
    question: VecDeque<CardStep>,
    question_at: f64,
}

impl DevReader {
    pub(crate) fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self::parse(
            var("TIDEWATCH_DEV_READER").as_deref(),
            var("TIDEWATCH_DEV_READER_KEY").as_deref(),
            var("TIDEWATCH_DEV_ANSWER").as_deref(),
            var("TIDEWATCH_DEV_REPLY").as_deref(),
            var("TIDEWATCH_DEV_REPLY_AFTER").as_deref(),
            var("TIDEWATCH_DEV_QUESTION").as_deref(),
        )
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
            question_at: 0.0,
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
            && self.question.is_empty()
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
            Effect::Nothing | Effect::TypeOther => None,
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
            Some("pick2,next,type:both,send,bogus"),
        );
        assert!(dev.raw);
        assert_eq!(
            dev.question,
            [
                CardStep::Key(CardKey::Digit(2)),
                CardStep::Key(CardKey::Enter),
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
}
