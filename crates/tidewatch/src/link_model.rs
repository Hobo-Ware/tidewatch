//! What the Links card of the Progress panel shows, as plain data: how each row reads (its pills
//! and its meta line), the ink of each kind, how often each link is looked up and the links a
//! question asks the user to read first (pure).

use egui::Color32;
use tidewatch_core::link_board::{
    Access, Checks, Kind, Link, LinkBoard, LinkState, Reason, Review, Status,
};
use tidewatch_core::link_check::{state_of, status_of};
use tidewatch_core::pr::PullRequest;
use tidewatch_core::pr_status::Summary;

use crate::brand;
use crate::pr_card_model::age;

pub(crate) const SHOWN: usize = 3;
const LIVE_RECHECK_MS: i64 = 30 * MIN;
const FRONT_RECHECK_MS: i64 = 5 * MIN;
const BACK_RECHECK_MS: i64 = 30 * MIN;
pub(crate) const NEWS_GAP_MS: i64 = 2_000;

pub(crate) fn ink(kind: Kind) -> Color32 {
    match kind {
        Kind::Artifact => brand::VIOLET_TEXT,
        Kind::Pr => brand::TEAL,
        Kind::Issue => brand::GREEN,
        Kind::Doc => brand::BLUE,
        Kind::Web => brand::SOFT,
    }
}

/// The part of `rows` to draw: all of it, or the first `SHOWN`.
pub(crate) fn listed<'a, 'b>(rows: &'b [&'a Link], all: bool) -> &'b [&'a Link] {
    if all { rows } else { &rows[..rows.len().min(SHOWN)] }
}

/// The text of the fold under the rows, `None` while every link is listed and there is no fold to close.
pub(crate) fn fold_label(count: usize, all: bool) -> Option<String> {
    match (count > SHOWN, all) {
        (false, _) => None,
        (true, false) => Some(format!("Show all {count}")),
        (true, true) => Some("Show less".to_owned()),
    }
}

/// How a link's state reaches the app, which sets how often it is looked up besides the news.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// A forwarder delivers for the link's repo: events bring the news, a slow look backs them up.
    Live,
    /// Polled, and the tab is the one in front.
    Front,
    /// Polled, and the tab is in the background.
    Back,
}

impl Reach {
    /// The one of the two that is looked at more often.
    pub(crate) fn min_by_recheck(self, other: Self) -> Self {
        if other.recheck_ms() < self.recheck_ms() { other } else { self }
    }

    pub(crate) fn recheck_ms(self) -> i64 {
        match self {
            Self::Live => LIVE_RECHECK_MS,
            Self::Front => FRONT_RECHECK_MS,
            Self::Back => BACK_RECHECK_MS,
        }
    }
}

/// Whether the link is worth a lookup now: it is a pull request or issue still going that GitHub
/// answers for, and it was never looked up, or news named it and the last look was `NEWS_GAP_MS`
/// ago, or the last look is older than its `Reach` allows.
pub(crate) fn check_due(
    link: &Link,
    last_checked_ms: Option<i64>,
    now_ms: i64,
    reach: Reach,
    news: bool,
) -> bool {
    if !link.is_watchable() {
        return false;
    }
    let wait = if news { NEWS_GAP_MS } else { reach.recheck_ms() };
    last_checked_ms.is_none_or(|at| now_ms - at >= wait)
}

/// What a pill says about the link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Bad,
    Warn,
    Good,
    Quiet,
    Live,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pill {
    pub(crate) text: &'static str,
    pub(crate) tone: Tone,
}

const fn pill(text: &'static str, tone: Tone) -> Pill {
    Pill { text, tone }
}

/// The pills of a row, the one that wants the user first and `live` last: what the user must act
/// on, then how the link stands.
pub(crate) fn pills(link: &Link, live: bool) -> Vec<Pill> {
    let status = &link.status;
    let mut pills = Vec::new();
    match status.access {
        Access::Gone => pills.push(pill("gone", Tone::Bad)),
        Access::NoAccess => pills.push(pill("no access", Tone::Warn)),
        Access::Readable => {}
    }
    match link.state {
        LinkState::Merged => pills.push(pill("merged", Tone::Good)),
        LinkState::Closed => pills.push(match status.reason {
            Some(Reason::Completed) => pill("completed", Tone::Good),
            Some(Reason::NotPlanned) => pill("not planned", Tone::Quiet),
            None => pill("closed", Tone::Quiet),
        }),
        LinkState::Open | LinkState::Unknown if status.access.is_readable() => {
            if status.checks == Some(Checks::Fail) {
                pills.push(pill("checks failing", Tone::Bad));
            }
            if status.review == Some(Review::ChangesRequested) {
                pills.push(pill("changes requested", Tone::Warn));
            }
            if status.conflicts {
                pills.push(pill("conflicts", Tone::Warn));
            }
            if status.review == Some(Review::Approved) {
                pills.push(pill("approved", Tone::Good));
            }
            match status.checks {
                Some(Checks::Pass) => pills.push(pill("checks pass", Tone::Good)),
                Some(Checks::Running) => pills.push(pill("checks running", Tone::Quiet)),
                Some(Checks::Fail) | None => {}
            }
            if pills.is_empty() && link.state == LinkState::Open {
                pills.push(pill("open", Tone::Quiet));
            }
        }
        LinkState::Open | LinkState::Unknown => {}
    }
    if live && link.is_watchable() {
        pills.push(pill("live", Tone::Live));
    }
    pills
}

/// `claude.ai · for: Which split? · 12m ago`; for a pull request or issue, `owner/name · draft ·
/// for: the opt-out flow · 3 comments · updated 2m ago`, and `checked 12m ago` while no forwarder
/// delivers for its repo.
pub(crate) fn meta(link: &Link, now_ms: i64, live: bool) -> String {
    let status = &link.status;
    let mut parts = vec![link.repo_name().unwrap_or_else(|| link.host())];
    if status.draft {
        parts.push("draft".to_owned());
    }
    if let Some(note) = link.note.as_deref().map(str::trim).filter(|note| !note.is_empty()) {
        parts.push(format!("for: {note}"));
    }
    match status.comments {
        Some(1) => parts.push("1 comment".to_owned()),
        Some(count) if count > 1 => parts.push(format!("{count} comments")),
        _ => {}
    }
    match status.updated_ms {
        Some(at) => parts.push(format!("updated {}", age(now_ms, at))),
        None => parts.push(age(now_ms, link.first_seen_ms)),
    }
    if let Some(at) = status.checked_ms.filter(|_| !live && link.repo().is_some()) {
        parts.push(format!("checked {}", age(now_ms, at)));
    }
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
    pub(crate) status: Status,
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
        status: Status { checked_ms: Some(now_ms), ..pr.status.clone() },
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
            state: state_of(summary.state),
            status: Status { checked_ms: None, ..status_of(summary, false, None, 0) },
        }),
        (None, Some(pr)) => Some(OwnPr {
            number: pr.number,
            url: &pr.url,
            title: None,
            state: LinkState::Unknown,
            status: Status::default(),
        }),
        (None, None) => None,
    }
}

const MIN: i64 = 60_000;

/// The repo the fake board treats as live.
pub(crate) const FAKE_LIVE_REPO: &str = "hobo-ware/tidewatch";

/// The links the mockup of the card shows, for `TIDEWATCH_FAKE_LINKS=1`: pull requests and issues of
/// a live repo and of one that is polled, in every state, an artifact, a doc and a web page.
pub(crate) fn fake_board(now_ms: i64) -> LinkBoard {
    use tidewatch_core::link_board::TitleFrom;
    let mut board = LinkBoard::default();
    let looked = |checked: i64, updated: i64| Status {
        checked_ms: Some(now_ms - checked * MIN),
        updated_ms: Some(now_ms - updated * MIN),
        ..Status::default()
    };
    let rows = [
        (
            "https://github.com/Hobo-Ware/tidewatch/pull/42",
            "#42 feat: profit and spend dashboard",
            Some("the spend dashboard"),
            40,
            LinkState::Open,
            Status {
                checks: Some(Checks::Fail),
                review: Some(Review::ChangesRequested),
                comments: Some(2),
                ..looked(1, 2)
            },
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/pull/44",
            "#44 fix: round the export totals",
            None,
            90,
            LinkState::Open,
            Status { draft: true, conflicts: true, ..looked(1, 14) },
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/issues/38",
            "#38 Spend page loads slowly",
            None,
            180,
            LinkState::Open,
            Status { comments: Some(3), ..looked(1, 60) },
        ),
        (
            "https://github.com/Hobo-Ware/showly/pull/191",
            "#191 chore: drop the client secret from sync",
            None,
            200,
            LinkState::Open,
            Status { checks: Some(Checks::Pass), review: Some(Review::Approved), ..looked(12, 30) },
        ),
        (
            "https://claude.ai/artifact/4JqXyaNvpN1W289NyzXiT9",
            "Split options compared",
            Some("Which split for the bonus pool?"),
            12,
            LinkState::Unknown,
            Status::default(),
        ),
        (
            "https://docs.google.com/document/d/1mercury-spend-export",
            "Mercury spend export, October",
            None,
            600,
            LinkState::Unknown,
            Status::default(),
        ),
        (
            "https://example.com/notes/bonus-pools",
            "How other teams split a pool",
            None,
            300,
            LinkState::Unknown,
            Status::default(),
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/pull/39",
            "#39 fix: round the spend totals",
            None,
            700,
            LinkState::Merged,
            looked(1, 700),
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/issues/31",
            "#31 Export fails on empty months",
            None,
            2_400,
            LinkState::Closed,
            Status { reason: Some(Reason::Completed), ..looked(1, 2_400) },
        ),
        (
            "https://github.com/Hobo-Ware/tidewatch/issues/30",
            "#30 Support a CSV export",
            None,
            2_600,
            LinkState::Closed,
            Status { reason: Some(Reason::NotPlanned), ..looked(1, 2_600) },
        ),
    ];
    for (url, title, note, ago, state, status) in rows {
        board.absorb(&Link {
            url: url.to_owned(),
            title: title.to_owned(),
            note: note.map(str::to_owned),
            first_seen_ms: now_ms - ago * MIN,
            title_from: TitleFrom::Agent,
            state,
            status,
        });
    }
    board
}

#[cfg(test)]
mod tests;
