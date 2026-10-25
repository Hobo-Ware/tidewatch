//! The house-style end block (PLAN.md, D13): the agent finishes a turn with `Done`, `Needs you` and
//! `Next` sections. [`parse`] reads them out of the agent's last message so the app can draw a card.
//!
//! Pure text in, data out. Models vary the markup a little (`**Done**`, `**Done:**`, `## Done`,
//! `Done:`), so headings are matched loosely, but all three must appear, in order, or there is no
//! block.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EndBlock {
    pub done: Vec<String>,
    pub needs_you: Vec<String>,
    pub next: Vec<String>,
}

impl EndBlock {
    /// True when the `Needs you` section asks for something, not "nothing".
    #[must_use]
    pub fn needs_input(&self) -> bool {
        self.needs_you.iter().any(|item| !is_nothing(item))
    }
}

fn is_nothing(item: &str) -> bool {
    let item = item.trim().trim_matches(|c: char| c.is_ascii_punctuation()).to_lowercase();
    matches!(item.as_str(), "" | "nothing" | "none" | "n/a" | "no" | "nothing needed")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Done,
    NeedsYou,
    Next,
}

fn heading(line: &str) -> Option<(Section, &str)> {
    let stripped = line.trim().trim_start_matches('#').trim_start();
    let stripped = stripped.trim_start_matches(['*', '_']);
    for (label, section) in
        [("needs you", Section::NeedsYou), ("done", Section::Done), ("next", Section::Next)]
    {
        let Some(head) = stripped.get(..label.len()) else { continue };
        if !head.eq_ignore_ascii_case(label) {
            continue;
        }
        let rest = stripped[label.len()..].trim_start_matches(['*', '_']);
        let (colon, rest) = match rest.strip_prefix(':') {
            Some(after) => (true, after.trim_start_matches(['*', '_']).trim()),
            None => (false, rest.trim()),
        };
        if colon || rest.is_empty() {
            return Some((section, rest));
        }
    }
    None
}

fn bullet(line: &str) -> Option<&str> {
    let line = line.trim_start();
    for marker in ["- ", "* ", "\u{2022} "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = line[digits..].strip_prefix(". ")
    {
        return Some(rest.trim());
    }
    None
}

/// Reads the last Done / Needs you / Next block of `message`. `None` when the message has no
/// complete block.
#[must_use]
pub fn parse(message: &str) -> Option<EndBlock> {
    let lines: Vec<&str> = message.lines().collect();
    let heads = heads(&lines);
    let start = last_block(&heads)?;
    let mut block = EndBlock::default();
    for (n, (index, section, rest)) in heads[start..start + 3].iter().enumerate() {
        let end = heads.get(start + n + 1).map_or(lines.len(), |next| next.0);
        let items = items(rest, &lines[index + 1..end]);
        match section {
            Section::Done => block.done = items,
            Section::NeedsYou => block.needs_you = items,
            Section::Next => block.next = items,
        }
    }
    Some(block)
}

fn heads<'a>(lines: &[&'a str]) -> Vec<(usize, Section, &'a str)> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| heading(line).map(|(section, rest)| (i, section, rest)))
        .collect()
}

fn last_block(heads: &[(usize, Section, &str)]) -> Option<usize> {
    heads.windows(3).rposition(|w| {
        w[0].1 == Section::Done && w[1].1 == Section::NeedsYou && w[2].1 == Section::Next
    })
}

/// The lines of the last complete block that are only a heading (`**Done**`, `## Needs you`,
/// `Next:`), by line index with the caption each one shows. A heading with text after it on the
/// same line, a lone `Done` with no `Needs you` and `Next` after it, and a sentence that merely
/// starts with one of the words are not listed.
#[must_use]
pub fn caption_lines(message: &str) -> Vec<(usize, &'static str)> {
    let lines: Vec<&str> = message.lines().collect();
    let heads = heads(&lines);
    let Some(start) = last_block(&heads) else { return Vec::new() };
    heads[start..start + 3]
        .iter()
        .filter(|(_, _, rest)| rest.is_empty())
        .map(|(index, section, _)| {
            let caption = match section {
                Section::Done => "Done",
                Section::NeedsYou => "Needs you",
                Section::Next => "Next",
            };
            (*index, caption)
        })
        .collect()
}

fn items(same_line: &str, body: &[&str]) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    if !same_line.is_empty() {
        items.push(same_line.to_owned());
    }
    let mut started = !items.is_empty();
    let mut blank = false;
    for line in body {
        if line.trim().is_empty() {
            blank = started;
            continue;
        }
        if let Some(text) = bullet(line) {
            items.push(text.to_owned());
        } else if blank {
            break;
        } else if line.starts_with(char::is_whitespace) && !items.is_empty() {
            let last = items.last_mut().expect("checked non-empty");
            last.push(' ');
            last.push_str(line.trim());
        } else {
            items.push(line.trim().to_owned());
        }
        started = true;
        blank = false;
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUSE: &str = "Added the helper and a test.\n\n**Done**\n- added `expiry()` in `src/auth.rs:12`\n- 3 tests\n\n**Needs you**\n- nothing\n\n**Next**\n- review the diff\n";

    #[test]
    fn the_house_block_is_read_into_three_sections() {
        let block = parse(HOUSE).unwrap();
        assert_eq!(block.done, ["added `expiry()` in `src/auth.rs:12`", "3 tests"]);
        assert_eq!(block.needs_you, ["nothing"]);
        assert_eq!(block.next, ["review the diff"]);
        assert!(!block.needs_input());
    }

    #[test]
    fn a_real_ask_in_needs_you_is_input() {
        let text = "**Done**\n- x\n\n**Needs you**\n- which base branch?\n\n**Next**\n- wait\n";
        assert!(parse(text).unwrap().needs_input());
    }

    #[test]
    fn heading_markup_varies() {
        let variants = [
            "**Done:**\n- a\n**Needs you:**\n- b\n**Next:**\n- c",
            "## Done\n- a\n## Needs you\n- b\n## Next\n- c",
            "Done:\n* a\nNeeds you:\n* b\nNext:\n* c",
            "**done**\n1. a\n**NEEDS YOU**\n1. b\n**Next**\n1. c",
        ];
        for text in variants {
            let block = parse(text).unwrap_or_else(|| panic!("no block in {text:?}"));
            assert_eq!(
                (block.done, block.needs_you, block.next),
                (vec!["a".into()], vec!["b".into()], vec!["c".into()]),
                "{text:?}"
            );
        }
    }

    #[test]
    fn text_on_the_heading_line_and_wrapped_bullets_are_kept() {
        let text =
            "**Done:** fixed it\n**Needs you**\n- a long ask that\n  wraps here\n**Next:** ship";
        let block = parse(text).unwrap();
        assert_eq!(block.done, ["fixed it"]);
        assert_eq!(block.needs_you, ["a long ask that wraps here"]);
        assert_eq!(block.next, ["ship"]);
    }

    #[test]
    fn prose_after_the_block_is_not_part_of_next() {
        let text = format!("{HOUSE}\nThanks for your patience, that was fun.");
        assert_eq!(parse(&text).unwrap().next, ["review the diff"]);
    }

    #[test]
    fn the_last_block_wins() {
        let text = format!(
            "{HOUSE}\nmore work\n\n**Done**\n- second\n**Needs you**\n- nothing\n**Next**\n- stop\n"
        );
        assert_eq!(parse(&text).unwrap().done, ["second"]);
    }

    #[test]
    fn an_incomplete_or_misordered_block_is_none() {
        let cases = [
            "",
            "just prose about the Done button and what is Next",
            "**Done**\n- a\n**Next**\n- c",
            "**Next**\n- c\n**Needs you**\n- b\n**Done**\n- a",
            "**Done**\n- a\n**Needs you**\n- b",
        ];
        for text in cases {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_sentence_that_starts_with_a_label_word_is_not_a_heading() {
        let text = "Done well, thanks.\nNeeds you to look.\nNext time.";
        assert_eq!(parse(text), None);
    }

    #[test]
    fn heading_only_lines_of_the_last_block_are_captions() {
        let cases: [(&str, Vec<(usize, &str)>); 7] = [
            (HOUSE, vec![(2, "Done"), (6, "Needs you"), (9, "Next")]),
            (
                "## Done\n- x\n### Needs you\n- y\nNext:\n- z",
                vec![(0, "Done"), (2, "Needs you"), (4, "Next")],
            ),
            (
                "**Done:** x\n**Needs you:**\n- a\n**Next**\n- b",
                vec![(1, "Needs you"), (3, "Next")],
            ),
            ("Done with the refactor.\nNeeds you to look.\nNext steps below.", vec![]),
            ("**Done**\n- x\n**Next**\n- y", vec![]),
            ("**Done**\n**Needs you**\n", vec![]),
            ("", vec![]),
        ];
        for (input, want) in cases {
            assert_eq!(caption_lines(input), want, "input {input:?}");
        }
    }

    #[test]
    fn only_the_last_complete_block_is_captioned() {
        let text = "**Done**\n- a\n**Needs you**\n- b\n**Next**\n- c\n\nMore.\n\n**Done**\n- d\n**Needs you**\n- e\n**Next**\n- f";
        let lines: Vec<usize> = caption_lines(text).iter().map(|(i, _)| *i).collect();
        assert_eq!(lines, [9, 11, 13]);
    }
}
