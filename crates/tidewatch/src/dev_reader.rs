//! Dev-only switches for the Reader's checks, read once at startup.
//!
//! - `TIDEWATCH_DEV_READER=raw`: agent tabs start on the raw terminal instead of the Reader.
//! - `TIDEWATCH_DEV_READER_KEY=<seconds>`: press Cmd+Shift+T once, that long after startup, through
//!   the same key path a real press takes.
//! - `TIDEWATCH_DEV_ANSWER=allow|deny`: click that permission button once, when the Reader shows one.
//! - `TIDEWATCH_DEV_REPLY=<text>` after `TIDEWATCH_DEV_REPLY_AFTER=<seconds>` (default 0): send a
//!   reply from the Reader's box once, when it is enabled.

use egui::{Context, Event, Key, Modifiers};

use crate::reader_keys::Answer;
use crate::reader_view::{Action, Prompt};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevReader {
    pub(crate) raw: bool,
    key_after: Option<f64>,
    answer: Option<Answer>,
    reply: Option<(String, f64)>,
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
        )
    }

    fn parse(
        reader: Option<&str>,
        key: Option<&str>,
        answer: Option<&str>,
        reply: Option<&str>,
        reply_after: Option<&str>,
    ) -> Self {
        Self {
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
        self.key_after.is_none() && self.answer.is_none() && self.reply.is_none()
    }

    /// Queues the Cmd+Shift+T key event for this frame once its time has come.
    pub(crate) fn press_toggle(&mut self, ctx: &Context, now: f64) {
        if self.key_after.is_none_or(|after| now < after) {
            return;
        }
        self.key_after = None;
        let modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };
        ctx.input_mut(|i| {
            i.events.push(Event::Key {
                key: Key::T,
                physical_key: Some(Key::T),
                pressed: true,
                repeat: false,
                modifiers,
            });
        });
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
        assert!(DevReader::parse(None, None, None, None, None).is_idle());
    }

    #[test]
    fn the_switches_are_read() {
        let dev = DevReader::parse(Some("raw"), Some("12"), Some("allow"), Some("hi"), Some("5"));
        assert!(dev.raw);
        assert_eq!(dev.key_after, Some(12.0));
        assert_eq!(dev.answer, Some(Answer::Allow));
        assert_eq!(dev.reply, Some(("hi".into(), 5.0)));
    }

    #[test]
    fn an_answer_waits_for_a_permission_prompt_and_a_reply_for_a_free_box() {
        let mut dev = DevReader::parse(None, None, Some("deny"), Some("go"), None);
        assert_eq!(dev.actions(1.0, Some(Prompt::Question)), Vec::<Action>::new());
        assert_eq!(dev.actions(1.0, Some(Prompt::Permission)), [Action::Answer(Answer::Deny)]);
        assert_eq!(dev.actions(1.0, Some(Prompt::Permission)), Vec::<Action>::new());
        assert_eq!(dev.actions(1.0, None), [Action::Reply("go".into())]);
        assert!(dev.is_idle());
    }
}
