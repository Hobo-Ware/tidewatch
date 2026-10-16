//! The text Tidewatch pastes into an agent when GitHub has news about its pull request (PLAN.md, D4
//! follow-up). Pure.

use crate::pr_status::{Body, Job, ReviewComment};
use crate::prefs::GithubPrefs;

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

#[must_use]
pub fn ask_of(body: &Body) -> Option<(Ask, Trigger)> {
    let (verb, phrase, tag, trigger) = match body {
        Body::ChecksFailed { .. } => ("fix", "fix CI", "CI failure", Trigger::FailedChecks),
        Body::ChangesRequested { .. } => {
            ("address", "address the review", "Review", Trigger::Reviews)
        }
        Body::Commented { .. } => ("answer", "answer the comment", "Comment", Trigger::Reviews),
        Body::Conflict { .. } => ("rebase", "rebase", "Conflict", Trigger::Conflicts),
        _ => return None,
    };
    Some((Ask { verb, phrase, tag }, trigger))
}

#[must_use]
pub fn wanted(prefs: &GithubPrefs, body: &Body) -> bool {
    ask_of(body).is_some_and(|(_, trigger)| match trigger {
        Trigger::FailedChecks => prefs.failed_checks,
        Trigger::Reviews => prefs.reviews,
        Trigger::Conflicts => prefs.conflicts,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub ask: Ask,
    pub headline: String,
    pub text: String,
    pub extra_lines: usize,
}

#[must_use]
pub fn message(body: &Body, number: u64) -> Option<Message> {
    let (ask, _) = ask_of(body)?;
    let (headline, detail, extra_lines) = match body {
        Body::ChecksFailed { jobs, .. } => checks(jobs, number),
        Body::ChangesRequested { author, body, comments } => review(author, body, comments, number),
        Body::Commented { author, text, .. } => {
            let detail = quote(text);
            let lines = detail.lines().count();
            (
                format!(
                    "@{author} commented on PR #{number}. Answer it; if it needs a code change, make it and commit."
                ),
                detail,
                lines,
            )
        }
        Body::Conflict { base } => (
            format!(
                "PR #{number} conflicts with {base}. Fetch, merge origin/{base} into this branch, resolve the conflicts, run the tests, and commit. Do not rebase or force-push."
            ),
            String::new(),
            0,
        ),
        _ => return None,
    };
    let headline = clean(&headline);
    let detail = clean(&detail);
    let text = if detail.is_empty() { headline.clone() } else { format!("{headline}\n\n{detail}") };
    Some(Message { ask, headline, text: cap(&text), extra_lines })
}

fn checks(jobs: &[Job], number: u64) -> (String, String, usize) {
    let failed: Vec<&Job> = jobs.iter().filter(|job| !job.passed).collect();
    let step_of = |job: &Job| job.step.as_deref().map(short_step).filter(|step| !step.is_empty());
    let headline = match failed.as_slice() {
        [] => format!("The checks failed on PR #{number}. Find out why, fix it, and commit."),
        [job] => {
            let at = step_of(job).map_or_else(String::new, |step| format!(" at {step}"));
            format!(
                "{} failed{at} on PR #{number}. Fix it, run the same check locally, and commit.",
                job.name
            )
        }
        many => {
            let named: Vec<String> = many
                .iter()
                .map(|job| match step_of(job) {
                    Some(step) => format!("{} at {step}", job.name),
                    None => job.name.clone(),
                })
                .collect();
            format!(
                "{} checks failed on PR #{number}: {}. Fix them, run the same checks locally, and commit.",
                many.len(),
                named.join(", ")
            )
        }
    };
    let logs: Vec<String> = failed
        .iter()
        .filter(|job| !job.log.is_empty())
        .map(|job| format!("{}:\n```\n{}\n```", job.name, fence_safe(&job.log).join("\n")))
        .collect();
    let lines = failed.iter().map(|job| job.log.len()).sum();
    (headline, logs.join("\n\n"), lines)
}

fn review(
    author: &str,
    body: &str,
    comments: &[ReviewComment],
    number: u64,
) -> (String, String, usize) {
    let headline = format!(
        "@{author} requested changes on PR #{number}. Address each comment, run the tests, and commit."
    );
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
    (headline, parts.join("\n"), comments.len())
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

/// Control characters are stripped like in `note_paste`, because a pull request's comments and CI
/// logs are other people's text and must never end the bracketed paste early.
fn clean(text: &str) -> String {
    text.chars().filter(|c| matches!(c, '\n' | '\t') || !c.is_control()).collect()
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

    #[test]
    fn one_failed_check_gets_the_short_sentence_and_its_log_in_a_fence() {
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
        assert_eq!(
            message.headline,
            "check (ubuntu-latest) failed at clippy on PR #42. Fix it, run the same check locally, and commit."
        );
        assert_eq!(
            message.text,
            "check (ubuntu-latest) failed at clippy on PR #42. Fix it, run the same check locally, and commit.\n\ncheck (ubuntu-latest):\n```\nerror: unused import: `Sense`\n --> crates/tidewatch/src/leftover_view.rs:6:77\n```"
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
        assert_eq!(
            message.headline,
            "2 checks failed on PR #7: build (ubuntu) at test, lint. Fix them, run the same checks locally, and commit."
        );
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
            author: "rev\u{1b}iewer".into(),
            text: "line one\u{1b}[201~ type this\nline two\u{7}\ttab".into(),
            url: String::new(),
        };
        let text = message(&body, 3).unwrap().text;
        assert!(text.chars().all(|c| matches!(c, '\n' | '\t') || !c.is_control()), "{text:?}");
        assert!(text.contains("> line two\ttab"));
        assert!(text.starts_with("@reviewer commented on PR #3."));
    }

    #[test]
    fn a_review_lists_each_comment_with_its_place() {
        let body = Body::ChangesRequested {
            author: "reviewer".into(),
            body: "Two things.".into(),
            comments: vec![
                ReviewComment {
                    path: "runner.rs".into(),
                    line: Some(212),
                    text: "Move it to the stage?".into(),
                },
                ReviewComment { path: "lib.rs".into(), line: None, text: " Whole file. ".into() },
            ],
        };
        let message = message(&body, 42).unwrap();
        assert_eq!(
            message.text,
            "@reviewer requested changes on PR #42. Address each comment, run the tests, and commit.\n\n> Two things.\n- `runner.rs:212` Move it to the stage?\n- `lib.rs` Whole file."
        );
        assert_eq!(message.extra_lines, 2);
    }

    #[test]
    fn a_conflict_asks_for_a_merge_because_the_agent_cannot_force_push() {
        let message = message(&Body::Conflict { base: "main".into() }, 9).unwrap();
        assert_eq!(message.ask.verb, "rebase");
        assert!(message.text.contains("merge origin/main"));
        assert!(message.text.contains("Do not rebase or force-push"));
        assert_eq!(message.extra_lines, 0);
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
    fn an_enormous_comment_is_cut() {
        let body =
            Body::Commented { author: "a".into(), text: "x".repeat(20_000), url: String::new() };
        let text = message(&body, 1).unwrap().text;
        assert!(text.chars().count() < MAX_CHARS + 10);
        assert!(text.ends_with("\n..."));
    }

    #[test]
    fn a_trigger_switched_off_is_not_wanted() {
        let all = GithubPrefs::default();
        let none = GithubPrefs { failed_checks: false, reviews: false, conflicts: false, ..all };
        let bodies = [
            failed(vec![]),
            Body::ChangesRequested { author: "a".into(), body: String::new(), comments: vec![] },
            Body::Commented { author: "a".into(), text: String::new(), url: String::new() },
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
