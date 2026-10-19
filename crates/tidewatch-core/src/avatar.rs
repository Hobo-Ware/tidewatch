//! The avatar an agent draws for its task: an SVG it sends, cut down to a short list of shapes and
//! drawing attributes. Anything else is dropped, and an SVG that does not parse is refused.

use std::fmt::Write;

use roxmltree::{Document, Node, ParsingOptions};
use serde::{Deserialize, Serialize};

pub const MAX_SVG_BYTES: usize = 8 * 1024;
pub const MAX_REASON_CHARS: usize = 160;
const MAX_ELEMENTS: usize = 150;
const MAX_DEPTH: usize = 8;
const MAX_VALUE_CHARS: usize = 4_000;
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const DEFAULT_VIEW_BOX: &str = "0 0 64 64";

const CONTAINERS: [&str; 2] = ["svg", "g"];
const SHAPES: [&str; 7] = ["rect", "circle", "ellipse", "line", "polyline", "polygon", "path"];
const COMMON: [&str; 8] = [
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "opacity",
    "fill-opacity",
    "stroke-opacity",
];
const COLOR_NAMES: [&str; 22] = [
    "none", "black", "white", "gray", "grey", "silver", "red", "green", "blue", "yellow", "orange",
    "purple", "pink", "teal", "cyan", "magenta", "brown", "navy", "maroon", "olive", "lime",
    "gold",
];
const TRANSFORMS: [&str; 6] = ["translate", "scale", "rotate", "skewX", "skewY", "matrix"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Avatar {
    pub svg: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AvatarError {
    TooLarge(usize),
    Unreadable(String),
    NotSvg,
    NothingDrawn,
    NoReason,
}

impl std::fmt::Display for AvatarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(bytes) => {
                write!(f, "the svg is {bytes} bytes, over the {} KB limit", MAX_SVG_BYTES / 1024)
            }
            Self::Unreadable(why) => write!(f, "the svg does not parse ({why})"),
            Self::NotSvg => write!(f, "the root element must be <svg>"),
            Self::NothingDrawn => write!(
                f,
                "the svg has no allowed shape (use rect, circle, ellipse, line, polyline, polygon or path)"
            ),
            Self::NoReason => write!(f, "reason must say in one line why this icon fits the task"),
        }
    }
}

/// The avatar for a call: the svg cleaned and the reason on one line, cut to length.
pub fn clean(svg: &str, reason: &str) -> Result<Avatar, AvatarError> {
    let reason: String = reason
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_REASON_CHARS)
        .collect::<String>()
        .trim_end()
        .to_owned();
    let svg = sanitize(svg)?;
    if reason.is_empty() {
        return Err(AvatarError::NoReason);
    }
    Ok(Avatar { svg, reason })
}

/// A clean SVG with only the allowed elements and attributes, always with the SVG namespace and a
/// `viewBox`.
pub fn sanitize(input: &str) -> Result<String, AvatarError> {
    if input.len() > MAX_SVG_BYTES {
        return Err(AvatarError::TooLarge(input.len()));
    }
    let options = ParsingOptions { allow_dtd: false, ..ParsingOptions::default() };
    let doc = Document::parse_with_options(input, options)
        .map_err(|e| AvatarError::Unreadable(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" || !in_svg_namespace(root) {
        return Err(AvatarError::NotSvg);
    }
    let view_box = root
        .attribute("viewBox")
        .and_then(|value| valid_view_box(value).then(|| value.trim().to_owned()))
        .unwrap_or_else(|| DEFAULT_VIEW_BOX.to_owned());
    let mut out = String::new();
    let mut state = Walk { elements: 0, drawn: 0 };
    let _ = write!(out, "<svg xmlns=\"{SVG_NS}\" viewBox=\"{}\"", escape(&view_box));
    push_attrs(&mut out, root, "svg");
    out.push('>');
    walk_children(root, 1, &mut state, &mut out);
    out.push_str("</svg>");
    if state.drawn == 0 {
        return Err(AvatarError::NothingDrawn);
    }
    Ok(out)
}

struct Walk {
    elements: usize,
    drawn: usize,
}

fn in_svg_namespace(node: Node) -> bool {
    node.tag_name().namespace().is_none_or(|ns| ns == SVG_NS)
}

fn walk_children(parent: Node, depth: usize, state: &mut Walk, out: &mut String) {
    for node in parent.children().filter(Node::is_element) {
        let name = node.tag_name().name();
        let container = CONTAINERS.contains(&name);
        let shape = SHAPES.contains(&name);
        if !(container || shape) || !in_svg_namespace(node) {
            continue;
        }
        if depth > MAX_DEPTH || state.elements >= MAX_ELEMENTS {
            return;
        }
        state.elements += 1;
        out.push('<');
        out.push_str(name);
        push_attrs(out, node, name);
        out.push('>');
        if shape {
            state.drawn += 1;
        } else {
            walk_children(node, depth + 1, state, out);
        }
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }
}

fn geometry(name: &str) -> &'static [&'static str] {
    match name {
        "rect" => &["x", "y", "width", "height", "rx", "ry"],
        "circle" => &["cx", "cy", "r"],
        "ellipse" => &["cx", "cy", "rx", "ry"],
        "line" => &["x1", "y1", "x2", "y2"],
        "polyline" | "polygon" => &["points"],
        "path" => &["d"],
        _ => &[],
    }
}

fn push_attrs(out: &mut String, node: Node, name: &str) {
    let allowed = geometry(name).iter().chain(COMMON.iter()).chain(["transform"].iter());
    for key in allowed {
        let Some(value) = node.attributes().find(|a| a.namespace().is_none() && a.name() == *key)
        else {
            continue;
        };
        let value = value.value().trim();
        if value.len() <= MAX_VALUE_CHARS && valid(key, value) {
            let _ = write!(out, " {key}=\"{}\"", escape(value));
        }
    }
}

fn valid(key: &str, value: &str) -> bool {
    match key {
        "fill" | "stroke" => valid_color(value),
        "stroke-linecap" => matches!(value, "butt" | "round" | "square"),
        "stroke-linejoin" => matches!(value, "miter" | "round" | "bevel"),
        "opacity" | "fill-opacity" | "stroke-opacity" => {
            number(value).is_some_and(|n| (0.0..=1.0).contains(&n))
        }
        "stroke-width" => number(value).is_some_and(|n| n >= 0.0),
        "points" => number_list(value),
        "d" => valid_path(value),
        "transform" => valid_transform(value),
        _ => number(value).is_some(),
    }
}

fn number(text: &str) -> Option<f64> {
    let ok = !text.is_empty()
        && text.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'));
    ok.then(|| text.parse::<f64>().ok()).flatten().filter(|n| n.is_finite())
}

fn number_list(text: &str) -> bool {
    let mut any = false;
    for part in text.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()) {
        if number(part).is_none() {
            return false;
        }
        any = true;
    }
    any
}

fn valid_view_box(text: &str) -> bool {
    let parts: Vec<_> =
        text.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    parts.len() == 4
        && parts.iter().all(|p| number(p).is_some())
        && parts[2..].iter().all(|p| number(p).is_some_and(|n| n > 0.0))
}

fn valid_color(text: &str) -> bool {
    if let Some(hex) = text.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(args) = text.strip_prefix("rgb(").or_else(|| text.strip_prefix("rgba(")) {
        return args.strip_suffix(')').is_some_and(|inner| {
            inner.chars().all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '%' | ' '))
                && inner.chars().any(|c| c.is_ascii_digit())
        });
    }
    COLOR_NAMES.contains(&text.to_ascii_lowercase().as_str())
}

fn valid_path(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| {
            c.is_ascii_digit() || c.is_whitespace() || "MmLlHhVvCcSsQqTtAaZz.,+-eE".contains(c)
        })
}

fn valid_transform(text: &str) -> bool {
    let mut rest = text;
    let mut any = false;
    while !rest.trim().is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        let Some(open) = rest.find('(') else { return false };
        let Some(close) = rest.find(')') else { return false };
        if close < open
            || !TRANSFORMS.contains(&rest[..open].trim())
            || !number_list(&rest[open + 1..close])
        {
            return false;
        }
        rest = &rest[close + 1..];
        any = true;
    }
    any
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests;
