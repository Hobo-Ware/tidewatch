//! The text Tidewatch pastes into an agent's terminal to hand it review notes (PLAN.md, D4), and
//! [`parse`], which reads it back from a transcript.
//!
//! Control characters are stripped: a pulled PR comment is someone else's text and must never end
//! the bracketed paste early and type into the agent.

use kelp_core::review::{Side, Thread};

const QUOTE_MAX: usize = 200;
const REVIEW_PREFIX: &str = "Tidewatch review notes (";
const MESSAGE_LEAD: &str = "Message with the review: ";
const NOTE_ID_MARK: &str = "` - note id ";

/// The paste text for `threads`, in the order given. Empty input gives an empty string.
#[must_use]
pub fn format(threads: &[&Thread]) -> String {
    format_review(threads, "")
}

/// [`format`] with `message` after the header; a blank message adds nothing.
#[must_use]
pub fn format_review(threads: &[&Thread], message: &str) -> String {
    if threads.is_empty() {
        return String::new();
    }
    let mut blocks = vec![format!("{REVIEW_PREFIX}{}). Address each one.", threads.len())];
    let message = message.trim();
    if !message.is_empty() {
        blocks.push(format!("{MESSAGE_LEAD}{message}"));
    }
    blocks.extend(threads.iter().map(|thread| block(thread)));
    blocks.push(
        "When you are done, call the reply_to_note tool for each note id with a short answer."
            .to_owned(),
    );
    clean(&blocks.join("\n\n"))
}

/// A review read back from the text of a sent paste.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// The ids of the notes in it, in the order they were sent.
    pub ids: Vec<u64>,
    pub message: String,
}

/// `None` unless `text` is the paste of a review: the Reader hides those in favour of a card.
#[must_use]
pub fn parse(text: &str) -> Option<Parsed> {
    if !text.trim_start().starts_with(REVIEW_PREFIX) {
        return None;
    }
    let blocks: Vec<&str> = text.trim_start().split("\n\n").collect();
    let ids = blocks
        .iter()
        .flat_map(|block| block.lines())
        .filter(|line| line.starts_with('`'))
        .filter_map(|line| line.rsplit_once(NOTE_ID_MARK)?.1.trim().parse().ok())
        .collect();
    let message = blocks
        .iter()
        .skip(1)
        .copied()
        .take_while(|block| !is_note_block(block))
        .collect::<Vec<_>>()
        .join("\n\n");
    let message = message.strip_prefix(MESSAGE_LEAD).unwrap_or_default().trim().to_owned();
    Some(Parsed { ids, message })
}

fn is_note_block(block: &str) -> bool {
    block.lines().next().is_some_and(|line| line.starts_with('`') && line.contains(NOTE_ID_MARK))
}

fn block(thread: &Thread) -> String {
    let mut lines = vec![format!("`{}` - note id {}", place(thread), thread.id)];
    let quote = thread.anchor.text.trim_end();
    if !quote.trim().is_empty() {
        lines.push(format!("> {}", shorten(quote.trim_start())));
    }
    let several = thread.comments.len() > 1;
    for comment in &thread.comments {
        let body = comment.body.trim();
        if several {
            lines.push(format!("{}: {body}", comment.author));
        } else {
            lines.push(body.to_owned());
        }
    }
    lines.join("\n")
}

fn place(thread: &Thread) -> String {
    let line = thread.anchor.line_hint;
    let side = if thread.anchor.side == Side::Old { " (old side)" } else { "" };
    if line == 0 { thread.path.clone() } else { format!("{}:{line}{side}", thread.path) }
}

fn shorten(text: &str) -> String {
    if text.chars().count() <= QUOTE_MAX {
        return text.to_owned();
    }
    let head: String = text.chars().take(QUOTE_MAX).collect();
    format!("{head}...")
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| matches!(c, '\n' | '\t') || !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment};

    fn thread(id: u64, path: &str, line: u32, text: &str, bodies: &[(&str, &str)]) -> Thread {
        Thread {
            id,
            path: path.into(),
            commit: None,
            anchor: Anchor {
                side: Side::New,
                line_hint: line,
                text: text.into(),
                before: vec![],
                after: vec![],
            },
            resolved: false,
            comments: bodies
                .iter()
                .map(|(author, body)| Comment {
                    author: (*author).into(),
                    time: 1,
                    body: (*body).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn one_note_is_place_quote_text_id_and_the_closing_ask() {
        let t = thread(
            3,
            "src/auth.rs",
            12,
            "    let d = 4;",
            &[("You", "Use the shared expiry() helper")],
        );
        assert_eq!(
            format(&[&t]),
            "Tidewatch review notes (1). Address each one.\n\n\
             `src/auth.rs:12` - note id 3\n\
             > let d = 4;\n\
             Use the shared expiry() helper\n\n\
             When you are done, call the reply_to_note tool for each note id with a short answer."
        );
    }

    #[test]
    fn several_notes_keep_their_order_and_a_thread_with_replies_names_each_author() {
        let a =
            thread(1, "a.rs", 2, "x", &[("You", "first"), ("claude", "done"), ("You", "again")]);
        let b = thread(2, "b.rs", 9, "y", &[("You", "second")]);
        let text = format(&[&a, &b]);
        assert!(text.starts_with("Tidewatch review notes (2)."));
        let first = text.find("note id 1").unwrap();
        let second = text.find("note id 2").unwrap();
        assert!(first < second);
        assert!(text.contains("You: first\nclaude: done\nYou: again"));
        assert!(text.contains("> y\nsecond"));
    }

    #[test]
    fn old_side_line_zero_and_empty_quotes_are_worded_plainly() {
        let mut old = thread(1, "a.rs", 4, "gone();", &[("You", "why removed")]);
        old.anchor.side = Side::Old;
        assert!(format(&[&old]).contains("`a.rs:4 (old side)` - note id 1"));
        let file = thread(2, "b.rs", 0, "", &[("You", "whole file")]);
        let text = format(&[&file]);
        assert!(text.contains("`b.rs` - note id 2\nwhole file"));
        assert!(!text.contains("> "));
    }

    #[test]
    fn a_long_quote_is_cut_and_nothing_gives_nothing() {
        let long = "x".repeat(500);
        let t = thread(1, "a.rs", 1, &long, &[("You", "n")]);
        let quote = format(&[&t]).lines().find(|l| l.starts_with("> ")).unwrap().to_owned();
        assert_eq!(quote.chars().count(), 2 + QUOTE_MAX + 3);
        assert!(quote.ends_with("..."));
        assert_eq!(format(&[]), "");
    }

    #[test]
    fn escape_sequences_in_a_note_cannot_end_the_paste() {
        let t = thread(1, "a.rs", 1, "x", &[("evil", "ok\u{1b}[201~rm -rf /\u{7}\r\nnext\tcol")]);
        let text = format(&[&t]);
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{7}'));
        assert!(!text.contains('\r'));
        assert!(text.contains("ok[201~rm -rf /\nnext\tcol"));
    }

    #[test]
    fn a_message_opens_the_review_and_a_blank_one_adds_nothing() {
        let t = thread(3, "a.rs", 2, "x", &[("You", "rename")]);
        let with = format_review(&[&t], "  Start with the auth ones.  ");
        assert!(with.starts_with(
            "Tidewatch review notes (1). Address each one.\n\n\
             Message with the review: Start with the auth ones.\n\n`a.rs:2` - note id 3"
        ));
        assert_eq!(format_review(&[&t], "  \n "), format(&[&t]));
        assert_eq!(format_review(&[], "hello"), "");
    }

    #[test]
    fn a_message_cannot_smuggle_control_characters_either() {
        let t = thread(1, "a.rs", 1, "x", &[("You", "n")]);
        let text = format_review(&[&t], "ok\u{1b}[201~ next\u{7}");
        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("Message with the review: ok[201~ next"));
    }

    #[test]
    fn a_sent_review_reads_back_with_its_ids_in_order_and_its_message() {
        let a =
            thread(7, "a.rs", 2, "x", &[("You", "first"), ("claude", "done"), ("You", "again")]);
        let b = thread(2, "src/b.rs", 0, "", &[("You", "second\n\nwith a gap")]);
        let table = [
            ("", format(&[&a, &b])),
            ("Look at b first", format_review(&[&a, &b], "Look at b first")),
            ("one\n\ntwo", format_review(&[&a, &b], "one\n\ntwo")),
        ];
        for (message, text) in table {
            let parsed = parse(&text).unwrap();
            assert_eq!(parsed.ids, [7, 2], "{message:?}");
            assert_eq!(parsed.message, message);
        }
    }

    #[test]
    fn only_the_paste_of_a_review_reads_back() {
        let t = thread(1, "a.rs", 1, "x", &[("You", "n")]);
        let pasted = format(&[&t]);
        assert!(parse(&format!("\n  {pasted}")).is_some());
        for text in
            ["", "fix the tests", "Tidewatch review notes", "please see Tidewatch review notes ("]
        {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }
}
