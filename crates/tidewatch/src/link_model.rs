//! What the Links card of the Progress panel shows, as plain data: which rows, how each one reads,
//! the ink of each kind and the links a question asks the user to read first (pure).

use egui::Color32;
use tidewatch_core::link_board::{Kind, Link, LinkBoard, LinkState};
use tidewatch_core::pr::PullRequest;
use tidewatch_core::pr_status::{State, Summary};

use crate::brand;
use crate::pr_card_model::age;

pub(crate) const SHOWN: usize = 3;
pub(crate) const RECHECK_MS: i64 = 30 * MIN;

pub(crate) fn ink(kind: Kind) -> Color32 {
    match kind {
        Kind::Artifact => brand::VIOLET_TEXT,
        Kind::Pr => brand::TEAL,
        Kind::Issue => brand::GREEN,
        Kind::Doc => brand::BLUE,
        Kind::Web => brand::SOFT,
    }
}

/// The links of the card, the newest first, split in the ones that stay in the main list (open or
/// not known) and the merged and closed ones that go in the folded group at the bottom.
pub(crate) fn split(board: &LinkBoard) -> (Vec<&Link>, Vec<&Link>) {
    board.newest_first().into_iter().partition(|link| !link.state.archived())
}

/// The part of the main list to draw: all of it, or the first `SHOWN`.
pub(crate) fn listed<'a, 'b>(main: &'b [&'a Link], all: bool) -> &'b [&'a Link] {
    if all { main } else { &main[..main.len().min(SHOWN)] }
}

/// The text of the fold under the rows, `None` while every link is listed and there is no fold to close.
pub(crate) fn fold_label(count: usize, all: bool) -> Option<String> {
    match (count > SHOWN, all) {
        (false, _) => None,
        (true, false) => Some(format!("Show all {count}")),
        (true, true) => Some("Show less".to_owned()),
    }
}

/// The header of the folded group, `None` while nothing is merged or closed.
pub(crate) fn archive_label(count: usize) -> Option<String> {
    (count > 0).then(|| format!("Merged and closed ({count})"))
}

/// The small tag a merged or closed row carries.
pub(crate) fn state_tag(link: &Link) -> Option<&'static str> {
    match link.state {
        LinkState::Merged => Some("Merged"),
        LinkState::Closed => Some("Closed"),
        LinkState::Unknown | LinkState::Open => None,
    }
}

/// Whether the link's state is worth a lookup now: it is a pull request or an issue that is not
/// merged or closed yet, and it was never looked up or not for `RECHECK_MS`.
pub(crate) fn check_due(link: &Link, last_checked_ms: Option<i64>, now_ms: i64) -> bool {
    link.kind().has_state()
        && !link.state.archived()
        && last_checked_ms.is_none_or(|at| now_ms - at >= RECHECK_MS)
}

/// A tab's pull request state as the link keeps it.
pub(crate) fn link_state(state: State) -> LinkState {
    match state {
        State::Open => LinkState::Open,
        State::Merged => LinkState::Merged,
        State::Closed => LinkState::Closed,
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

/// A tab's own pull request as the lookup and the watch know it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnPr<'a> {
    pub(crate) number: u64,
    pub(crate) url: &'a str,
    pub(crate) title: Option<&'a str>,
    pub(crate) state: LinkState,
}

/// A tab's own pull request as a link: `#42 title`, or `PR #42` while the title is not known.
pub(crate) fn pr_link(pr: &OwnPr<'_>, now_ms: i64) -> Link {
    use tidewatch_core::link_board::{MAX_TITLE_CHARS, TitleFrom, one_line};
    let named = pr.title.and_then(|title| one_line(title, MAX_TITLE_CHARS));
    let (title, title_from) = match named {
        Some(title) => {
            (one_line(&format!("#{} {title}", pr.number), MAX_TITLE_CHARS), TitleFrom::Seen)
        }
        None => (None, TitleFrom::Guess),
    };
    Link {
        url: pr.url.to_owned(),
        title: title.unwrap_or_else(|| format!("PR #{}", pr.number)),
        note: None,
        first_seen_ms: now_ms,
        title_from,
        state: pr.state,
    }
}

/// The tab's pull request as the lookup and the watch know it: the watch's summary when it has
/// one, because it has the title and the state, else the lookup's.
pub(crate) fn own_pr<'a>(
    summary: Option<&'a Summary>,
    lookup: Option<&'a PullRequest>,
) -> Option<OwnPr<'a>> {
    match (summary, lookup) {
        (Some(summary), _) => Some(OwnPr {
            number: summary.number,
            url: &summary.url,
            title: Some(&summary.title),
            state: link_state(summary.state),
        }),
        (None, Some(pr)) => {
            Some(OwnPr { number: pr.number, url: &pr.url, title: None, state: LinkState::Unknown })
        }
        (None, None) => None,
    }
}

const MIN: i64 = 60_000;

/// The links the mockup of the card shows, for `TIDEWATCH_FAKE_LINKS=1`: five in the main list and
/// three merged or closed ones in the folded group.
pub(crate) fn fake_board(now_ms: i64) -> LinkBoard {
    use tidewatch_core::link_board::TitleFrom;
    let mut board = LinkBoard::default();
    let rows = [
        (
            "https://docs.google.com/document/d/1mercury-spend-export",
            "Mercury spend export, October",
            None,
            60 * MIN,
            LinkState::Unknown,
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/issues/38",
            "#38 Spend page loads slowly",
            None,
            180 * MIN,
            LinkState::Open,
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/pull/42",
            "#42 feat: profit and spend dashboard",
            None,
            40 * MIN,
            LinkState::Open,
        ),
        (
            "https://claude.ai/artifact/4JqXyaNvpN1W289NyzXiT9",
            "Split options compared",
            Some("Which split for the bonus pool?"),
            12 * MIN,
            LinkState::Unknown,
        ),
        (
            "https://example.com/notes/bonus-pools",
            "How other teams split a pool",
            None,
            300 * MIN,
            LinkState::Unknown,
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/pull/39",
            "#39 fix: round the spend totals",
            None,
            700 * MIN,
            LinkState::Merged,
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/pull/35",
            "#35 chore: drop the old export script",
            None,
            1_500 * MIN,
            LinkState::Merged,
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/issues/31",
            "#31 Export fails on empty months",
            None,
            2_400 * MIN,
            LinkState::Closed,
        ),
    ];
    for (url, title, note, ago, state) in rows {
        board.absorb(&Link {
            url: url.to_owned(),
            title: title.to_owned(),
            note: note.map(str::to_owned),
            first_seen_ms: now_ms - ago,
            title_from: TitleFrom::Agent,
            state,
        });
    }
    board
}

#[cfg(test)]
mod tests;
