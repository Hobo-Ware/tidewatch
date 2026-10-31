//! Up and Down in an empty reply box walk the messages the owner sent this tab (pure, table-tested).
//! A walk starts from an empty box, or from the text "Edit and send" loaded, and keeps the draft it
//! replaced so Esc, or Down past the newest message, puts it back. Typing over a recalled message
//! ends the walk's hold on the keys: Up and Down then move the cursor as in any text.

use tidewatch_core::message_history;

use crate::tokens::from_sent_on_disk;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stashed {
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Walk {
    at: Option<usize>,
    loaded: String,
    draft: Stashed,
}

#[derive(Debug, Default)]
pub(crate) struct Recall {
    entries: Vec<String>,
    walk: Option<Walk>,
}

impl Recall {
    pub(crate) fn with_entries(entries: Vec<String>) -> Self {
        Self { entries, walk: None }
    }

    pub(crate) fn entries(&self) -> &[String] {
        &self.entries
    }

    pub(crate) fn remember(&mut self, text: &str) -> bool {
        message_history::remember(&mut self.entries, text)
    }

    pub(crate) fn walking(&self) -> bool {
        self.walk.is_some()
    }

    fn unedited(&self, text: &str) -> bool {
        self.walk.as_ref().is_some_and(|walk| walk.loaded == text)
    }

    /// Whether Up belongs to the walk: an empty box starts one, an unedited recalled message
    /// continues it.
    pub(crate) fn claims_up(&self, text: &str) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        if self.walk.is_some() {
            return self.unedited(text);
        }
        text.is_empty()
    }

    pub(crate) fn claims_down(&self, text: &str) -> bool {
        self.unedited(text)
    }

    pub(crate) fn up(&mut self, text: &mut String) -> bool {
        if !self.claims_up(text) {
            return false;
        }
        let (at, draft) = match self.walk.take() {
            Some(walk) => {
                (walk.at.map_or(self.entries.len() - 1, |at| at.saturating_sub(1)), walk.draft)
            }
            None => (self.entries.len() - 1, Stashed { text: std::mem::take(text) }),
        };
        self.load(at, draft, text);
        true
    }

    fn load(&mut self, at: usize, draft: Stashed, text: &mut String) {
        let loaded = from_sent_on_disk(&self.entries[at]);
        text.clone_from(&loaded);
        self.walk = Some(Walk { at: Some(at), loaded, draft });
    }

    pub(crate) fn down(&mut self, text: &mut String) -> bool {
        if !self.claims_down(text) {
            return false;
        }
        let Some(walk) = self.walk.take() else { return false };
        match walk.at {
            Some(at) if at + 1 < self.entries.len() => {
                let loaded = from_sent_on_disk(&self.entries[at + 1]);
                text.clone_from(&loaded);
                self.walk = Some(Walk { at: Some(at + 1), loaded, draft: walk.draft });
            }
            _ => Self::restore(walk.draft, text),
        }
        true
    }

    /// Esc during a walk: the draft that was in the box comes back.
    pub(crate) fn escape(&mut self, text: &mut String) -> bool {
        let Some(walk) = self.walk.take() else { return false };
        Self::restore(walk.draft, text);
        true
    }

    /// "Edit and send" on an earlier message: `message` goes into the box and the draft it replaces
    /// waits for Esc or for the send.
    pub(crate) fn load_message(&mut self, message: &str, text: &mut String) {
        let draft = self
            .walk
            .take()
            .map_or_else(|| Stashed { text: std::mem::take(text) }, |walk| walk.draft);
        let message = from_sent_on_disk(message);
        text.clone_from(&message);
        self.walk = Some(Walk { at: None, loaded: message, draft });
    }

    /// The box was sent: the walk ends and the draft it held comes back into the emptied box.
    pub(crate) fn sent(&mut self, text: &mut String) {
        if let Some(walk) = self.walk.take() {
            Self::restore(walk.draft, text);
        }
    }

    fn restore(draft: Stashed, text: &mut String) {
        *text = draft.text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recall() -> Recall {
        Recall::with_entries(vec!["first".into(), "second".into(), "third".into()])
    }

    #[test]
    fn up_in_an_empty_box_recalls_the_newest_and_goes_back_one_message_at_a_time() {
        let (mut recall, mut text) = (recall(), String::new());
        let mut seen = Vec::new();
        for _ in 0..4 {
            assert!(recall.up(&mut text));
            seen.push(text.clone());
        }
        assert_eq!(seen, ["third", "second", "first", "first"]);
    }

    #[test]
    fn down_walks_forward_and_past_the_newest_the_empty_box_returns() {
        let (mut recall, mut text) = (recall(), String::new());
        for _ in 0..3 {
            recall.up(&mut text);
        }
        let mut seen = Vec::new();
        for _ in 0..3 {
            assert!(recall.down(&mut text));
            seen.push(text.clone());
        }
        assert_eq!(seen, ["second", "third", ""]);
        assert!(!recall.walking());
        assert!(!recall.down(&mut text));
    }

    #[test]
    fn esc_returns_to_the_draft_and_a_box_with_text_is_left_to_the_cursor() {
        let (mut recall, mut text) = (recall(), "my draft".to_owned());
        assert!(!recall.up(&mut text), "a typed draft keeps its Up");
        assert!(!recall.escape(&mut text));
        text.clear();
        recall.up(&mut text);
        assert!(recall.escape(&mut text));
        assert_eq!((text.as_str(), recall.walking()), ("", false));
    }

    #[test]
    fn a_recalled_message_that_was_edited_no_longer_claims_up_or_down() {
        let (mut recall, mut text) = (recall(), String::new());
        recall.up(&mut text);
        text.push_str(" and more");
        assert!(!recall.up(&mut text));
        assert!(!recall.down(&mut text));
        assert!(recall.escape(&mut text), "Esc still ends the walk");
        assert_eq!(text, "");
    }

    #[test]
    fn nothing_is_recalled_when_nothing_was_sent() {
        let mut text = String::new();
        assert!(!Recall::default().up(&mut text));
    }

    #[test]
    fn edit_and_send_keeps_the_draft_and_gives_it_back_after_the_send_or_esc() {
        let mut recall = recall();
        let mut text = "my draft".to_owned();
        recall.load_message("an old one", &mut text);
        assert_eq!(text, "an old one");
        text = String::new();
        recall.sent(&mut text);
        assert_eq!((text.as_str(), recall.walking()), ("my draft", false));
        recall.load_message("again", &mut text);
        assert!(recall.escape(&mut text));
        assert_eq!(text, "my draft");
    }

    #[test]
    fn up_from_an_edit_and_send_message_goes_to_the_newest_and_keeps_the_draft() {
        let mut recall = recall();
        let mut text = "my draft".to_owned();
        recall.load_message("an old one", &mut text);
        assert!(recall.up(&mut text));
        assert_eq!(text, "third");
        assert!(recall.down(&mut text));
        assert_eq!(text, "my draft");
    }

    #[test]
    fn an_earlier_message_with_a_file_comes_back_with_its_token_in_the_same_place() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("q3 report.pdf");
        std::fs::write(&file, "").unwrap();
        let sent = format!("the numbers in '{}' look off", file.display());
        let mut recall = Recall::with_entries(vec![sent.clone()]);
        let mut text = String::new();
        assert!(recall.up(&mut text));
        assert_eq!(crate::tokens::targets(&text), [crate::tokens::Target::File(file)]);
        assert_eq!(crate::tokens::sendable(&text), sent);
        assert!(recall.claims_down(&text), "an unedited recalled message still walks");
    }

    #[test]
    fn edit_and_send_loads_a_held_message_with_its_files_as_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shot-1.png");
        std::fs::write(&file, "").unwrap();
        let mut recall = recall();
        let mut text = String::new();
        recall.load_message(&format!("look at {} now", file.display()), &mut text);
        assert_eq!(crate::tokens::targets(&text), [crate::tokens::Target::File(file)]);
        assert_eq!(crate::tokens::plain(&text), "look at  now");
    }

    #[test]
    fn remember_skips_a_repeat_and_blank_text() {
        let mut recall = Recall::default();
        let outcome: Vec<bool> =
            ["a", "a", " ", "b"].iter().map(|text| recall.remember(text)).collect();
        assert_eq!(outcome, [true, false, false, true]);
        assert_eq!(recall.entries(), ["a", "b"]);
    }
}
