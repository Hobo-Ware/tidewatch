//! Live markdown in the message box: `pulldown-cmark` finds each construct's byte range and the layout
//! styles it as it is typed. A mark (`**`, `#`, `>`, a fence, `[ ]`) is laid out at a hair's size and
//! transparent off the cursor's lines, so the text itself never changes. One parse per text change.

use std::cell::RefCell;
use std::ops::{Range, RangeInclusive};
use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Galley, Pos2, Stroke, Ui};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::brand;
use crate::md_view::{CODE_EM, HEADING_EM, QUOTE_BAR, QUOTE_INDENT};
use crate::screen_kit::{body, heading, mono};

const HIDDEN_PX: f32 = 0.25;
const FENCE_PAD: f32 = 6.0;
const CACHED: usize = 6;
const CODE_PAD_X: f32 = 6.0;
const BAR_INSET: f32 = 2.0;
const SCHEME: [&str; 2] = ["https://", "http://"];
const URL_TAIL: &str = ".,;:!?)]}'\"";

/// A run of the text, as byte positions of the real text, drawn in another colour.
pub(crate) type Tint = Option<(Range<usize>, Color32)>;

/// The lines whose marks show: the ones the cursor or the selection touches. `None` shows none.
pub(crate) type Shown = Option<RangeInclusive<usize>>;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Flags(u16);

impl Flags {
    pub(crate) const BOLD: Self = Self(1);
    pub(crate) const ITALIC: Self = Self(1 << 1);
    pub(crate) const STRIKE: Self = Self(1 << 2);
    pub(crate) const CODE: Self = Self(1 << 3);
    pub(crate) const LINK: Self = Self(1 << 4);
    pub(crate) const MARK: Self = Self(1 << 5);
    pub(crate) const QUOTE: Self = Self(1 << 6);
    pub(crate) const FENCED: Self = Self(1 << 7);
    pub(crate) const DIM: Self = Self(1 << 8);

    pub(crate) const fn has(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Cell {
    flags: Flags,
    heading: u8,
}

/// A stretch of one style, as bytes of the text. Never holds a newline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) range: Range<usize>,
    pub(crate) flags: Flags,
    pub(crate) heading: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Block {
    #[default]
    Plain,
    /// A line of a block quote: how many `>` it opens with and the bytes of that prefix.
    Quote { depth: usize, prefix: usize },
    /// A line of a fenced code block; `fence` for its opening and closing line.
    Code { fence: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Task {
    pub(crate) done: bool,
    /// The bytes of `[ ] ` or `[x] `, with the space after it.
    pub(crate) marker: Range<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) block: Block,
    pub(crate) task: Option<Task>,
}

#[derive(Debug, Default)]
pub(crate) struct Parsed {
    runs: Vec<Run>,
    starts: Vec<usize>,
    lines: Vec<Line>,
}

impl Parsed {
    #[cfg(test)]
    pub(crate) fn runs(&self) -> &[Run] {
        &self.runs
    }

    pub(crate) fn line(&self, index: usize) -> Option<&Line> {
        self.lines.get(index)
    }

    pub(crate) fn line_start(&self, line: usize) -> usize {
        self.starts.get(line).copied().unwrap_or_default()
    }

    pub(crate) fn line_of(&self, byte: usize) -> usize {
        self.starts.partition_point(|start| *start <= byte).saturating_sub(1)
    }

    pub(crate) fn has_quotes(&self) -> bool {
        self.lines.iter().any(|line| matches!(line.block, Block::Quote { .. }))
    }

    fn runs_in(&self, range: &Range<usize>) -> &[Run] {
        let from = self.runs.partition_point(|run| run.range.end <= range.start);
        let to = self.runs.partition_point(|run| run.range.start < range.end);
        &self.runs[from..to.max(from)]
    }
}

thread_local! {
    static CACHE: RefCell<Vec<(String, Arc<Parsed>)>> = const { RefCell::new(Vec::new()) };
}

/// The styling of `text`, parsed once per text and kept for the last few.
pub(crate) fn parse(text: &str) -> Arc<Parsed> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(at) = cache.iter().position(|(known, _)| known == text) {
            let hit = cache.remove(at);
            let parsed = Arc::clone(&hit.1);
            cache.insert(0, hit);
            return parsed;
        }
        let parsed = Arc::new(compute(text));
        cache.insert(0, (text.to_owned(), Arc::clone(&parsed)));
        cache.truncate(CACHED);
        parsed
    })
}

/// The lines a cursor at char `primary` and a selection end at char `secondary` touch.
pub(crate) fn shown_for(text: &str, (primary, secondary): (usize, usize)) -> RangeInclusive<usize> {
    let line_at = |chars: usize| text.chars().take(chars).filter(|c| *c == '\n').count();
    let (a, b) = (line_at(primary), line_at(secondary));
    a.min(b)..=a.max(b)
}

pub(crate) fn shows(shown: &Shown, line: usize) -> bool {
    shown.as_ref().is_some_and(|lines| lines.contains(&line))
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1)).collect()
}

fn bounds(text: &str, starts: &[usize], line: usize) -> Range<usize> {
    let start = starts[line];
    let end = starts.get(line + 1).map_or(text.len(), |next| next - 1);
    start..end
}

fn paint(cells: &mut [Cell], range: &Range<usize>, change: impl Fn(&mut Cell)) {
    let end = range.end.min(cells.len());
    if range.start < end {
        cells[range.start..end].iter_mut().for_each(change);
    }
}

fn flag(cells: &mut [Cell], range: &Range<usize>, flags: Flags) {
    paint(cells, range, |cell| cell.flags = cell.flags.with(flags));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edges {
    Marked,
    Open,
}

struct Frame {
    edges: Edges,
    range: Range<usize>,
    first: Option<usize>,
    last: usize,
}

impl Frame {
    fn new(edges: Edges, range: Range<usize>) -> Self {
        Self { edges, range, first: None, last: 0 }
    }
}

fn cover(frames: &mut [Frame], range: &Range<usize>) {
    if let Some(parent) = frames.last_mut() {
        parent.first = Some(parent.first.map_or(range.start, |first| first.min(range.start)));
        parent.last = parent.last.max(range.end);
    }
}

fn mark_edges(cells: &mut [Cell], frame: &Frame) {
    let (head_end, tail_start) = match frame.first {
        Some(first) => (first, frame.last.max(first)),
        None => (frame.range.end, frame.range.end),
    };
    flag(cells, &(frame.range.start..head_end), Flags::MARK);
    flag(cells, &(tail_start..frame.range.end), Flags::MARK);
}

fn fence_of(line: &str) -> Option<(char, usize)> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let mark = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = rest.chars().take_while(|c| *c == mark).count();
    (run >= 3).then_some((mark, run))
}

fn closes(line: &str, (mark, run): (char, usize)) -> bool {
    fence_of(line).is_some_and(|(found, count)| {
        found == mark && count >= run && line.trim_matches([' ', mark]).is_empty()
    })
}

fn fenced_lines(text: &str, starts: &[usize], lines: &mut [Line], range: &Range<usize>) {
    let first = starts.partition_point(|start| *start <= range.start).saturating_sub(1);
    let opener = &text[bounds(text, starts, first)];
    let Some(fence) = fence_of(opener) else { return };
    let last = starts
        .partition_point(|start| *start <= range.end.saturating_sub(1).max(range.start))
        .saturating_sub(1);
    let closed = last > first && closes(&text[bounds(text, starts, last)], fence);
    for (index, line) in lines.iter_mut().enumerate().take(last + 1).skip(first) {
        line.block = Block::Code { fence: index == first || (closed && index == last) };
    }
}

fn quote_prefix(line: &str) -> (usize, usize) {
    let bytes = line.as_bytes();
    let mut at = bytes.iter().take(3).take_while(|b| **b == b' ').count();
    let mut depth = 0;
    while bytes.get(at) == Some(&b'>') {
        depth += 1;
        at += 1;
        if bytes.get(at) == Some(&b' ') {
            at += 1;
        }
        let spaces = bytes[at..].iter().take(3).take_while(|b| **b == b' ').count();
        if bytes.get(at + spaces) == Some(&b'>') {
            at += spaces;
        }
    }
    (depth, at)
}

fn quoted_lines(text: &str, starts: &[usize], lines: &mut [Line], range: &Range<usize>) {
    let at = |byte: usize| starts.partition_point(|start| *start <= byte).saturating_sub(1);
    for index in at(range.start)..=at(range.end.saturating_sub(1).max(range.start)) {
        let (depth, prefix) = quote_prefix(&text[bounds(text, starts, index)]);
        if lines[index].block == Block::Plain && depth > 0 {
            lines[index].block = Block::Quote { depth, prefix };
        }
    }
}

/// The addresses in a stretch of plain text, cut before trailing punctuation.
fn web_addresses(text: &str, range: &Range<usize>) -> Vec<Range<usize>> {
    let stretch = &text[range.clone()];
    stretch
        .match_indices("http")
        .filter(|(at, _)| !stretch[..*at].chars().next_back().is_some_and(char::is_alphanumeric))
        .filter_map(|(at, _)| {
            let scheme = SCHEME.iter().find(|scheme| stretch[at..].starts_with(**scheme))?;
            let word = stretch[at..].split(char::is_whitespace).next().unwrap_or_default();
            let word = word.trim_end_matches(|c: char| URL_TAIL.contains(c));
            (word.len() > scheme.len()).then(|| range.start + at..range.start + at + word.len())
        })
        .collect()
}

fn compute(text: &str) -> Parsed {
    let starts = line_starts(text);
    let mut cells = vec![Cell::default(); text.len()];
    let mut lines = vec![Line::default(); starts.len()];
    let mut frames: Vec<Frame> = Vec::new();
    let mut plain_text: Vec<Range<usize>> = Vec::new();
    let (mut in_table, mut in_code, mut in_link) = (false, false, 0_usize);
    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        if in_table {
            in_table = !matches!(event, Event::End(TagEnd::Table));
            continue;
        }
        match event {
            Event::Start(tag) => {
                cover(&mut frames, &range);
                let edges = match tag {
                    Tag::Table(_) => {
                        in_table = true;
                        continue;
                    }
                    Tag::Heading { .. } if !text[range.start..].starts_with('#') => Edges::Open,
                    Tag::Heading { level, .. } => {
                        paint(&mut cells, &range, |cell| cell.heading = level as u8);
                        Edges::Marked
                    }
                    Tag::Emphasis => {
                        flag(&mut cells, &range, Flags::ITALIC);
                        Edges::Marked
                    }
                    Tag::Strong => {
                        flag(&mut cells, &range, Flags::BOLD);
                        Edges::Marked
                    }
                    Tag::Strikethrough => {
                        flag(&mut cells, &range, Flags::STRIKE);
                        Edges::Marked
                    }
                    Tag::Link { .. } | Tag::Image { .. } => {
                        flag(&mut cells, &range, Flags::LINK);
                        in_link += 1;
                        Edges::Marked
                    }
                    Tag::CodeBlock(kind) => {
                        in_code = true;
                        if matches!(kind, CodeBlockKind::Fenced(_)) {
                            fenced_lines(text, &starts, &mut lines, &range);
                        }
                        Edges::Open
                    }
                    Tag::BlockQuote(_) => {
                        quoted_lines(text, &starts, &mut lines, &range);
                        Edges::Open
                    }
                    _ => Edges::Open,
                };
                frames.push(Frame::new(edges, range));
            }
            Event::End(end) => {
                match end {
                    TagEnd::CodeBlock => in_code = false,
                    TagEnd::Link | TagEnd::Image => in_link = in_link.saturating_sub(1),
                    _ => {}
                }
                if let Some(frame) = frames.pop().filter(|frame| frame.edges == Edges::Marked) {
                    mark_edges(&mut cells, &frame);
                }
            }
            Event::Code(_) => {
                cover(&mut frames, &range);
                flag(&mut cells, &range, Flags::CODE);
                let ticks = text[range.clone()].bytes().take_while(|b| *b == b'`').count();
                if ticks * 2 <= range.len() {
                    flag(&mut cells, &(range.start..range.start + ticks), Flags::MARK);
                    flag(&mut cells, &(range.end - ticks..range.end), Flags::MARK);
                }
            }
            Event::Text(_) => {
                cover(&mut frames, &range);
                if in_code || in_link > 0 {
                    continue;
                }
                match plain_text.last_mut() {
                    Some(last) if last.end == range.start => last.end = range.end,
                    _ => plain_text.push(range),
                }
            }
            Event::TaskListMarker(done) => {
                cover(&mut frames, &range);
                let spaced = text.as_bytes().get(range.end) == Some(&b' ');
                let marker = range.start..range.end + usize::from(spaced);
                let line = starts.partition_point(|start| *start <= range.start).saturating_sub(1);
                lines[line].task = Some(Task { done, marker });
            }
            _ => cover(&mut frames, &range),
        }
    }
    for range in &plain_text {
        for address in web_addresses(text, range) {
            flag(&mut cells, &address, Flags::LINK);
        }
    }
    for (index, line) in lines.iter().enumerate() {
        let span = bounds(text, &starts, index);
        match line.block {
            Block::Quote { prefix, .. } => {
                flag(&mut cells, &span, Flags::QUOTE);
                flag(&mut cells, &(span.start..span.start + prefix), Flags::MARK);
            }
            Block::Code { fence } => {
                flag(&mut cells, &span, Flags::FENCED);
                if fence {
                    flag(&mut cells, &span, Flags::MARK);
                }
            }
            Block::Plain => {}
        }
        if let Some(task) = &line.task {
            flag(&mut cells, &task.marker, Flags::MARK);
            if task.done {
                flag(&mut cells, &(task.marker.end..span.end), Flags::DIM);
            }
        }
    }
    Parsed { runs: runs_of(text, &cells), starts, lines }
}

fn runs_of(text: &str, cells: &[Cell]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (at, (byte, cell)) in text.bytes().zip(cells).enumerate() {
        if byte == b'\n' || (cell.flags == Flags::default() && cell.heading == 0) {
            continue;
        }
        match runs.last_mut() {
            Some(last)
                if last.range.end == at
                    && last.flags == cell.flags
                    && last.heading == cell.heading =>
            {
                last.range.end = at + 1;
            }
            _ => runs.push(Run { range: at..at + 1, flags: cell.flags, heading: cell.heading }),
        }
    }
    runs
}

/// What the layout of a text needs besides the text: the font and ink, a tinted run and the lines
/// whose marks show.
pub(crate) struct Look<'a> {
    pub(crate) font: &'a FontId,
    pub(crate) ink: Color32,
    pub(crate) tint: &'a Tint,
    pub(crate) shown: &'a Shown,
}

impl Look<'_> {
    pub(crate) fn shows(&self, line: usize) -> bool {
        shows(self.shown, line)
    }
}

fn ascent(ui: &Ui, font: &FontId) -> f32 {
    let galley = ui.painter().layout_no_wrap("x".to_owned(), font.clone(), Color32::WHITE);
    galley.rows.first().and_then(|row| row.glyphs.first()).map_or(0.0, |glyph| glyph.pos.y)
}

fn row_height(ui: &Ui, font: &FontId) -> f32 {
    ui.ctx().fonts_mut(|fonts| fonts.row_height(font))
}

fn format_of(
    ui: &Ui,
    look: &Look,
    run: Option<&Run>,
    tint: Option<Color32>,
    hidden: bool,
) -> TextFormat {
    let Some(run) = run else {
        let color = tint.unwrap_or(look.ink);
        return TextFormat::simple(look.font.clone(), color);
    };
    let flags = run.flags;
    let size =
        look.font.size * HEADING_EM.get(usize::from(run.heading).wrapping_sub(1)).unwrap_or(&1.0);
    let strong = flags.has(Flags::BOLD) || run.heading > 0;
    let around = if strong { heading(size) } else { body(size) };
    let mut color = look.ink;
    if flags.has(Flags::QUOTE) {
        color = brand::SOFT;
    }
    if run.heading == 6 || flags.has(Flags::DIM) {
        color = brand::MUTED;
    }
    if flags.has(Flags::LINK) {
        color = brand::TEAL_TEXT;
    }
    if let Some(tint) = tint {
        color = tint;
    }
    let mut format = TextFormat::simple(around.clone(), color);
    format.italics = flags.has(Flags::ITALIC);
    if flags.has(Flags::STRIKE) {
        format.strikethrough = Stroke::new(1.0, color);
    }
    if flags.has(Flags::LINK) {
        format.underline = Stroke::new(1.0, brand::link_rest());
    }
    let code = flags.has(Flags::CODE) || flags.has(Flags::FENCED);
    if code {
        let face = mono(size * CODE_EM);
        let lift = (ascent(ui, &around) - ascent(ui, &face)).max(0.0);
        format.line_height = Some(row_height(ui, &around) - lift);
        format.font_id = face;
        if flags.has(Flags::CODE) {
            format.background = brand::ITEM;
        }
    }
    if flags.has(Flags::MARK) {
        if hidden {
            let line = if flags.has(Flags::FENCED) { FENCE_PAD } else { row_height(ui, &around) };
            return TextFormat {
                font_id: FontId::new(HIDDEN_PX, around.family),
                color: Color32::TRANSPARENT,
                line_height: Some(line),
                italics: false,
                underline: Stroke::NONE,
                strikethrough: Stroke::NONE,
                background: Color32::TRANSPARENT,
                ..format
            };
        }
        format.color = brand::FAINT;
        format.underline = Stroke::NONE;
        format.strikethrough = Stroke::NONE;
    }
    format
}

fn append_run(
    ui: &Ui,
    job: &mut LayoutJob,
    (text, start): (&str, usize),
    (range, run): (Range<usize>, Option<&Run>),
    look: &Look,
    parsed: &Parsed,
) {
    if range.is_empty() {
        return;
    }
    let hidden = run.is_some_and(|run| {
        run.flags.has(Flags::MARK) && !look.shows(parsed.line_of(start + range.start))
    });
    let inside = look.tint.as_ref().and_then(|(span, color)| {
        let lo = span.start.max(start + range.start).saturating_sub(start);
        let hi = span.end.min(start + range.end).saturating_sub(start);
        (lo < hi && text.is_char_boundary(lo) && text.is_char_boundary(hi))
            .then_some((lo..hi, *color))
    });
    let mut put = |piece: Range<usize>, tint: Option<Color32>| {
        if !piece.is_empty() {
            job.append(&text[piece], 0.0, format_of(ui, look, run, tint, hidden));
        }
    };
    match inside {
        Some((tinted, color)) => {
            put(range.start..tinted.start, None);
            put(tinted.clone(), Some(color));
            put(tinted.end..range.end, None);
        }
        None => put(range, None),
    }
}

/// Adds `text`, which starts at byte `start` of the real text, to `job` in the style of its
/// constructs.
pub(crate) fn append(
    ui: &Ui,
    job: &mut LayoutJob,
    (text, start): (&str, usize),
    look: &Look,
    parsed: &Parsed,
) {
    let mut at = 0;
    for run in parsed.runs_in(&(start..start + text.len())) {
        let lo = run.range.start.max(start) - start;
        let hi = run.range.end.min(start + text.len()) - start;
        if !text.is_char_boundary(lo) || !text.is_char_boundary(hi) || lo < at {
            continue;
        }
        append_run(ui, job, (text, start), (at..lo, None), look, parsed);
        append_run(ui, job, (text, start), (lo..hi, Some(run)), look, parsed);
        at = hi;
    }
    append_run(ui, job, (text, start), (at..text.len(), None), look, parsed);
}

/// Where the blocks of a laid-out text sit, as rows of the galley painted from `origin`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Blocks {
    /// One bar per level of a block quote: its x, and the top and bottom it spans.
    pub(crate) bars: Vec<(f32, f32, f32)>,
    /// The top and bottom of each fenced code block, fences included.
    pub(crate) codes: Vec<(f32, f32)>,
}

pub(crate) fn bar_width() -> f32 {
    QUOTE_BAR
}

pub(crate) fn code_pad() -> f32 {
    CODE_PAD_X
}

/// The quote bars and code backgrounds of `galley`. A quote line shows its bar where its marks are
/// hidden; where they show, the line is the raw text.
pub(crate) fn blocks(galley: &Galley, origin: Pos2, shown: &Shown) -> Blocks {
    let parsed = parse(galley.text());
    if !parsed.has_quotes()
        && !parsed.lines.iter().any(|line| matches!(line.block, Block::Code { .. }))
    {
        return Blocks::default();
    }
    let mut rows: Vec<(usize, f32, f32)> = Vec::new();
    let mut line = 0;
    for placed in &galley.rows {
        rows.push((line, origin.y + placed.pos.y, origin.y + placed.pos.y + placed.size.y));
        if placed.ends_with_newline {
            line += 1;
        }
    }
    let depth_of = |line: usize| match parsed.line(line).map(|line| line.block) {
        Some(Block::Quote { depth, .. }) if !shows(shown, line) => depth,
        _ => 0,
    };
    let deepest = rows.iter().map(|row| depth_of(row.0)).max().unwrap_or(0);
    let mut out = Blocks::default();
    for level in 0..deepest {
        let mut spans: Vec<(f32, f32)> = Vec::new();
        let mut last_line: Option<usize> = None;
        for &(index, top, bottom) in rows.iter().filter(|row| depth_of(row.0) > level) {
            match (spans.last_mut(), last_line) {
                (Some(span), Some(prev))
                    if prev == index || (prev + 1 == index && (span.1 - top).abs() < 0.5) =>
                {
                    span.1 = bottom;
                }
                _ => spans.push((top, bottom)),
            }
            last_line = Some(index);
        }
        let x = origin.x + level as f32 * QUOTE_INDENT + BAR_INSET;
        out.bars.extend(spans.into_iter().map(|(top, bottom)| (x, top, bottom)));
    }
    let mut last_code: Option<usize> = None;
    for &(index, top, bottom) in &rows {
        if !matches!(parsed.line(index).map(|line| line.block), Some(Block::Code { .. })) {
            continue;
        }
        match (out.codes.last_mut(), last_code) {
            (Some(span), Some(prev)) if prev == index || prev + 1 == index => span.1 = bottom,
            _ => out.codes.push((top, bottom)),
        }
        last_code = Some(index);
    }
    out
}

/// The edge of a quote's text: where its body starts from the left edge of the box text.
pub(crate) fn quote_edge(depth: usize) -> f32 {
    QUOTE_INDENT * depth as f32
}

#[cfg(test)]
mod tests;
