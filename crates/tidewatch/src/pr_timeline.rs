//! Where the PR cards sit among the Reader's rows: by time, the way a chat reads. A card goes before
//! the first row written after it. The user message Tidewatch pasted shows as its own bubble card, so
//! the transcript's copy of the same text is left out. Pure.

use tidewatch_core::pr_cards::Card;
use tidewatch_core::pr_status::Body;

use crate::reader_rows::{Kind, Rows};

/// How far before a card the transcript's copy of its message may be stamped (the clocks of two
/// processes).
const SLACK_MS: i64 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// The n-th row of the transcript.
    Row(usize),
    /// The n-th card of the timeline.
    Card(usize),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RowInfo<'a> {
    pub(crate) at_ms: Option<i64>,
    pub(crate) user_text: Option<&'a str>,
}

/// The order to draw `rows` and `cards` in. `cards` are sorted by time.
pub(crate) fn merge(rows: &[RowInfo<'_>], cards: &[Card]) -> Vec<Place> {
    let hidden = copies_of_sent(rows, cards);
    let mut places = Vec::with_capacity(rows.len() + cards.len());
    let mut next_card = 0;
    let mut stamp = i64::MIN;
    for (index, row) in rows.iter().enumerate() {
        stamp = row.at_ms.unwrap_or(stamp);
        while next_card < cards.len() && cards[next_card].at_ms < stamp {
            places.push(Place::Card(next_card));
            next_card += 1;
        }
        if !hidden[index] {
            places.push(Place::Row(index));
        }
    }
    places.extend((next_card..cards.len()).map(Place::Card));
    places
}

/// The rows to leave out: for each sent message, the first user row after it with the same text.
fn copies_of_sent(rows: &[RowInfo<'_>], cards: &[Card]) -> Vec<bool> {
    let mut hidden = vec![false; rows.len()];
    for card in cards {
        let Body::Sent { text, .. } = &card.body else { continue };
        let found = rows.iter().enumerate().find(|(index, row)| {
            !hidden[*index]
                && row.user_text.is_some_and(|said| said.trim() == text.trim())
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
    merge(&infos, cards)
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

    fn user(at_ms: i64, text: &'static str) -> RowInfo<'static> {
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
        let rows = [user(100, "fix it"), user(21_000, "fix it"), user(22_000, "fix it")];
        let places = merge(&rows, &[sent(1, 20_000, "fix it")]);
        assert_eq!(places, [Place::Row(0), Place::Card(0), Place::Row(2)]);
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
