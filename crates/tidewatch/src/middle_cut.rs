//! Cuts a path in the middle so its file name and `:line` stay whole, or at the start so its last
//! folders stay, measured with whatever measure the caller owns (pure: a closure from text to
//! width).

const ELLIPSIS: &str = "…";

/// `text` itself when it fits `max_w`, else its leading and trailing folders around an ellipsis
/// (`projects/client/…/recap/File.svelte:58`). The last segment is never cut while `…/` plus it
/// fits; past that its own middle goes, keeping the start and the `:line` end.
pub(crate) fn middle_cut(text: &str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(text) <= max_w {
        return text.to_owned();
    }
    let parts: Vec<&str> = text.split('/').collect();
    if parts.len() > 1
        && let Some(cut) = cut_folders(&parts, max_w, measure)
    {
        return cut;
    }
    cut_chars(parts.last().copied().unwrap_or(text), max_w, measure)
}

/// `text` itself when it fits `max_w`, else its last folders after `…/` (`…/Hoboware/kelp`): whole
/// folders while one fits, else the end of the last name behind an ellipsis.
pub(crate) fn start_cut(text: &str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(text) <= max_w {
        return text.to_owned();
    }
    let parts: Vec<&str> = text.split('/').collect();
    for first in 1..parts.len() {
        let cut = format!("{ELLIPSIS}/{}", parts[first..].join("/"));
        if measure(&cut) <= max_w {
            return cut;
        }
    }
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len())
        .map(|skip| format!("{ELLIPSIS}{}", chars[skip..].iter().collect::<String>()))
        .find(|cut| measure(cut) <= max_w)
        .unwrap_or_else(|| ELLIPSIS.to_owned())
}

/// Whether `cut` still ends in the whole last segment of `original` (its file name and line).
pub(crate) fn keeps_name(cut: &str, original: &str) -> bool {
    original.rsplit('/').next().is_some_and(|name| cut.ends_with(name))
}

fn cut_folders(parts: &[&str], max_w: f32, measure: &dyn Fn(&str) -> f32) -> Option<String> {
    let build = |head: usize, tail: usize| {
        let mut pieces: Vec<&str> = parts[..head].to_vec();
        pieces.push(ELLIPSIS);
        pieces.extend_from_slice(&parts[parts.len() - tail..]);
        pieces.join("/")
    };
    let fits = |head: usize, tail: usize| {
        head + tail < parts.len() && measure(&build(head, tail)) <= max_w
    };
    if !fits(0, 1) {
        return None;
    }
    let (mut head, mut tail) = (usize::from(parts[0].is_empty() && fits(1, 1)), 1);
    loop {
        let mut grew = false;
        if fits(head + 1, tail) {
            head += 1;
            grew = true;
        }
        if fits(head, tail + 1) {
            tail += 1;
            grew = true;
        }
        if !grew {
            return Some(build(head, tail));
        }
    }
}

fn cut_chars(text: &str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let suffix = line_suffix_len(&chars);
    let build = |head: usize, tail: usize| {
        let start: String = chars[..head].iter().collect();
        let end: String = chars[chars.len() - tail..].iter().collect();
        format!("{start}{ELLIPSIS}{end}")
    };
    let fits = |head: usize, tail: usize| {
        head + tail < chars.len() && measure(&build(head, tail)) <= max_w
    };
    let (mut head, mut tail) = (1, suffix);
    if !fits(head, tail) {
        return if fits(0, suffix) { build(0, suffix) } else { ELLIPSIS.to_owned() };
    }
    loop {
        let mut grew = false;
        if fits(head + 1, tail) {
            head += 1;
            grew = true;
        }
        if fits(head, tail + 1) {
            tail += 1;
            grew = true;
        }
        if !grew {
            return build(head, tail);
        }
    }
}

fn line_suffix_len(chars: &[char]) -> usize {
    let digits = chars.iter().rev().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && chars.len() > digits && chars[chars.len() - digits - 1] == ':' {
        digits + 1
    } else {
        0
    }
}

#[cfg(test)]
mod tests;
