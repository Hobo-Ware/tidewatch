//! Looking up the state of a pull request or an issue a tab holds a link to, with
//! `gh pr view <url> --json state` or `gh issue view <url> --json state`. It blocks until `gh`
//! exits: call it from a background thread, never the UI thread.

use std::path::Path;

use crate::link_board::{Kind, LinkState, kind_of};
use crate::tools::{Tools, run_gh};

/// The state of the pull request or issue at `url`, run from `dir`. Anything else, and any `gh`
/// failure, is `Unknown`.
#[must_use]
pub fn state_of(tools: &Tools, dir: &Path, url: &str) -> LinkState {
    let noun = match kind_of(url) {
        Kind::Pr => "pr",
        Kind::Issue => "issue",
        _ => return LinkState::Unknown,
    };
    run_gh(tools, dir, &[noun, "view", url, "--json", "state"])
        .map_or(LinkState::Unknown, |out| parse_state(&out))
}

#[must_use]
pub fn parse_state(json: &str) -> LinkState {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| value.get("state")?.as_str().map(LinkState::from_gh))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
