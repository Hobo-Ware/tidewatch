//! The text Tidewatch pastes into an agent when GitHub has news about its pull request (PLAN.md, D4
//! follow-up). Pure.

use crate::note_paste::clean;
use crate::pr_status::{Body, Job, ReviewComment};
use crate::prefs::{AutoJob, GithubPrefs};
use crate::text::plural;

const MAX_CHARS: usize = 6000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ask {
    pub verb: &'static str,
    pub phrase: &'static str,
    pub tag: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    FailedChecks,
    Reviews,
    Conflicts,
}

impl Trigger {
    #[must_use]
    pub fn job(self) -> AutoJob {
        match self {
            Self::FailedChecks => AutoJob::FailedChecks,
            Self::Reviews => AutoJob::Reviews,
            Self::Conflicts => AutoJob::Conflicts,
        }
    }

    /// The kind in a sentence: `conflict`, `failed check`, `review`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::FailedChecks => "failed check",
            Self::Reviews => "review",
            Self::Conflicts => "conflict",
        }
    }
}

#[must_use]
pub fn ask_of(body: &Body) -> Option<(Ask, Trigger)> {
    let (verb, phrase, tag, trigger) = match body {
        Body::ChecksFailed { .. } => ("fix", "fix CI", "CI failure", Trigger::FailedChecks),
        Body::ChangesRequested { .. } | Body::Reviewed { .. } => {
            ("address", "address the review", "Review", Trigger::Reviews)
        }
        Body::Commented { .. } => ("answer", "answer the comment", "Comment", Trigger::Reviews),
        Body::Conflict { .. } => {
            ("resolve", "resolve the conflicts", "Conflict", Trigger::Conflicts)
        }
        _ => return None,
    };
    Some((Ask { verb, phrase, tag }, trigger))
}

/// A review or comment from someone whose text an agent must not act on without the owner reading
/// it: Auto-fix leaves it to the Ask button.
#[must_use]
pub fn untrusted(body: &Body) -> bool {
    matches!(
        body,
        Body::ChangesRequested { trusted: false, .. }
            | Body::Reviewed { trusted: false, .. }
            | Body::Commented { trusted: false, .. }
    )
}

#[must_use]
pub fn wanted(prefs: &GithubPrefs, body: &Body) -> bool {
    !untrusted(body)
        && ask_of(body).is_some_and(|(_, trigger)| match trigger {
            Trigger::FailedChecks => prefs.failed_checks,
            Trigger::Reviews => prefs.reviews,
            Trigger::Conflicts => prefs.conflicts,
        })
}

/// What the agent hears once when its pull request ends, so the owner never has to tell it.
#[must_use]
pub fn ended_text(body: &Body) -> Option<String> {
    match body {
        Body::Merged { number, base } => Some(format!("PR #{number} was merged into {base}.")),
        Body::Closed { number } => Some(format!("PR #{number} was closed without merging.")),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub ask: Ask,
    pub trigger: Trigger,
    pub headline: String,
    pub detail: String,
    pub text: String,
    pub extra_lines: usize,
}

impl Message {
    /// The same facts with the owner's short request after them, for a click on Ask or Send now.
    #[must_use]
    pub fn asked(&self) -> Self {
        let headline = format!("{} Please {}.", self.headline, self.ask.phrase);
        let text = join(&headline, &self.detail);
        Self { text: cap(&text), ..self.clone() }
    }
}

fn join(headline: &str, detail: &str) -> String {
    if detail.is_empty() { headline.to_owned() } else { format!("{headline}\n\n{detail}") }
}

/// What happened on the pull request, as facts: the agent was told at launch how to react.
#[must_use]
pub fn message(body: &Body, number: u64) -> Option<Message> {
    let (ask, trigger) = ask_of(body)?;
    let (headline, detail, extra_lines) = match body {
        Body::ChecksFailed { jobs, .. } => checks(jobs, number),
        Body::ChangesRequested { author, body, comments, .. } => {
            let with = match comments.len() {
                0 => String::new(),
                n => format!(" ({})", plural(n, "review comment", "review comments")),
            };
            review(&format!("PR #{number}: {author} requested changes{with}."), body, comments)
        }
        Body::Reviewed { author, body, comments, .. } => {
            let did = match comments.len() {
                0 => format!("PR #{number}: {author} left a review."),
                n => format!(
                    "PR #{number}: {} from {author}.",
                    plural(n, "new review comment", "new review comments")
                ),
            };
            review(&did, body, comments)
        }
        Body::Commented { author, text, .. } => {
            let detail = quote(text);
            let lines = detail.lines().count();
            (format!("PR #{number}: {author} commented."), detail, lines)
        }
        Body::Conflict { base } => {
            (format!("PR #{number} conflicts with {base}."), String::new(), 0)
        }
        _ => return None,
    };
    let headline = clean(&headline);
    let detail = clean(&detail);
    let text = cap(&join(&headline, &detail));
    Some(Message { ask, trigger, headline, detail, text, extra_lines })
}

fn checks(jobs: &[Job], number: u64) -> (String, String, usize) {
    let failed: Vec<&Job> = jobs.iter().filter(|job| !job.passed).collect();
    let step_of = |job: &Job| job.step.as_deref().map(short_step).filter(|step| !step.is_empty());
    let named: Vec<String> = failed
        .iter()
        .map(|job| match step_of(job) {
            Some(step) => format!("{} at \"{step}\"", job.name),
            None => job.name.clone(),
        })
        .collect();
    let headline = if named.is_empty() {
        format!("PR #{number}: checks failed.")
    } else {
        format!("PR #{number}: checks failed: {}.", named.join(", "))
    };
    let logs: Vec<String> = failed
        .iter()
        .filter(|job| !job.log.is_empty())
        .map(|job| format!("{}:\n```\n{}\n```", job.name, fence_safe(&job.log).join("\n")))
        .collect();
    let lines = failed.iter().map(|job| job.log.len()).sum();
    (headline, logs.join("\n\n"), lines)
}

fn review(headline: &str, body: &str, comments: &[ReviewComment]) -> (String, String, usize) {
    let mut parts: Vec<String> = Vec::new();
    if !body.trim().is_empty() {
        parts.push(quote(body));
    }
    parts.extend(comments.iter().map(|comment| {
        let place = match comment.line {
            Some(line) => format!("{}:{line}", comment.path),
            None => comment.path.clone(),
        };
        format!("- `{place}` {}", comment.text.trim())
    }));
    (headline.to_owned(), parts.join("\n"), comments.len())
}

fn quote(text: &str) -> String {
    text.trim().lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n")
}

fn fence_safe(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| line.replace("```", "'''")).collect()
}

#[must_use]
pub fn short_step(step: &str) -> String {
    let step = step.strip_prefix("Run ").unwrap_or(step);
    let step = step.strip_prefix("cargo ").unwrap_or(step);
    step.split_whitespace()
        .take_while(|word| !word.starts_with('-'))
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

fn cap(text: &str) -> String {
    if text.chars().count() <= MAX_CHARS {
        return text.to_owned();
    }
    let head: String = text.chars().take(MAX_CHARS).collect();
    format!("{head}\n...")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(name: &str, passed: bool, step: Option<&str>, log: &[&str]) -> Job {
        Job {
            name: name.into(),
            passed,
            step: step.map(str::to_owned),
            secs: None,
            url: String::new(),
            run_id: None,
            job_id: None,
            log: log.iter().map(|line| (*line).to_owned()).collect(),
        }
    }

    fn failed(jobs: Vec<Job>) -> Body {
        Body::ChecksFailed { head: "h".into(), jobs }
    }

    fn comment(path: &str, line: Option<u32>, text: &str) -> ReviewComment {
        ReviewComment { path: path.into(), line, text: text.into() }
    }

    #[test]
    fn one_failed_check_is_a_fact_and_its_log_follows_in_a_fence() {
        let body = failed(vec![
            job("check (macos-14)", true, None, &[]),
            job(
                "check (ubuntu-latest)",
                false,
                Some("Run cargo clippy --workspace --all-targets -- -D warnings"),
                &[
                    "error: unused import: `Sense`",
                    " --> crates/tidewatch/src/leftover_view.rs:6:77",
                ],
            ),
        ]);
        let message = message(&body, 42).unwrap();
        assert_eq!(message.headline, "PR #42: checks failed: check (ubuntu-latest) at \"clippy\".");
        assert_eq!(
            message.text,
            "PR #42: checks failed: check (ubuntu-latest) at \"clippy\".\n\ncheck (ubuntu-latest):\n```\nerror: unused import: `Sense`\n --> crates/tidewatch/src/leftover_view.rs:6:77\n```"
        );
        assert_eq!(message.extra_lines, 2);
        assert_eq!((message.ask.tag, message.ask.verb), ("CI failure", "fix"));
    }

    #[test]
    fn several_failed_checks_are_listed_in_one_sentence() {
        let body = failed(vec![
            job("build (ubuntu)", false, Some("Run cargo test"), &["boom"]),
            job("lint", false, None, &[]),
        ]);
        let message = message(&body, 7).unwrap();
        assert_eq!(message.headline, "PR #7: checks failed: build (ubuntu) at \"test\", lint.");
        assert!(message.text.ends_with("build (ubuntu):\n```\nboom\n```"));
    }

    #[test]
    fn a_log_cannot_close_its_own_fence() {
        let body = failed(vec![job("a", false, None, &["```", "rm -rf"])]);
        let text = message(&body, 1).unwrap().text;
        assert_eq!(text.matches("```").count(), 2, "{text}");
    }

    #[test]
    fn control_characters_never_reach_the_agent() {
        let body = Body::Commented {
            trusted: true,
            author: "rev\u{1b}iewer".into(),
            text: "line one\u{1b}[201~ type this\nline two\u{7}\ttab".into(),
            url: String::new(),
        };
        let text = message(&body, 3).unwrap().text;
        assert!(text.chars().all(|c| matches!(c, '\n' | '\t') || !c.is_control()), "{text:?}");
        assert!(text.contains("> line two\ttab"));
        assert!(text.starts_with("PR #3: reviewer commented."));
    }

    #[test]
    fn a_review_lists_each_comment_with_its_place() {
        let body = Body::ChangesRequested {
            trusted: true,
            author: "reviewer".into(),
            body: "Two things.".into(),
            comments: vec![
                comment("runner.rs", Some(212), "Move it to the stage?"),
                comment("lib.rs", None, " Whole file. "),
            ],
        };
        let message = message(&body, 42).unwrap();
        assert_eq!(
            message.text,
            "PR #42: reviewer requested changes (2 review comments).\n\n> Two things.\n- `runner.rs:212` Move it to the stage?\n- `lib.rs` Whole file."
        );
        assert_eq!(message.extra_lines, 2);
    }

    #[test]
    fn a_review_without_a_verdict_counts_its_comments() {
        let body = Body::Reviewed {
            trusted: true,
            author: "chatgpt-codex-connector".into(),
            body: "### Codex Review\n\nHere are some automated review suggestions.".into(),
            comments: vec![comment("stage.rs", Some(54), "Reset the counter between stages.")],
        };
        let message = message(&body, 7).unwrap();
        assert_eq!(
            message.text,
            "PR #7: 1 new review comment from chatgpt-codex-connector.\n\n> ### Codex Review\n> \n> Here are some automated review suggestions.\n- `stage.rs:54` Reset the counter between stages."
        );
        assert_eq!((message.ask.verb, message.trigger), ("address", Trigger::Reviews));
    }

    #[test]
    fn a_conflict_is_one_fact_with_no_method() {
        let message = message(&Body::Conflict { base: "main".into() }, 9).unwrap();
        assert_eq!(message.text, "PR #9 conflicts with main.");
        assert_eq!((message.ask.verb, message.extra_lines), ("resolve", 0));
    }

    fn every_kind() -> Vec<Body> {
        vec![
            failed(vec![job("lint", false, Some("Run bun run check"), &["boom"])]),
            failed(vec![job("a", false, None, &[]), job("b", false, None, &[])]),
            failed(vec![]),
            Body::ChangesRequested {
                trusted: true,
                author: "rev".into(),
                body: "Two things.".into(),
                comments: vec![comment("a.rs", Some(1), "x")],
            },
            Body::Commented {
                trusted: true,
                author: "rev".into(),
                text: "Why?".into(),
                url: String::new(),
            },
            Body::Conflict { base: "develop".into() },
            Body::Reviewed {
                trusted: true,
                author: "rev".into(),
                body: String::new(),
                comments: vec![comment("a.rs", None, "x")],
            },
            Body::Reviewed {
                trusted: true,
                author: "rev".into(),
                body: "Looks fine.".into(),
                comments: vec![],
            },
        ]
    }

    #[test]
    fn each_message_headline_states_the_fact() {
        let table = [
            "PR #5: checks failed: lint at \"bun run check\".",
            "PR #5: checks failed: a, b.",
            "PR #5: checks failed.",
            "PR #5: rev requested changes (1 review comment).",
            "PR #5: rev commented.",
            "PR #5 conflicts with develop.",
            "PR #5: 1 new review comment from rev.",
            "PR #5: rev left a review.",
        ];
        for (body, want) in every_kind().iter().zip(table) {
            assert_eq!(message(body, 5).unwrap().headline, want);
        }
    }

    #[test]
    fn no_message_tells_the_agent_what_to_do() {
        for body in every_kind() {
            let text = message(&body, 5).unwrap().text.to_lowercase();
            for banned in
                ["fix ", "please", "rebase", "merge origin", "push", "commit", "run the", "answer"]
            {
                assert!(!text.contains(banned), "{banned:?} in {text}");
            }
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
        }
    }

    #[test]
    fn a_click_adds_the_owners_short_request_after_the_facts() {
        let bodies = every_kind();
        let table = [
            (
                0,
                "PR #5: checks failed: lint at \"bun run check\". Please fix CI.\n\nlint:\n```\nboom\n```",
            ),
            (4, "PR #5: rev commented. Please answer the comment.\n\n> Why?"),
            (5, "PR #5 conflicts with develop. Please resolve the conflicts."),
        ];
        for (at, want) in table {
            let message = message(&bodies[at], 5).unwrap();
            assert_eq!(message.asked().text, want, "{at}");
            assert!(!message.text.contains("Please"), "the facts stay without a request");
        }
    }

    #[test]
    fn a_review_that_is_only_a_body_has_no_comment_count() {
        let review = |body: &str| Body::ChangesRequested {
            trusted: true,
            author: "a".into(),
            body: body.into(),
            comments: vec![],
        };
        assert_eq!(
            message(&review("Please."), 1).unwrap().text,
            "PR #1: a requested changes.\n\n> Please."
        );
        assert_eq!(message(&review(""), 1).unwrap().text, "PR #1: a requested changes.");
    }

    #[test]
    fn only_the_four_asking_cards_have_a_message() {
        let silent = [
            Body::Approved { author: "a".into() },
            Body::Merged { number: 1, base: "main".into() },
            Body::Closed { number: 1 },
            Body::ChecksPassed { total: 2, secs: None },
            Body::ChecksRunning { head: "h".into(), done: 0, total: 1 },
            Body::PushFix { commits: 1, head: "h".into() },
        ];
        for body in silent {
            assert_eq!(message(&body, 1), None, "{body:?}");
            assert_eq!(ask_of(&body), None, "{body:?}");
        }
    }

    #[test]
    fn only_a_merge_or_a_close_has_an_ending_note_and_it_is_a_fact() {
        assert_eq!(
            ended_text(&Body::Merged { number: 7, base: "main".into() }).as_deref(),
            Some("PR #7 was merged into main.")
        );
        assert_eq!(
            ended_text(&Body::Closed { number: 7 }).as_deref(),
            Some("PR #7 was closed without merging.")
        );
        assert_eq!(ended_text(&Body::Approved { author: "a".into() }), None);
        assert_eq!(ended_text(&Body::Conflict { base: "main".into() }), None);
    }

    #[test]
    fn an_enormous_comment_is_cut() {
        let body = Body::Commented {
            trusted: true,
            author: "a".into(),
            text: "x".repeat(20_000),
            url: String::new(),
        };
        let message = message(&body, 1).unwrap();
        assert!(message.text.chars().count() < MAX_CHARS + 10);
        assert!(message.text.ends_with("\n..."));
        assert!(message.asked().text.chars().count() < MAX_CHARS + 10);
    }

    #[test]
    fn a_trigger_switched_off_is_not_wanted() {
        let all = GithubPrefs::default();
        let none = GithubPrefs { failed_checks: false, reviews: false, conflicts: false, ..all };
        let bodies = [
            failed(vec![]),
            Body::ChangesRequested {
                trusted: true,
                author: "a".into(),
                body: String::new(),
                comments: vec![],
            },
            Body::Commented {
                trusted: true,
                author: "a".into(),
                text: String::new(),
                url: String::new(),
            },
            Body::Conflict { base: "main".into() },
        ];
        for body in &bodies {
            assert!(wanted(&all, body), "{body:?}");
            assert!(!wanted(&none, body), "{body:?}");
        }
        let only_ci = GithubPrefs { reviews: false, conflicts: false, ..all };
        assert!(wanted(&only_ci, &bodies[0]));
        assert!(!wanted(&only_ci, &bodies[1]));
        assert!(!wanted(&only_ci, &bodies[2]));
        assert!(!wanted(&only_ci, &bodies[3]));
        assert!(!wanted(&all, &Body::Approved { author: "a".into() }));
    }

    #[test]
    fn steps_shorten_to_the_command() {
        let cases = [
            ("Run cargo clippy --workspace --all-targets -- -D warnings", "clippy"),
            ("Run tests", "tests"),
            ("cargo test --workspace", "test"),
            ("Run npm run build --if-present", "npm run build"),
            ("Set up job", "Set up job"),
            ("", ""),
        ];
        for (step, want) in cases {
            assert_eq!(short_step(step), want, "{step:?}");
        }
    }
}
