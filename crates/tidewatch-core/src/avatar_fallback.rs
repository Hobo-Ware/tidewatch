//! The icon Tidewatch picks from the goal, or else the owner's first message, when the agent sets
//! none within 30 seconds of the ask; a later `set_avatar` replaces it.

use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::avatar::{Avatar, clean};
use crate::avatar_ask::Face;
use crate::avatar_check;
use crate::avatar_icons;
use crate::avatar_store;
use crate::fsutil::write_atomic;
use crate::paths::{
    avatar_asked_file_beside, avatar_fallback_file_beside, avatar_file_beside, progress_file_beside,
};
use crate::progress_store;

pub const WAIT_MS: i64 = 30_000;
pub const GENERIC: &str = "sparkles";
pub const REASON: &str = "Picked by Tidewatch from the task: the agent set no icon.";

/// Goal words and the icon each group stands for, the first group with a match wins, so the
/// objects come before the verbs.
pub const KEYWORDS: &[(&[&str], &str)] = &[
    (&["test", "tests", "testing", "spec", "specs", "flaky", "coverage"], "flask-conical"),
    (
        &["database", "db", "sql", "migration", "migrations", "schema", "postgres", "sqlite"],
        "database",
    ),
    (
        &["security", "auth", "login", "password", "permission", "permissions", "secret", "token"],
        "shield",
    ),
    (&["release", "deploy", "deployment", "ship", "shipping", "publish", "launch"], "rocket"),
    (&["docs", "doc", "documentation", "readme", "guide", "changelog", "ledger"], "book-open"),
    (&["meter", "meters", "usage", "limit", "limits", "quota", "gauge"], "gauge"),
    (
        &["ui", "ux", "screen", "screens", "layout", "pane", "panes", "dialog", "button", "css"],
        "layout-dashboard",
    ),
    (&["performance", "perf", "speed", "faster", "slow", "latency", "cpu"], "zap"),
    (&["git", "branch", "merge", "rebase", "commit", "commits", "pr"], "git-branch"),
    (&["icon", "icons", "avatar", "avatars", "image", "images"], "image"),
    (&["chat", "message", "messages", "reply", "reader", "transcript"], "message-square"),
    (&["terminal", "shell", "pty", "cli"], "terminal"),
    (&["refactor", "refactoring", "cleanup", "dedupe", "rename", "tidy"], "refresh-cw"),
    (&["bug", "bugs", "fix", "fixes", "fixing", "crash", "broken", "error", "regression"], "bug"),
];

/// The icon and color for a goal: the first keyword group one of its words names, else
/// [`GENERIC`]; the color is a stable pick from the `set_avatar` list by a hash of the goal.
#[must_use]
pub fn pick(goal: &str) -> (&'static str, &'static str) {
    let lower = goal.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
    let icon = KEYWORDS
        .iter()
        .find(|(keys, _)| keys.iter().any(|key| words.contains(key)))
        .map_or(GENERIC, |(_, icon)| icon);
    let colors = &avatar_icons::COLORS;
    let at = usize::try_from(fnv1a(goal.trim().as_bytes()) % colors.len() as u64).unwrap_or(0);
    (icon, colors[at].0)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The picked icon drawn and checked like a `set_avatar` one.
#[must_use]
pub fn face(goal: &str) -> Option<Avatar> {
    let (icon, color) = pick(goal);
    let svg = avatar_icons::svg(icon, &avatar_icons::color_hex(color)?)?;
    clean(&svg, REASON).ok().filter(|avatar| avatar_check::check(&avatar.svg).is_ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub face: Face,
    pub settled: bool,
    pub has_words: bool,
    pub asked_at_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    Never,
    Later,
    In(i64),
    Now,
}

/// Whether the fallback icon is due: never for a fixed face or a settled tab, not yet without an
/// ask or words to pick from or while an icon is there, and only once [`WAIT_MS`] passed since the ask.
#[must_use]
pub fn due(facts: &Facts, now_ms: i64) -> Due {
    if facts.face == Face::Fixed || facts.settled {
        return Due::Never;
    }
    let (Face::Letter, true, Some(asked)) = (facts.face, facts.has_words, facts.asked_at_ms) else {
        return Due::Later;
    };
    let left = asked.saturating_add(WAIT_MS).saturating_sub(now_ms);
    if left > 0 { Due::In(left) } else { Due::Now }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Saved(Avatar),
    Wait(i64),
    Later,
    Never,
}

/// Reads the tab's state folder, saves the fallback icon when it is due and says what came of it.
#[must_use]
pub fn apply(notes_file: &Path, fixed: bool, now_ms: i64) -> Outcome {
    let file = avatar_file_beside(notes_file);
    let marker = avatar_asked_file_beside(notes_file);
    let words = progress_store::load(&progress_file_beside(notes_file))
        .map(|ledger| ledger.goal)
        .filter(|goal| !goal.trim().is_empty())
        .or_else(|| avatar_store::asked_message(&marker));
    let shown = if fixed {
        Face::Fixed
    } else if avatar_store::load(&file).is_some() {
        Face::Picture
    } else {
        Face::Letter
    };
    let facts = Facts {
        face: shown,
        settled: avatar_fallback_file_beside(notes_file).exists(),
        has_words: words.is_some(),
        asked_at_ms: modified_ms(&marker),
    };
    match due(&facts, now_ms) {
        Due::Never => Outcome::Never,
        Due::Later => Outcome::Later,
        Due::In(left) => Outcome::Wait(left),
        Due::Now => {
            let Some(avatar) = words.as_deref().and_then(face) else { return Outcome::Never };
            if settle(notes_file).is_err() || avatar_store::save(&file, &avatar).is_err() {
                return Outcome::Never;
            }
            Outcome::Saved(avatar)
        }
    }
}

/// No fallback icon for this tab from now on: it got one, or the owner chose the letter.
pub fn settle(notes_file: &Path) -> std::io::Result<()> {
    write_atomic(&avatar_fallback_file_beside(notes_file), "")
}

fn modified_ms(file: &Path) -> Option<i64> {
    let since = std::fs::metadata(file).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

#[cfg(test)]
mod tests;
