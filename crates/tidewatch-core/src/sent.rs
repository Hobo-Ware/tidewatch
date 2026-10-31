//! What Tidewatch itself typed or told an agent, kept per tab in `tidewatch-sent.jsonl` beside its
//! notes. Every sender appends one line; the Reader matches transcript turns against the records so
//! a message Tidewatch sent is drawn as Tidewatch's, also after a restart.

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::prefs::AutoJob;

const KEPT: usize = 200;

/// The `because` of each sender: a clause that reads after "Because". It starts in lower case,
/// except for a feature name listed in `NAMED`.
pub mod because {
    pub const AUTO_CHECKS: &str = "Auto fixes failed checks on this PR";
    pub const AUTO_REVIEWS: &str = "Auto answers reviews on this PR";
    pub const AUTO_CONFLICTS: &str = "Auto fixes conflicts on this PR";
    pub const AUTO_PUSH: &str = "Auto may ask the agent to open the pull request on this tab";
    pub const ASK_PR_CLICK: &str = "you asked the agent to open the pull request";
    pub const ASK_PUSH_OWN: &str =
        "this tab runs on its own and has commits the pull request does not have";
    pub const ASK_PUSH_CLICK: &str = "you asked the agent to push its new commits";
    pub const LEDGER_STOP: &str = "the progress report was not updated this turn";
    pub const LEDGER_FINAL: &str = "the progress report still said working at the end of the turn";
    pub const AVATAR_DRAW: &str = "you asked for an avatar";
    pub const AVATAR_REDRAW: &str = "you asked for a new avatar";
    pub const DUE_CLICK: &str = "you pressed Ask on the Due today card";
    pub const DUE_OWN: &str = "this tab runs on its own and an item is due today";
    pub const STUCK_JOB: &str = "you pressed Ask agent on a stuck job";
    pub const DONE_CHECK_OWN: &str = "the done check found something to fix";
    pub const DONE_CHECK_CLICK: &str = "you asked the agent to fix what the done check found";
    pub const COMMENTS: &str = "the branch has new comments the comment guard flags";
    pub const MEESEEKS: &str = "a check of this Meeseeks' work did not pass";
    pub const BUILD_WORKFLOW: &str = "you ran Build workflow from this session";
    pub const BACKGROUND_JOB: &str = "a background job has reported no progress";

    pub const NAMED: [&str; 1] = ["Auto"];

    pub const ALL: [&str; 20] = [
        AUTO_CHECKS,
        AUTO_REVIEWS,
        AUTO_CONFLICTS,
        AUTO_PUSH,
        ASK_PR_CLICK,
        ASK_PUSH_OWN,
        ASK_PUSH_CLICK,
        LEDGER_STOP,
        LEDGER_FINAL,
        AVATAR_DRAW,
        AVATAR_REDRAW,
        DUE_CLICK,
        DUE_OWN,
        STUCK_JOB,
        DONE_CHECK_OWN,
        DONE_CHECK_CLICK,
        COMMENTS,
        MEESEEKS,
        BUILD_WORKFLOW,
        BACKGROUND_JOB,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    AutoFix,
    AutoPush,
    PullRequest,
    MeeseeksCheck,
    DueToday,
    LedgerGuard,
    CommentGuard,
    DoneCheck,
    Avatar,
    Workflow,
    Resume,
    StuckJob,
    BackgroundJob,
}

impl Reason {
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::AutoFix | Self::AutoPush => "auto",
            Self::PullRequest => "pull request",
            Self::MeeseeksCheck => "Meeseeks check",
            Self::DueToday => "due today",
            Self::LedgerGuard => "report check",
            Self::CommentGuard => "comment guard",
            Self::DoneCheck => "done check",
            Self::Avatar => "avatar",
            Self::Workflow => "workflow",
            Self::Resume => "resume",
            Self::StuckJob => "stuck job",
            Self::BackgroundJob => "background job",
        }
    }

    /// What a reminder of this reason says in the Reader's quiet line; `None` for a reason that
    /// changes what the agent does and keeps its card.
    #[must_use]
    pub fn quiet_words(self) -> Option<&'static str> {
        match self {
            Self::LedgerGuard => Some("Tidewatch reminded the agent to update its report"),
            Self::Avatar => Some("Tidewatch asked for an avatar"),
            Self::DoneCheck => Some("Tidewatch asked the agent to fix what the done check found"),
            Self::CommentGuard => Some("Tidewatch told the agent about new branch comments"),
            Self::Resume => Some("Tidewatch resumed the agent"),
            Self::StuckJob => Some("Tidewatch asked the agent about a stuck job"),
            Self::AutoFix
            | Self::AutoPush
            | Self::PullRequest
            | Self::MeeseeksCheck
            | Self::DueToday
            | Self::Workflow
            | Self::BackgroundJob => None,
        }
    }

    /// The message was filled in from the repo's workflow profile, so the profile is what changes it.
    #[must_use]
    pub fn uses_workflow(self) -> bool {
        matches!(self, Self::AutoFix | Self::AutoPush | Self::PullRequest)
    }

    /// Told through a hook, so the transcript holds no user turn for it.
    #[must_use]
    pub fn timed(self) -> bool {
        matches!(self, Self::LedgerGuard | Self::Avatar)
    }
}

/// The setting that stops a kind of message, shown on the message's own row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Auto(AutoJob),
    DoneCheck,
    Comments,
}

impl Switch {
    /// The button's words.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto(AutoJob::FailedChecks) => "Turn off for failed checks",
            Self::Auto(AutoJob::Reviews) => "Turn off for reviews",
            Self::Auto(AutoJob::Conflicts) => "Turn off for conflicts",
            Self::Auto(AutoJob::Push) => "Turn off pushing",
            Self::DoneCheck => "Turn off the done check",
            Self::Comments => "Turn off the comment guard for this tab",
        }
    }
}

/// The reminders older versions sent, as the `because` they were saved with. Their rows leave no
/// trace: the senders are gone, and so is the line in the Reader.
const DROPPED: [&str; 6] = [
    "this tab has not saved a progress report yet",
    "you answered and the progress report should show it",
    "this tab has no avatar yet",
    "the report said working and nothing was reported for an hour",
    "the agent kept working while its report went stale",
    "the agent's last message asks something and its report has no needs_you",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cap {
    pub n: u32,
    pub max: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    pub at_ms: i64,
    pub reason: Reason,
    pub because: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<Cap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub typed: bool,
    /// The user pressed the button that sent it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hand: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Line {
    Claim { claim: u64, seq: u64 },
    Record(Record),
}

impl Record {
    #[must_use]
    pub fn new(reason: Reason, because: &str, text: &str, cap: Option<Cap>) -> Self {
        Self {
            id: new_id(),
            at_ms: now_ms(),
            reason,
            because: because.to_owned(),
            text: text.to_owned(),
            cap,
            claimed_seq: None,
            typed: false,
            hand: false,
        }
    }

    /// A hook told it and nothing was typed.
    #[must_use]
    pub fn timed(&self) -> bool {
        self.reason.timed() && !self.typed
    }

    /// A reminder that changes nothing about the work: the Reader folds it into a quiet line. A
    /// message the user's own click sent, and every other reason, keeps its card.
    #[must_use]
    pub fn quiet(&self) -> bool {
        !self.hand && self.reason.quiet_words().is_some()
    }

    /// `Because Auto fixes conflicts on this PR · 1 of 10`.
    #[must_use]
    pub fn footer(&self) -> String {
        match self.cap {
            Some(Cap { n, max }) => format!("Because {} \u{b7} {n} of {max}", self.because),
            None => format!("Because {}", self.because),
        }
    }

    /// The setting that stops this kind of message, when there is one. A message the user's own
    /// click sent has none: the click was the choice.
    #[must_use]
    pub fn switch(&self) -> Option<Switch> {
        if self.hand {
            return None;
        }
        match self.reason {
            Reason::AutoFix => [
                (because::AUTO_CHECKS, AutoJob::FailedChecks),
                (because::AUTO_REVIEWS, AutoJob::Reviews),
                (because::AUTO_CONFLICTS, AutoJob::Conflicts),
            ]
            .into_iter()
            .find_map(|(clause, job)| (self.because == clause).then_some(Switch::Auto(job))),
            Reason::AutoPush => Some(Switch::Auto(AutoJob::Push)),
            Reason::DoneCheck => Some(Switch::DoneCheck),
            Reason::CommentGuard => Some(Switch::Comments),
            _ => None,
        }
    }

    /// A reminder an older version sent and no sender sends any more.
    #[must_use]
    pub fn dropped(&self) -> bool {
        DROPPED.iter().any(|gone| self.because.contains(gone))
    }
}

#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

fn new_id() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_nanos()).unwrap_or(u64::MAX));
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last.saturating_add(1));
        match LAST.compare_exchange(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(seen) => last = seen,
        }
    }
}

fn append_line(file: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut open = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
    open.write_all(format!("{line}\n").as_bytes())
}

pub fn append(file: &Path, record: &Record) -> std::io::Result<()> {
    append_line(file, &serde_json::to_string(record).map_err(std::io::Error::other)?)
}

pub fn append_claim(file: &Path, id: u64, seq: u64) -> std::io::Result<()> {
    append_line(file, &serde_json::json!({ "claim": id, "seq": seq }).to_string())
}

/// The records of `file`, oldest first, with the claims folded in. A line that does not parse, such
/// as one cut by a crash, is skipped.
#[must_use]
pub fn load(file: &Path) -> Vec<Record> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    let mut records: Vec<Record> = Vec::new();
    for line in text.lines() {
        match serde_json::from_str::<Line>(line) {
            Ok(Line::Record(record)) => records.push(record),
            Ok(Line::Claim { claim, seq }) => {
                if let Some(record) = records.iter_mut().find(|record| record.id == claim) {
                    record.claimed_seq = Some(seq);
                }
            }
            Err(_) => {}
        }
    }
    records.sort_by_key(|record| record.at_ms);
    let drop = records.len().saturating_sub(KEPT);
    records.drain(..drop);
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(reason: Reason, text: &str, at_ms: i64) -> Record {
        Record { at_ms, ..Record::new(reason, "a rule fired", text, None) }
    }

    #[test]
    fn only_a_reminder_nobody_clicked_is_quiet() {
        let cases = [
            (Reason::LedgerGuard, false, true),
            (Reason::Avatar, false, true),
            (Reason::DoneCheck, false, true),
            (Reason::CommentGuard, false, true),
            (Reason::Resume, false, true),
            (Reason::StuckJob, false, true),
            (Reason::DoneCheck, true, false),
            (Reason::Avatar, true, false),
            (Reason::AutoFix, false, false),
            (Reason::AutoPush, false, false),
            (Reason::PullRequest, false, false),
            (Reason::MeeseeksCheck, false, false),
            (Reason::DueToday, false, false),
            (Reason::BackgroundJob, false, false),
        ];
        for (reason, hand, quiet) in cases {
            let sent = Record { hand, ..Record::new(reason, "why", "text", None) };
            assert_eq!(sent.quiet(), quiet, "{reason:?} by hand {hand}");
        }
    }

    #[test]
    fn a_click_flag_survives_the_file_and_an_old_line_reads_as_not_clicked() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sent.jsonl");
        append(&file, &Record { hand: true, ..Record::new(Reason::DoneCheck, "b", "t", None) })
            .unwrap();
        assert!(load(&file)[0].hand);
        let old = r#"{"id":1,"at_ms":5,"reason":"avatar","because":"b","text":"t"}"#;
        std::fs::write(&file, format!("{old}\n")).unwrap();
        assert!(!load(&file)[0].hand);
    }

    #[test]
    fn a_record_survives_the_file_with_its_cap_and_claim() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("tidewatch-sent.jsonl");
        let mut first = record(Reason::AutoFix, "fix it\n\n> quoted", 10);
        first.cap = Some(Cap { n: 1, max: 3 });
        let second = record(Reason::LedgerGuard, "update the ledger", 20);
        append(&file, &first).unwrap();
        append(&file, &second).unwrap();
        append_claim(&file, first.id, 7).unwrap();
        let loaded = load(&file);
        assert_eq!(loaded, [Record { claimed_seq: Some(7), ..first }, second]);
    }

    #[test]
    fn a_missing_file_a_cut_line_and_a_claim_for_nothing_load_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sent.jsonl");
        assert_eq!(load(&file), []);
        let kept = record(Reason::DueToday, "start the thing", 5);
        append(&file, &kept).unwrap();
        append_claim(&file, 1, 2).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(b"{\"id\":9,\"at_ms\":1,\"rea")
            .unwrap();
        assert_eq!(load(&file), [kept]);
    }

    #[test]
    fn the_file_loads_oldest_first_and_keeps_only_the_newest_records() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sent.jsonl");
        for at in (0..KEPT as i64 + 5).rev() {
            append(&file, &record(Reason::DoneCheck, "x", at)).unwrap();
        }
        let loaded = load(&file);
        assert_eq!(loaded.len(), KEPT);
        assert_eq!(loaded.first().map(|record| record.at_ms), Some(5));
        assert_eq!(loaded.last().map(|record| record.at_ms), Some(KEPT as i64 + 4));
    }

    #[test]
    fn the_footer_says_why_and_counts_a_capped_send() {
        let plain = record(Reason::DueToday, "x", 0);
        assert_eq!(plain.footer(), "Because a rule fired");
        let capped = Record {
            because: because::AUTO_CONFLICTS.into(),
            cap: Some(Cap { n: 1, max: 10 }),
            ..record(Reason::AutoFix, "x", 0)
        };
        assert_eq!(capped.footer(), "Because Auto fixes conflicts on this PR \u{b7} 1 of 10");
        let sweep =
            Record { cap: Some(Cap { n: 2, max: 3 }), ..record(Reason::CommentGuard, "x", 0) };
        assert!(sweep.footer().ends_with("\u{b7} 2 of 3"));
    }

    #[test]
    fn every_because_reads_as_a_clause_after_because() {
        for text in because::ALL {
            let first = text.chars().next().unwrap();
            let named = because::NAMED.iter().any(|name| text.starts_with(name));
            assert!(first.is_lowercase() || named, "{text}");
            assert!(!text.ends_with('.'), "{text}");
        }
        assert_eq!(
            Record::new(Reason::LedgerGuard, because::LEDGER_STOP, "x", None).footer(),
            "Because the progress report was not updated this turn"
        );
    }

    #[test]
    fn the_dropped_reminders_leave_no_reason_to_send_them() {
        let dropped = [
            "has not saved a progress report yet",
            "you answered and the progress report should show it",
            "has no avatar yet",
            "nothing was reported for an hour",
            "kept working while its report went stale",
            "has no needs_you",
        ];
        for text in because::ALL {
            for gone in dropped {
                assert!(!text.contains(gone), "{text}");
            }
        }
    }

    #[test]
    fn what_the_reader_shows_never_uses_the_internal_word_ledger() {
        let tags = [
            Reason::AutoFix,
            Reason::AutoPush,
            Reason::PullRequest,
            Reason::MeeseeksCheck,
            Reason::DueToday,
            Reason::LedgerGuard,
            Reason::CommentGuard,
            Reason::DoneCheck,
            Reason::Avatar,
            Reason::Workflow,
            Reason::Resume,
            Reason::StuckJob,
            Reason::BackgroundJob,
        ]
        .map(Reason::tag);
        for shown in because::ALL.into_iter().chain(tags) {
            assert!(!shown.to_lowercase().contains("ledger"), "{shown}");
        }
    }

    #[test]
    fn a_hook_told_reason_typed_into_the_terminal_has_a_user_turn() {
        let told = Record::new(Reason::LedgerGuard, because::LEDGER_STOP, "x", None);
        assert!(told.timed());
        let typed = Record { typed: true, ..told.clone() };
        assert!(!typed.timed());
        assert!(!serde_json::to_string(&told).unwrap().contains("typed"));
        let back: Record = serde_json::from_str(&serde_json::to_string(&typed).unwrap()).unwrap();
        assert!(back.typed);
    }

    #[test]
    fn only_hook_told_reasons_have_no_user_turn() {
        assert!(Reason::LedgerGuard.timed());
        assert!(Reason::Avatar.timed());
        assert!(!Reason::AutoFix.timed());
        assert!(!Reason::MeeseeksCheck.timed());
        assert_eq!(Reason::MeeseeksCheck.tag(), "Meeseeks check");
    }

    #[test]
    fn each_auto_fix_names_the_switch_that_stops_its_kind() {
        let table = [
            (Reason::AutoFix, because::AUTO_CHECKS, Some(Switch::Auto(AutoJob::FailedChecks))),
            (Reason::AutoFix, because::AUTO_REVIEWS, Some(Switch::Auto(AutoJob::Reviews))),
            (Reason::AutoFix, because::AUTO_CONFLICTS, Some(Switch::Auto(AutoJob::Conflicts))),
            (Reason::AutoFix, "Auto is on for this PR", None),
            (Reason::AutoPush, because::AUTO_PUSH, Some(Switch::Auto(AutoJob::Push))),
            (Reason::DoneCheck, because::DONE_CHECK_OWN, Some(Switch::DoneCheck)),
            (Reason::CommentGuard, because::COMMENTS, Some(Switch::Comments)),
            (Reason::LedgerGuard, because::LEDGER_STOP, None),
            (Reason::MeeseeksCheck, because::MEESEEKS, None),
        ];
        for (reason, why, want) in table {
            assert_eq!(Record::new(reason, why, "x", None).switch(), want, "{reason:?} {why}");
        }
    }

    #[test]
    fn a_message_the_users_click_sent_has_no_switch() {
        let clicked = Record {
            hand: true,
            ..Record::new(Reason::DoneCheck, because::DONE_CHECK_CLICK, "x", None)
        };
        assert_eq!(clicked.switch(), None);
    }

    #[test]
    fn the_switch_words_name_what_they_stop() {
        let table = [
            (Switch::Auto(AutoJob::FailedChecks), "Turn off for failed checks"),
            (Switch::Auto(AutoJob::Reviews), "Turn off for reviews"),
            (Switch::Auto(AutoJob::Conflicts), "Turn off for conflicts"),
            (Switch::Auto(AutoJob::Push), "Turn off pushing"),
            (Switch::DoneCheck, "Turn off the done check"),
            (Switch::Comments, "Turn off the comment guard for this tab"),
        ];
        for (switch, label) in table {
            assert_eq!(switch.label(), label);
        }
    }

    #[test]
    fn the_workflow_words_sit_on_the_pull_request_messages_only() {
        let table = [
            (Reason::AutoFix, true),
            (Reason::AutoPush, true),
            (Reason::PullRequest, true),
            (Reason::MeeseeksCheck, false),
            (Reason::LedgerGuard, false),
            (Reason::CommentGuard, false),
            (Reason::DoneCheck, false),
        ];
        for (reason, want) in table {
            assert_eq!(reason.uses_workflow(), want, "{reason:?}");
        }
    }

    #[test]
    fn a_reminder_an_older_version_sent_is_dropped_and_a_live_one_is_not() {
        let gone = [
            "this tab has not saved a progress report yet",
            "you answered and the progress report should show it",
            "this tab has no avatar yet",
            "the report said working and nothing was reported for an hour",
            "the agent kept working while its report went stale",
            "the agent's last message asks something and its report has no needs_you",
        ];
        for why in gone {
            assert!(Record::new(Reason::LedgerGuard, why, "x", None).dropped(), "{why}");
        }
        for why in because::ALL {
            assert!(!Record::new(Reason::LedgerGuard, why, "x", None).dropped(), "{why}");
        }
    }
}
