//! The pull request cards of one tab, kept in `.state/<repo-dir>/<tab>/pr_cards.json` next to its
//! notes (PLAN.md, D4 follow-up).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::pr_noise::is_noise;
use crate::pr_status::{Body, Seen, Summary};
use crate::prefs::GithubPrefs;

/// Older cards are dropped past this, so a long-lived tab keeps a small file.
pub const MAX_CARDS: usize = 80;
/// Auto-fix sends at most this many times per pull request, then the tab falls back to asking.
pub const MAX_AUTO_SENDS: u32 = 3;
/// A tab that runs on its own sends this many times per pull request, then pauses without asking.
pub const MAX_AUTONOMOUS_SENDS: u32 = 10;

/// The loop guard for a tab: how many auto-fix sends one pull request gets.
#[must_use]
pub fn send_cap(autonomous: bool) -> u32 {
    if autonomous { MAX_AUTONOMOUS_SENDS } else { MAX_AUTO_SENDS }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: u64,
    pub at_ms: i64,
    pub body: Body,
    #[serde(default)]
    pub handled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cards {
    #[serde(default)]
    items: Vec<Card>,
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub seen: Option<Seen>,
    #[serde(default)]
    pub summary: Option<Summary>,
    #[serde(default)]
    pub policy: Option<GithubPrefs>,
    #[serde(default)]
    pub auto_sends: u32,
    #[serde(default)]
    pub fixing: bool,
    /// Commits a tab that runs on its own left on the branch after its pull request merged.
    #[serde(default)]
    pub after_merge: usize,
}

impl Cards {
    #[must_use]
    pub fn load(file: &Path) -> Self {
        let Ok(bytes) = std::fs::read(file) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            eprintln!(
                "tidewatch: {} is unreadable ({e}); starting with no PR cards",
                file.display()
            );
            Self::default()
        })
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_atomic(file, &text)
    }

    #[must_use]
    pub fn cards(&self) -> &[Card] {
        &self.items
    }

    #[must_use]
    pub fn get(&self, id: u64) -> Option<&Card> {
        self.items.iter().find(|card| card.id == id)
    }

    #[must_use]
    pub fn policy_or(&self, default: GithubPrefs) -> GithubPrefs {
        self.policy.unwrap_or(default)
    }

    pub fn follow(&mut self, number: u64) {
        if self.number == Some(number) {
            return;
        }
        self.number = Some(number);
        self.seen = None;
        self.summary = None;
        self.auto_sends = 0;
        self.fixing = false;
        self.after_merge = 0;
    }

    /// The latest card that ended a pull request says it merged, not that it was closed.
    #[must_use]
    pub fn ended_merged(&self) -> bool {
        self.items
            .iter()
            .rev()
            .find(|card| matches!(card.body, Body::Merged { .. } | Body::Closed { .. }))
            .is_some_and(|card| matches!(card.body, Body::Merged { .. }))
    }

    pub fn add(&mut self, at_ms: i64, body: Body) -> u64 {
        match &body {
            Body::ChecksRunning { .. } => {
                if let Some(card) = self
                    .items
                    .iter_mut()
                    .rev()
                    .find(|card| matches!(card.body, Body::ChecksRunning { .. }))
                {
                    card.body = body;
                    return card.id;
                }
            }
            Body::ChecksFailed { .. } | Body::ChecksPassed { .. } => {
                self.items.retain(|card| !matches!(card.body, Body::ChecksRunning { .. }));
            }
            Body::Merged { .. } | Body::Closed { .. } => {
                self.items.retain(|card| {
                    !matches!(card.body, Body::ChecksRunning { .. } | Body::PushFix { .. })
                });
            }
            Body::PushFix { .. } => {
                self.items.retain(|card| !matches!(card.body, Body::PushFix { .. }));
            }
            _ => {}
        }
        self.next_id += 1;
        let card = Card { id: self.next_id, at_ms, body, handled: false };
        let at = self.items.iter().rposition(|c| c.at_ms <= at_ms).map_or(0, |i| i + 1);
        self.items.insert(at, card);
        if self.items.len() > MAX_CARDS {
            self.items.remove(0);
        }
        self.next_id
    }

    pub fn drop_pushed_fix(&mut self, remote_head: &str) {
        self.items.retain(
            |card| !matches!(&card.body, Body::PushFix { head, .. } if head == remote_head),
        );
    }

    /// Drops comment cards saved before noise was filtered, such as a review bot's receipt.
    pub fn drop_noise(&mut self) {
        self.items.retain(|card| {
            !matches!(&card.body, Body::Commented { author, text, .. } if is_noise(author, text))
        });
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|card| card.id != id);
    }

    pub fn handle(&mut self, id: u64) {
        if let Some(card) = self.items.iter_mut().find(|card| card.id == id) {
            card.handled = true;
        }
    }

    #[must_use]
    pub fn is_current(&self, index: usize) -> bool {
        let Some(card) = self.items.get(index) else { return false };
        if card.handled {
            return false;
        }
        let later = &self.items[index + 1..];
        let ended =
            later.iter().any(|c| matches!(c.body, Body::Merged { .. } | Body::Closed { .. }));
        match &card.body {
            Body::ChecksFailed { .. } => {
                !ended
                    && !later.iter().any(|c| {
                        matches!(
                            c.body,
                            Body::ChecksRunning { .. }
                                | Body::ChecksFailed { .. }
                                | Body::ChecksPassed { .. }
                        )
                    })
            }
            Body::ChangesRequested { .. } | Body::Approved { .. } => {
                !ended
                    && !later.iter().any(|c| {
                        matches!(c.body, Body::ChangesRequested { .. } | Body::Approved { .. })
                    })
            }
            Body::Commented { .. } | Body::PushFix { .. } => !ended,
            Body::Conflict { .. } => {
                !ended
                    && self.seen.as_ref().is_some_and(|seen| seen.conflicting)
                    && !later.iter().any(|c| matches!(c.body, Body::Conflict { .. }))
            }
            Body::Merged { .. } | Body::Closed { .. } => true,
            Body::Opened { .. }
            | Body::ChecksRunning { .. }
            | Body::ChecksPassed { .. }
            | Body::Sent { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_status::{Job, Via};

    fn running(done: usize) -> Body {
        Body::ChecksRunning { head: "a".into(), done, total: 2 }
    }

    fn failed() -> Body {
        Body::ChecksFailed {
            head: "a".into(),
            jobs: vec![Job {
                name: "check".into(),
                passed: false,
                step: None,
                secs: None,
                url: String::new(),
                run_id: None,
                job_id: None,
                log: Vec::new(),
            }],
        }
    }

    fn kinds(cards: &Cards) -> Vec<&'static str> {
        cards
            .cards()
            .iter()
            .map(|card| match card.body {
                Body::ChecksRunning { .. } => "running",
                Body::ChecksFailed { .. } => "failed",
                Body::ChecksPassed { .. } => "passed",
                Body::ChangesRequested { .. } => "changes",
                Body::Approved { .. } => "approved",
                Body::Conflict { .. } => "conflict",
                Body::Merged { .. } => "merged",
                Body::PushFix { .. } => "push-fix",
                Body::Commented { .. } => "commented",
                Body::Opened { .. } => "opened",
                Body::Closed { .. } => "closed",
                Body::Sent { .. } => "sent",
            })
            .collect()
    }

    fn opened() -> Body {
        Body::Opened {
            number: 1,
            title: "t".into(),
            base: "main".into(),
            branch: "b".into(),
            commits: None,
        }
    }

    #[test]
    fn running_checks_update_one_card_and_vanish_when_they_end() {
        let mut cards = Cards::default();
        let first = cards.add(10, running(0));
        assert_eq!(cards.add(20, running(1)), first, "progress edits the live card");
        assert_eq!(cards.cards().len(), 1);
        assert_eq!(cards.cards()[0].at_ms, 10, "it keeps the time it first appeared");
        cards.add(30, failed());
        assert_eq!(kinds(&cards), ["failed"]);
        cards.add(40, running(0));
        assert_eq!(kinds(&cards), ["failed", "running"]);
    }

    #[test]
    fn cards_stay_in_time_order_and_a_late_one_slots_in() {
        let mut cards = Cards::default();
        cards.add(30, failed());
        cards.add(10, opened());
        cards.add(30, Body::Conflict { base: "main".into() });
        let times: Vec<i64> = cards.cards().iter().map(|c| c.at_ms).collect();
        assert_eq!(times, [10, 30, 30]);
        assert_eq!(
            kinds(&cards),
            ["opened", "failed", "conflict"],
            "equal times keep the order they came in"
        );
    }

    #[test]
    fn a_new_push_fix_offer_replaces_the_old_one_and_a_matching_head_removes_it() {
        let mut cards = Cards::default();
        cards.add(10, Body::PushFix { commits: 1, head: "x".into() });
        cards.add(20, Body::PushFix { commits: 2, head: "y".into() });
        assert_eq!(kinds(&cards), ["push-fix"]);
        cards.drop_pushed_fix("x");
        assert_eq!(kinds(&cards), ["push-fix"]);
        cards.drop_pushed_fix("y");
        assert_eq!(kinds(&cards), Vec::<&str>::new());
    }

    #[test]
    fn merging_clears_the_live_cards() {
        let mut cards = Cards::default();
        cards.add(10, running(0));
        cards.add(11, Body::PushFix { commits: 1, head: "x".into() });
        cards.add(20, Body::Merged { number: 1, base: "main".into() });
        assert_eq!(kinds(&cards), ["merged"]);
    }

    #[test]
    fn a_tab_that_runs_on_its_own_gets_a_longer_loop_guard() {
        assert_eq!(send_cap(false), MAX_AUTO_SENDS);
        assert_eq!(send_cap(true), MAX_AUTONOMOUS_SENDS);
    }

    #[test]
    fn saved_noise_cards_go_and_real_comments_stay() {
        let mut cards = Cards::default();
        let comment = |author: &str, text: &str| Body::Commented {
            trusted: true,
            author: author.into(),
            text: text.into(),
            url: String::new(),
        };
        cards.add(10, comment("chatgpt-codex-connector", "<!-- codex-review-receipt:v1 -->"));
        cards.add(20, comment("reviewer", "Please rename this."));
        cards.add(30, failed());
        cards.drop_noise();
        assert_eq!(kinds(&cards), ["commented", "failed"]);
    }

    #[test]
    fn a_saved_codex_status_card_goes_on_load_whoever_it_was_saved_for() {
        let status = "<!-- codex-pull-request-review-summary -->\n<!-- codex-review-state:v1 {\"code_review\":{\"status\":\"running\"}} -->\n\n## Codex Review Summary\n\n| Review | Status |\n| --- | --- |\n| Code Review | Running |";
        let mut cards = Cards::default();
        for (n, author) in ["chatgpt-codex-connector", "someone"].into_iter().enumerate() {
            cards.add(
                n as i64,
                Body::Commented {
                    trusted: true,
                    author: author.into(),
                    text: status.into(),
                    url: String::new(),
                },
            );
        }
        cards.add(
            5,
            Body::Commented {
                trusted: true,
                author: "chatgpt-codex-connector".into(),
                text: "The retry loop never resets its counter.".into(),
                url: String::new(),
            },
        );
        cards.drop_noise();
        assert_eq!(kinds(&cards), ["commented"]);
        assert_eq!(cards.cards()[0].at_ms, 5);
    }

    #[test]
    fn the_oldest_cards_are_dropped_past_the_cap() {
        let mut cards = Cards::default();
        for n in 0..(MAX_CARDS + 5) {
            cards.add(
                n as i64,
                Body::Commented {
                    trusted: true,
                    author: "a".into(),
                    text: n.to_string(),
                    url: String::new(),
                },
            );
        }
        assert_eq!(cards.cards().len(), MAX_CARDS);
        assert_eq!(cards.cards()[0].at_ms, 5);
    }

    #[test]
    fn buttons_belong_to_the_newest_card_of_their_kind_until_the_story_moves_on() {
        let mut cards = Cards::default();
        cards.add(10, failed());
        assert!(cards.is_current(0));
        cards.add(20, running(0));
        assert!(!cards.is_current(0), "a new run overtakes the old failure");
        cards.add(
            30,
            Body::ChangesRequested {
                trusted: true,
                author: "r".into(),
                body: String::new(),
                comments: vec![],
            },
        );
        assert!(cards.is_current(2));
        cards.add(40, Body::Approved { author: "r".into() });
        assert!(!cards.is_current(2));
        assert!(cards.is_current(3));
        cards.add(50, Body::Merged { number: 1, base: "main".into() });
        assert_eq!(
            kinds(&cards),
            ["failed", "changes", "approved", "merged"],
            "the live run is gone"
        );
        assert!(!cards.is_current(2), "nothing is open once it merged");
        assert!(cards.is_current(3));
    }

    #[test]
    fn a_conflict_card_is_current_only_while_the_pull_request_conflicts() {
        let mut cards = Cards::default();
        cards.add(10, Body::Conflict { base: "main".into() });
        assert!(!cards.is_current(0), "nothing seen yet says it conflicts");
        cards.seen = Some(Seen { conflicting: true, ..Seen::default() });
        assert!(cards.is_current(0));
        cards.seen = Some(Seen { conflicting: false, ..Seen::default() });
        assert!(!cards.is_current(0), "resolved on GitHub");
    }

    #[test]
    fn a_handled_card_has_no_buttons() {
        let mut cards = Cards::default();
        let id = cards.add(10, Body::Merged { number: 1, base: "main".into() });
        assert!(cards.is_current(0));
        cards.handle(id);
        assert!(!cards.is_current(0));
        assert!(!cards.is_current(9), "no such card");
    }

    #[test]
    fn following_another_pull_request_forgets_the_look_but_keeps_the_history() {
        let mut cards = Cards::default();
        cards.follow(1);
        cards.seen = Some(Seen::default());
        cards.auto_sends = 2;
        cards.fixing = true;
        cards.add(10, opened());
        cards.follow(1);
        assert_eq!((cards.seen.is_some(), cards.auto_sends, cards.fixing), (true, 2, true));
        cards.follow(2);
        assert_eq!(
            (cards.number, cards.seen.is_some(), cards.auto_sends, cards.fixing),
            (Some(2), false, 0, false)
        );
        assert_eq!(kinds(&cards), ["opened"]);
    }

    #[test]
    fn the_policy_is_the_default_until_the_tab_chooses() {
        let mut cards = Cards::default();
        let default = GithubPrefs::default();
        assert_eq!(cards.policy_or(default), default);
        let own = GithubPrefs { conflicts: false, ..default };
        cards.policy = Some(own);
        assert_eq!(cards.policy_or(default), own);
    }

    #[test]
    fn a_saved_timeline_loads_back_and_a_bad_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("pr_cards.json");
        assert_eq!(Cards::load(&file), Cards::default());
        let mut cards = Cards::default();
        cards.follow(42);
        cards.add(10, opened());
        cards.add(
            20,
            Body::Sent {
                via: Via::You,
                tag: "CI failure".into(),
                headline: "h".into(),
                extra_lines: 3,
                text: "t".into(),
            },
        );
        cards.save(&file).unwrap();
        assert_eq!(Cards::load(&file), cards);
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(Cards::load(&file), Cards::default());
    }
}
