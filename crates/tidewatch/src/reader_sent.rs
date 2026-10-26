//! Messages the Reader knows about before the transcript does: what Tidewatch typed into the agent
//! (so the transcript's copy is drawn as Tidewatch's, not as a "You" bubble) and what the user sent
//! while the agent was busy (so it shows at once, queued, until the transcript has it). Pure.

use std::collections::{HashMap, HashSet};

use tidewatch_core::sent::{Reason, Record};

const SLACK_MS: i64 = 60_000;
const WINDOW_MS: i64 = 10 * 60_000;
const QUEUED_EXPIRES_MS: i64 = 15 * 60_000;
const KEPT: usize = 200;
const LOOSE_PREFIX_CHARS: usize = 120;
const PASTE_NOTE: &str = "[Pasted text #";
const PASTE_NOTE_MAX_CHARS: usize = 60;

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fit {
    Exact,
    Loose,
}

fn fit(record: &Record, line: &Line<'_>) -> Option<Fit> {
    let (_, seen, at) = line;
    if at.is_some_and(|at| at < record.at_ms - SLACK_MS) {
        return None;
    }
    if same_text(&record.text, seen) {
        return Some(Fit::Exact);
    }
    let inside = at.is_none_or(|at| at <= record.at_ms + WINDOW_MS);
    (inside && same_paste(&record.text, seen)).then_some(Fit::Loose)
}

fn distance(record: &Record, at: Option<i64>) -> i64 {
    at.map_or(WINDOW_MS, |at| (at - record.at_ms).abs())
}

#[derive(Debug, Default)]
pub(crate) struct Sent {
    records: Vec<Record>,
    queued: Vec<Queued>,
    claims: HashMap<u64, usize>,
    fresh: Vec<(u64, u64)>,
}

impl Sent {
    pub(crate) fn said(&mut self, record: Record) {
        if self.records.iter().any(|known| known.id == record.id) {
            return;
        }
        let at = self.records.partition_point(|known| known.at_ms <= record.at_ms);
        self.records.insert(at, record);
        if self.records.len() > KEPT {
            self.records.remove(0);
        }
    }

    pub(crate) fn queue(&mut self, text: &str, at_ms: i64) {
        self.queued.push(Queued { text: text.to_owned(), at_ms });
    }

    pub(crate) fn take_fresh(&mut self) -> Vec<(u64, u64)> {
        std::mem::take(&mut self.fresh)
    }

    /// Matches the transcript's user lines against what was sent. A queued message the transcript
    /// has is dropped for good.
    pub(crate) fn settle(&mut self, lines: &[Line<'_>]) {
        self.claims.clear();
        let mut taken: HashSet<u64> = HashSet::new();
        for (index, record) in self.records.iter_mut().enumerate() {
            if record.timed() {
                continue;
            }
            let found = match record.claimed_seq {
                Some(seq) => lines.iter().find(|line| {
                    line.0 == seq && !taken.contains(&seq) && fit(record, line).is_some()
                }),
                None => lines
                    .iter()
                    .filter(|line| !taken.contains(&line.0))
                    .filter_map(|line| Some((fit(record, line)?, line)))
                    .min_by_key(|(fit, line)| (*fit == Fit::Loose, distance(record, line.2)))
                    .map(|(_, line)| line),
            };
            let Some(&(seq, _, _)) = found else { continue };
            taken.insert(seq);
            self.claims.insert(seq, index);
            if record.claimed_seq != Some(seq) {
                record.claimed_seq = Some(seq);
                self.fresh.push((record.id, seq));
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
        self.records.is_empty() && self.queued.is_empty()
    }

    pub(crate) fn said_for(&self, seq: u64) -> Option<&Record> {
        self.claims.get(&seq).and_then(|index| self.records.get(*index))
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.records.iter().map(|record| record.id)
    }

    /// Whether an auto-fix record holds `text`, a message an older PR card also drew.
    pub(crate) fn told_auto_fix(&self, text: &str) -> bool {
        self.records
            .iter()
            .any(|record| record.reason == Reason::AutoFix && same_paste(&record.text, text))
    }

    pub(crate) fn timed(&self) -> impl Iterator<Item = &Record> {
        self.records.iter().filter(|record| record.timed())
    }

    /// The messages the transcript does not have yet, oldest first.
    pub(crate) fn waiting(&self, now_ms: i64) -> impl Iterator<Item = &Queued> {
        self.queued.iter().filter(move |queued| now_ms - queued.at_ms < QUEUED_EXPIRES_MS)
    }

    pub(crate) fn waiting_len(&self, now_ms: i64) -> usize {
        self.waiting(now_ms).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTO_FIX: &str = "@codex commented on PR #7. Answer it.\n\n> <!-- codex-review-state -->\n> | a | b |\n> |---|---|\n> <details>\n> <summary>x</summary>\n> a very long comment line that goes on and on and on and on and on and on and on";

    fn lines<'a>(rows: &'a [(&'a str, Option<i64>)]) -> Vec<Line<'a>> {
        rows.iter().enumerate().map(|(n, (text, at))| (n as u64, *text, *at)).collect()
    }

    fn record(text: &str, at_ms: i64) -> Record {
        Record { at_ms, ..Record::new(Reason::AutoFix, "a rule fired", text, None) }
    }

    fn sent_with(records: Vec<Record>) -> Sent {
        let mut sent = Sent::default();
        for record in records {
            sent.said(record);
        }
        sent
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
    fn a_record_claims_the_exact_line_written_after_it() {
        let mut sent = sent_with(vec![record(AUTO_FIX, 1_000)]);
        let rows = [("fix the build", Some(500)), (AUTO_FIX, Some(2_000)), (AUTO_FIX, Some(3_000))];
        sent.settle(&lines(&rows));
        assert_eq!(sent.said_for(0), None);
        assert_eq!(sent.said_for(1).map(|said| said.reason), Some(Reason::AutoFix));
        assert_eq!(sent.said_for(2), None);
    }

    #[test]
    fn exact_words_win_over_a_nearer_cut_copy_and_the_nearest_exact_line_wins() {
        let cut: String = AUTO_FIX.chars().take(150).collect();
        let mut sent = sent_with(vec![record(AUTO_FIX, 1_000)]);
        sent.settle(&lines(&[(&cut, Some(1_100)), (AUTO_FIX, Some(9_000))]));
        assert_eq!(sent.said_for(0), None);
        assert!(sent.said_for(1).is_some());
        let mut sent = sent_with(vec![record("again", 10_000)]);
        sent.settle(&lines(&[("again", Some(9_500)), ("again", Some(10_100))]));
        assert!(sent.said_for(1).is_some());
        assert_eq!(sent.said_for(0), None);
    }

    #[test]
    fn a_cut_or_placeholder_copy_counts_only_inside_the_window_after_the_record() {
        let cut: String = AUTO_FIX.chars().take(150).collect();
        let mut sent = sent_with(vec![record(AUTO_FIX, 1_000)]);
        sent.settle(&lines(&[(&cut, Some(1_000 + WINDOW_MS + 1))]));
        assert_eq!(sent.said_for(0), None);
        let mut sent = sent_with(vec![record(AUTO_FIX, 1_000)]);
        let placeholder = format!("{PASTE_NOTE}1 +8 lines]");
        sent.settle(&lines(&[(&placeholder, Some(1_000 + WINDOW_MS - 1))]));
        assert!(sent.said_for(0).is_some());
    }

    #[test]
    fn a_line_older_than_the_record_is_not_claimed() {
        let mut sent = sent_with(vec![record("hello there", 200_000)]);
        sent.settle(&lines(&[("hello there", Some(1_000))]));
        assert_eq!(sent.said_for(0), None);
    }

    #[test]
    fn what_the_user_typed_stays_theirs() {
        let mut sent = sent_with(vec![record(AUTO_FIX, 1_000)]);
        sent.settle(&lines(&[("please also fix the docs", Some(2_000))]));
        assert_eq!(sent.said_for(0), None);
    }

    #[test]
    fn a_claim_is_kept_and_never_taken_again_after_a_replay() {
        let mut sent = sent_with(vec![record("fix it", 1_000)]);
        let rows = [("fix it", Some(1_100)), ("fix it", Some(1_200))];
        sent.settle(&lines(&rows));
        let fresh = sent.take_fresh();
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].1, 0);
        sent.settle(&lines(&rows));
        assert_eq!(sent.take_fresh(), []);
        let mut replayed =
            sent_with(vec![Record { claimed_seq: Some(1), ..record("fix it", 1_000) }]);
        replayed.settle(&lines(&rows));
        assert_eq!(replayed.said_for(0), None);
        assert!(replayed.said_for(1).is_some());
        assert_eq!(replayed.take_fresh(), []);
    }

    #[test]
    fn a_record_whose_row_was_dropped_does_not_claim_a_later_copy() {
        let mut sent = sent_with(vec![Record { claimed_seq: Some(3), ..record("fix it", 1_000) }]);
        sent.settle(&lines(&[("fix it", Some(1_500))]));
        assert_eq!(sent.said_for(0), None);
    }

    #[test]
    fn two_records_with_one_text_take_one_line_each() {
        let mut sent = sent_with(vec![record("again", 1_000), record("again", 2_000)]);
        sent.settle(&lines(&[("again", Some(1_100)), ("again", Some(2_100))]));
        assert!(sent.said_for(0).is_some());
        assert!(sent.said_for(1).is_some());
        assert_ne!(sent.said_for(0), sent.said_for(1));
    }

    #[test]
    fn a_record_told_through_a_hook_claims_no_line_and_is_listed_as_timed() {
        let told = Record {
            at_ms: 1_000,
            ..Record::new(Reason::LedgerGuard, "the ledger was not updated", "update it", None)
        };
        let mut sent = sent_with(vec![told, record("fix it", 2_000)]);
        sent.settle(&lines(&[("update it", Some(1_100))]));
        assert_eq!(sent.said_for(0), None);
        let timed: Vec<Reason> = sent.timed().map(|record| record.reason).collect();
        assert_eq!(timed, [Reason::LedgerGuard]);
    }

    #[test]
    fn a_ledger_reminder_typed_into_the_terminal_claims_its_transcript_line() {
        let typed = Record {
            at_ms: 1_000,
            typed: true,
            ..Record::new(Reason::LedgerGuard, "the ledger was not updated", "update it", None)
        };
        let mut sent = sent_with(vec![typed]);
        sent.settle(&lines(&[("update it", Some(1_100))]));
        assert_eq!(sent.said_for(0).map(|record| record.reason), Some(Reason::LedgerGuard));
        assert_eq!(sent.timed().count(), 0);
    }

    #[test]
    fn the_same_record_added_twice_is_one_record() {
        let one = record("fix it", 1_000);
        let mut sent = sent_with(vec![one.clone()]);
        sent.said(one);
        sent.settle(&lines(&[("fix it", Some(1_100)), ("fix it", Some(1_200))]));
        assert!(sent.said_for(0).is_some());
        assert_eq!(sent.said_for(1), None);
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
}
