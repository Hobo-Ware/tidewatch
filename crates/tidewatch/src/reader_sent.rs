//! Messages the Reader knows about before the transcript does: what Tidewatch typed into the agent
//! for the user (so the transcript's copy is not drawn as a "You" bubble) and what the user sent
//! while the agent was busy (so it shows at once, queued, until the transcript has it). Pure.

use std::collections::{HashMap, HashSet};

use crate::pr_card_model::Bubble;

const SLACK_MS: i64 = 60_000;
const QUEUED_EXPIRES_MS: i64 = 15 * 60_000;
const KEPT: usize = 64;
const LOOSE_PREFIX_CHARS: usize = 120;
const PASTE_NOTE: &str = "[Pasted text #";
const PASTE_NOTE_MAX_CHARS: usize = 60;
const HEADLINE_CHARS: usize = 140;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Said {
    pub(crate) text: String,
    pub(crate) label: String,
    pub(crate) at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Queued {
    pub(crate) text: String,
    pub(crate) at_ms: i64,
}

/// One user line of the transcript: its row, its text and when it was written.
pub(crate) type Line<'a> = (u64, &'a str, Option<i64>);

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `seen`, a user line of the transcript, is the same message as `sent` up to spacing.
pub(crate) fn same_text(sent: &str, seen: &str) -> bool {
    squash(sent) == squash(seen)
}

/// Like `same_text`, and also true when the agent kept only the start of a long paste or wrote a
/// `[Pasted text #1 +40 lines]` note for it.
pub(crate) fn same_paste(sent: &str, seen: &str) -> bool {
    let (sent, seen) = (squash(sent), squash(seen));
    if sent == seen {
        return true;
    }
    let long = sent.chars().count() > LOOSE_PREFIX_CHARS;
    let head: String = sent.chars().take(LOOSE_PREFIX_CHARS).collect();
    let note = seen.starts_with(PASTE_NOTE) && seen.chars().count() <= PASTE_NOTE_MAX_CHARS;
    long && (seen.starts_with(&head) || note)
}

#[derive(Debug, Default)]
pub(crate) struct Sent {
    said: Vec<Said>,
    queued: Vec<Queued>,
    claims: HashMap<u64, usize>,
}

impl Sent {
    pub(crate) fn said(&mut self, text: &str, label: &str, at_ms: i64) {
        self.said.push(Said { text: text.to_owned(), label: label.to_owned(), at_ms });
        if self.said.len() > KEPT {
            self.said.remove(0);
        }
    }

    pub(crate) fn queue(&mut self, text: &str, at_ms: i64) {
        self.queued.push(Queued { text: text.to_owned(), at_ms });
    }

    /// Matches the transcript's user lines against what was sent. A queued message the transcript
    /// has is dropped for good; the claims on Tidewatch's messages are worked out again each time.
    pub(crate) fn settle(&mut self, lines: &[Line<'_>]) {
        self.claims.clear();
        let mut taken: HashSet<u64> = HashSet::new();
        for (index, said) in self.said.iter().enumerate() {
            let found = lines.iter().find(|(seq, seen, at)| {
                !taken.contains(seq)
                    && at.is_none_or(|at| at >= said.at_ms - SLACK_MS)
                    && same_paste(&said.text, seen)
            });
            if let Some((seq, _, _)) = found {
                taken.insert(*seq);
                self.claims.insert(*seq, index);
            }
        }
        self.queued.retain(|queued| {
            let found = lines.iter().find(|(seq, seen, at)| {
                !taken.contains(seq)
                    && at.is_none_or(|at| at >= queued.at_ms - SLACK_MS)
                    && same_text(&queued.text, seen)
            });
            match found {
                Some((seq, _, _)) => {
                    taken.insert(*seq);
                    false
                }
                None => true,
            }
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.said.is_empty() && self.queued.is_empty()
    }

    pub(crate) fn said_for(&self, seq: u64) -> Option<&Said> {
        self.claims.get(&seq).and_then(|index| self.said.get(*index))
    }

    /// The messages the transcript does not have yet, oldest first.
    pub(crate) fn waiting(&self, now_ms: i64) -> impl Iterator<Item = &Queued> {
        self.queued.iter().filter(move |queued| now_ms - queued.at_ms < QUEUED_EXPIRES_MS)
    }

    pub(crate) fn waiting_len(&self, now_ms: i64) -> usize {
        self.waiting(now_ms).count()
    }
}

/// The quiet card of a message Tidewatch typed: who, what kind, its first line and how many more.
pub(crate) fn bubble_of(said: &Said) -> Bubble {
    let mut lines = said.text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or_default();
    let more = lines.count();
    let headline: String = first.chars().take(HEADLINE_CHARS).collect();
    let cut = first.chars().count() > HEADLINE_CHARS;
    Bubble {
        who: "Tidewatch",
        tag: said.label.clone(),
        headline: if cut { format!("{headline}...") } else { headline },
        more: (more > 0).then(|| format!("+ {more} lines")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTO_FIX: &str = "@codex commented on PR #7. Answer it.\n\n> <!-- codex-review-state -->\n> | a | b |\n> |---|---|\n> <details>\n> <summary>x</summary>\n> a very long comment line that goes on and on and on and on and on and on and on";

    fn lines<'a>(rows: &'a [(&'a str, Option<i64>)]) -> Vec<Line<'a>> {
        rows.iter().enumerate().map(|(n, (text, at))| (n as u64, *text, *at)).collect()
    }

    #[test]
    fn the_same_words_with_other_spacing_match() {
        assert!(same_text("a  b\n\nc", "a b c\n"));
        assert!(same_text("tab\there", "tab here"));
        assert!(!same_text("a b", "a b c"));
    }

    #[test]
    fn a_long_paste_matches_by_its_start_or_its_paste_note() {
        assert!(same_paste(AUTO_FIX, &AUTO_FIX.replace('\n', "\r\n")));
        assert!(same_paste(AUTO_FIX, "[Pasted text #1 +8 lines]"));
        let head: String = AUTO_FIX.chars().take(150).collect();
        assert!(same_paste(AUTO_FIX, &head));
        assert!(!same_paste("short one", "[Pasted text #1 +8 lines]"));
        assert!(!same_paste(AUTO_FIX, "something else entirely"));
    }

    #[test]
    fn a_message_tidewatch_typed_claims_the_first_matching_line_after_it() {
        let mut sent = Sent::default();
        sent.said(AUTO_FIX, "Pull request", 1_000);
        let rows = [("fix the build", Some(500)), (AUTO_FIX, Some(2_000)), (AUTO_FIX, Some(3_000))];
        sent.settle(&lines(&rows));
        assert_eq!(sent.said_for(0), None);
        assert_eq!(sent.said_for(1).map(|said| said.label.as_str()), Some("Pull request"));
        assert_eq!(sent.said_for(2), None);
    }

    #[test]
    fn a_line_older_than_the_message_is_not_claimed() {
        let mut sent = Sent::default();
        sent.said("hello there", "Ask", 200_000);
        sent.settle(&lines(&[("hello there", Some(1_000))]));
        assert_eq!(sent.said_for(0), None);
    }

    #[test]
    fn what_the_user_typed_stays_theirs() {
        let mut sent = Sent::default();
        sent.said(AUTO_FIX, "Pull request", 1_000);
        sent.settle(&lines(&[("please also fix the docs", Some(2_000))]));
        assert_eq!(sent.said_for(0), None);
    }

    #[test]
    fn a_queued_message_waits_until_the_transcript_has_it() {
        let mut sent = Sent::default();
        sent.queue("do the thing", 1_000);
        sent.queue("and then this", 1_500);
        sent.settle(&lines(&[("something old", Some(10))]));
        assert_eq!(sent.waiting_len(2_000), 2);
        sent.settle(&lines(&[("do the thing", Some(5_000))]));
        let waiting: Vec<&str> = sent.waiting(6_000).map(|q| q.text.as_str()).collect();
        assert_eq!(waiting, ["and then this"]);
        sent.settle(&lines(&[("do the thing", Some(5_000)), ("and  then this", Some(6_000))]));
        assert_eq!(sent.waiting_len(7_000), 0);
    }

    #[test]
    fn the_same_text_sent_twice_is_matched_one_row_each() {
        let mut sent = Sent::default();
        sent.queue("again", 1_000);
        sent.queue("again", 2_000);
        sent.settle(&lines(&[("again", Some(3_000))]));
        assert_eq!(sent.waiting_len(4_000), 1);
    }

    #[test]
    fn a_queued_message_nobody_recorded_stops_showing_after_a_while() {
        let mut sent = Sent::default();
        sent.queue("lost", 0);
        assert_eq!(sent.waiting_len(60_000), 1);
        assert_eq!(sent.waiting_len(QUEUED_EXPIRES_MS), 0);
    }

    #[test]
    fn the_card_shows_the_first_line_and_the_count_of_the_rest() {
        let said = Said { text: AUTO_FIX.to_owned(), label: "Pull request".into(), at_ms: 0 };
        let bubble = bubble_of(&said);
        assert_eq!(bubble.who, "Tidewatch");
        assert_eq!(bubble.tag, "Pull request");
        assert_eq!(bubble.headline, "@codex commented on PR #7. Answer it.");
        assert_eq!(bubble.more.as_deref(), Some("+ 6 lines"));
        let one = Said { text: "just this".into(), label: "Ask".into(), at_ms: 0 };
        assert_eq!(bubble_of(&one).more, None);
    }
}
