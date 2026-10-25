//! What a tab that runs on its own does with a branch whose pull request is already merged: nothing.
//! A commit left on it is a quiet note on the merged card, never a new pull request (pure).

/// The pull request of a tab's branch, as far as delivery cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pr {
    None,
    Open,
    Merged,
}

/// What a tab that runs on its own may do about the branch's commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// Whether the delivery card may open a pull request from this branch.
    pub deliver: bool,
    /// The muted line for the merged card, when commits are left on the branch.
    pub note: Option<String>,
}

#[must_use]
pub fn verdict(pr: Pr, commits_after_merge: usize) -> Verdict {
    match pr {
        Pr::None | Pr::Open => Verdict { deliver: true, note: None },
        Pr::Merged => Verdict { deliver: false, note: note(commits_after_merge) },
    }
}

#[must_use]
pub fn note(commits: usize) -> Option<String> {
    let noun = if commits == 1 { "commit" } else { "commits" };
    (commits > 0).then(|| format!("{commits} {noun} after merge, not delivered"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_merged_branch_is_never_delivered_and_says_what_was_left_on_it() {
        let cases = [
            (0, false, None),
            (1, false, Some("1 commit after merge, not delivered")),
            (3, false, Some("3 commits after merge, not delivered")),
        ];
        for (commits, deliver, text) in cases {
            let want = Verdict { deliver, note: text.map(str::to_owned) };
            assert_eq!(verdict(Pr::Merged, commits), want, "{commits} commits");
        }
    }

    #[test]
    fn an_open_pull_request_and_no_pull_request_are_left_as_they_were() {
        for pr in [Pr::Open, Pr::None] {
            for commits in [0, 1, 4] {
                let want = Verdict { deliver: true, note: None };
                assert_eq!(verdict(pr, commits), want, "{pr:?} {commits} commits");
            }
        }
    }

    #[test]
    fn the_note_is_plain_ascii_on_one_line() {
        let text = note(2).unwrap();
        assert!(text.is_ascii());
        assert_eq!(text.lines().count(), 1);
    }
}
