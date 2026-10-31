//! The links Tidewatch finds on its own in what an agent wrote in the chat: claude.ai artifacts and
//! GitHub pull requests and issues (pure). A markdown link gives its text as the title, a bare link
//! a plain fallback. Only the agent's own messages are read, never the user's.

use crate::link_board::{Link, LinkBoard, LinkState, MAX_TITLE_CHARS, Status, TitleFrom, one_line};
use crate::transcript::{Entry, Role};

const ARTIFACT_PATHS: [&str; 3] = ["artifact/", "code/artifact/", "public/artifacts/"];

/// The links in the agent's messages among `entries`, one per address. An entry without a time is
/// stamped `now_ms`.
#[must_use]
pub fn found(entries: &[Entry], now_ms: i64) -> Vec<Link> {
    let mut board = LinkBoard::default();
    for entry in entries.iter().filter(|entry| entry.role == Role::Assistant) {
        let seen = entry.ts.unwrap_or(now_ms);
        for (url, text, fallback) in scan(&entry.text) {
            let (title, title_from) = match text {
                Some(text) => (text, TitleFrom::Seen),
                None => (fallback, TitleFrom::Guess),
            };
            board.absorb(&Link {
                url,
                title,
                note: None,
                first_seen_ms: seen,
                title_from,
                state: LinkState::Unknown,
                status: Status::default(),
            });
        }
    }
    board.links
}

/// The pull request addresses in `text`, in order, each cut to `https://github.com/<owner>/<repo>/pull/<n>`.
#[must_use]
pub fn pull_requests(text: &str) -> Vec<String> {
    scan(text).into_iter().map(|(url, ..)| url).filter(|url| url.contains("/pull/")).collect()
}

fn scan(text: &str) -> Vec<(String, Option<String>, String)> {
    text.match_indices("http")
        .filter(|(start, _)| !text[..*start].chars().next_back().is_some_and(char::is_alphanumeric))
        .filter_map(|(start, _)| {
            let after = text[start..]
                .strip_prefix("https://")
                .or_else(|| text[start..].strip_prefix("http://"))?;
            let (url, fallback) = known(after)?;
            Some((url, label_before(&text[..start]), fallback))
        })
        .collect()
}

fn label_before(head: &str) -> Option<String> {
    let inside = head.strip_suffix("](")?;
    let label = &inside[inside.rfind('[')? + 1..];
    if label.contains('\n') {
        return None;
    }
    one_line(label, MAX_TITLE_CHARS).filter(|label| !label.starts_with("http"))
}

fn known(after_scheme: &str) -> Option<(String, String)> {
    let (host, path) = after_scheme.split_once('/')?;
    match host.to_ascii_lowercase().as_str() {
        "claude.ai" => artifact(path),
        "github.com" => github(path),
        _ => None,
    }
}

fn artifact(path: &str) -> Option<(String, String)> {
    let (head, rest) =
        ARTIFACT_PATHS.iter().find_map(|head| path.strip_prefix(head).map(|rest| (*head, rest)))?;
    let id: String =
        rest.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
    (!id.is_empty()).then(|| (format!("https://claude.ai/{head}{id}"), "Artifact".to_owned()))
}

fn github(path: &str) -> Option<(String, String)> {
    let mut parts = path.splitn(4, '/');
    let (owner, repo, what, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let word = match what {
        "pull" => "PR",
        "issues" => "Issue",
        _ => return None,
    };
    let name_ok = |name: &str| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let glued = rest[digits.len()..].chars().next().is_some_and(char::is_alphanumeric);
    let number: u64 = digits.parse().ok().filter(|_| !glued)?;
    (name_ok(owner) && name_ok(repo)).then(|| {
        (format!("https://github.com/{owner}/{repo}/{what}/{number}"), format!("{word} #{number}"))
    })
}

#[cfg(test)]
mod tests;
