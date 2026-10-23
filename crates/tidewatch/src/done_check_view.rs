//! The done check card: a band on the agent tab with the outcome, what passed and one line for each
//! check that failed, could not run or does not apply. [`card_of`] is the data; `show` draws it.

use egui::{Align, Align2, Layout, Rect, Ui, UiBuilder, pos2, vec2};
use tidewatch_core::done_check::{Kind, Outcome, Report, Status};
use tidewatch_core::pr_status::Tone;

use crate::brand;
use crate::done_check_glue::Look;
use crate::pr_strip::tone_ink;
use crate::reader_view::{INSET, ellipsized, paint_item};
use crate::screen_kit::{Size, body, primary_button, quiet_button};

pub(crate) const BAND_H: f32 = 40.0;
const ROW_H: f32 = 18.0;
const ROWS_PAD: f32 = 8.0;
const MAX_ITEMS: usize = 3;
const BUTTONS_GAP: f32 = 8.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) tone: Tone,
    pub(crate) text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    pub(crate) title: &'static str,
    pub(crate) tone: Tone,
    pub(crate) summary: String,
    pub(crate) rows: Vec<Row>,
    pub(crate) ask_fix: bool,
    pub(crate) check_again: bool,
}

pub(crate) fn tone_of(outcome: Outcome) -> Tone {
    match outcome {
        Outcome::Done => Tone::Good,
        Outcome::Blocked => Tone::Bad,
        Outcome::NotVerified => Tone::Warn,
    }
}

pub(crate) fn title_of(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "Done",
        Outcome::Blocked => "Blocked",
        Outcome::NotVerified => "Not verified",
    }
}

pub(crate) fn tag_of(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "Check passed",
        Outcome::Blocked => "Check blocked",
        Outcome::NotVerified => "Not verified",
    }
}

pub(crate) fn card_of(report: &Report) -> Card {
    let outcome = report.outcome();
    let failed = report.lines.iter().filter(|line| line.status == Status::Fail).count();
    let passed: Vec<&str> = report
        .lines
        .iter()
        .filter(|line| line.status == Status::Pass)
        .map(|line| line.detail.as_str())
        .collect();
    let summary = match (outcome, failed) {
        (Outcome::Blocked, 1) => "1 check failed".to_owned(),
        (Outcome::Blocked, n) => format!("{n} checks failed"),
        _ => passed.join(" \u{b7} "),
    };
    let mut rows = Vec::new();
    for line in &report.lines {
        let (tone, text) = match line.status {
            Status::Pass => continue,
            Status::Fail => (Tone::Bad, format!("{}: {}", line.kind.label(), line.detail)),
            Status::Unknown => (Tone::Warn, format!("{}: {}", line.kind.label(), line.detail)),
            Status::Skipped => {
                (Tone::Quiet, format!("{}: not checked, {}", line.kind.label(), line.detail))
            }
        };
        rows.push(Row { tone, text });
        if line.kind == Kind::Unverified {
            let more = report.unverified.len().saturating_sub(MAX_ITEMS);
            rows.extend(
                report
                    .unverified
                    .iter()
                    .take(MAX_ITEMS)
                    .map(|item| Row { tone: Tone::Quiet, text: format!("    - {item}") }),
            );
            if more > 0 {
                rows.push(Row { tone: Tone::Quiet, text: format!("    and {more} more") });
            }
        }
    }
    Card {
        title: title_of(outcome),
        tone: tone_of(outcome),
        summary,
        rows,
        ask_fix: outcome == Outcome::Blocked,
        check_again: outcome != Outcome::Done,
    }
}

pub(crate) fn height(look: &Look) -> f32 {
    match look {
        Look::Checking => BAND_H,
        Look::Report(report) => {
            let rows = card_of(report).rows.len();
            if rows == 0 { BAND_H } else { BAND_H + ROWS_PAD + ROW_H * rows as f32 }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    AskFix,
    CheckAgain,
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    look: &Look,
    can_ask: Result<(), &'static str>,
) -> Option<Action> {
    let card_rect = rect.shrink2(vec2(INSET, 4.0));
    paint_item(ui.painter(), card_rect, false);
    let mid = card_rect.top() + (BAND_H - 8.0) / 2.0;
    let dot_x = card_rect.left() + 16.0;
    let text_x = card_rect.left() + 32.0;
    let Look::Report(report) = look else {
        ui.painter().circle_filled(pos2(dot_x, mid), 4.5, brand::MUTED);
        ui.painter().text(
            pos2(text_x, mid),
            Align2::LEFT_CENTER,
            "Checking that the work landed...",
            body(13.0),
            brand::MUTED,
        );
        return None;
    };
    let card = card_of(report);
    let ink = tone_ink(card.tone);
    ui.painter().circle_filled(pos2(dot_x, mid), 4.5, ink);
    let title = ui.painter().layout_no_wrap(card.title.to_owned(), body(13.0), ink);
    let title_w = title.size().x;
    ui.painter().galley(pos2(text_x, mid - title.size().y / 2.0), title, ink);

    let header = Rect::from_min_size(card_rect.min, vec2(card_rect.width(), BAND_H - 8.0));
    let mut action = None;
    let mut buttons_w = 0.0;
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(header.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = BUTTONS_GAP;
            let start = ui.cursor().right();
            if card.ask_fix {
                let sent = primary_button(ui, "Ask agent to fix", can_ask.is_ok(), Size::Compact);
                let sent = match can_ask {
                    Ok(()) => sent,
                    Err(why) => sent.on_hover_text(why),
                };
                if sent.clicked() {
                    action = Some(Action::AskFix);
                }
            }
            if card.check_again && quiet_button(ui, "Check again", true, Size::Compact).clicked() {
                action = Some(Action::CheckAgain);
            }
            buttons_w = start - ui.cursor().right();
        },
    );
    let room = (card_rect.width() - 32.0 - title_w - 16.0 - buttons_w - 20.0).max(0.0);
    let summary = ellipsized(ui.painter(), &card.summary, body(12.5), brand::MUTED, room);
    let summary_x = text_x + title_w + 12.0;
    ui.painter().galley(pos2(summary_x, mid - summary.size().y / 2.0), summary, brand::MUTED);

    let row_room = card_rect.width() - 32.0 - 16.0;
    for (i, row) in card.rows.iter().enumerate() {
        let y = card_rect.top() + BAND_H - 8.0 + ROWS_PAD / 2.0 + ROW_H * (i as f32 + 0.5);
        let ink = if row.tone == Tone::Quiet { brand::MUTED } else { tone_ink(row.tone) };
        let galley = ellipsized(ui.painter(), &row.text, body(12.5), ink, row_room);
        ui.painter().galley(pos2(text_x, y - galley.size().y / 2.0), galley, ink);
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::done_check::Line;

    fn line(kind: Kind, status: Status, detail: &str) -> Line {
        Line { kind, status, detail: detail.to_owned() }
    }

    fn report(lines: Vec<Line>, unverified: &[&str]) -> Report {
        Report {
            head: String::new(),
            lines,
            unverified: unverified.iter().map(|item| (*item).to_owned()).collect(),
        }
    }

    fn texts(card: &Card) -> Vec<&str> {
        card.rows.iter().map(|row| row.text.as_str()).collect()
    }

    #[test]
    fn a_green_report_is_one_line_with_what_passed() {
        let card = card_of(&report(
            vec![
                line(Kind::Committed, Status::Pass, "clean tree, 1 commit ahead of main"),
                line(Kind::Pushed, Status::Pass, "origin has 3f9a1c2"),
            ],
            &[],
        ));
        assert_eq!(card.title, "Done");
        assert_eq!(card.tone, Tone::Good);
        assert_eq!(card.summary, "clean tree, 1 commit ahead of main \u{b7} origin has 3f9a1c2");
        assert_eq!(card.rows.len(), 0);
        assert_eq!((card.ask_fix, card.check_again), (false, false));
    }

    #[test]
    fn a_blocked_report_names_every_failed_check_in_one_line_and_offers_the_fix() {
        let card = card_of(&report(
            vec![
                line(Kind::Committed, Status::Pass, "clean tree, 2 commits ahead of main"),
                line(Kind::Pushed, Status::Fail, "2 commits not pushed"),
                line(Kind::PullRequest, Status::Fail, "PR #7 checks failing: build"),
            ],
            &[],
        ));
        assert_eq!((card.title, card.tone), ("Blocked", Tone::Bad));
        assert_eq!(card.summary, "2 checks failed");
        assert_eq!(
            texts(&card),
            ["Pushed: 2 commits not pushed", "Pull request: PR #7 checks failing: build"]
        );
        assert!(card.rows.iter().all(|row| row.tone == Tone::Bad));
        assert_eq!((card.ask_fix, card.check_again), (true, true));
    }

    #[test]
    fn checks_that_could_not_run_say_why_and_the_ledger_items_are_listed() {
        let card = card_of(&report(
            vec![
                line(Kind::Committed, Status::Pass, "clean tree, 1 commit ahead of main"),
                line(Kind::PullRequest, Status::Unknown, "the GitHub CLI (gh) is not installed"),
                line(Kind::Unverified, Status::Unknown, "5 items the agent did not verify"),
            ],
            &["a", "b", "c", "d", "e"],
        ));
        assert_eq!((card.title, card.tone), ("Not verified", Tone::Warn));
        assert_eq!(
            texts(&card),
            [
                "Pull request: the GitHub CLI (gh) is not installed",
                "Not verified: 5 items the agent did not verify",
                "    - a",
                "    - b",
                "    - c",
                "    and 2 more",
            ]
        );
        assert_eq!((card.ask_fix, card.check_again), (false, true));
    }

    #[test]
    fn a_check_that_does_not_apply_shows_as_not_checked_with_the_reason() {
        let card = card_of(&report(
            vec![
                line(Kind::Committed, Status::Pass, "clean tree, 1 commit ahead of main"),
                line(Kind::PullRequest, Status::Skipped, "no pull request for this branch"),
            ],
            &[],
        ));
        assert_eq!(card.title, "Done");
        assert_eq!(texts(&card), ["Pull request: not checked, no pull request for this branch"]);
        assert_eq!(card.rows[0].tone, Tone::Quiet);
    }

    #[test]
    fn the_band_grows_with_its_rows() {
        let none = Look::Report(report(vec![line(Kind::Pushed, Status::Pass, "ok")], &[]));
        let two = Look::Report(report(
            vec![line(Kind::Pushed, Status::Fail, "x"), line(Kind::Committed, Status::Fail, "y")],
            &[],
        ));
        assert_eq!(height(&Look::Checking), BAND_H);
        assert_eq!(height(&none), BAND_H);
        assert_eq!(height(&two), BAND_H + ROWS_PAD + 2.0 * ROW_H);
    }
}
