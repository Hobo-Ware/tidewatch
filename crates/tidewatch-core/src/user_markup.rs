//! The tagged records Claude Code writes into user lines besides the user's words (checked on real
//! 2.1.3xx transcripts): slash commands, local command output, caveats, `<pasted_content id>` wrappers
//! `<task-notification>` records and `[Image #1]` markers. Pure text functions.

const COMMAND_TAGS: [&str; 3] = ["command-name", "command-message", "command-args"];
const OUTPUT_TAGS: [&str; 4] =
    ["local-command-stdout", "local-command-stderr", "bash-stdout", "bash-stderr"];
const BASH_INPUT_TAG: [&str; 1] = ["bash-input"];
/// What a `!` shell command starts with, in the owner's message and in the command row.
pub const BASH_PREFIX: &str = "! ";
const HIDDEN_PREFIXES: [&str; 2] = ["<local-command-caveat>", "<system-reminder>"];
const PASTE_TAG: &str = "pasted_content";
const TASK_TAG: &str = "task-notification";
const TASK_EVENT_CHARS: usize = 140;
const IMAGE_MARKER: &str = "[Image #";
const COMPACTED_OPENING: &str = "This session is being continued from a previous conversation";

/// What one user text of a Claude transcript is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    /// A slash command the user ran: `/rate-limit-options`, with its arguments after a space, or a
    /// `! command` the shell ran.
    Command(String),
    /// What a local command printed, tags and terminal colors removed.
    Output(String),
    /// The user's own words, the paste wrapper removed.
    Text(String),
    /// A background task Claude Code reports on, as `Background task - <summary>` and, when the
    /// task sent one, the first line of its event under it.
    Background(String),
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
    if let Some(note) = background(text) {
        return Record::Background(note);
    }
    if let Some(parts) = tag_sequence(text, &COMMAND_TAGS) {
        return command(&parts);
    }
    if let Some(parts) = tag_sequence(text, &BASH_INPUT_TAG) {
        return bash_command(&parts);
    }
    if let Some(parts) = tag_sequence(text, &OUTPUT_TAGS) {
        return output(&parts);
    }
    Record::Text(unwrap_pasted(text))
}

/// Turns a user text into the markdown the chat bubble draws: `[Image #1]` becomes a small code
/// chip `Image 1` with a space after it, unless its number is in `shown` (the bubble draws that
/// picture itself), when the marker and the space beside it go. A single line break stays a line
/// break instead of folding into a space.
#[must_use]
pub fn chat_markdown(text: &str, shown: &[u32]) -> String {
    keep_line_breaks(&image_chips(&crate::github_html::scrub(text), shown))
}

/// `text` with every `[Image #N]` marker taken out and the spacing around each one tidied, for a
/// label that has no room for a picture.
#[must_use]
pub fn without_image_markers(text: &str) -> String {
    image_chips(text, &image_numbers(text))
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

const BACKGROUND_PREFIX: &str = "Background task - ";

/// How a background task ended, as the slim card in the Reader says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskResult {
    Done(String),
    Failed(String),
    Stopped,
}

/// A background task note (`Background task - <summary>`, the first event line under it) read back
/// into the task's name and how it ended. `None` for any other text.
#[must_use]
pub fn background_card(note: &str) -> Option<(String, TaskResult)> {
    let rest = note.strip_prefix(BACKGROUND_PREFIX)?;
    let (summary, event) = rest.split_once('\n').unwrap_or((rest, ""));
    let name = match summary.split_once('"').and_then(|(_, tail)| tail.split_once('"')) {
        Some((quoted, _)) if !quoted.is_empty() => quoted.to_owned(),
        _ => summary.to_owned(),
    };
    let code = summary
        .split_once("exit code ")
        .and_then(|(_, tail)| tail.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse::<u32>().ok());
    let result = match code {
        Some(0) | None if event.contains("passed") => TaskResult::Done(cut(event.trim(), 24)),
        Some(0) => TaskResult::Done("Done".to_owned()),
        Some(code) => TaskResult::Failed(format!("Failed, exit {code}")),
        None if summary.contains("was stopped") || summary.contains("killed") => {
            TaskResult::Stopped
        }
        None if summary.contains("failed") => TaskResult::Failed("Failed".to_owned()),
        None => TaskResult::Done("Done".to_owned()),
    };
    Some((name, result))
}

fn background(text: &str) -> Option<String> {
    let open = format!("<{TASK_TAG}>");
    let close = format!("</{TASK_TAG}>");
    let inner = text.strip_prefix(open.as_str())?;
    let (inner, after) = inner.split_once(close.as_str())?;
    let after = after.trim();
    if !after.is_empty() && !after.starts_with("<system-reminder>") {
        return None;
    }
    let summary = one_line(child(inner, "summary")?);
    if summary.is_empty() {
        return None;
    }
    let mut note = format!("{BACKGROUND_PREFIX}{summary}");
    let event =
        child(inner, "event").and_then(|event| event.lines().find(|l| !l.trim().is_empty()));
    if let Some(event) = event {
        note.push('\n');
        note.push_str(&cut(&one_line(event), TASK_EVENT_CHARS));
    }
    Some(note)
}

fn child<'a>(inner: &'a str, tag: &str) -> Option<&'a str> {
    let (_, rest) = inner.split_once(format!("<{tag}>").as_str())?;
    Some(rest.split_once(format!("</{tag}>").as_str())?.0)
}

fn cut(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    let kept: String = text.chars().take(chars).collect();
    format!("{}\u{2026}", kept.trim_end())
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

fn bash_command(parts: &[(&str, &str)]) -> Record {
    let command = parts.iter().map(|(_, inner)| inner.trim()).collect::<Vec<_>>().join(" ");
    if command.is_empty() {
        return Record::Hidden;
    }
    Record::Command(format!("{BASH_PREFIX}{command}"))
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

fn markers(text: &str) -> Vec<(std::ops::Range<usize>, u32)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(IMAGE_MARKER).map(|at| at + from) {
        let digits = &text[at + IMAGE_MARKER.len()..];
        let len = digits.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 || !digits[len..].starts_with(']') {
            from = at + IMAGE_MARKER.len();
            continue;
        }
        let end = at + IMAGE_MARKER.len() + len + 1;
        if let Ok(number) = digits[..len].parse() {
            found.push((at..end, number));
        }
        from = end;
    }
    found
}

/// The numbers of the `[Image #N]` markers of `text`, in the order they appear.
#[must_use]
pub fn image_numbers(text: &str) -> Vec<u32> {
    markers(text).into_iter().map(|(_, number)| number).collect()
}

fn image_chips(text: &str, shown: &[u32]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut removed = false;
    for (range, number) in markers(text) {
        out.push_str(&text[last..range.start]);
        let after = &text[range.end..];
        if shown.contains(&number) {
            removed = true;
            let own_place = out.is_empty() || out.ends_with([' ', '\t', '\n']);
            let skipped = if own_place {
                after.len() - after.trim_start_matches([' ', '\t']).len()
            } else {
                0
            };
            if after[skipped..].starts_with(['.', ',', ';', ':', '!', '?', ')']) {
                out.truncate(out.trim_end_matches([' ', '\t']).len());
            }
            last = range.end + skipped;
        } else {
            out.push_str("`Image ");
            out.push_str(&number.to_string());
            out.push('`');
            if after.chars().next().is_some_and(|next| !next.is_whitespace()) {
                out.push(' ');
            }
            last = range.end;
        }
    }
    out.push_str(&text[last..]);
    if removed { out.trim().to_owned() } else { out }
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
            (
                "<bash-input> pscale branch create trakt_production media-merge-pairs</bash-input>",
                Record::Command("! pscale branch create trakt_production media-merge-pairs".into()),
            ),
            ("<bash-input>  </bash-input>", Record::Hidden),
            (
                "<bash-stdout>\u{1b}[32mBranch\u{1b}[39m created\n</bash-stdout><bash-stderr></bash-stderr>",
                Record::Output("Branch created".into()),
            ),
            (
                "<bash-stdout></bash-stdout><bash-stderr>Error: no such branch</bash-stderr>",
                Record::Output("Error: no such branch".into()),
            ),
            ("<bash-stdout></bash-stdout><bash-stderr></bash-stderr>", Record::Hidden),
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
    fn a_task_notification_is_a_background_line_with_its_event() {
        let cases = [
            (
                "<task-notification>\n<task-id>bs6</task-id>\n<tool-use-id>toolu_01</tool-use-id>\n<output-file>/tmp/x.output</output-file>\n<status>killed</status>\n<summary>Background command \"Start v2 dev server\" was stopped</summary>\n</task-notification>",
                "Background task - Background command \"Start v2 dev server\" was stopped",
            ),
            (
                "<task-notification>\n<task-id>bq6</task-id>\n<summary>Monitor event: \"R2 corpus pull\"</summary>\n<event>[####-----] 33% 104/314</event>\n</task-notification>",
                "Background task - Monitor event: \"R2 corpus pull\"\n[####-----] 33% 104/314",
            ),
            (
                "<task-notification><summary>Agent  finished</summary><event>\n\nfirst\nsecond</event><note>fires again</note></task-notification>\n<system-reminder>Goal check-in</system-reminder>",
                "Background task - Agent finished\nfirst",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(classify(input), Record::Background(want.into()), "input {input:?}");
        }
        let long = format!(
            "<task-notification><summary>s</summary><event>{}</event></task-notification>",
            "w".repeat(200)
        );
        let Record::Background(note) = classify(&long) else { panic!("not a note") };
        assert_eq!(
            note.lines().nth(1).map(|line| line.chars().count()),
            Some(TASK_EVENT_CHARS + 1)
        );
        assert!(note.ends_with('\u{2026}'));
    }

    #[test]
    fn a_message_that_mentions_the_task_tag_stays_a_message() {
        let cases = [
            "why does <task-notification> show up in my chat?",
            "<task-notification> is what I typed, please explain",
            "<task-notification><summary>x</summary></task-notification> what is this?",
            "<task-notification>done</task-notification>",
            "<task-notification><summary>  </summary></task-notification>",
            "see\n<task-notification><summary>x</summary></task-notification>",
        ];
        for input in cases {
            assert_eq!(classify(input), text(input), "input {input:?}");
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
            assert_eq!(image_chips(input, &[]), want, "input {input:?}");
        }
    }

    #[test]
    fn a_chip_never_runs_into_the_text_after_it() {
        let cases = [
            ("[Image #2]Also this", "`Image 2` Also this"),
            ("[Image #2]\nAlso this", "`Image 2`\nAlso this"),
            ("[Image #2] Also this", "`Image 2` Also this"),
            ("see [Image #1].", "see `Image 1` ."),
            ("[Image #1][Image #2]x", "`Image 1` `Image 2` x"),
        ];
        for (input, want) in cases {
            assert_eq!(image_chips(input, &[]), want, "input {input:?}");
        }
    }

    #[test]
    fn a_marker_whose_picture_is_drawn_goes_and_one_without_stays_a_chip() {
        let cases = [
            ("[Image #2]Also this", &[2][..], "Also this"),
            ("[Image #2] Also this", &[2], "Also this"),
            ("[Image #1]\n\nwhat is this", &[1], "what is this"),
            ("see [Image #1] here", &[1], "see here"),
            ("[Image #1]", &[1], ""),
            ("[Image #1][Image #2] both", &[1, 2], "both"),
            ("[Image #1] and [Image #2] x", &[1], "and `Image 2` x"),
            ("[Image #1] and [Image #2] x", &[2], "`Image 1` and x"),
            ("[Image #3] x", &[1, 2], "`Image 3` x"),
            ("What is [Image #5]?", &[5], "What is?"),
        ];
        for (input, shown, want) in cases {
            assert_eq!(image_chips(input, shown), want, "input {input:?} shown {shown:?}");
        }
    }

    #[test]
    fn a_label_text_loses_every_marker_and_keeps_its_words() {
        let cases = [
            ("[Image #2]Also this should just be costs", "Also this should just be costs"),
            ("[Image #1] [Image #2] compare", "compare"),
            ("see [Image #3] now", "see now"),
            ("[Image #1]", ""),
            ("[Image #1]\n\n[Image #2]", ""),
            ("no markers [Image] here", "no markers [Image] here"),
        ];
        for (input, want) in cases {
            assert_eq!(without_image_markers(input), want, "input {input:?}");
        }
    }

    #[test]
    fn the_marker_numbers_come_in_reading_order() {
        assert_eq!(image_numbers("[Image #9] a [Image #x] b [Image #10]"), [9, 10]);
        assert_eq!(image_numbers("none"), Vec::<u32>::new());
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
            assert_eq!(chat_markdown(input, &[]), want, "input {input:?}");
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

    #[test]
    fn a_background_note_reads_back_as_a_name_and_a_result() {
        let done = |text: &str| TaskResult::Done(text.to_owned());
        let cases = [
            (
                "Background task - Background command \"Run the importer tests\" completed (exit code 0)",
                Some(("Run the importer tests", done("Done"))),
            ),
            (
                "Background task - Background command \"Build it\" failed with exit code 101",
                Some(("Build it", TaskResult::Failed("Failed, exit 101".to_owned()))),
            ),
            (
                "Background task - Background command \"Start dev server\" was stopped",
                Some(("Start dev server", TaskResult::Stopped)),
            ),
            (
                "Background task - Monitor event: \"tests\"\n41 passed, 0 failed",
                Some(("tests", done("41 passed, 0 failed"))),
            ),
            ("Background task - Agent finished", Some(("Agent finished", done("Done")))),
            ("Context usage: 42k", None),
        ];
        for (note, want) in cases {
            let want = want.map(|(name, result)| (name.to_owned(), result));
            assert_eq!(background_card(note), want, "note {note:?}");
        }
    }
}
