//! Text-level edits of JSON files that may hold comments, so everything an edit does not touch
//! (key order, indentation, comments, numbers as written) stays byte for byte.
//!
//! The scanner accepts JSON plus `//` and `/* */` comments and nothing else: a trailing comma, a
//! bare word or a second document is an error, and callers refuse to edit a file that fails it.

use std::fmt::{self, Write};

const MAX_DEPTH: usize = 64;
const UNIT: &str = "  ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    pub(crate) at: usize,
    pub(crate) what: &'static str,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.what, self.at)
    }
}

type Scan<T> = Result<T, SyntaxError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Member {
    pub(crate) key: String,
    pub(crate) key_start: usize,
    pub(crate) value: Span,
}

#[derive(Clone, Debug)]
pub(crate) struct Object {
    pub(crate) open: usize,
    pub(crate) close: usize,
    pub(crate) members: Vec<Member>,
}

#[derive(Clone, Debug)]
pub(crate) struct Array {
    pub(crate) open: usize,
    pub(crate) close: usize,
    pub(crate) elements: Vec<Span>,
}

impl Object {
    pub(crate) fn get(&self, key: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.key == key)
    }
}

/// A value to write. Objects and arrays are laid out to match the container they go into.
#[derive(Clone, Debug)]
pub(crate) enum Value {
    Bool(bool),
    Str(String),
    Object(Vec<(String, Value)>),
    Array(Vec<Value>),
}

struct Scanner<'a> {
    text: &'a str,
}

impl Scanner<'_> {
    fn byte(&self, at: usize) -> Option<u8> {
        self.text.as_bytes().get(at).copied()
    }

    fn trivia(&self, mut at: usize) -> Scan<usize> {
        loop {
            match self.byte(at) {
                Some(b' ' | b'\t' | b'\n' | b'\r') => at += 1,
                Some(b'/') if self.byte(at + 1) == Some(b'/') => {
                    at = self.text[at..].find('\n').map_or(self.text.len(), |n| at + n);
                }
                Some(b'/') if self.byte(at + 1) == Some(b'*') => {
                    let Some(n) = self.text[at + 2..].find("*/") else {
                        return Err(SyntaxError { at, what: "unterminated comment" });
                    };
                    at += 2 + n + 2;
                }
                _ => return Ok(at),
            }
        }
    }

    fn string(&self, start: usize) -> Scan<(String, usize)> {
        let mut out = String::new();
        let mut chars = self.text[start + 1..].char_indices();
        while let Some((i, c)) = chars.next() {
            let here = start + 1 + i;
            match c {
                '"' => return Ok((out, here + 1)),
                '\\' => {
                    let Some((_, esc)) = chars.next() else { break };
                    match esc {
                        '"' | '\\' | '/' => out.push(esc),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let unit = hex4(&mut chars)
                                .ok_or(SyntaxError { at: here, what: "bad \\u escape" })?;
                            out.push(code_point(unit, &mut chars));
                        }
                        _ => return Err(SyntaxError { at: here, what: "bad escape" }),
                    }
                }
                c if (c as u32) < 0x20 => {
                    return Err(SyntaxError { at: here, what: "control character in string" });
                }
                c => out.push(c),
            }
        }
        Err(SyntaxError { at: start, what: "unterminated string" })
    }

    fn value_end(&self, at: usize, depth: usize) -> Scan<usize> {
        if depth > MAX_DEPTH {
            return Err(SyntaxError { at, what: "nested too deep" });
        }
        match self.byte(at) {
            Some(b'{') => Ok(self.object(at, depth)?.close + 1),
            Some(b'[') => Ok(self.array(at, depth)?.close + 1),
            Some(b'"') => Ok(self.string(at)?.1),
            Some(b't') => self.literal(at, "true"),
            Some(b'f') => self.literal(at, "false"),
            Some(b'n') => self.literal(at, "null"),
            Some(b'-' | b'0'..=b'9') => self.number(at),
            _ => Err(SyntaxError { at, what: "expected a value" }),
        }
    }

    fn literal(&self, at: usize, word: &str) -> Scan<usize> {
        if self.text[at..].starts_with(word) {
            Ok(at + word.len())
        } else {
            Err(SyntaxError { at, what: "expected a value" })
        }
    }

    fn number(&self, at: usize) -> Scan<usize> {
        let mut end = at;
        while matches!(self.byte(end), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
            end += 1;
        }
        if self.text[at..end].parse::<f64>().is_ok() {
            Ok(end)
        } else {
            Err(SyntaxError { at, what: "bad number" })
        }
    }

    fn object(&self, open: usize, depth: usize) -> Scan<Object> {
        let mut members = Vec::new();
        let mut at = self.trivia(open + 1)?;
        if self.byte(at) == Some(b'}') {
            return Ok(Object { open, close: at, members });
        }
        loop {
            if self.byte(at) != Some(b'"') {
                return Err(SyntaxError { at, what: "expected a key" });
            }
            let (key, after_key) = self.string(at)?;
            let key_start = at;
            at = self.trivia(after_key)?;
            if self.byte(at) != Some(b':') {
                return Err(SyntaxError { at, what: "expected a colon" });
            }
            let start = self.trivia(at + 1)?;
            let end = self.value_end(start, depth + 1)?;
            members.push(Member { key, key_start, value: Span { start, end } });
            at = self.trivia(end)?;
            match self.byte(at) {
                Some(b',') => at = self.trivia(at + 1)?,
                Some(b'}') => return Ok(Object { open, close: at, members }),
                _ => return Err(SyntaxError { at, what: "expected a comma or a closing brace" }),
            }
        }
    }

    fn array(&self, open: usize, depth: usize) -> Scan<Array> {
        let mut elements = Vec::new();
        let mut at = self.trivia(open + 1)?;
        if self.byte(at) == Some(b']') {
            return Ok(Array { open, close: at, elements });
        }
        loop {
            let end = self.value_end(at, depth + 1)?;
            elements.push(Span { start: at, end });
            at = self.trivia(end)?;
            match self.byte(at) {
                Some(b',') => at = self.trivia(at + 1)?,
                Some(b']') => return Ok(Array { open, close: at, elements }),
                _ => return Err(SyntaxError { at, what: "expected a comma or a closing bracket" }),
            }
        }
    }
}

fn code_point(unit: u32, chars: &mut std::str::CharIndices<'_>) -> char {
    if !(0xD800..0xDC00).contains(&unit) {
        return char::from_u32(unit).unwrap_or('\u{FFFD}');
    }
    let mut ahead = chars.clone();
    if ahead.next().map(|(_, c)| c) == Some('\\')
        && ahead.next().map(|(_, c)| c) == Some('u')
        && let Some(low) = hex4(&mut ahead)
        && (0xDC00..0xE000).contains(&low)
    {
        *chars = ahead;
        let joined = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
        return char::from_u32(joined).unwrap_or('\u{FFFD}');
    }
    '\u{FFFD}'
}

fn hex4(chars: &mut std::str::CharIndices<'_>) -> Option<u32> {
    let mut unit = 0;
    for _ in 0..4 {
        unit = unit * 16 + chars.next()?.1.to_digit(16)?;
    }
    Some(unit)
}

/// The top-level object, `None` for a file with nothing but whitespace and comments.
pub(crate) fn document(text: &str) -> Scan<Option<Object>> {
    let scanner = Scanner { text };
    let start =
        scanner.trivia(usize::from(text.starts_with('\u{feff}')) * '\u{feff}'.len_utf8())?;
    if start == text.len() {
        return Ok(None);
    }
    if scanner.byte(start) != Some(b'{') {
        return Err(SyntaxError { at: start, what: "expected an object" });
    }
    let object = scanner.object(start, 0)?;
    let rest = scanner.trivia(object.close + 1)?;
    if rest != text.len() {
        return Err(SyntaxError { at: rest, what: "text after the document" });
    }
    Ok(Some(object))
}

pub(crate) fn object_at(text: &str, span: Span) -> Option<Object> {
    (Scanner { text }.byte(span.start) == Some(b'{'))
        .then(|| Scanner { text }.object(span.start, 0).ok())
        .flatten()
}

pub(crate) fn array_at(text: &str, span: Span) -> Option<Array> {
    (Scanner { text }.byte(span.start) == Some(b'['))
        .then(|| Scanner { text }.array(span.start, 0).ok())
        .flatten()
}

pub(crate) fn string_at(text: &str, span: Span) -> Option<String> {
    (Scanner { text }.byte(span.start) == Some(b'"'))
        .then(|| Scanner { text }.string(span.start).ok().map(|(s, _)| s))
        .flatten()
}

pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn line_indent(text: &str, at: usize) -> &str {
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let line = &text[line_start..];
    let width = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..width]
}

/// How the items of one container are laid out: one per line at an indent, or on one line.
enum Layout {
    Lines(String),
    Compact,
}

fn layout(text: &str, open: usize, close: usize, first_item: Option<usize>) -> Layout {
    match first_item {
        Some(first) => {
            let line_start = text[..first].rfind('\n').map_or(0, |n| n + 1);
            let before = &text[line_start..first];
            if line_start > open && before.chars().all(|c| c == ' ' || c == '\t') {
                Layout::Lines(before.to_owned())
            } else {
                Layout::Compact
            }
        }
        None if text[open + 1..close].contains('\n') => {
            Layout::Lines(format!("{}{UNIT}", line_indent(text, open)))
        }
        None => Layout::Compact,
    }
}

fn render(value: &Value, indent: &str, lines: bool) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => escape(s),
        Value::Object(members) if members.is_empty() => "{}".to_owned(),
        Value::Array(items) if items.is_empty() => "[]".to_owned(),
        Value::Object(members) => {
            let inner = format!("{indent}{UNIT}");
            let rows: Vec<String> = members
                .iter()
                .map(|(k, v)| format!("{}: {}", escape(k), render(v, &inner, lines)))
                .collect();
            wrap(&rows, ('{', '}'), indent, lines)
        }
        Value::Array(items) => {
            let inner = format!("{indent}{UNIT}");
            let rows: Vec<String> = items.iter().map(|v| render(v, &inner, lines)).collect();
            wrap(&rows, ('[', ']'), indent, lines)
        }
    }
}

fn wrap(rows: &[String], (open, close): (char, char), indent: &str, lines: bool) -> String {
    if lines {
        let inner = format!("{indent}{UNIT}");
        let body: Vec<String> = rows.iter().map(|r| format!("{inner}{r}")).collect();
        format!("{open}\n{}\n{indent}{close}", body.join(",\n"))
    } else {
        format!("{open}{}{close}", rows.join(", "))
    }
}

/// Inserts one item (`"key": value` or an array element) after the last existing one, or into
/// the empty container.
fn insert_item(
    text: &str,
    (open, close): (usize, usize),
    first_item: Option<usize>,
    last_end: Option<usize>,
    build: impl Fn(&str, bool) -> String,
) -> String {
    let (indent, lines) = match layout(text, open, close, first_item) {
        Layout::Lines(indent) => (indent, true),
        Layout::Compact => (line_indent(text, open).to_owned(), false),
    };
    let item = build(&indent, lines);
    match (last_end, lines) {
        (Some(end), true) => format!("{},\n{indent}{item}{}", &text[..end], &text[end..]),
        (Some(end), false) => format!("{}, {item}{}", &text[..end], &text[end..]),
        (None, true) => {
            let outer = line_indent(text, open);
            format!("{}\n{indent}{item}\n{outer}{}", &text[..=open], &text[close..])
        }
        (None, false) => format!("{}{item}{}", &text[..=open], &text[close..]),
    }
}

pub(crate) fn insert_member(text: &str, object: &Object, key: &str, value: &Value) -> String {
    let first = object.members.first().map(|m| m.key_start);
    let last = object.members.last().map(|m| m.value.end);
    insert_item(text, (object.open, object.close), first, last, |indent, lines| {
        format!("{}: {}", escape(key), render(value, indent, lines))
    })
}

pub(crate) fn push_element(text: &str, array: &Array, value: &Value) -> String {
    let first = array.elements.first().map(|s| s.start);
    let last = array.elements.last().map(|s| s.end);
    insert_item(text, (array.open, array.close), first, last, |indent, lines| {
        render(value, indent, lines)
    })
}

/// A new document holding one top-level member, laid out like `JSON.stringify(_, null, 2)`.
pub(crate) fn fresh_document(key: &str, value: &Value) -> String {
    format!("{{\n{UNIT}{}: {}\n}}\n", escape(key), render(value, UNIT, true))
}

pub(crate) fn replace(text: &str, span: Span, with: &str) -> String {
    format!("{}{with}{}", &text[..span.start], &text[span.end..])
}

/// Removes item `index` of a container whose items start at `starts` and end at `ends`, with the
/// comma that joined it to its neighbour. Comments between the two neighbours go with it.
fn remove_item(
    text: &str,
    (open, close): (usize, usize),
    items: &[(usize, usize)],
    index: usize,
) -> String {
    let (start, end) = items[index];
    if let Some(&(next_start, _)) = items.get(index + 1) {
        format!("{}{}", &text[..start], &text[next_start..])
    } else if index > 0 {
        format!("{}{}", &text[..items[index - 1].1], &text[end..])
    } else {
        format!("{}{}", &text[..=open], &text[close..])
    }
}

pub(crate) fn remove_member(text: &str, object: &Object, index: usize) -> String {
    let items: Vec<(usize, usize)> =
        object.members.iter().map(|m| (m.key_start, m.value.end)).collect();
    remove_item(text, (object.open, object.close), &items, index)
}

pub(crate) fn remove_element(text: &str, array: &Array, index: usize) -> String {
    let items: Vec<(usize, usize)> = array.elements.iter().map(|s| (s.start, s.end)).collect();
    remove_item(text, (array.open, array.close), &items, index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str) -> Vec<String> {
        document(text).unwrap().unwrap().members.into_iter().map(|m| m.key).collect()
    }

    #[test]
    fn scans_comments_escapes_and_nesting() {
        let text = "// head\n{\n  /* c */ \"a\\u00e9\\\"\": [1, 2.5e3, {\"x\": null}], \"b\": true // tail\n}\n";
        assert_eq!(keys(text), ["a\u{e9}\"", "b"]);
    }

    #[test]
    fn nothing_but_trivia_is_no_document() {
        assert!(document("").unwrap().is_none());
        assert!(document("  // only a comment\n").unwrap().is_none());
    }

    #[test]
    fn refuses_what_is_not_strict_json_with_comments() {
        for bad in [
            "{",
            "{\"a\": 1,}",
            "{\"a\": }",
            "[1]",
            "{\"a\": 1} {}",
            "{a: 1}",
            "{\"a\": 1} x",
            "{\"a\": 'x'}",
            "{\"a\": \"line\nbreak\"}",
            "{\"a\": 1 /* open }",
            "{\"a\": 01x}",
        ] {
            assert!(document(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn nesting_past_the_limit_is_refused_not_a_stack_overflow() {
        let deep = format!("{{\"a\": {}{}}}", "[".repeat(200), "]".repeat(200));
        assert_eq!(document(&deep).unwrap_err().what, "nested too deep");
    }

    #[test]
    fn surrogate_pairs_decode() {
        let text = "{\"k\": \"\\ud83d\\ude00\"}";
        let doc = document(text).unwrap().unwrap();
        assert_eq!(string_at(text, doc.members[0].value).as_deref(), Some("\u{1F600}"));
    }

    #[test]
    fn inserts_into_a_multi_line_object_at_the_members_indent() {
        let text = "{\n  \"a\": 1\n}\n";
        let doc = document(text).unwrap().unwrap();
        let out = insert_member(text, &doc, "b", &Value::Bool(true));
        assert_eq!(out, "{\n  \"a\": 1,\n  \"b\": true\n}\n");
    }

    #[test]
    fn inserts_into_empty_and_compact_objects() {
        for (text, want) in [
            ("{}", "{\"k\": true}"),
            ("{\"a\": 1}", "{\"a\": 1, \"k\": true}"),
            ("{\n}", "{\n  \"k\": true\n}"),
        ] {
            let doc = document(text).unwrap().unwrap();
            assert_eq!(insert_member(text, &doc, "k", &Value::Bool(true)), want);
        }
    }

    #[test]
    fn nested_values_follow_the_container_layout() {
        let text = "{\n  \"projects\": {\n    \"/a\": {\n      \"x\": 1\n    }\n  }\n}\n";
        let doc = document(text).unwrap().unwrap();
        let projects = object_at(text, doc.get("projects").unwrap().value).unwrap();
        let entry = Value::Object(vec![("t".into(), Value::Bool(true))]);
        let out = insert_member(text, &projects, "/b", &entry);
        assert_eq!(
            out,
            "{\n  \"projects\": {\n    \"/a\": {\n      \"x\": 1\n    },\n    \"/b\": {\n      \"t\": true\n    }\n  }\n}\n"
        );
        assert!(document(&out).is_ok());
    }

    #[test]
    fn appends_to_arrays_in_each_layout() {
        for (text, want) in [
            ("{\"l\": []}", "{\"l\": [\"p\"]}"),
            ("{\"l\": [\"a\"]}", "{\"l\": [\"a\", \"p\"]}"),
            ("{\n  \"l\": [\n    \"a\"\n  ]\n}", "{\n  \"l\": [\n    \"a\",\n    \"p\"\n  ]\n}"),
            ("{\n  \"l\": [\n  ]\n}", "{\n  \"l\": [\n    \"p\"\n  ]\n}"),
        ] {
            let doc = document(text).unwrap().unwrap();
            let array = array_at(text, doc.get("l").unwrap().value).unwrap();
            assert_eq!(push_element(text, &array, &Value::Str("p".into())), want);
        }
    }

    #[test]
    fn removes_first_middle_last_and_only_items() {
        let text = "{\n  \"a\": 1,\n  \"b\": 2,\n  \"c\": 3\n}";
        let doc = document(text).unwrap().unwrap();
        assert_eq!(remove_member(text, &doc, 0), "{\n  \"b\": 2,\n  \"c\": 3\n}");
        assert_eq!(remove_member(text, &doc, 1), "{\n  \"a\": 1,\n  \"c\": 3\n}");
        assert_eq!(remove_member(text, &doc, 2), "{\n  \"a\": 1,\n  \"b\": 2\n}");
        let one = "{\n  \"a\": 1\n}";
        let doc = document(one).unwrap().unwrap();
        assert_eq!(remove_member(one, &doc, 0), "{}");
    }

    #[test]
    fn escapes_what_json_requires_and_leaves_the_rest() {
        assert_eq!(escape("a\"b\\c\n\u{1}\u{e9}"), "\"a\\\"b\\\\c\\n\\u0001\u{e9}\"");
    }

    #[test]
    fn a_byte_order_mark_is_allowed() {
        assert!(document("\u{feff}{\"a\": 1}").is_ok());
    }
}
