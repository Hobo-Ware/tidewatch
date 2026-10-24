//! The tagged records Claude Code writes into user lines besides the user's words (checked on real
//! 2.1.3xx transcripts): slash commands, local command output, caveats, `<pasted_content id>` wrappers
//! and `[Image #1]` markers. Pure text functions.

const COMMAND_TAGS: [&str; 3] = ["command-name", "command-message", "command-args"];
const OUTPUT_TAGS: [&str; 2] = ["local-command-stdout", "local-command-stderr"];
const HIDDEN_PREFIXES: [&str; 2] = ["<local-command-caveat>", "<system-reminder>"];
const PASTE_TAG: &str = "pasted_content";
const IMAGE_MARKER: &str = "[Image #";
const COMPACTED_OPENING: &str = "This session is being continued from a previous conversation";

/// What one user text of a Claude transcript is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    /// A slash command the user ran: `/rate-limit-options`, with its arguments after a space.
    Command(String),
    /// What a local command printed, tags and terminal colors removed.
    Output(String),
    /// The user's own words, the paste wrapper removed.
    Text(String),
    /// The summary Claude Code writes after compacting the conversation.
    Compacted,
    /// Bookkeeping that carries nothing to show.
    Hidden,
}

/// Sorts one user text. A tag is only markup when the whole text is made of such tags, so a message
/// that mentions `<command-name>` in a sentence stays the user's words.
#[must_use]
pub fn classify(text: &str) -> Record {
    let text = text.trim();
    if HIDDEN_PREFIXES.iter().any(|prefix| text.starts_with(prefix)) {
        return Record::Hidden;
    }
    if text.starts_with(COMPACTED_OPENING) {
        return Record::Compacted;
    }
    if let Some(parts) = tag_sequence(text, &COMMAND_TAGS) {
        return command(&parts);
    }
    if let Some(parts) = tag_sequence(text, &OUTPUT_TAGS) {
        return output(&parts);
    }
    Record::Text(unwrap_pasted(text))
}

/// Turns a user text into the markdown the chat bubble draws: `[Image #1]` becomes a small code
/// chip `Image 1`, and a single line break stays a line break instead of folding into a space.
#[must_use]
pub fn chat_markdown(text: &str) -> String {
    keep_line_breaks(&image_chips(text))
}

/// Whether some line of `text` opens a list item or a quote, which the bubble indents.
#[must_use]
pub fn has_indented_block(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("> ")
            || line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("+ ")
            || list_number_len(line).is_some()
    })
}

fn command(parts: &[(&str, &str)]) -> Record {
    let find = |tag: &str| {
        parts.iter().find(|(name, _)| *name == tag).map_or("", |(_, inner)| inner.trim())
    };
    let name = Some(find("command-name")).filter(|name| !name.is_empty());
    let name = name.or(Some(find("command-message")).filter(|name| !name.is_empty()));
    let Some(name) = name else {
        return Record::Hidden;
    };
    let mut line = if name.starts_with('/') { name.to_owned() } else { format!("/{name}") };
    let args = one_line(find("command-args"));
    if !args.is_empty() {
        line.push(' ');
        line.push_str(&args);
    }
    Record::Command(line)
}

fn output(parts: &[(&str, &str)]) -> Record {
    let shown: Vec<String> = parts
        .iter()
        .map(|(_, inner)| strip_ansi(inner).trim().to_owned())
        .filter(|inner| !inner.is_empty())
        .collect();
    if shown.is_empty() { Record::Hidden } else { Record::Output(shown.join("\n")) }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tag_sequence<'a>(text: &'a str, known: &[&'static str]) -> Option<Vec<(&'static str, &'a str)>> {
    let mut rest = text.trim();
    let mut parts = Vec::new();
    while !rest.is_empty() {
        let tag = known.iter().find(|tag| {
            rest.strip_prefix('<')
                .and_then(|after| after.strip_prefix(**tag))
                .is_some_and(|after| after.starts_with('>'))
        })?;
        let open = tag.len() + 2;
        let close = format!("</{tag}>");
        let end = rest.find(&close)?;
        parts.push((*tag, &rest[open..end]));
        rest = rest[end + close.len()..].trim_start();
    }
    (!parts.is_empty()).then_some(parts)
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.next_if_eq(&'[').is_some() {
            for next in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&next) {
                    break;
                }
            }
        }
    }
    out
}

fn unwrap_pasted(text: &str) -> String {
    let open_start = format!("<{PASTE_TAG} id=\"");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&open_start) {
        let after_open = &rest[at + open_start.len()..];
        let Some((id, tail)) = after_open.split_once("\">") else {
            break;
        };
        let close = format!("</{PASTE_TAG} id=\"{id}\">");
        let Some(end) = tail.find(&close) else {
            out.push_str(&rest[..at + open_start.len()]);
            rest = after_open;
            continue;
        };
        let inner = &tail[..end];
        out.push_str(&rest[..at]);
        if inner.starts_with('\n') && !out.trim().is_empty() {
            out.truncate(out.trim_end().len());
            out.push_str("\n\n");
        }
        out.push_str(inner.trim_matches('\n'));
        rest = &tail[end + close.len()..];
        if inner.ends_with('\n') && !rest.trim().is_empty() {
            out.push_str("\n\n");
            rest = rest.trim_start();
        }
    }
    out.push_str(rest);
    out.trim().to_owned()
}

fn image_chips(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(IMAGE_MARKER) {
        let digits = &rest[at + IMAGE_MARKER.len()..];
        let len = digits.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 || !digits[len..].starts_with(']') {
            out.push_str(&rest[..at + IMAGE_MARKER.len()]);
            rest = digits;
            continue;
        }
        out.push_str(&rest[..at]);
        out.push_str("`Image ");
        out.push_str(&digits[..len]);
        out.push('`');
        rest = &digits[len + 1..];
    }
    out.push_str(rest);
    out
}

fn keep_line_breaks(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::with_capacity(text.len() + lines.len() * 2);
    let mut fenced = false;
    for (index, line) in lines.iter().enumerate() {
        out.push_str(line);
        let fence = line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~");
        if fence {
            fenced = !fenced;
        }
        if index + 1 < lines.len() {
            let next_blank = lines[index + 1].trim().is_empty();
            let breaks = !fenced && !fence && !next_blank && !line.trim().is_empty();
            if breaks {
                out.push_str("  ");
            }
            out.push('\n');
        }
    }
    out
}

fn list_number_len(line: &str) -> Option<usize> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let marker = line[digits..].chars().next()?;
    (digits > 0 && matches!(marker, '.' | ')') && line[digits + 1..].starts_with(' '))
        .then_some(digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(body: &str) -> Record {
        Record::Text(body.to_owned())
    }

    #[test]
    fn records_are_sorted_by_their_tags() {
        let cases = [
            (
                "<command-name>/rate-limit-options</command-name>\n            <command-message>rate-limit-options</command-message>\n            <command-args></command-args>",
                Record::Command("/rate-limit-options".into()),
            ),
            (
                "<command-message>go</command-message>\n<command-name>/go</command-name>\n<command-args>  the   plan\nnow </command-args>",
                Record::Command("/go the plan now".into()),
            ),
            ("<command-message>review</command-message>", Record::Command("/review".into())),
            ("<command-name></command-name><command-args></command-args>", Record::Hidden),
            (
                "<local-command-stdout>Claude Code will continue automatically at 3pm</local-command-stdout>",
                Record::Output("Claude Code will continue automatically at 3pm".into()),
            ),
            (
                "<local-command-stdout>\u{1b}[1mBold\u{1b}[22m and plain</local-command-stdout>",
                Record::Output("Bold and plain".into()),
            ),
            (
                "<local-command-stdout><link rel=\"icon\"> kept</local-command-stdout>",
                Record::Output("<link rel=\"icon\"> kept".into()),
            ),
            (
                "<local-command-stdout></local-command-stdout><local-command-stderr>oops</local-command-stderr>",
                Record::Output("oops".into()),
            ),
            ("<local-command-stdout>  \n </local-command-stdout>", Record::Hidden),
            ("<local-command-caveat>Caveat: ignore this</local-command-caveat>", Record::Hidden),
            ("<system-reminder>Remember x</system-reminder>", Record::Hidden),
            ("fix the bug", text("fix the bug")),
            (
                "This session is being continued from a previous conversation that ran out of context.",
                Record::Compacted,
            ),
        ];
        for (input, want) in cases {
            assert_eq!(classify(input), want, "input {input:?}");
        }
    }

    #[test]
    fn prose_that_mentions_a_tag_stays_as_written() {
        let cases = [
            "why does <command-name> show up in my chat?",
            "<command-name>/go</command-name> is what I typed, please explain",
            "run <local-command-stdout>x</local-command-stdout> then stop",
            "<command-name>/go</command-name> and <unknown>tag</unknown>",
            "<command-name>/go",
            "mind the <system-reminder>",
            "the <pasted_content id=\"ab12\"> wrapper has no end",
            "the <pasted_content> tag, and </pasted_content> too",
            "use [Image] or [Image #x] in prose",
            "someone wrote: This session is being continued from a previous conversation",
        ];
        for input in cases {
            assert_eq!(classify(input), text(input), "input {input:?}");
        }
    }

    #[test]
    fn a_paste_wrapper_goes_and_its_content_stays() {
        let cases = [
            (
                "<pasted_content id=\"e482\">\n1. one\n2. two\n</pasted_content id=\"e482\">",
                "1. one\n2. two",
            ),
            (
                "look at this\n<pasted_content id=\"a1\">\nline one\nline two\n</pasted_content id=\"a1\">\nand then fix it",
                "look at this\n\nline one\nline two\n\nand then fix it",
            ),
            (
                "see: <pasted_content id=\"a1\">\nx\n</pasted_content id=\"a1\"> ok",
                "see:\n\nx\n\nok",
            ),
            (
                "<pasted_content id=\"a\">one</pasted_content id=\"a\"> and <pasted_content id=\"b\">two</pasted_content id=\"b\">",
                "one and two",
            ),
            (
                "<pasted_content id=\"a\">\nx\n</pasted_content id=\"b\">",
                "<pasted_content id=\"a\">\nx\n</pasted_content id=\"b\">",
            ),
            (
                "[Image #1] <pasted_content id=\"z\">\ncode\n</pasted_content id=\"z\">",
                "[Image #1]\n\ncode",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(classify(input), text(want), "input {input:?}");
        }
    }

    #[test]
    fn an_image_marker_becomes_a_chip() {
        let cases = [
            ("[Image #1]", "`Image 1`"),
            ("[Image #2] what is this?", "`Image 2` what is this?"),
            ("a [Image #1] b [Image #12] c", "a `Image 1` b `Image 12` c"),
            ("[Image #] and [Image #3", "[Image #] and [Image #3"),
            ("[Image] stays", "[Image] stays"),
        ];
        for (input, want) in cases {
            assert_eq!(image_chips(input), want, "input {input:?}");
        }
    }

    #[test]
    fn a_line_break_survives_markdown() {
        let cases = [
            ("one\ntwo", "one  \ntwo"),
            ("one\n\ntwo", "one\n\ntwo"),
            ("1. a\n2. b\n3. c", "1. a  \n2. b  \n3. c"),
            ("```\nlet a = 1;\nlet b = 2;\n```\nafter", "```\nlet a = 1;\nlet b = 2;\n```\nafter"),
            ("single", "single"),
            ("[Image #1]\nwhat is this", "`Image 1`  \nwhat is this"),
        ];
        for (input, want) in cases {
            assert_eq!(chat_markdown(input), want, "input {input:?}");
        }
    }

    #[test]
    fn lists_and_quotes_need_an_indent() {
        let cases = [
            ("1. one", true),
            ("intro\n  2) two", true),
            ("- a", true),
            ("> quoted", true),
            ("12.5 percent", false),
            ("1.x", false),
            ("just words", false),
        ];
        for (input, want) in cases {
            assert_eq!(has_indented_block(input), want, "input {input:?}");
        }
    }
}
