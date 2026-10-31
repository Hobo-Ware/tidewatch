//! `TIDEWATCH_DEV_SEE=rows`: a short chat for a capture of Tidewatch's own rows in the Reader. An
//! Auto fix with its why and the buttons under it, a reminder of the end-of-turn check, and an older
//! reminder that must leave no row. Nothing is saved.

use tidewatch_core::sent::{Cap, Reason, Record, because};
use tidewatch_core::transcript::{Entry, Role};

use crate::reader_rows::Rows;

const MINUTE_MS: i64 = 60_000;

fn said(rows: &mut Rows, record: Record) {
    let (text, at) = (record.text.clone(), record.at_ms);
    rows.tidewatch_said(vec![record]);
    rows.push(entry(Role::User, &text, at + 500));
}

fn entry(role: Role, text: &str, at: i64) -> Entry {
    Entry { role, text: text.to_owned(), tool: None, ts: Some(at), images: Vec::new() }
}

pub(crate) fn seed(rows: &mut Rows, now_ms: i64) {
    let at = |minutes: i64| now_ms - minutes * MINUTE_MS;
    rows.push(entry(Role::User, "Take the PR over the line, please.", at(30)));
    rows.push(entry(
        Role::Assistant,
        "On it. I opened the pull request and I am watching its checks.",
        at(28),
    ));
    said(
        rows,
        Record {
            typed: true,
            at_ms: at(12),
            cap: Some(Cap { n: 1, max: 10 }),
            ..Record::new(
                Reason::AutoFix,
                because::AUTO_CONFLICTS,
                "PR #2421 conflicts with main. Rebase onto origin/main, resolve the conflicts, run the tests, and push with --force-with-lease.",
                None,
            )
        },
    );
    rows.push(entry(
        Role::Assistant,
        "Rebased onto main, resolved two conflicts in the sync code and pushed. The tests pass.",
        at(9),
    ));
    said(
        rows,
        Record {
            typed: true,
            at_ms: at(7),
            ..Record::new(
                Reason::LedgerGuard,
                "you answered and the progress report should show it",
                "The user answered; call update_progress so your report shows the new state.",
                None,
            )
        },
    );
    said(
        rows,
        Record {
            typed: true,
            at_ms: at(5),
            ..Record::new(
                Reason::LedgerGuard,
                because::LEDGER_STOP,
                "Before you stop: send your progress report with the final state.",
                None,
            )
        },
    );
    rows.push(entry(Role::Assistant, "Done. The PR is green and ready for review.", at(2)));
}
