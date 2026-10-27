//! What the Links card of the Progress panel shows, as plain data: which rows, how each one reads,
//! the ink of each kind and the links a question asks the user to read first (pure).

use egui::Color32;
use tidewatch_core::link_board::{Kind, Link, LinkBoard};
use tidewatch_core::pr::PullRequest;
use tidewatch_core::pr_status::Summary;

use crate::brand;
use crate::pr_card_model::age;

pub(crate) const SHOWN: usize = 3;

pub(crate) fn ink(kind: Kind) -> Color32 {
    match kind {
        Kind::Artifact => brand::VIOLET_TEXT,
        Kind::Pr => brand::TEAL,
        Kind::Issue => brand::GREEN,
        Kind::Doc => brand::BLUE,
        Kind::Web => brand::SOFT,
    }
}

/// The links to list, the newest first: all of them, or the first `SHOWN`.
pub(crate) fn listed(board: &LinkBoard, all: bool) -> Vec<&Link> {
    let links = board.newest_first();
    let take = if all { links.len() } else { SHOWN };
    links.into_iter().take(take).collect()
}

/// The text of the fold under the rows, `None` while every link is listed and there is no fold to close.
pub(crate) fn fold_label(count: usize, all: bool) -> Option<String> {
    match (count > SHOWN, all) {
        (false, _) => None,
        (true, false) => Some(format!("Show all {count}")),
        (true, true) => Some("Show less".to_owned()),
    }
}

/// `claude.ai · for: Which split? · 12m ago`.
pub(crate) fn meta(link: &Link, now_ms: i64) -> String {
    let mut parts = vec![link.host()];
    if let Some(note) = link.note.as_deref().map(str::trim).filter(|note| !note.is_empty()) {
        parts.push(format!("for: {note}"));
    }
    parts.push(age(now_ms, link.first_seen_ms));
    parts.join(" \u{b7} ")
}

/// The links sent for this very question, in the order they were seen.
pub(crate) fn read_first<'a>(board: &'a LinkBoard, question: &str) -> Vec<&'a Link> {
    let question = question.trim();
    board
        .links
        .iter()
        .filter(|link| {
            link.note.as_deref().map(str::trim) == Some(question) && !question.is_empty()
        })
        .collect()
}

/// A tab's own pull request as a link: `#42 title`, or `PR #42` while the title is not known.
pub(crate) fn pr_link(number: u64, url: &str, title: Option<&str>, now_ms: i64) -> Link {
    use tidewatch_core::link_board::{MAX_TITLE_CHARS, TitleFrom, one_line};
    let named = title.and_then(|title| one_line(title, MAX_TITLE_CHARS));
    let (title, title_from) = match named {
        Some(title) => (one_line(&format!("#{number} {title}"), MAX_TITLE_CHARS), TitleFrom::Seen),
        None => (None, TitleFrom::Guess),
    };
    Link {
        url: url.to_owned(),
        title: title.unwrap_or_else(|| format!("PR #{number}")),
        note: None,
        first_seen_ms: now_ms,
        title_from,
    }
}

/// The tab's pull request as the lookup and the watch know it: the watch's summary when it has
/// one, because it has the title, else the lookup's.
pub(crate) fn own_pr<'a>(
    summary: Option<&'a Summary>,
    lookup: Option<&'a PullRequest>,
) -> Option<(u64, &'a str, Option<&'a str>)> {
    match (summary, lookup) {
        (Some(summary), _) => Some((summary.number, &summary.url, Some(&summary.title))),
        (None, Some(pr)) => Some((pr.number, &pr.url, None)),
        (None, None) => None,
    }
}

const MIN: i64 = 60_000;

/// The links the mockup of the card shows, for `TIDEWATCH_FAKE_LINKS=1`.
pub(crate) fn fake_board(now_ms: i64) -> LinkBoard {
    use tidewatch_core::link_board::TitleFrom;
    let mut board = LinkBoard::default();
    let rows = [
        (
            "https://docs.google.com/document/d/1mercury-spend-export",
            "Mercury spend export, October",
            None,
            60 * MIN,
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/issues/38",
            "#38 Spend page loads slowly",
            None,
            180 * MIN,
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/pull/42",
            "#42 feat: profit and spend dashboard",
            None,
            40 * MIN,
        ),
        (
            "https://claude.ai/artifact/4JqXyaNvpN1W289NyzXiT9",
            "Split options compared",
            Some("Which split for the bonus pool?"),
            12 * MIN,
        ),
        ("https://example.com/notes/bonus-pools", "How other teams split a pool", None, 300 * MIN),
    ];
    for (url, title, note, ago) in rows {
        board.absorb(&Link {
            url: url.to_owned(),
            title: title.to_owned(),
            note: note.map(str::to_owned),
            first_seen_ms: now_ms - ago,
            title_from: TitleFrom::Agent,
        });
    }
    board
}

#[cfg(test)]
mod tests;
