//! Finds the web links in text an agent wrote: `http(s)://` URLs, markdown links, and bare
//! `localhost:<port>` / `127.0.0.1:<port>` (which open as `http://`). Only http and https ever
//! come out; any other scheme is a plain word. `plain` serves painted text (the Progress panel),
//! `markdown` rewrites a chat message so the markdown renderer shows every link as one. Pure.

use std::borrow::Cow;
use std::ops::Range;

const LOCAL_HOSTS: [&str; 2] = ["localhost:", "127.0.0.1:"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plain,
    Markdown,
}

enum Kind {
    Bare,
    Markdown { label: Range<usize> },
    Autolink,
    Code,
}

struct Found {
    range: Range<usize>,
    kind: Kind,
    url: Option<String>,
}

/// A link inside the shown text: its byte range and where it opens.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) range: Range<usize>,
    pub(crate) url: String,
}

/// The text as it is shown (a markdown link is its label) and the links in it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plain {
    pub(crate) text: String,
    pub(crate) links: Vec<Link>,
}

/// True for an `http://` or `https://` URL with a host and no whitespace or control characters.
pub(crate) fn is_web_url(url: &str) -> bool {
    let rest = ["https://", "http://"].iter().find_map(|scheme| {
        url.get(..scheme.len()).filter(|head| head.eq_ignore_ascii_case(scheme))?;
        Some(&url[scheme.len()..])
    });
    rest.is_some_and(|rest| {
        rest.chars().next().is_some_and(char::is_alphanumeric)
            && !url.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

fn web_target(target: &str) -> Option<String> {
    if is_web_url(target) {
        return Some(target.to_owned());
    }
    (local_end(target, 0) == Some(target.len())).then(|| format!("http://{target}"))
}

pub(crate) fn plain(text: &str) -> Plain {
    let mut shown = String::with_capacity(text.len());
    let mut links = Vec::new();
    let mut last = 0;
    for found in scan(text, Mode::Plain) {
        shown.push_str(&text[last..found.range.start]);
        last = found.range.end;
        let piece = match &found.kind {
            Kind::Markdown { label } => &text[label.clone()],
            _ => &text[found.range.clone()],
        };
        if let Some(url) = found.url {
            links.push(Link { range: shown.len()..shown.len() + piece.len(), url });
        }
        shown.push_str(piece);
    }
    shown.push_str(&text[last..]);
    Plain { text: shown, links }
}

/// `md` with every web link written as a markdown link and every non-web link (`javascript:`,
/// `file:`, an email) turned into plain text, so the renderer can only ever open http and https.
/// Fenced code and reference definitions are left alone.
pub(crate) fn markdown(md: &str) -> Cow<'_, str> {
    if !md.contains(['h', 'H', 'l', '1', '[', '<']) {
        return Cow::Borrowed(md);
    }
    let mut out = String::with_capacity(md.len() + 32);
    let mut prose = 0..0;
    let mut fence: Option<(char, usize)> = None;
    let mut at = 0;
    for line in md.split_inclusive('\n') {
        let verbatim = if let Some(open) = fence {
            if closes_fence(line, open) {
                fence = None;
            }
            true
        } else {
            fence = opens_fence(line);
            fence.is_some() || is_definition(line)
        };
        if verbatim {
            out.push_str(&rewrite(&md[prose.clone()]));
            prose = at + line.len()..at + line.len();
            out.push_str(line);
        } else {
            prose.end = at + line.len();
        }
        at += line.len();
    }
    out.push_str(&rewrite(&md[prose]));
    if out == md { Cow::Borrowed(md) } else { Cow::Owned(out) }
}

fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let mark = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = trimmed.chars().take_while(|c| *c == mark).count();
    (run >= 3).then(|| (mark, run, &trimmed[run..]))
}

fn opens_fence(line: &str) -> Option<(char, usize)> {
    fence_of(line).map(|(mark, run, _)| (mark, run))
}

fn closes_fence(line: &str, (mark, run): (char, usize)) -> bool {
    fence_of(line).is_some_and(|(m, r, rest)| m == mark && r >= run && rest.trim().is_empty())
}

fn is_definition(line: &str) -> bool {
    let trimmed = line.trim_start_matches(' ');
    line.len() - trimmed.len() <= 3
        && trimmed.strip_prefix('[').is_some_and(|rest| {
            rest.find("]:").is_some_and(|end| !rest[..end].contains(['[', ']', '\n']))
        })
}

fn rewrite(chunk: &str) -> Cow<'_, str> {
    let found = scan(chunk, Mode::Markdown);
    let mut out = String::with_capacity(chunk.len() + 32);
    let mut last = 0;
    for item in &found {
        let before = &chunk[last..item.range.start];
        last = item.range.end;
        let whole = &chunk[item.range.clone()];
        let linked = match (&item.kind, &item.url) {
            (Kind::Markdown { label }, None) => {
                out.push_str(before);
                out.push_str(&chunk[label.clone()]);
                continue;
            }
            (Kind::Autolink, None) => {
                out.push_str(before);
                out.push('\\');
                out.push_str(whole);
                continue;
            }
            (Kind::Bare, Some(url)) => (escaped_label(whole), url),
            (Kind::Autolink, Some(url)) => (escaped_label(&whole[1..whole.len() - 1]), url),
            (Kind::Code, Some(url)) => (whole.to_owned(), url),
            (Kind::Markdown { .. }, Some(_)) | (_, None) => {
                out.push_str(before);
                out.push_str(whole);
                continue;
            }
        };
        match before.strip_suffix('!') {
            Some(head) => {
                out.push_str(head);
                out.push_str("\\!");
            }
            None => out.push_str(before),
        }
        out.push('[');
        out.push_str(&linked.0);
        out.push_str("](");
        out.push_str(&escaped_target(linked.1));
        out.push(')');
    }
    out.push_str(&chunk[last..]);
    if found.is_empty() { Cow::Borrowed(chunk) } else { Cow::Owned(out) }
}

fn escaped_label(text: &str) -> String {
    escaped(text, "\\*_[]<>&~`")
}

fn escaped_target(url: &str) -> String {
    escaped(url, "\\()<>&")
}

fn escaped(text: &str, special: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if special.contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn scan(text: &str, mode: Mode) -> Vec<Found> {
    let mut found = Vec::new();
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        let step = match c {
            '[' => markdown_link(text, i).map(|f| (f.range.end, Some(f))),
            '!' if mode == Mode::Markdown && text[i + 1..].starts_with('[') => {
                markdown_link(text, i + 1).map(|f| (f.range.end, None))
            }
            '<' if mode == Mode::Markdown => autolink(text, i).map(|f| (f.range.end, Some(f))),
            '`' if mode == Mode::Markdown => Some(code_span(text, i)),
            _ => None,
        };
        if let Some((end, item)) = step {
            found.extend(item);
            i = end;
            continue;
        }
        if let Some(item) = bare(text, i) {
            i = item.range.end;
            found.push(item);
            continue;
        }
        i += c.len_utf8();
    }
    found
}

fn markdown_link(text: &str, open: usize) -> Option<Found> {
    let mut depth = 0_usize;
    let mut close = None;
    let mut chars = text[open..].char_indices();
    while let Some((offset, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '\n' => return None,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let after = text[close + 1..].strip_prefix('(')?;
    let mut level = 0_usize;
    let mut dest_end = None;
    for (offset, c) in after.char_indices() {
        match c {
            c if c.is_whitespace() => {
                dest_end = Some(offset);
                break;
            }
            '(' => level += 1,
            ')' if level == 0 => {
                dest_end = Some(offset);
                break;
            }
            ')' => level -= 1,
            _ => {}
        }
    }
    let dest_end = dest_end?;
    let dest = &after[..dest_end];
    if dest.is_empty() {
        return None;
    }
    let mut rest = after[dest_end..].trim_start_matches([' ', '\t']);
    if let Some(quote) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) {
        let title = &rest[1..];
        let end = title.find([quote, '\n'])?;
        if title[end..].starts_with('\n') {
            return None;
        }
        rest = rest[end + 2..].trim_start_matches([' ', '\t']);
    }
    rest.strip_prefix(')')?;
    let end = text.len() - rest.len() + 1;
    Some(Found {
        range: open..end,
        kind: Kind::Markdown { label: open + 1..close },
        url: web_target(dest),
    })
}

fn autolink(text: &str, open: usize) -> Option<Found> {
    let inner = &text[open + 1..];
    let end = inner.find(['>', '<', '\n', ' ', '\t'])?;
    if !inner[end..].starts_with('>') {
        return None;
    }
    let content = &inner[..end];
    let scheme = content.split_once(':').map(|(scheme, _)| scheme);
    let is_scheme = scheme.is_some_and(|s| {
        s.len() >= 2
            && s.len() <= 32
            && s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    });
    let is_email = scheme.is_none() && content.contains('@');
    (is_scheme || is_email).then(|| Found {
        range: open..open + end + 2,
        kind: Kind::Autolink,
        url: if is_scheme { web_target(content).filter(|_| is_web_url(content)) } else { None },
    })
}

fn code_span(text: &str, open: usize) -> (usize, Option<Found>) {
    let run = text[open..].chars().take_while(|c| *c == '`').count();
    let mut at = open + run;
    while let Some(offset) = text[at..].find('`') {
        let start = at + offset;
        let len = text[start..].chars().take_while(|c| *c == '`').count();
        if text[open..start].contains("\n\n") {
            break;
        }
        if len == run {
            let end = start + len;
            let url = web_target(text[open + run..start].trim());
            let found = url.map(|url| Found { range: open..end, kind: Kind::Code, url: Some(url) });
            return (end, found);
        }
        at = start + len;
    }
    (open + run, None)
}

fn boundary(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|prev| !(prev.is_alphanumeric() || "_-./@:".contains(prev)))
}

fn bare(text: &str, at: usize) -> Option<Found> {
    if !boundary(text, at) {
        return None;
    }
    let end = web_end(text, at).or_else(|| local_end(text, at))?;
    let range = at..end;
    let target = &text[range.clone()];
    let url = if is_web_url(target) { target.to_owned() } else { format!("http://{target}") };
    Some(Found { range, kind: Kind::Bare, url: Some(url) })
}

fn web_end(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at..];
    let scheme = ["https://", "http://"]
        .iter()
        .find(|scheme| rest.get(..scheme.len()).is_some_and(|h| h.eq_ignore_ascii_case(scheme)))?;
    let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`'));
    let end = at + end.unwrap_or(rest.len());
    let end = trimmed_end(text, at + scheme.len(), end);
    is_web_url(&text[at..end]).then_some(end)
}

fn local_end(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at..];
    let host = LOCAL_HOSTS.iter().find(|host| rest.starts_with(**host))?;
    let digits = rest[host.len()..].chars().take_while(char::is_ascii_digit).count();
    if digits == 0
        || digits > 5
        || rest[host.len()..host.len() + digits].parse::<u32>().ok()? > 65535
    {
        return None;
    }
    let port_end = at + host.len() + digits;
    let Some(next) = text[port_end..].chars().next() else { return Some(port_end) };
    if next.is_alphanumeric() || next == '_' {
        return None;
    }
    if !matches!(next, '/' | '?' | '#') {
        return Some(port_end);
    }
    let tail = text[port_end..]
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`'))
        .map_or(text.len(), |n| port_end + n);
    Some(trimmed_end(text, port_end, tail))
}

fn trimmed_end(text: &str, floor: usize, mut end: usize) -> usize {
    while end > floor {
        let span = &text[floor..end];
        let Some(last) = span.chars().next_back() else { break };
        let strip = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '*' | '`' => true,
            ')' => span.matches(')').count() > span.matches('(').count(),
            ']' => span.matches(']').count() > span.matches('[').count(),
            '}' => span.matches('}').count() > span.matches('{').count(),
            _ => false,
        };
        if !strip {
            break;
        }
        end -= last.len_utf8();
    }
    end
}

#[cfg(test)]
mod tests;
