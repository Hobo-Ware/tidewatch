//! The messages the owner sent while the agent could not take them, held by Tidewatch in order and
//! typed one at a time when the agent is free (pure, table-tested). Each can be edited, deleted,
//! moved or sent at once until it goes. The line refuses a message past `MAX_HELD` instead of
//! dropping an older one, so nothing leaves it without the owner or the agent taking it.

pub(crate) const MAX_HELD: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Held {
    pub(crate) id: u64,
    pub(crate) text: String,
    pub(crate) at_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Way {
    Earlier,
    Later,
}

#[derive(Debug, Default)]
pub(crate) struct SendLine {
    items: Vec<Held>,
    next_id: u64,
}

impl SendLine {
    pub(crate) fn items(&self) -> &[Held] {
        &self.items
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn newest_id(&self) -> Option<u64> {
        self.items.last().map(|held| held.id)
    }

    pub(crate) fn text_of(&self, id: u64) -> Option<&str> {
        self.items.iter().find(|held| held.id == id).map(|held| held.text.as_str())
    }

    /// Adds `text` at the end. A full line gives the text back so the caller can keep it.
    pub(crate) fn hold(&mut self, text: &str, at_ms: i64) -> Result<u64, String> {
        if self.items.len() >= MAX_HELD {
            return Err(text.to_owned());
        }
        self.next_id += 1;
        self.items.push(Held { id: self.next_id, text: text.to_owned(), at_ms });
        Ok(self.next_id)
    }

    /// Replaces the text of message `id`. A blank text deletes it.
    pub(crate) fn set_text(&mut self, id: u64, text: &str) {
        if text.trim().is_empty() {
            self.delete(id);
        } else if let Some(held) = self.items.iter_mut().find(|held| held.id == id) {
            text.clone_into(&mut held.text);
        }
    }

    pub(crate) fn delete(&mut self, id: u64) -> Option<Held> {
        let at = self.items.iter().position(|held| held.id == id)?;
        Some(self.items.remove(at))
    }

    pub(crate) fn shift(&mut self, id: u64, way: Way) -> bool {
        let Some(at) = self.items.iter().position(|held| held.id == id) else { return false };
        let to = match way {
            Way::Earlier if at > 0 => at - 1,
            Way::Later if at + 1 < self.items.len() => at + 1,
            _ => return false,
        };
        self.items.swap(at, to);
        true
    }

    /// The message to type next: the first one, unless the owner is editing it, which holds the
    /// whole line so the order stays what the owner sees.
    pub(crate) fn ready(&self, editing: Option<u64>) -> Option<&Held> {
        self.items.first().filter(|held| editing != Some(held.id))
    }
}

/// Whether a message is a slash command: it runs at once, whatever the agent is doing, and is
/// never held.
pub(crate) fn is_command(text: &str) -> bool {
    text.trim_start().starts_with('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(line: &SendLine) -> Vec<&str> {
        line.items().iter().map(|held| held.text.as_str()).collect()
    }

    fn line_of(texts: &[&str]) -> SendLine {
        let mut line = SendLine::default();
        for text in texts {
            line.hold(text, 0).unwrap();
        }
        line
    }

    #[test]
    fn messages_wait_in_the_order_they_were_sent() {
        let line = line_of(&["one", "two", "three"]);
        assert_eq!(texts(&line), ["one", "two", "three"]);
        assert_eq!(line.ready(None).map(|held| held.text.as_str()), Some("one"));
        assert_eq!(line.newest_id().and_then(|id| line.text_of(id)), Some("three"));
    }

    #[test]
    fn a_full_line_refuses_the_new_message_and_drops_nothing() {
        let mut line = SendLine::default();
        for n in 0..MAX_HELD {
            line.hold(&format!("m{n}"), 0).unwrap();
        }
        assert_eq!(line.hold("one too many", 0), Err("one too many".to_owned()));
        assert_eq!(line.len(), MAX_HELD);
        assert_eq!(texts(&line).first().copied(), Some("m0"));
    }

    #[test]
    fn a_message_moves_one_place_and_the_ends_stay_put() {
        let mut line = line_of(&["a", "b", "c"]);
        let ids: Vec<u64> = line.items().iter().map(|held| held.id).collect();
        assert!(line.shift(ids[2], Way::Earlier));
        assert_eq!(texts(&line), ["a", "c", "b"]);
        assert!(!line.shift(ids[0], Way::Earlier));
        assert!(!line.shift(ids[1], Way::Later));
        assert!(line.shift(ids[0], Way::Later));
        assert_eq!(texts(&line), ["c", "a", "b"]);
    }

    #[test]
    fn editing_the_text_keeps_the_place_and_a_blank_text_deletes() {
        let mut line = line_of(&["a", "b"]);
        let (first, second) = (line.items()[0].id, line.items()[1].id);
        line.set_text(second, "b, edited");
        assert_eq!(texts(&line), ["a", "b, edited"]);
        line.set_text(first, "  \n");
        assert_eq!(texts(&line), ["b, edited"]);
        assert_eq!(line.delete(first), None);
        assert_eq!(line.delete(second).map(|held| held.text), Some("b, edited".to_owned()));
    }

    #[test]
    fn the_first_message_in_edit_holds_the_whole_line() {
        let line = line_of(&["a", "b"]);
        let first = line.items()[0].id;
        assert_eq!(line.ready(Some(first)), None);
        assert_eq!(line.ready(Some(line.items()[1].id)).map(|held| held.id), Some(first));
    }

    #[test]
    fn only_a_slash_is_a_command() {
        let cases = [("/model opus", true), ("  /clear", true), ("hello /x", false), ("", false)];
        for (text, want) in cases {
            assert_eq!(is_command(text), want, "{text:?}");
        }
    }
}
