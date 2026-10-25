//! A question nobody answered in a tab that runs on its own (YOLO or a Meeseeks): after a wait,
//! Tidewatch answers it with the agent's recommended option, else the first one. This is the clock
//! of that wait, pure: the time comes in as a number, so a test drives it with a fake clock.

use tidewatch_core::question::Question;

pub(crate) const WAIT_SECS: f64 = 300.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Poll {
    Idle,
    Counting { left: f64 },
    Due(u64),
}

/// Which waiting question the wait counts for, since when, and which one already fired.
#[derive(Debug, Default)]
pub(crate) struct Countdown {
    since: Option<(u64, f64)>,
    fired: Option<u64>,
}

impl Countdown {
    /// `waiting` is the row of the oldest question that needs an answer. The wait starts when a row
    /// first shows here, restarts for the next row, and fires once per row.
    pub(crate) fn poll(&mut self, waiting: Option<u64>, now: f64, wait: f64) -> Poll {
        let Some(seq) = waiting else {
            self.since = None;
            return Poll::Idle;
        };
        if self.fired == Some(seq) {
            return Poll::Idle;
        }
        let since = match self.since {
            Some((row, since)) if row == seq => since,
            _ => {
                self.since = Some((seq, now));
                now
            }
        };
        let left = wait - (now - since);
        if left <= 0.0 {
            self.fired = Some(seq);
            self.since = None;
            return Poll::Due(seq);
        }
        Poll::Counting { left }
    }

    /// What is left of the wait of row `seq`, for the card's line.
    pub(crate) fn left(&self, seq: u64, now: f64, wait: f64) -> Option<f64> {
        let (row, since) = self.since?;
        (row == seq).then(|| (wait - (now - since)).max(0.0))
    }
}

/// `4:12`, rounded up so the line never reads `0:00` before it fires.
pub(crate) fn clock_text(left: f64) -> String {
    let secs = left.max(0.0).ceil() as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The card's quiet line.
pub(crate) fn line(left: f64, questions: &[Question]) -> String {
    let recommended = questions.iter().all(|question| !question.picks().is_empty());
    let which = if recommended { "recommended" } else { "first" };
    format!("Picks the {which} answer in {}", clock_text(left))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::question::{QuestionOption, Select};

    struct FakeClock(f64);

    impl FakeClock {
        fn advance(&mut self, secs: f64) -> f64 {
            self.0 += secs;
            self.0
        }
    }

    #[test]
    fn nothing_waiting_means_nothing_counts() {
        let mut countdown = Countdown::default();
        assert_eq!(countdown.poll(None, 10.0, WAIT_SECS), Poll::Idle);
        assert_eq!(countdown.left(1, 10.0, WAIT_SECS), None);
    }

    #[test]
    fn a_question_is_picked_for_the_user_after_five_minutes() {
        let mut clock = FakeClock(1_000.0);
        let mut countdown = Countdown::default();
        assert_eq!(countdown.poll(Some(7), clock.0, WAIT_SECS), Poll::Counting { left: 300.0 });
        let now = clock.advance(299.0);
        assert_eq!(countdown.poll(Some(7), now, WAIT_SECS), Poll::Counting { left: 1.0 });
        let now = clock.advance(1.0);
        assert_eq!(countdown.poll(Some(7), now, WAIT_SECS), Poll::Due(7));
    }

    #[test]
    fn it_fires_once_and_the_next_question_gets_its_own_five_minutes() {
        let mut clock = FakeClock(0.0);
        let mut countdown = Countdown::default();
        countdown.poll(Some(1), clock.0, 60.0);
        let now = clock.advance(60.0);
        assert_eq!(countdown.poll(Some(1), now, 60.0), Poll::Due(1));
        let now = clock.advance(5.0);
        assert_eq!(countdown.poll(Some(1), now, 60.0), Poll::Idle);
        assert_eq!(countdown.poll(Some(2), now, 60.0), Poll::Counting { left: 60.0 });
        let now = clock.advance(30.0);
        assert_eq!(countdown.poll(Some(2), now, 60.0), Poll::Counting { left: 30.0 });
    }

    #[test]
    fn an_answer_given_first_cancels_the_wait() {
        let mut clock = FakeClock(0.0);
        let mut countdown = Countdown::default();
        countdown.poll(Some(1), clock.0, 60.0);
        let now = clock.advance(40.0);
        assert_eq!(countdown.poll(None, now, 60.0), Poll::Idle);
        let now = clock.advance(40.0);
        assert_eq!(countdown.poll(Some(2), now, 60.0), Poll::Counting { left: 60.0 });
    }

    #[test]
    fn the_card_reads_what_is_left_without_moving_the_wait() {
        let mut countdown = Countdown::default();
        countdown.poll(Some(3), 100.0, WAIT_SECS);
        assert_eq!(countdown.left(3, 148.0, WAIT_SECS), Some(252.0));
        assert_eq!(countdown.left(4, 148.0, WAIT_SECS), None);
        assert_eq!(countdown.left(3, 148.0, 60.0), Some(12.0));
    }

    #[test]
    fn the_clock_text_rounds_up_and_pads_seconds() {
        let cases =
            [(252.0, "4:12"), (251.2, "4:12"), (300.0, "5:00"), (59.0, "0:59"), (0.2, "0:01")];
        for (left, want) in cases {
            assert_eq!(clock_text(left), want, "{left}");
        }
    }

    #[test]
    fn the_line_names_the_recommended_answer_or_the_first() {
        let question = |labels: &[&str]| Question {
            text: "Which?".into(),
            header: None,
            decision: None,
            select: Select::Single,
            options: labels.iter().map(|label| QuestionOption::new(*label)).collect(),
        };
        let with = [question(&["A", "B (Recommended)"])];
        let without = [question(&["A", "B"])];
        assert_eq!(line(252.0, &with), "Picks the recommended answer in 4:12");
        assert_eq!(line(252.0, &without), "Picks the first answer in 4:12");
    }
}
