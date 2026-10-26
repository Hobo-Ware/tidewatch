//! Where the PR cards sit among the Reader's rows: by time, the way a chat reads. A card goes before
//! the first row written after it. The user message Tidewatch pasted shows as its own bubble card, so
//! the transcript's copy of the same text is left out. Pure.

use tidewatch_core::pr_cards::Card;
use tidewatch_core::pr_status::{Body, Via};

use crate::reader_rows::{Kind, Rows};
use crate::reader_sent;

/// How far before a card the transcript's copy of its message may be stamped (the clocks of two
/// processes).
const SLACK_MS: i64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// The n-th row of the transcript.
    Row(usize),
    /// The n-th card of the timeline.
    Card(usize),
    Said(usize),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RowInfo<'a> {
    pub(crate) at_ms: Option<i64>,
    pub(crate) user_text: Option<&'a str>,
}

/// The order to draw `rows` and `cards` in. `cards` are sorted by time.
#[cfg(test)]
pub(crate) fn merge(rows: &[RowInfo<'_>], cards: &[Card]) -> Vec<Place> {
    merge_with(rows, cards, &[], &vec![false; cards.len()])
}

/// `merge` with the times of the messages a hook sent, sorted, placed like cards, and the cards
/// to leave out because Tidewatch's own record of the same message draws instead.
pub(crate) fn merge_with(
    rows: &[RowInfo<'_>],
    cards: &[Card],
    told: &[i64],
    shadowed: &[bool],
) -> Vec<Place> {
    let hidden = copies_of_sent(rows, cards, shadowed);
    let mut timeline: Vec<(i64, Place)> = cards
        .iter()
        .enumerate()
        .filter(|(n, _)| !shadowed[*n])
        .map(|(n, card)| (card.at_ms, Place::Card(n)))
        .chain(told.iter().enumerate().map(|(n, at)| (*at, Place::Said(n))))
        .collect();
    timeline.sort_by_key(|(at, _)| *at);
    let mut places = Vec::with_capacity(rows.len() + timeline.len());
    let mut next = 0;
    let mut stamp = i64::MIN;
    for (index, row) in rows.iter().enumerate() {
        stamp = row.at_ms.unwrap_or(stamp);
        while next < timeline.len() && timeline[next].0 < stamp {
            places.push(timeline[next].1);
            next += 1;
        }
        if !hidden[index] {
            places.push(Place::Row(index));
        }
    }
    places.extend(timeline[next..].iter().map(|(_, place)| *place));
    places
}

/// The rows to leave out: for each sent message, the first user row after it with the same text.
fn copies_of_sent(rows: &[RowInfo<'_>], cards: &[Card], shadowed: &[bool]) -> Vec<bool> {
    let mut hidden = vec![false; rows.len()];
    for (card, _) in cards.iter().zip(shadowed).filter(|(_, shadowed)| !**shadowed) {
        let Body::Sent { text, .. } = &card.body else { continue };
        let found = rows.iter().enumerate().find(|(index, row)| {
            !hidden[*index]
                && row.user_text.is_some_and(|said| reader_sent::same_paste(text, said))
                && row.at_ms.is_none_or(|at| at >= card.at_ms - SLACK_MS)
        });
        if let Some((index, _)) = found {
            hidden[index] = true;
        }
    }
    hidden
}

/// `merge` over the Reader's own rows.
pub(crate) fn places(rows: &Rows, cards: &[Card]) -> Vec<Place> {
    let infos: Vec<RowInfo<'_>> = rows
        .stamped()
        .map(|(row, at_ms)| RowInfo {
            at_ms,
            user_text: matches!(row.kind, Kind::User).then_some(row.text.as_str()),
        })
        .collect();
    let told: Vec<i64> = rows.timed().iter().map(|record| record.at_ms).collect();
    let shadowed: Vec<bool> = cards
        .iter()
        .map(|card| match &card.body {
            Body::Sent { via: Via::Auto, text, .. } => rows.told_auto_fix(text),
            _ => false,
        })
        .collect();
    merge_with(&infos, cards, &told, &shadowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::pr_status::Via;
    use tidewatch_core::transcript::{Entry, Role};

    fn card(id: u64, at_ms: i64) -> Card {
        Card { id, at_ms, body: Body::Closed { number: 1 }, handled: false }
    }

    fn sent(id: u64, at_ms: i64, text: &str) -> Card {
        Card {
            id,
            at_ms,
            body: Body::Sent {
                via: Via::You,
                tag: "CI failure".into(),
                headline: text.into(),
                extra_lines: 0,
                text: text.into(),
            },
            handled: false,
        }
    }

    fn row(at_ms: Option<i64>) -> RowInfo<'static> {
        RowInfo { at_ms, user_text: None }
    }

    fn user(at_ms: i64, text: &str) -> RowInfo<'_> {
        RowInfo { at_ms: Some(at_ms), user_text: Some(text) }
    }

    #[test]
    fn a_card_goes_before_the_first_row_written_after_it() {
        let rows = [row(Some(100)), row(Some(200)), row(Some(300))];
        let places = merge(&rows, &[card(1, 150), card(2, 250), card(3, 999)]);
        assert_eq!(
            places,
            [
                Place::Row(0),
                Place::Card(0),
                Place::Row(1),
                Place::Card(1),
                Place::Row(2),
                Place::Card(2)
            ]
        );
    }

    #[test]
    fn rows_without_a_time_take_the_time_of_the_row_before() {
        let rows = [row(Some(100)), row(None), row(Some(300))];
        let places = merge(&rows, &[card(1, 200)]);
        assert_eq!(places, [Place::Row(0), Place::Row(1), Place::Card(0), Place::Row(2)]);
    }

    #[test]
    fn no_cards_and_no_rows_are_just_each_other() {
        assert_eq!(merge(&[row(Some(1)), row(None)], &[]), [Place::Row(0), Place::Row(1)]);
        assert_eq!(merge(&[], &[card(1, 5), card(2, 6)]), [Place::Card(0), Place::Card(1)]);
    }

    #[test]
    fn the_transcripts_copy_of_a_sent_message_is_left_out() {
        let rows = [row(Some(100)), user(1_100, "  fix it please \n"), row(Some(1_200))];
        let places = merge(&rows, &[sent(1, 1_000, "fix it please")]);
        assert_eq!(places, [Place::Row(0), Place::Card(0), Place::Row(2)]);
    }

    #[test]
    fn only_the_copy_after_the_card_and_only_one_per_card_is_left_out() {
        let rows = [user(100, "fix it"), user(201_000, "fix it"), user(202_000, "fix it")];
        let places = merge(&rows, &[sent(1, 200_000, "fix it")]);
        assert_eq!(places, [Place::Row(0), Place::Card(0), Place::Row(2)]);
    }

    #[test]
    fn a_copy_the_agent_respaced_or_cut_is_still_the_same_message() {
        let text = "PR #7 has a review. Answer it.\n\n> a very long quoted comment line that goes on and on and on and on and on and on and on and on";
        let respaced = text.replace('\n', "\r\n");
        let rows = [user(1_100, &respaced)];
        let places = merge(&rows, &[sent(1, 1_000, text)]);
        assert_eq!(places, [Place::Card(0)]);
        let rows = [user(1_100, "[Pasted text #1 +3 lines]")];
        assert_eq!(merge(&rows, &[sent(1, 1_000, text)]), [Place::Card(0)]);
    }

    #[test]
    fn a_message_a_hook_sent_sits_among_the_rows_by_its_time() {
        let rows = [row(Some(100)), row(Some(300))];
        let places = merge_with(&rows, &[card(1, 250)], &[200, 900], &[false]);
        assert_eq!(
            places,
            [Place::Row(0), Place::Said(0), Place::Card(0), Place::Row(1), Place::Said(1)]
        );
    }

    #[test]
    fn an_auto_fix_card_that_tidewatch_has_a_record_of_leaves_its_transcript_copy_to_the_record() {
        let rows = [user(1_100, "fix it please")];
        let auto = Card {
            body: Body::Sent {
                via: Via::Auto,
                tag: "CI failure".into(),
                headline: "fix it please".into(),
                extra_lines: 0,
                text: "fix it please".into(),
            },
            ..card(1, 1_000)
        };
        let places = merge_with(&rows, &[auto], &[], &[true]);
        assert_eq!(places, [Place::Row(0)]);
    }

    #[test]
    fn the_readers_rows_say_which_auto_fix_cards_their_records_replace() {
        use tidewatch_core::sent::{Reason, Record};
        let mut rows = Rows::default();
        let auto = |text: &str| Card {
            body: Body::Sent {
                via: Via::Auto,
                tag: "CI failure".into(),
                headline: text.into(),
                extra_lines: 0,
                text: text.into(),
            },
            ..card(1, 1_000)
        };
        rows.tidewatch_said(vec![Record::new(
            Reason::AutoFix,
            "Auto is on",
            "fix the build",
            None,
        )]);
        assert_eq!(places(&rows, &[auto("fix the build")]), []);
        assert_eq!(places(&rows, &[auto("fix the docs")]), [Place::Card(0)]);
    }

    #[test]
    fn a_different_user_message_stays() {
        let rows = [user(1_100, "something else")];
        let places = merge(&rows, &[sent(1, 1_000, "fix it")]);
        assert_eq!(places, [Place::Card(0), Place::Row(0)]);
    }

    #[test]
    fn the_readers_rows_carry_their_entry_times() {
        let mut rows = Rows::default();
        let entry = |role, text: &str, ts| Entry { role, text: text.into(), tool: None, ts };
        rows.push_all(vec![
            entry(Role::Assistant, "before", Some(100)),
            entry(Role::User, "fix it", Some(1_100)),
            entry(Role::Assistant, "after", Some(1_200)),
        ]);
        let places = places(&rows, &[sent(1, 1_000, "fix it"), card(2, 1_150)]);
        assert_eq!(places, [Place::Row(0), Place::Card(0), Place::Card(1), Place::Row(2)]);
    }
}
