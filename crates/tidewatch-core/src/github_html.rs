//! The HTML GitHub comments carry, turned into what the Reader can draw: comments vanish, `<br>` is
//! a line break, `<details>` and `<summary>` become plain sections, `<relative-time>` is its text.
//! Fenced code is left alone, and so is a `<` that does not open one of these tags. Pure.

use std::borrow::Cow;

const COMMENT_OPEN: &str = "<!--";
const COMMENT_CLOSE: &str = "-->";

enum Tag {
    Text(&'static str),
    Break,
    Link,
}

fn known(name: &str, closing: bool) -> Option<Tag> {
    let text =
        |open: &'static str, close: &'static str| Tag::Text(if closing { close } else { open });
    Some(match name {
        "br" => Tag::Break,
        "details" | "p" | "div" | "table" | "thead" | "tbody" | "tr" | "td" | "th" | "ul"
        | "ol" | "li" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "hr" => {
            if matches!(name, "hr") { Tag::Text("\n---\n") } else { Tag::Break }
        }
        "summary" => text("**", "**\n"),
        "b" | "strong" => text("**", "**"),
        "i" | "em" => text("*", "*"),
        "code" | "kbd" | "tt" => text("`", "`"),
        "relative-time" | "time" | "span" | "sub" | "sup" | "small" | "ins" | "u" | "font"
        | "picture" | "source" | "img" => Tag::Text(""),
        "a" => Tag::Link,
        _ => return None,
    })
}

struct Parsed<'a> {
    name: String,
    closing: bool,
    attrs: &'a str,
    len: usize,
}

fn parse_tag(text: &str) -> Option<Parsed<'_>> {
    let rest = text.strip_prefix('<')?;
    let (closing, rest) = match rest.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    let name_len = rest.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'-').count();
    if name_len == 0 || !rest.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    let after = &rest[name_len..];
    let mut quote: Option<char> = None;
    for (at, c) in after.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                let attrs = after[..at].trim_end_matches('/').trim();
                let consumed = text.len() - after.len() + at + 1;
                return Some(Parsed {
                    name: rest[..name_len].to_ascii_lowercase(),
                    closing,
                    attrs,
                    len: consumed,
                });
            }
            (None, '<' | '\n') => return None,
            _ => {}
        }
    }
    None
}

fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let at = attrs.find(&format!("{name}="))?;
    let value = &attrs[at + name.len() + 1..];
    let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let value = &value[1..];
    value.find(quote).map(|end| &value[..end])
}

/// One line without its comments and tags. `lead` goes after every line break the tags make, so a
/// break inside a quote stays in the quote.
fn line(body: &str, lead: &str, in_comment: &mut bool, links: &mut Vec<Option<String>>) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while !rest.is_empty() {
        if *in_comment {
            match rest.find(COMMENT_CLOSE) {
                Some(end) => {
                    rest = &rest[end + COMMENT_CLOSE.len()..];
                    *in_comment = false;
                }
                None => rest = "",
            }
            continue;
        }
        let Some(at) = rest.find('<') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if rest.starts_with(COMMENT_OPEN) {
            *in_comment = true;
            rest = &rest[COMMENT_OPEN.len()..];
            continue;
        }
        let Some(tag) = parse_tag(rest) else {
            out.push('<');
            rest = &rest[1..];
            continue;
        };
        let Some(known) = known(&tag.name, tag.closing) else {
            out.push('<');
            rest = &rest[1..];
            continue;
        };
        match known {
            Tag::Text(text) => out.push_str(text),
            Tag::Break => {
                out.push('\n');
                out.push_str(lead);
            }
            Tag::Link if tag.closing => {
                if let Some(href) = links.pop().flatten() {
                    out.push_str("](");
                    out.push_str(&href);
                    out.push(')');
                }
            }
            Tag::Link => {
                let href = attribute(tag.attrs, "href").filter(|href| !href.is_empty());
                if href.is_some() {
                    out.push('[');
                }
                links.push(href.map(str::to_owned));
            }
        }
        if tag.name == "img"
            && let Some(alt) = attribute(tag.attrs, "alt")
        {
            out.push_str(alt);
        }
        rest = &rest[tag.len..];
    }
    out
}

fn quote_lead(text: &str) -> usize {
    let mut at = 0;
    loop {
        let rest = &text[at..];
        let spaces = rest.len() - rest.trim_start_matches(' ').len();
        if spaces <= 3 && rest[spaces..].starts_with('>') {
            at += spaces + 1;
            if text[at..].starts_with(' ') {
                at += 1;
            }
        } else {
            return at;
        }
    }
}

fn fence_mark(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    ["```", "~~~"].into_iter().find(|mark| trimmed.starts_with(mark))
}

fn needs_work(md: &str) -> bool {
    md.match_indices('<').any(|(at, _)| {
        md.as_bytes()
            .get(at + 1)
            .is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, b'!' | b'/'))
    })
}

/// `md` with its comments removed and its common GitHub tags turned into markdown. Text without a
/// `<` comes back untouched.
#[must_use]
pub fn scrub(md: &str) -> Cow<'_, str> {
    if !needs_work(md) {
        return Cow::Borrowed(md);
    }
    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<String> = None;
    let mut in_comment = false;
    let mut links: Vec<Option<String>> = Vec::new();
    for raw in md.split('\n') {
        let lead_len = quote_lead(raw);
        let (lead, body) = raw.split_at(lead_len);
        if !in_comment {
            match (&fence, fence_mark(body)) {
                (Some(open), Some(mark)) if mark == open => {
                    fence = None;
                    out.push(raw.to_owned());
                    continue;
                }
                (Some(_), _) => {
                    out.push(raw.to_owned());
                    continue;
                }
                (None, Some(mark)) => {
                    fence = Some(mark.to_owned());
                    out.push(raw.to_owned());
                    continue;
                }
                (None, None) => {}
            }
        }
        let had_markup = in_comment || body.contains('<');
        let text = line(body, lead, &mut in_comment, &mut links);
        if had_markup && text.trim().is_empty() {
            out.push(lead.trim_end().to_owned());
        } else {
            out.push(format!("{lead}{text}"));
        }
    }
    let joined = out.join("\n");
    Cow::Owned(squeeze_blank_lines(&joined).trim_matches('\n').to_owned())
}

fn squeeze_blank_lines(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut fenced = false;
    for line in text.split('\n') {
        let body = &line[quote_lead(line)..];
        if fence_mark(body).is_some() {
            fenced = !fenced;
        }
        let blank = !fenced && body.trim().is_empty();
        let after_blank =
            out.last().is_some_and(|last| last.trim_start_matches([' ', '>']).is_empty());
        if blank && (out.is_empty() || after_blank) {
            continue;
        }
        out.push(line);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn is(input: &str, expected: &str) {
        assert_eq!(scrub(input), expected, "input: {input:?}");
    }

    #[test]
    fn text_without_markup_is_untouched() {
        assert!(matches!(scrub("plain **text** and a < b"), Cow::Borrowed(_)));
        is("a < b and c > d", "a < b and c > d");
    }

    #[test]
    fn comments_vanish_on_one_line_and_across_lines() {
        is("keep <!-- gone --> this", "keep  this");
        is("a\n<!-- codex-review-state:v1 {\"x\":1} -->\nb", "a\n\nb");
        is("a\n<!-- start\nmiddle\nend -->\nb", "a\n\nb");
        is("> <!-- hidden -->\n> shown", "> shown");
    }

    #[test]
    fn an_unclosed_comment_is_hidden_until_it_ends_but_text_before_stays() {
        is("before <!-- never closed\nstill hidden", "before ");
    }

    #[test]
    fn br_is_a_line_break_and_stays_inside_a_quote() {
        is("one<br>two<br/>three<br />four", "one\ntwo\nthree\nfour");
        is("> one<br/>two", "> one\n> two");
    }

    #[test]
    fn details_and_summary_become_a_bold_line() {
        is(
            "<details>\n<summary>Show more</summary>\n\nbody text\n</details>",
            "**Show more**\n\nbody text",
        );
        is("<details><summary>One</summary>inner</details>", "**One**\ninner");
    }

    #[test]
    fn relative_time_is_its_text() {
        is(
            "at <relative-time datetime=\"2026-10-25T10:00:00Z\" class=\"no-wrap\">Oct 25, 2026</relative-time> ok",
            "at Oct 25, 2026 ok",
        );
    }

    #[test]
    fn inline_tags_turn_into_markdown() {
        is("<b>bold</b> <em>it</em> <code>x</code>", "**bold** *it* `x`");
        is("<a href=\"https://x.test/a?b=1\">site</a>", "[site](https://x.test/a?b=1)");
        is("<a name=\"top\">anchor</a>", "anchor");
        is("<img src=\"a.png\" alt=\"shot\">", "shot");
    }

    #[test]
    fn code_fences_are_left_alone() {
        let code = "```html\n<!-- keep -->\n<br>\n```";
        is(code, code);
        is("<br>x\n```\n<br>\n```\n<br>y", "x\n```\n<br>\n```\n\ny");
    }

    #[test]
    fn unknown_tags_and_stray_angles_stay_as_written() {
        is(
            "<custom-thing>x</custom-thing> and <3 and <br",
            "<custom-thing>x</custom-thing> and <3 and <br",
        );
    }

    #[test]
    fn runs_of_blank_lines_collapse_and_the_start_is_trimmed() {
        is("<!-- a -->\n<!-- b -->\n\nbody\n<br>\n<br>\nend", "body\n\nend");
    }

    #[test]
    fn a_codex_style_summary_reads_cleanly() {
        let input = "<!-- codex-pull-request-review-summary -->\n> ### Review\n> <details>\n> <summary>Details</summary>\n>\n> | a | b |\n> |---|---|\n> | 1 | 2 |\n>\n> </details>\n> <!-- codex-review-state:v1 {\"k\":1} -->";
        let out = scrub(input);
        assert!(!out.contains("<!--") && !out.contains("<details") && !out.contains("<summary"));
        assert!(out.contains("> **Details**"));
        assert!(out.contains("> | a | b |"));
    }
}
