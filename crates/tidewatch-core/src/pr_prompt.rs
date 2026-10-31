//! The text Tidewatch pastes into an agent when GitHub has news about its pull request (PLAN.md, D4
//! follow-up). Pure.

use crate::pr_status::{Body, Job, ReviewComment};
use crate::prefs::GithubPrefs;
use crate::workflow_profile::{Fix, Workflow};

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
        Body::ChangesRequested { trusted: false, .. } | Body::Commented { trusted: false, .. }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub ask: Ask,
    pub headline: String,
    pub text: String,
    pub extra_lines: usize,
}

#[must_use]
pub fn message(body: &Body, number: u64, workflow: Workflow) -> Option<Message> {
    let (ask, _) = ask_of(body)?;
    let (headline, detail, extra_lines) = match body {
        Body::ChecksFailed { jobs, .. } => checks(jobs, number, workflow),
        Body::ChangesRequested { author, body, comments, .. } => {
            review(author, body, comments, number, workflow)
        }
        Body::Commented { author, text, .. } => {
            let detail = quote(text);
            let lines = detail.lines().count();
            (
                format!(
                    "@{author} commented on PR #{number} (below). Answer on GitHub. If it needs a code change, {}.",
                    workflow.fix_and_push(Fix::It)
                ),
                detail,
                lines,
            )
        }
        Body::Conflict { base } => (
            format!(
                "PR #{number} conflicts with {base}. {}, resolve the conflicts, run the tests, and {}.",
                workflow.catch_up_clause(base),
                workflow.push_after_catch_up()
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

fn checks(jobs: &[Job], number: u64, workflow: Workflow) -> (String, String, usize) {
    let failed: Vec<&Job> = jobs.iter().filter(|job| !job.passed).collect();
    let step_of = |job: &Job| job.step.as_deref().map(short_step).filter(|step| !step.is_empty());
    let has_log = failed.iter().any(|job| !job.log.is_empty());
    let headline = match failed.as_slice() {
        [] => format!(
            "The checks failed on PR #{number}. Find out why, fix it, run the same checks locally, then {}.",
            workflow.fix_and_push(Fix::The)
        ),
        [job] => {
            let at = step_of(job).map_or_else(String::new, |step| format!(" at \"{step}\""));
            let log = if has_log { " (log below)" } else { "" };
            format!(
                "{} failed{at} on PR #{number}{log}. Fix it, run the same check locally, then {}.",
                job.name,
                workflow.fix_and_push(Fix::The)
            )
        }
        many => {
            let named: Vec<String> = many
                .iter()
                .map(|job| match step_of(job) {
                    Some(step) => format!("{} at \"{step}\"", job.name),
                    None => job.name.clone(),
                })
                .collect();
            let log = if has_log { " (logs below)" } else { "" };
            format!(
                "{} checks failed on PR #{number}: {}{log}. Fix them, run the same checks locally, then {}.",
                many.len(),
                named.join(", "),
                workflow.fix_and_push(Fix::Each)
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
    workflow: Workflow,
) -> (String, String, usize) {
    let below = match comments.len() {
        0 if body.trim().is_empty() => String::new(),
        0 => " (below)".to_owned(),
        1 => " (1 comment below)".to_owned(),
        n => format!(" ({n} comments below)"),
    };
    let headline = format!(
        "@{author} requested changes on PR #{number}{below}. Fix them, run the tests, {}.",
        workflow.fix_and_push(Fix::Each)
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
    use crate::workflow_profile::{CatchUp, Fixes};

    const REBASE: Workflow = Workflow {
        catch_up: CatchUp::Rebase,
        fixes: Fixes::Fixup,
        pull_requests: crate::workflow_profile::PullRequests::Draft,
    };
    const MERGE: Workflow = Workflow {
        catch_up: CatchUp::Merge,
        fixes: Fixes::NewCommits,
        pull_requests: crate::workflow_profile::PullRequests::Draft,
    };

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
        let message = message(&body, 42, REBASE).unwrap();
        assert_eq!(
            message.headline,
            "check (ubuntu-latest) failed at \"clippy\" on PR #42 (log below). Fix it, run the same check locally, then fold the fix into the commit it belongs to and push with --force-with-lease."
        );
        assert_eq!(
            message.text,
            "check (ubuntu-latest) failed at \"clippy\" on PR #42 (log below). Fix it, run the same check locally, then fold the fix into the commit it belongs to and push with --force-with-lease.\n\ncheck (ubuntu-latest):\n```\nerror: unused import: `Sense`\n --> crates/tidewatch/src/leftover_view.rs:6:77\n```"
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
        let message = message(&body, 7, MERGE).unwrap();
        assert_eq!(
            message.headline,
            "2 checks failed on PR #7: build (ubuntu) at \"test\", lint (logs below). Fix them, run the same checks locally, then commit each fix and push."
        );
        assert!(message.text.ends_with("build (ubuntu):\n```\nboom\n```"));
    }

    #[test]
    fn a_log_cannot_close_its_own_fence() {
        let body = failed(vec![job("a", false, None, &["```", "rm -rf"])]);
        let text = message(&body, 1, REBASE).unwrap().text;
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
        let text = message(&body, 3, REBASE).unwrap().text;
        assert!(text.chars().all(|c| matches!(c, '\n' | '\t') || !c.is_control()), "{text:?}");
        assert!(text.contains("> line two\ttab"));
        assert!(text.starts_with("@reviewer commented on PR #3 (below)."));
    }

    #[test]
    fn a_review_lists_each_comment_with_its_place() {
        let body = Body::ChangesRequested {
            trusted: true,
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
        let message = message(&body, 42, REBASE).unwrap();
        assert_eq!(
            message.text,
            "@reviewer requested changes on PR #42 (2 comments below). Fix them, run the tests, fold each fix into its commit and push with --force-with-lease.\n\n> Two things.\n- `runner.rs:212` Move it to the stage?\n- `lib.rs` Whole file."
        );
        assert_eq!(message.extra_lines, 2);
    }

    #[test]
    fn a_conflict_names_the_fact_and_fills_the_method_from_the_profile() {
        let conflict = Body::Conflict { base: "main".into() };
        let rebase = message(&conflict, 9, REBASE).unwrap();
        assert_eq!(
            rebase.text,
            "PR #9 conflicts with main. Rebase onto origin/main, resolve the conflicts, run the tests, and push with --force-with-lease."
        );
        let merge = message(&conflict, 9, MERGE).unwrap();
        assert_eq!(
            merge.text,
            "PR #9 conflicts with main. Merge origin/main into this branch, resolve the conflicts, run the tests, and push."
        );
        assert_eq!((rebase.ask.verb, rebase.extra_lines), ("resolve", 0));
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
                comments: vec![ReviewComment {
                    path: "a.rs".into(),
                    line: Some(1),
                    text: "x".into(),
                }],
            },
            Body::Commented {
                trusted: true,
                author: "rev".into(),
                text: "Why?".into(),
                url: String::new(),
            },
            Body::Conflict { base: "develop".into() },
        ]
    }

    #[test]
    fn no_message_says_a_method_the_profile_did_not_choose() {
        for body in every_kind() {
            let rebase = message(&body, 5, REBASE).unwrap().text;
            for banned in [
                "Do not rebase",
                "do not rebase",
                "merge origin",
                "Merge origin",
                "and commit",
                "Fetch, merge",
            ] {
                assert!(!rebase.contains(banned), "{banned:?} in {rebase}");
            }
            let merge = message(&body, 5, MERGE).unwrap().text;
            for banned in ["Rebase onto", "fold ", "fixup", "autosquash"] {
                assert!(!merge.contains(banned), "{banned:?} in {merge}");
            }
            for text in [&rebase, &merge] {
                assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
            }
        }
    }

    #[test]
    fn each_message_under_the_two_profiles() {
        let table = [
            (
                0,
                "lint failed at \"bun run check\" on PR #5 (log below). Fix it, run the same check locally, then fold the fix into the commit it belongs to and push with --force-with-lease.",
                "lint failed at \"bun run check\" on PR #5 (log below). Fix it, run the same check locally, then commit the fix and push.",
            ),
            (
                2,
                "The checks failed on PR #5. Find out why, fix it, run the same checks locally, then fold the fix into the commit it belongs to and push with --force-with-lease.",
                "The checks failed on PR #5. Find out why, fix it, run the same checks locally, then commit the fix and push.",
            ),
            (
                3,
                "@rev requested changes on PR #5 (1 comment below). Fix them, run the tests, fold each fix into its commit and push with --force-with-lease.",
                "@rev requested changes on PR #5 (1 comment below). Fix them, run the tests, commit each fix and push.",
            ),
            (
                4,
                "@rev commented on PR #5 (below). Answer on GitHub. If it needs a code change, fold it into its commit and push with --force-with-lease.",
                "@rev commented on PR #5 (below). Answer on GitHub. If it needs a code change, commit it and push.",
            ),
            (
                5,
                "PR #5 conflicts with develop. Rebase onto origin/develop, resolve the conflicts, run the tests, and push with --force-with-lease.",
                "PR #5 conflicts with develop. Merge origin/develop into this branch, resolve the conflicts, run the tests, and push.",
            ),
        ];
        let bodies = every_kind();
        for (at, under_rebase, under_merge) in table {
            assert_eq!(message(&bodies[at], 5, REBASE).unwrap().headline, under_rebase, "{at}");
            assert_eq!(message(&bodies[at], 5, MERGE).unwrap().headline, under_merge, "{at}");
        }
    }

    #[test]
    fn a_review_that_is_only_a_body_says_below_and_one_with_nothing_says_nothing() {
        let review = |body: &str| Body::ChangesRequested {
            trusted: true,
            author: "a".into(),
            body: body.into(),
            comments: vec![],
        };
        assert!(
            message(&review("Please."), 1, REBASE).unwrap().headline.contains("PR #1 (below).")
        );
        assert!(message(&review(""), 1, REBASE).unwrap().headline.contains("PR #1. Fix them"));
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
            assert_eq!(message(&body, 1, REBASE), None, "{body:?}");
            assert_eq!(ask_of(&body), None, "{body:?}");
        }
    }

    #[test]
    fn an_enormous_comment_is_cut() {
        let body = Body::Commented {
            trusted: true,
            author: "a".into(),
            text: "x".repeat(20_000),
            url: String::new(),
        };
        let text = message(&body, 1, REBASE).unwrap().text;
        assert!(text.chars().count() < MAX_CHARS + 10);
        assert!(text.ends_with("\n..."));
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
