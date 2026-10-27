//! A chat message as a tree of markdown blocks, read from `pulldown-cmark` with the GitHub options
//! on (tables, task lists, strikethrough, footnotes). The Reader draws the tree itself in
//! `md_view`. Pure.

use std::iter::Peekable;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    #[default]
    Text,
    Code,
    Note,
    Image,
    Task,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) strike: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) link: Option<String>,
    pub(crate) kind: Kind,
}

pub(crate) type Inline = Vec<Run>;

#[derive(Debug, PartialEq)]
pub(crate) struct Item {
    pub(crate) task: Option<bool>,
    pub(crate) blocks: Vec<Block>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Note {
    pub(crate) label: String,
    pub(crate) blocks: Vec<Block>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Block {
    Paragraph { runs: Inline, tight: bool },
    Heading { level: u8, runs: Inline },
    Rule,
    Code { lang: Option<String>, text: String },
    Quote(Vec<Block>),
    List { start: Option<u64>, items: Vec<Item> },
    Table { aligns: Vec<Alignment>, head: Vec<Inline>, rows: Vec<Vec<Inline>> },
    Notes(Vec<Note>),
}

pub(crate) fn list_spans(md: &str) -> Vec<std::ops::Range<usize>> {
    let mut depth = 0_usize;
    let mut quoted = 0_usize;
    let mut spans = Vec::new();
    for (event, range) in Parser::new_ext(md, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::BlockQuote(_)) => quoted += 1,
            Event::End(TagEnd::BlockQuote(_)) => quoted = quoted.saturating_sub(1),
            Event::Start(Tag::List(_)) => {
                if depth == 0 && quoted == 0 {
                    spans.push(range);
                }
                depth += 1;
            }
            Event::End(TagEnd::List(_)) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    spans
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
}

type Events<'a> = Peekable<Parser<'a>>;

pub(crate) fn parse(md: &str) -> Vec<Block> {
    let mut events = Parser::new_ext(md, options()).peekable();
    let mut notes = Vec::new();
    let mut out = blocks(&mut events, &mut notes);
    if !notes.is_empty() {
        out.push(Block::Notes(notes));
    }
    out
}

fn blocks(events: &mut Events, notes: &mut Vec<Note>) -> Vec<Block> {
    let mut out = Vec::new();
    while let Some(event) = events.peek() {
        match event {
            Event::End(_) => break,
            Event::Rule => {
                events.next();
                out.push(Block::Rule);
            }
            Event::Start(tag) if is_block(tag) => {
                let Some(Event::Start(tag)) = events.next() else { break };
                block(tag, events, notes, &mut out);
            }
            _ => {
                let runs = inline(events);
                if runs.is_empty() {
                    events.next();
                } else {
                    out.push(Block::Paragraph { runs, tight: true });
                }
            }
        }
    }
    out
}

fn is_block(tag: &Tag) -> bool {
    !matches!(
        tag,
        Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }
    )
}

fn block(tag: Tag, events: &mut Events, notes: &mut Vec<Note>, out: &mut Vec<Block>) {
    match tag {
        Tag::Paragraph => {
            let runs = inline(events);
            end(events);
            out.push(Block::Paragraph { runs, tight: false });
        }
        Tag::Heading { level, .. } => {
            let runs = inline(events);
            end(events);
            out.push(Block::Heading { level: heading_level(level), runs });
        }
        Tag::BlockQuote(_) => {
            let inner = blocks(events, notes);
            end(events);
            out.push(Block::Quote(inner));
        }
        Tag::CodeBlock(kind) => {
            let mut text = String::new();
            while let Some(Event::Text(chunk)) = events.peek() {
                text.push_str(chunk);
                events.next();
            }
            end(events);
            let lang = match kind {
                CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(str::to_owned),
                CodeBlockKind::Indented => None,
            };
            out.push(Block::Code { lang, text: text.trim_end_matches('\n').to_owned() });
        }
        Tag::List(start) => {
            let mut items = Vec::new();
            while matches!(events.peek(), Some(Event::Start(Tag::Item))) {
                events.next();
                let mut inner = blocks(events, notes);
                end(events);
                let task = take_task(&mut inner);
                items.push(Item { task, blocks: inner });
            }
            end(events);
            out.push(Block::List { start, items });
        }
        Tag::Table(aligns) => out.push(table(aligns, events)),
        Tag::FootnoteDefinition(label) => {
            let inner = blocks(events, notes);
            end(events);
            notes.push(Note { label: label.to_string(), blocks: inner });
        }
        Tag::HtmlBlock => {
            let mut runs = inline(events);
            end(events);
            if let Some(last) = runs.last_mut() {
                last.text = last.text.trim_end_matches('\n').to_owned();
            }
            out.push(Block::Paragraph { runs, tight: false });
        }
        _ => {
            let _ = blocks(events, notes);
            end(events);
        }
    }
}

fn take_task(inner: &mut Vec<Block>) -> Option<bool> {
    let Some(Block::Paragraph { runs, .. }) = inner.first_mut() else { return None };
    if runs.first()?.kind != Kind::Task {
        return None;
    }
    let done = runs.remove(0).text == "x";
    if runs.is_empty() {
        inner.remove(0);
    }
    Some(done)
}

fn table(aligns: Vec<Alignment>, events: &mut Events) -> Block {
    let mut head = Vec::new();
    let mut rows = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableHead) => head = cells(events),
            Event::Start(Tag::TableRow) => rows.push(cells(events)),
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    Block::Table { aligns, head, rows }
}

fn cells(events: &mut Events) -> Vec<Inline> {
    let mut out = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableCell) => {
                out.push(inline(events));
                end(events);
            }
            Event::End(_) => break,
            _ => {}
        }
    }
    out
}

fn end(events: &mut Events) {
    if matches!(events.peek(), Some(Event::End(_))) {
        events.next();
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn inline(events: &mut Events) -> Inline {
    let mut out = Vec::new();
    spans(events, &Run::default(), &mut out);
    out
}

fn spans(events: &mut Events, style: &Run, out: &mut Inline) {
    while let Some(event) = events.peek() {
        let run = |text: &str| Run { text: text.to_owned(), ..style.clone() };
        match event {
            Event::Text(text) | Event::InlineHtml(text) | Event::Html(text) => {
                out.push(run(text));
            }
            Event::Code(text) => out.push(Run { kind: Kind::Code, ..run(text) }),
            Event::TaskListMarker(done) => {
                let mark = if *done { "x" } else { "" };
                out.push(Run { kind: Kind::Task, ..run(mark) });
            }
            Event::SoftBreak => out.push(run(" ")),
            Event::HardBreak => out.push(run("\n")),
            Event::FootnoteReference(label) => {
                out.push(Run { kind: Kind::Note, ..run(label) });
            }
            Event::Start(Tag::Emphasis) => {
                events.next();
                spans(
                    events,
                    &Run { style: Style { italic: true, ..style.style }, ..style.clone() },
                    out,
                );
                end(events);
                continue;
            }
            Event::Start(Tag::Strong) => {
                events.next();
                spans(
                    events,
                    &Run { style: Style { bold: true, ..style.style }, ..style.clone() },
                    out,
                );
                end(events);
                continue;
            }
            Event::Start(Tag::Strikethrough) => {
                events.next();
                spans(
                    events,
                    &Run { style: Style { strike: true, ..style.style }, ..style.clone() },
                    out,
                );
                end(events);
                continue;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let link = Some(dest_url.to_string());
                events.next();
                spans(events, &Run { link, ..style.clone() }, out);
                end(events);
                continue;
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                let link = Some(dest_url.to_string());
                events.next();
                let mut alt = Vec::new();
                spans(events, &Run::default(), &mut alt);
                end(events);
                let alt: String = alt.iter().map(|run| run.text.as_str()).collect();
                out.push(Run { text: alt, link, kind: Kind::Image, ..style.clone() });
                continue;
            }
            _ => break,
        }
        events.next();
    }
}

#[cfg(test)]
mod tests;
