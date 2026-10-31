//! What Tidewatch asks an agent about a pull request: the agent writes and opens it, Tidewatch never does.

const RULES: &str = "\
- Title: Conventional Commits style (`type(scope): summary`), saying what changed for the user.
- Body: what changed and why, how you verified it, and what you did not verify. Keep it short.
- Follow the house style rules you already have: no em or en dashes, no Co-Authored-By trailer, no \"generated with\" footer, and no private repository names when the repository is public.";

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

/// The one message that asks the agent to push and open a draft pull request.
#[must_use]
pub fn open_pull_request(opening: &Opening) -> String {
    let Opening { branch, base, commits, merged_head } = *opening;
    let count = commits_text(commits);
    let how = match merged_head {
        None => format!(
            "You have {count} on `{branch}` that `{base}` does not. Push the branch with `git push -u origin {branch}`, then open a draft pull request into `{base}` with `gh pr create --draft --base {base}`."
        ),
        Some(sha) => format!(
            "Your earlier pull request for `{branch}` merged at `{sha}`, and {count} came after it that `{base}` does not have. Do not push to the merged branch. Move only those commits onto a new branch off `origin/{base}` with `git rebase --onto origin/{base} {sha} {branch}`, push them as `{branch}-2` (the next free number) with `git push -u origin {branch}:{branch}-2`, then open a draft pull request into `{base}` with `gh pr create --draft --base {base} --head {branch}-2`."
        ),
    };
    format!(
        "{how} Write the title and body yourself.\n\n{RULES}\n\nReply with the pull request link once it is open."
    )
}

/// The message that asks the agent to push commits the open pull request does not have yet.
#[must_use]
pub fn push_commits(branch: &str, commits: usize) -> String {
    let count = commits_text(commits);
    format!(
        "You made {count} on `{branch}` after the pull request was opened, and they are not pushed. Push `{branch}` so the pull request has them (`git push`, or `git push --force-with-lease` after a rebase)."
    )
}

/// The fix note of a failed Meeseeks hand-off: the work has no open pull request at its head.
#[must_use]
pub fn meeseeks_fix(branch: &str, base: Option<&str>) -> String {
    let base_flag = base.map_or_else(String::new, |base| format!(" --base {base}"));
    format!(
        "No open pull request has this worktree's HEAD as its head. Push `{branch}` and open a draft pull request yourself with `gh pr create --draft{base_flag}`, with a title and body you write, then call `meeseeks_done` again. If a pull request is already open for this branch, push the latest commits to it.\n\n{RULES}"
    )
}

/// The fix note of a Meeseeks hand-off whose task names a pull request: commits are missing from
/// that pull request's branch. It never asks for a new pull request.
#[must_use]
pub fn meeseeks_push_to_pr(number: u64, head: &str, missing: usize) -> String {
    let count = commits_text(missing);
    let verb = if missing == 1 { "is" } else { "are" };
    format!(
        "{count} of this worktree {verb} not on `{head}`, the branch of PR #{number}. Push your commits to `{head}` (`git push origin HEAD:{head}`, with `--force-with-lease` after a rebase), then call `meeseeks_done` again."
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

    fn opening(commits: usize, merged_head: Option<&str>) -> Opening<'_> {
        Opening { branch: "agent/brave-otter", base: "develop", commits, merged_head }
    }

    #[test]
    fn the_ask_names_the_branch_the_base_the_draft_flag_and_the_house_rules() {
        let text = open_pull_request(&opening(2, None));
        for needle in [
            "agent/brave-otter",
            "into `develop`",
            "gh pr create --draft --base develop",
            "git push -u origin agent/brave-otter",
            "Conventional Commits",
            "how you verified it",
            "what you did not verify",
            "no em or en dashes",
            "Co-Authored-By",
            "generated with",
            "private repository names",
            "Write the title and body yourself",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in {text}");
        }
    }

    #[test]
    fn the_ask_counts_commits_and_never_composes_the_text_for_the_agent() {
        let table = [(1, "You have 1 commit on"), (3, "You have 3 commits on")];
        for (commits, start) in table {
            assert!(open_pull_request(&opening(commits, None)).starts_with(start), "{commits}");
        }
        let text = open_pull_request(&opening(1, None));
        assert!(!text.contains("--title") && !text.contains("--body"), "{text}");
    }

    #[test]
    fn after_a_merge_the_ask_points_at_a_new_branch_and_not_the_merged_one() {
        let text = open_pull_request(&opening(2, Some("e3dba69")));
        for needle in [
            "merged at `e3dba69`",
            "2 commits came after it",
            "Do not push to the merged branch",
            "git rebase --onto origin/develop e3dba69 agent/brave-otter",
            "--head agent/brave-otter-2",
            "gh pr create --draft --base develop",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in {text}");
        }
    }

    #[test]
    fn no_dash_the_house_style_forbids_is_in_what_tidewatch_writes() {
        let all = [
            open_pull_request(&opening(2, None)),
            open_pull_request(&opening(2, Some("abc"))),
            push_commits("agent/x", 1),
            meeseeks_fix("agent/x", Some("main")),
            meeseeks_push_to_pr(7, "fix/x", 2),
            meeseeks_pr_closed(7),
        ];
        for text in all {
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
        }
    }

    #[test]
    fn the_push_ask_names_the_branch_and_the_count() {
        let text = push_commits("agent/x", 2);
        assert!(text.starts_with("You made 2 commits on `agent/x`"), "{text}");
        assert!(push_commits("agent/x", 1).starts_with("You made 1 commit on"));
    }

    #[test]
    fn the_meeseeks_fix_says_to_open_the_draft_itself_and_to_report_again() {
        let text = meeseeks_fix("agent/x", Some("main"));
        for needle in ["gh pr create --draft --base main", "yourself", "meeseeks_done", "agent/x"] {
            assert!(text.contains(needle), "missing {needle:?}");
        }
        assert!(meeseeks_fix("agent/x", None).contains("`gh pr create --draft`,"));
    }

    #[test]
    fn the_notes_for_a_named_pull_request_name_it_and_never_ask_for_a_new_one() {
        let push = meeseeks_push_to_pr(2437, "claude/elegant-fermi", 1);
        for needle in [
            "1 commit of this worktree is not on `claude/elegant-fermi`, the branch of PR #2437",
            "git push origin HEAD:claude/elegant-fermi",
            "call `meeseeks_done` again",
        ] {
            assert!(push.contains(needle), "missing {needle:?} in {push}");
        }
        assert!(meeseeks_push_to_pr(1, "b", 3).starts_with("3 commits of this worktree"));
        for text in [push, meeseeks_pr_closed(2437)] {
            assert!(!text.contains("gh pr create") && !text.contains("open a"), "{text}");
        }
        assert!(meeseeks_pr_closed(2437).contains("PR #2437 was closed without being merged"));
    }
}
