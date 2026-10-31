//! What Tidewatch asks an agent about a pull request: the agent writes and opens it, Tidewatch never does.

use crate::workflow_profile::Workflow;

/// The facts the ask names: the branch, the base it targets and what is ahead of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening<'a> {
    pub branch: &'a str,
    pub base: &'a str,
    pub commits: usize,
    /// The head the branch's earlier pull request merged with, when it did.
    pub merged_head: Option<&'a str>,
}

fn commits_text(commits: usize) -> String {
    match commits {
        1 => "1 commit".to_owned(),
        n => format!("{n} commits"),
    }
}

/// The one message that asks the agent to push and open a pull request.
#[must_use]
pub fn open_pull_request(opening: &Opening, workflow: Workflow) -> String {
    let Opening { branch, base, commits, merged_head } = *opening;
    let count = commits_text(commits);
    let pull_request = workflow.new_pull_request();
    let how = match merged_head {
        None => {
            let verb = if commits == 1 { "is" } else { "are" };
            format!(
                "{count} on `{branch}` {verb} not in `{base}`. Push the branch and open {pull_request} into `{base}`."
            )
        }
        Some(sha) => format!(
            "Your earlier pull request for `{branch}` merged at `{sha}`, and {count} came after it that `{base}` does not have. Move only those commits onto a new branch off `origin/{base}`, push it as `{branch}-2` (the next free number), and open {pull_request} into `{base}`."
        ),
    };
    format!("{how} Write the title and body yourself. Reply with the link.")
}

/// The message that asks the agent to push commits the open pull request does not have yet.
#[must_use]
pub fn push_commits(number: Option<u64>, commits: usize, workflow: Workflow) -> String {
    let verb = if commits == 1 { "is" } else { "are" };
    let target = number.map_or_else(|| "the pull request".to_owned(), |n| format!("PR #{n}"));
    let lease = if workflow.pushes_unpushed_with_lease() { " with --force-with-lease" } else { "" };
    format!("{} {verb} not on {target} yet. Push them{lease}.", commits_text(commits))
}

/// The fix note of a failed Meeseeks hand-off: the work has no open pull request at its head.
#[must_use]
pub fn meeseeks_fix(branch: &str, base: Option<&str>, workflow: Workflow) -> String {
    let base_flag = base.map_or_else(String::new, |base| format!(" --base {base}"));
    let draft = workflow.draft_flag();
    let pull_request = workflow.new_pull_request();
    format!(
        "No open pull request has this worktree's HEAD as its head. Push `{branch}` and open {pull_request} yourself with `gh pr create{draft}{base_flag}`, with a title and body you write, then call `meeseeks_done` again. If a pull request is already open for this branch, push the latest commits to it."
    )
}

/// The fix note of a Meeseeks hand-off whose task names a pull request: commits are missing from
/// that pull request's branch. It never asks for a new pull request.
#[must_use]
pub fn meeseeks_push_to_pr(number: u64, head: &str, missing: usize, workflow: Workflow) -> String {
    let count = commits_text(missing);
    let verb = if missing == 1 { "is" } else { "are" };
    let lease = if workflow.pushes_unpushed_with_lease() {
        ", with `--force-with-lease` after a rebase"
    } else {
        ""
    };
    format!(
        "{count} of this worktree {verb} not on `{head}`, the branch of PR #{number}. Push your commits to `{head}` (`git push origin HEAD:{head}`{lease}), then call `meeseeks_done` again."
    )
}

/// The fix note of a Meeseeks hand-off whose pull request was closed without merging.
#[must_use]
pub fn meeseeks_pr_closed(number: u64) -> String {
    format!(
        "PR #{number} was closed without being merged, so there is nothing to update. Ask the user what to do with your question tool, then call `meeseeks_done` again."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_profile::{CatchUp, Fixes, PullRequests};

    const DRAFT: Workflow = Workflow {
        catch_up: CatchUp::Rebase,
        fixes: Fixes::Fixup,
        pull_requests: PullRequests::Draft,
    };
    const READY: Workflow = Workflow {
        catch_up: CatchUp::Merge,
        fixes: Fixes::NewCommits,
        pull_requests: PullRequests::Ready,
    };

    fn opening(commits: usize, merged_head: Option<&str>) -> Opening<'_> {
        Opening { branch: "agent/brave-otter", base: "develop", commits, merged_head }
    }

    #[test]
    fn the_ask_states_the_facts_and_fills_the_pull_request_kind_from_the_profile() {
        assert_eq!(
            open_pull_request(&opening(3, None), DRAFT),
            "3 commits on `agent/brave-otter` are not in `develop`. Push the branch and open a draft pull request into `develop`. Write the title and body yourself. Reply with the link."
        );
        assert_eq!(
            open_pull_request(&opening(1, None), READY),
            "1 commit on `agent/brave-otter` is not in `develop`. Push the branch and open a pull request (not a draft) into `develop`. Write the title and body yourself. Reply with the link."
        );
    }

    #[test]
    fn the_ask_no_longer_repeats_what_the_guards_enforce_or_composes_the_text() {
        for workflow in [DRAFT, READY] {
            for merged in [None, Some("abc")] {
                let text = open_pull_request(&opening(2, merged), workflow);
                for banned in [
                    "em or en dashes",
                    "Co-Authored-By",
                    "generated with",
                    "private repository",
                    "Conventional Commits",
                    "--title",
                    "--body",
                    "gh pr create",
                    "git push",
                ] {
                    assert!(!text.contains(banned), "{banned:?} in {text}");
                }
            }
        }
    }

    #[test]
    fn after_a_merge_the_ask_points_at_a_new_branch_named_with_the_next_number() {
        assert_eq!(
            open_pull_request(&opening(2, Some("e3dba69")), DRAFT),
            "Your earlier pull request for `agent/brave-otter` merged at `e3dba69`, and 2 commits came after it that `develop` does not have. Move only those commits onto a new branch off `origin/develop`, push it as `agent/brave-otter-2` (the next free number), and open a draft pull request into `develop`. Write the title and body yourself. Reply with the link."
        );
        let ready = open_pull_request(&opening(1, Some("e3dba69")), READY);
        assert!(ready.contains("open a pull request (not a draft) into `develop`"), "{ready}");
        assert!(!ready.contains("rebase --onto"), "{ready}");
    }

    #[test]
    fn no_dash_the_house_style_forbids_is_in_what_tidewatch_writes() {
        for workflow in [DRAFT, READY] {
            let all = [
                open_pull_request(&opening(2, None), workflow),
                open_pull_request(&opening(2, Some("abc")), workflow),
                push_commits(Some(7), 1, workflow),
                push_commits(None, 2, workflow),
                meeseeks_fix("agent/x", Some("main"), workflow),
                meeseeks_push_to_pr(7, "fix/x", 2, workflow),
                meeseeks_pr_closed(7),
            ];
            for text in all {
                assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
            }
        }
    }

    #[test]
    fn the_push_ask_names_the_pull_request_and_the_lease_only_when_the_profile_rewrites() {
        assert_eq!(
            push_commits(Some(2421), 2, DRAFT),
            "2 commits are not on PR #2421 yet. Push them with --force-with-lease."
        );
        assert_eq!(
            push_commits(Some(2421), 1, READY),
            "1 commit is not on PR #2421 yet. Push them."
        );
        assert_eq!(
            push_commits(None, 3, READY),
            "3 commits are not on the pull request yet. Push them."
        );
    }

    #[test]
    fn the_meeseeks_fix_says_to_open_the_pull_request_itself_and_to_report_again() {
        let text = meeseeks_fix("agent/x", Some("main"), DRAFT);
        for needle in [
            "a draft pull request yourself",
            "gh pr create --draft --base main",
            "meeseeks_done",
            "agent/x",
        ] {
            assert!(text.contains(needle), "missing {needle:?}");
        }
        assert!(meeseeks_fix("agent/x", None, DRAFT).contains("`gh pr create --draft`,"));
        let ready = meeseeks_fix("agent/x", Some("main"), READY);
        assert!(ready.contains("`gh pr create --base main`"), "{ready}");
        assert!(ready.contains("(not a draft)"), "{ready}");
    }

    #[test]
    fn the_notes_for_a_named_pull_request_name_it_and_never_ask_for_a_new_one() {
        let push = meeseeks_push_to_pr(2437, "claude/elegant-fermi", 1, DRAFT);
        for needle in [
            "1 commit of this worktree is not on `claude/elegant-fermi`, the branch of PR #2437",
            "git push origin HEAD:claude/elegant-fermi",
            "call `meeseeks_done` again",
        ] {
            assert!(push.contains(needle), "missing {needle:?} in {push}");
        }
        assert!(meeseeks_push_to_pr(1, "b", 3, DRAFT).starts_with("3 commits of this worktree"));
        assert!(push.contains("`--force-with-lease` after a rebase"), "{push}");
        let merge = meeseeks_push_to_pr(2437, "claude/elegant-fermi", 1, READY);
        assert!(!merge.contains("force-with-lease"), "{merge}");
        for text in [push, meeseeks_pr_closed(2437)] {
            assert!(!text.contains("gh pr create") && !text.contains("open a"), "{text}");
        }
        assert!(meeseeks_pr_closed(2437).contains("PR #2437 was closed without being merged"));
    }
}
