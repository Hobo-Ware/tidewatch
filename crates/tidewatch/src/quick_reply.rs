//! Quick replies: the chips under an agent's last message when it ends with a clear yes or no
//! question, or with a numbered list of choices. Pure text in, chips out. It is conservative on
//! purpose: a question that offers alternatives ("A or B?"), asks "which" or "what", or a numbered
//! list that reads as steps gets no chips.

use tidewatch_core::end_block;

const MAX_CHOICES: usize = 6;
const MAX_CHOICE_CHARS: usize = 48;
const YES_NO_OPENERS: [&str; 22] = [
    "should i",
    "shall i",
    "should we",
    "shall we",
    "do you want",
    "would you like",
    "would you want",
    "want me to",
    "can i",
    "may i",
    "could i",
    "is it ok",
    "is it okay",
    "is that ok",
    "is that okay",
    "is that fine",
    "is this ok",
    "is this okay",
    "does that work",
    "does this work",
    "ok to",
    "okay to",
];
const CHOICE_LEAD_WORDS: [&str; 8] =
    ["which", "choose", "pick", "option", "prefer", "select", "reply with", "let me know"];

/// One chip: what it says and what it sends as the reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Quick {
    pub(crate) label: String,
    pub(crate) send: String,
}

/// The chips for the end of `message`, in order, without the trailing "Reply..." chip the view adds.
pub(crate) fn from_message(message: &str) -> Vec<Quick> {
    let body = asked_part(message);
    if let Some(choices) = numbered_choices(&body) {
        return choices;
    }
    if is_yes_no(&last_sentence(&body)) {
        return vec![
            Quick { label: "Yes".into(), send: "Yes".into() },
            Quick { label: "No".into(), send: "No".into() },
        ];
    }
    Vec::new()
}

/// What the agent asks at the end: the "Needs you" lines of a house-style end block (nothing when it
/// needs nothing), else the whole message.
fn asked_part(message: &str) -> String {
    match end_block::parse(message) {
        Some(block) if block.needs_input() => block.needs_you.join("\n"),
        Some(_) => String::new(),
        None => message.trim_end().to_owned(),
    }
}

fn plain(text: &str) -> String {
    text.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect::<String>().trim().to_owned()
}

/// The last sentence of `body`, as plain text: what follows the last full stop, `!` or `?` that is
/// followed by a space, on the last line.
fn last_sentence(body: &str) -> String {
    let body = plain(body.trim_end());
    let last_line = body.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or_default();
    let line = last_line.trim().trim_start_matches(['-', '\u{2022}']).trim();
    let line = line.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches(". ");
    let mut start = 0;
    let mut chars = line.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?')
            && chars.peek().is_some_and(|(_, next)| next.is_whitespace())
        {
            start = at + c.len_utf8();
        }
    }
    line[start..].trim().to_owned()
}

fn is_yes_no(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    let lower = lower.trim_start_matches(|c: char| !c.is_alphanumeric());
    if !lower.ends_with('?') || lower.contains(" or ") || lower.contains('\n') {
        return false;
    }
    let lower = lower.strip_prefix("also, ").unwrap_or(lower);
    YES_NO_OPENERS.iter().any(|opener| {
        lower
            .strip_prefix(opener)
            .is_some_and(|rest| rest.starts_with(|c: char| !c.is_alphanumeric()))
    })
}

fn without_trailing_blanks(lines: &[&str], mut end: usize) -> usize {
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    end
}

fn offers_a_choice(text: &str) -> bool {
    let lower = text.to_lowercase();
    CHOICE_LEAD_WORDS.iter().any(|word| lower.contains(word))
}

/// The `1.`, `2.`, ... list at the end of `body`, when it is offered as a choice: a question about
/// it follows, or a line before it ending in `:` or `?` asks to pick.
fn numbered_choices(body: &str) -> Option<Vec<Quick>> {
    let lines: Vec<&str> = body.lines().map(str::trim_end).collect();
    let mut end = without_trailing_blanks(&lines, lines.len());
    let mut asked_after = false;
    if end > 0 && number_of(lines[end - 1]).is_none() {
        let last = lines[end - 1];
        if !last.ends_with('?') || !offers_a_choice(last) {
            return None;
        }
        asked_after = true;
        end = without_trailing_blanks(&lines, end - 1);
    }
    let mut start = end;
    while start > 0 && number_of(lines[start - 1]).is_some() {
        start -= 1;
    }
    let items = &lines[start..end];
    if !(2..=MAX_CHOICES).contains(&items.len()) {
        return None;
    }
    if !items.iter().enumerate().all(|(index, line)| number_of(line) == Some(index + 1)) {
        return None;
    }
    let lead = lines[..start].iter().rev().find(|line| !line.trim().is_empty()).copied();
    let offered =
        asked_after || lead.is_some_and(|lead| lead.ends_with(['?', ':']) && offers_a_choice(lead));
    offered.then(|| {
        items
            .iter()
            .enumerate()
            .map(|(index, line)| Quick {
                label: format!("{}. {}", index + 1, short(&plain(item_text(line)))),
                send: (index + 1).to_string(),
            })
            .collect()
    })
}

fn number_of(line: &str) -> Option<usize> {
    let line = line.trim_start().trim_start_matches(['*', '_']);
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 2 {
        return None;
    }
    let rest = &line[digits..];
    (rest.starts_with(". ") || rest.starts_with(") ")).then(|| line[..digits].parse().ok())?
}

fn item_text(line: &str) -> &str {
    let line = line.trim_start().trim_start_matches(['*', '_']);
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    line[digits + 2..].trim()
}

fn short(text: &str) -> String {
    let text = text.split(" - ").next().unwrap_or(text).trim();
    if text.chars().count() <= MAX_CHOICE_CHARS {
        return text.to_owned();
    }
    let cut: String = text.chars().take(MAX_CHOICE_CHARS - 3).collect();
    format!("{}...", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yes_no() -> Vec<Quick> {
        vec![
            Quick { label: "Yes".into(), send: "Yes".into() },
            Quick { label: "No".into(), send: "No".into() },
        ]
    }

    fn numbered(items: &[&str]) -> Vec<Quick> {
        items
            .iter()
            .enumerate()
            .map(|(index, item)| Quick {
                label: format!("{}. {item}", index + 1),
                send: (index + 1).to_string(),
            })
            .collect()
    }

    #[test]
    fn clear_yes_or_no_questions_at_the_end_get_yes_and_no() {
        let cases = [
            "Tests pass. Should I also update the README section on workflows?",
            "Done.\n\nWould you like me to open a PR?",
            "All green. Do you want me to continue with the second part?",
            "Want me to run the full suite too?",
            "I changed `src/a.rs:4`.\n\n**Shall I** commit it?",
            "Fixed. Is that okay?",
            "Fixed it.\n\nCan I delete the old helper?  \n",
            "- noted\n- Should we merge this now?",
            "Tests pass. Also, should I bump the version?",
        ];
        for message in cases {
            assert_eq!(from_message(message), yes_no(), "{message:?}");
        }
    }

    #[test]
    fn a_question_that_is_not_clearly_yes_or_no_gets_no_chips() {
        let cases = [
            "Which approach do you prefer?",
            "Should I use a trait or an enum?",
            "What should the name be?",
            "How do you want to handle errors?",
            "Tests pass. I will update the README next.",
            "Should I update the README? I will wait.",
            "Should I update the README? Let me know when you are back",
            "Done.",
            "",
            "   \n\n",
            "Is the build green",
            "Why does this fail?",
        ];
        for message in cases {
            assert_eq!(from_message(message), Vec::<Quick>::new(), "{message:?}");
        }
    }

    #[test]
    fn only_the_last_thing_asked_counts() {
        assert_eq!(from_message("Should I refactor it?\n\nI did it anyway. All done."), Vec::new());
        assert_eq!(from_message("All done.\n\nShould I refactor it?"), yes_no());
    }

    #[test]
    fn a_numbered_list_of_choices_gets_one_chip_per_item() {
        let message = "I found three ways to do it:\n\n1. Keep the retry count on the stage\n2. Keep it on the run\n3. Drop retries\n\nWhich one do you prefer?";
        assert_eq!(
            from_message(message),
            numbered(&["Keep the retry count on the stage", "Keep it on the run", "Drop retries"])
        );
        let lead_only = "Pick one of these:\n1. Fast\n2. Careful";
        assert_eq!(from_message(lead_only), numbered(&["Fast", "Careful"]));
        let bold = "Which option?\n\n1. **Fast** - quickest\n2. **Careful** - slower";
        assert_eq!(from_message(bold), numbered(&["Fast", "Careful"]));
    }

    #[test]
    fn a_numbered_list_that_is_not_a_choice_gets_no_chips() {
        let cases = [
            "Here is what I did:\n\n1. Added the field\n2. Updated the tests\n3. Ran the suite",
            "Steps:\n1. Run the tests\n2. Commit",
            "1. only one\n\nWhich?",
            "Which one?\n\n1. a\n3. b\n4. c",
            "Which one?\n\n2. a\n3. b",
            "Which one?\n\n1. a\n2. b\n3. c\n4. d\n5. e\n6. f\n7. g",
            "1. a\n2. b\n\nAll done, nothing to decide.",
        ];
        for message in cases {
            assert_eq!(from_message(message), Vec::<Quick>::new(), "{message:?}");
        }
    }

    #[test]
    fn long_choices_are_cut_for_the_chip_but_the_reply_is_the_number() {
        let long = "x".repeat(80);
        let message = format!("Which one?\n\n1. {long}\n2. short");
        let chips = from_message(&message);
        assert_eq!(chips[0].send, "1");
        assert_eq!(chips[0].label.chars().count(), "1. ".len() + MAX_CHOICE_CHARS);
        assert!(chips[0].label.ends_with("..."));
        assert_eq!(chips[1].label, "2. short");
    }

    #[test]
    fn an_end_block_that_needs_nothing_gets_no_chips_and_one_that_asks_is_read() {
        let nothing = "Changed it.\n\n**Done**\n- fixed\n\n**Needs you**\n- nothing\n\n**Next**\n- should I merge it?";
        assert_eq!(from_message(nothing), Vec::new());
        let asks = "Changed it.\n\n**Done**\n- fixed\n\n**Needs you**\n- Should I also update the README?\n\n**Next**\n- wait";
        assert_eq!(from_message(asks), yes_no());
    }
}
