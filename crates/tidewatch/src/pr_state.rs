//! What a tab knows about its branch's pull request (PLAN.md, D4): whether one is open, the lines of
//! its diff that take a comment, and whether a note may be posted to it. The lookup is cheap but
//! still a network call, so it runs on tab focus and after a delivery, never every frame.

use kelp_core::review::{Side, Thread};
use tidewatch_core::pr::PullRequest;
use tidewatch_core::pr_lines::PrDiffLines;
use tidewatch_core::tools::GhError;

/// How long a focus change trusts the last answer.
pub(crate) const REFRESH_SECS: f64 = 60.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Lookup {
    #[default]
    Unknown,
    Checking,
    NoPr,
    Open(PullRequest),
    Unavailable(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Why {
    Focus,
    Delivered,
}

#[derive(Debug, Default)]
pub(crate) struct PrState {
    pub(crate) lookup: Lookup,
    checked_at: Option<f64>,
    lines: Option<PrDiffLines>,
    pub(crate) pulling: bool,
}

/// The chip in the Diff pane header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chip {
    pub(crate) text: String,
    pub(crate) url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PostAbility {
    Yes,
    No(String),
}

impl PrState {
    pub(crate) fn pr(&self) -> Option<&PullRequest> {
        match &self.lookup {
            Lookup::Open(pr) => Some(pr),
            _ => None,
        }
    }

    pub(crate) fn chip(&self) -> Option<Chip> {
        let pr = self.pr()?;
        Some(Chip { text: format!("PR #{} open", pr.number), url: pr.url.clone() })
    }

    /// Whether to look now. After a delivery always; on focus when the last answer is stale.
    pub(crate) fn should_find(&self, why: Why, now: f64) -> bool {
        if matches!(self.lookup, Lookup::Checking) {
            return false;
        }
        match why {
            Why::Delivered => true,
            Why::Focus => self.checked_at.is_none_or(|at| now - at >= REFRESH_SECS),
        }
    }

    pub(crate) fn find_started(&mut self) {
        self.lookup = Lookup::Checking;
    }

    /// The answer of `pr::find_open`. The diff lines belong to the pull request they were read for,
    /// so they are dropped when it changes.
    pub(crate) fn found(&mut self, result: Result<Option<PullRequest>, GhError>, now: f64) {
        self.checked_at = Some(now);
        let previous = self.pr().cloned();
        self.lookup = match result {
            Ok(Some(pr)) => Lookup::Open(pr),
            Ok(None) => Lookup::NoPr,
            Err(error) => Lookup::Unavailable(error.to_string()),
        };
        if self.pr() != previous.as_ref() {
            self.lines = None;
        }
    }

    pub(crate) fn lines_loaded(&mut self, lines: Option<PrDiffLines>) {
        self.lines = lines;
    }

    pub(crate) fn lines_known(&self) -> bool {
        self.lines.is_some()
    }

    pub(crate) fn needs_lines(&self) -> bool {
        self.pr().is_some() && self.lines.is_none()
    }

    /// Whether `thread` may be posted, and if not why. A thread already on GitHub is answered with a
    /// reply, which has no line to check. With the diff lines unknown the post is allowed and gh's
    /// own message is shown if GitHub refuses.
    pub(crate) fn post_ability(&self, thread: &Thread, linked: bool) -> PostAbility {
        if self.pr().is_none() {
            return PostAbility::No("This branch has no open pull request".into());
        }
        if linked {
            return PostAbility::Yes;
        }
        let Some(lines) = &self.lines else { return PostAbility::Yes };
        let (first, last) = (thread.anchor.line_hint, thread.anchor.last_line_hint());
        let outside = [first, last]
            .into_iter()
            .find(|line| *line > 0 && !lines.contains(&thread.path, thread.anchor.side, *line));
        if let Some(line) = outside {
            let side = if thread.anchor.side == Side::Old { "old " } else { "" };
            return PostAbility::No(format!(
                "{side}line {line} of {} is not in the pull request's diff. GitHub only takes comments on changed lines and the lines around them",
                thread.path
            ));
        }
        PostAbility::Yes
    }
}

/// `owner/repo` from a pull request URL, for the confirm dialog.
pub(crate) fn repo_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let mut parts = rest.split('/');
    let _host = parts.next()?;
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment};

    fn pr(number: u64) -> PullRequest {
        PullRequest {
            number,
            url: format!("https://github.com/Hobo-Ware/demo/pull/{number}"),
            head_sha: "abc".into(),
        }
    }

    fn thread(path: &str, line: u32, side: Side) -> Thread {
        Thread {
            id: 1,
            path: path.into(),
            commit: None,
            anchor: Anchor {
                side,
                line_hint: line,
                text: String::new(),
                before: vec![],
                after: vec![],
                span: 0,
                block: vec![],
            },
            resolved: false,
            comments: vec![Comment { author: "You".into(), time: 0, body: "x".into() }],
        }
    }

    fn open_with_lines() -> PrState {
        let mut state = PrState::default();
        state.found(Ok(Some(pr(12))), 0.0);
        state.lines_loaded(Some(PrDiffLines::parse(
            "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,3 @@\n a\n+b\n c\n",
        )));
        state
    }

    #[test]
    fn the_chip_names_the_number_and_state_only_while_a_pull_request_is_open() {
        let mut state = PrState::default();
        assert_eq!(state.chip(), None);
        state.found(Ok(Some(pr(12))), 0.0);
        assert_eq!(
            state.chip(),
            Some(Chip {
                text: "PR #12 open".into(),
                url: "https://github.com/Hobo-Ware/demo/pull/12".into()
            })
        );
        state.found(Ok(None), 1.0);
        assert_eq!(state.chip(), None);
        state.found(Err(GhError::Missing), 2.0);
        assert_eq!(state.chip(), None);
        assert!(matches!(&state.lookup, Lookup::Unavailable(m) if m.contains("not installed")));
    }

    #[test]
    fn the_lookup_runs_on_first_focus_then_waits_out_the_refresh_and_always_follows_a_delivery() {
        let mut state = PrState::default();
        assert!(state.should_find(Why::Focus, 5.0));
        state.find_started();
        assert!(!state.should_find(Why::Focus, 6.0));
        assert!(!state.should_find(Why::Delivered, 6.0));
        state.found(Ok(None), 10.0);
        assert!(!state.should_find(Why::Focus, 10.0 + REFRESH_SECS - 1.0));
        assert!(state.should_find(Why::Focus, 10.0 + REFRESH_SECS));
        assert!(state.should_find(Why::Delivered, 11.0));
    }

    #[test]
    fn a_different_pull_request_drops_the_diff_lines_and_the_same_one_keeps_them() {
        let mut state = open_with_lines();
        assert!(!state.needs_lines());
        state.found(Ok(Some(pr(12))), 5.0);
        assert!(!state.needs_lines());
        state.found(Ok(Some(pr(13))), 6.0);
        assert!(state.needs_lines());
    }

    #[test]
    fn a_note_on_a_changed_line_may_be_posted_and_one_outside_the_diff_may_not() {
        let state = open_with_lines();
        assert_eq!(state.post_ability(&thread("src/a.rs", 2, Side::New), false), PostAbility::Yes);
        let PostAbility::No(reason) = state.post_ability(&thread("src/a.rs", 40, Side::New), false)
        else {
            panic!("line 40 is outside the diff");
        };
        assert!(
            reason.contains("line 40 of src/a.rs is not in the pull request's diff"),
            "{reason}"
        );
        assert!(matches!(
            state.post_ability(&thread("src/other.rs", 1, Side::New), false),
            PostAbility::No(_)
        ));
    }

    #[test]
    fn a_block_note_needs_both_its_first_and_its_last_line_in_the_diff() {
        let state = open_with_lines();
        let block = |line, span| {
            let mut t = thread("src/a.rs", line, Side::New);
            t.anchor.span = span;
            t
        };
        assert_eq!(state.post_ability(&block(1, 2), false), PostAbility::Yes);
        let PostAbility::No(reason) = state.post_ability(&block(2, 9), false) else {
            panic!("line 11 is outside the diff");
        };
        assert!(reason.contains("line 11 of src/a.rs"), "{reason}");
        let PostAbility::No(reason) = state.post_ability(&block(30, 1), false) else {
            panic!("line 30 is outside the diff");
        };
        assert!(reason.contains("line 30 of src/a.rs"), "{reason}");
    }

    #[test]
    fn a_reply_to_a_thread_already_on_github_needs_no_line_and_unknown_lines_allow_the_post() {
        let state = open_with_lines();
        assert_eq!(
            state.post_ability(&thread("elsewhere.rs", 99, Side::New), true),
            PostAbility::Yes
        );
        let mut unknown = PrState::default();
        unknown.found(Ok(Some(pr(1))), 0.0);
        assert_eq!(unknown.post_ability(&thread("a.rs", 99, Side::New), false), PostAbility::Yes);
    }

    #[test]
    fn without_a_pull_request_nothing_can_be_posted() {
        let state = PrState::default();
        assert!(matches!(
            state.post_ability(&thread("a.rs", 1, Side::New), false),
            PostAbility::No(_)
        ));
    }

    #[test]
    fn the_repo_comes_from_the_pull_request_url() {
        assert_eq!(
            repo_of("https://github.com/Hobo-Ware/demo/pull/12").as_deref(),
            Some("Hobo-Ware/demo")
        );
        assert_eq!(repo_of("https://ghe.example.com/o/r/pull/1").as_deref(), Some("o/r"));
        assert_eq!(repo_of("https://github.com/"), None);
        assert_eq!(repo_of(""), None);
    }
}
