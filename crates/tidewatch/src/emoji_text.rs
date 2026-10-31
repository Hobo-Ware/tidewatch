//! Finds the emoji in a text and reserves a transparent slot for each one in its layout, so
//! `emoji_paint` can draw the colour picture over it while copy, select and send keep the character.

use std::ops::Range;

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{Color32, Context, FontId};

use crate::emoji_pack;

pub(crate) const SLOT_EM: f32 = 1.25;
const TAIL_PX: f32 = 0.25;
const ZWJ: char = '\u{200d}';
const VS16: char = '\u{fe0f}';
const KEYCAP: char = '\u{20e3}';

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cluster {
    pub(crate) bytes: Range<usize>,
    pub(crate) key: String,
}

fn is_regional(c: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&c)
}

fn is_joiner(c: char) -> bool {
    c == VS16
        || c == KEYCAP
        || ('\u{1f3fb}'..='\u{1f3ff}').contains(&c)
        || ('\u{e0020}'..='\u{e007f}').contains(&c)
}

pub(crate) fn key_of(cluster: &str) -> String {
    let parts: Vec<String> =
        cluster.chars().filter(|c| *c != VS16).map(|c| format!("{:x}", u32::from(c))).collect();
    parts.join("_")
}

fn may_start(text: &str, at: usize, c: char) -> bool {
    if c.is_ascii() {
        let mut after = text[at + 1..].chars();
        return matches!(c, '#' | '*' | '0'..='9')
            && after.next().is_some_and(|next| next == VS16 || next == KEYCAP);
    }
    c >= '\u{a9}' && c != ZWJ && !is_joiner(c)
}

fn extent(text: &str, at: usize) -> usize {
    let mut chars = text[at..].char_indices();
    let Some((_, first)) = chars.next() else { return at };
    let mut end = at + first.len_utf8();
    let mut open_flag = is_regional(first);
    let mut after_zwj = false;
    for (offset, c) in chars {
        let take = after_zwj || is_joiner(c) || c == ZWJ || (open_flag && is_regional(c));
        if !take {
            break;
        }
        if is_regional(c) {
            open_flag = false;
        }
        after_zwj = c == ZWJ;
        end = at + offset + c.len_utf8();
    }
    end
}

fn accepted(candidate: &str, has: &dyn Fn(&str) -> bool) -> Option<(usize, String)> {
    let mut ends: Vec<usize> = candidate.char_indices().map(|(at, _)| at).skip(1).collect();
    ends.push(candidate.len());
    ends.into_iter().rev().find_map(|end| {
        let piece = &candidate[..end];
        let key = key_of(piece);
        (emojis::get(piece).is_some_and(|emoji| emoji.as_str() == piece) && has(&key))
            .then_some((end, key))
    })
}

/// The emoji of `text` that `has` a picture for. A character that is text without its variation
/// selector stays text.
pub(crate) fn find(text: &str, has: &dyn Fn(&str) -> bool) -> Vec<Cluster> {
    let mut found = Vec::new();
    if text.is_ascii() {
        return found;
    }
    let mut at = 0;
    while let Some(c) = text[at..].chars().next() {
        let step = c.len_utf8();
        if !may_start(text, at, c) {
            at += step;
            continue;
        }
        match accepted(&text[at..extent(text, at)], has) {
            Some((len, key)) => {
                found.push(Cluster { bytes: at..at + len, key });
                at += len;
            }
            None => at += step,
        }
    }
    found
}

pub(crate) fn clusters(text: &str) -> Vec<Cluster> {
    find(text, &|key| emoji_pack::still().contains(key))
}

fn slot_format(plain: &TextFormat, line: f32, size: f32) -> TextFormat {
    TextFormat {
        font_id: FontId::new(size, plain.font_id.family.clone()),
        color: Color32::TRANSPARENT,
        line_height: Some(line),
        underline: egui::Stroke::NONE,
        strikethrough: egui::Stroke::NONE,
        ..plain.clone()
    }
}

/// `job` with a transparent slot per emoji: the first code point at `SLOT_EM` times the size, the
/// rest at a hair's width, all on one explicit line height so rows keep their height.
pub(crate) fn slot_job(ctx: &Context, job: LayoutJob) -> LayoutJob {
    let job = crate::token_view::slot_job(ctx, job);
    let found = clusters(&job.text);
    if found.is_empty() {
        return job;
    }
    let mut out = LayoutJob { text: String::new(), sections: Vec::new(), ..job.clone() };
    for section in &job.sections {
        let range = usize::from(section.byte_range.start)..usize::from(section.byte_range.end);
        let line = section
            .format
            .line_height
            .unwrap_or_else(|| ctx.fonts_mut(|fonts| fonts.row_height(&section.format.font_id)));
        let plain = TextFormat { line_height: Some(line), ..section.format.clone() };
        let mut lead = section.leading_space;
        let mut put = |out: &mut LayoutJob, text: &str, format: &TextFormat| {
            if !text.is_empty() {
                out.append(text, lead, format.clone());
                lead = 0.0;
            }
        };
        let mut from = range.start;
        for cluster in
            found.iter().filter(|c| c.bytes.start >= range.start && c.bytes.end <= range.end)
        {
            put(&mut out, &job.text[from..cluster.bytes.start], &plain);
            let head_len = job.text[cluster.bytes.clone()].chars().next().map_or(0, char::len_utf8);
            let head_end = cluster.bytes.start + head_len;
            let size = plain.font_id.size * SLOT_EM;
            put(
                &mut out,
                &job.text[cluster.bytes.start..head_end],
                &slot_format(&plain, line, size),
            );
            put(
                &mut out,
                &job.text[head_end..cluster.bytes.end],
                &slot_format(&plain, line, TAIL_PX),
            );
            from = cluster.bytes.end;
        }
        put(&mut out, &job.text[from..range.end], &plain);
    }
    out
}

#[cfg(test)]
mod tests;
