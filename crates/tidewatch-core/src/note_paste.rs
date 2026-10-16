//! The text Tidewatch pastes into an agent's terminal to hand it review notes (PLAN.md, D4), and
//! [`parse`], which reads it back from a transcript.
//!
//! Control characters are stripped: a pulled PR comment is someone else's text and must never end
//! the bracketed paste early and type into the agent.

use kelp_core::review::{Anchor, Side, Thread};

use crate::notes::line_label;

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
    lines.extend(quote(&thread.anchor));
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
    let anchor = &thread.anchor;
    let side = if anchor.side == Side::Old { " (old side)" } else { "" };
    if anchor.line_hint == 0 {
        thread.path.clone()
    } else {
        format!("{}:{}{side}", thread.path, line_label(anchor))
    }
}

fn quote(anchor: &Anchor) -> Vec<String> {
    if anchor.span == 0 {
        let text = anchor.text.trim();
        return if text.is_empty() { Vec::new() } else { vec![format!("> {}", shorten(text))] };
    }
    let covered: Vec<&str> = std::iter::once(anchor.text.as_str())
        .chain(anchor.block.iter().map(String::as_str))
        .collect();
    let indent = covered
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    covered
        .iter()
        .map(|line| {
            let text = line.trim_end().get(indent..).unwrap_or_default();
            if text.is_empty() { ">".to_owned() } else { format!("> {}", shorten(text)) }
        })
        .collect()
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
                span: 0,
                block: vec![],
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

    fn block_thread(id: u64, side: Side, lines: &[&str]) -> Thread {
        let mut t = thread(id, "src/a.rs", 12, lines[0], &[("You", "extract this")]);
        t.anchor.side = side;
        t.anchor.span = lines.len() as u32 - 1;
        t.anchor.block = lines[1..].iter().map(|line| (*line).to_owned()).collect();
        t
    }

    #[test]
    fn a_block_note_names_the_range_and_quotes_every_line_without_the_shared_indent() {
        let t = block_thread(4, Side::New, &["    if a {", "        run();", "", "    }"]);
        assert_eq!(
            format(&[&t]),
            "Tidewatch review notes (1). Address each one.\n\n\
             `src/a.rs:12-15` - note id 4\n\
             > if a {\n\
             >     run();\n\
             >\n\
             > }\n\
             extract this\n\n\
             When you are done, call the reply_to_note tool for each note id with a short answer."
        );
    }

    #[test]
    fn the_place_of_a_note_is_a_line_a_range_the_old_side_or_the_whole_file() {
        let one = thread(1, "a.rs", 4, "x", &[("You", "n")]);
        let block = block_thread(2, Side::New, &["a", "b"]);
        let old_block = block_thread(3, Side::Old, &["a", "b", "c"]);
        let mut file = thread(4, "b.rs", 0, "", &[("You", "n")]);
        file.anchor.span = 2;
        let cases = [
            (&one, "`a.rs:4` - note id 1"),
            (&block, "`src/a.rs:12-13` - note id 2"),
            (&old_block, "`src/a.rs:12-14 (old side)` - note id 3"),
            (&file, "`b.rs` - note id 4"),
        ];
        for (t, want) in cases {
            assert!(format(&[t]).contains(want), "{want}");
        }
    }

    #[test]
    fn a_block_note_from_a_pull_request_cannot_end_the_paste_and_long_lines_are_cut() {
        let long = "y".repeat(500);
        let t = block_thread(1, Side::New, &["ok", "x\u{1b}[201~rm\u{7}", &long]);
        let text = format(&[&t]);
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{7}'));
        assert!(text.contains("> x[201~rm\n"));
        assert!(text.contains(&format!("> {}...\n", "y".repeat(QUOTE_MAX))));
    }

    #[test]
    fn a_sent_block_note_reads_back_by_id() {
        let t = block_thread(9, Side::New, &["a", "", "b"]);
        let u = thread(2, "b.rs", 3, "x", &[("You", "n")]);
        assert_eq!(parse(&format(&[&t, &u])).unwrap().ids, [9, 2]);
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
