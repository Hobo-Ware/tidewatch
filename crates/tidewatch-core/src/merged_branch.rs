//! What the merged card says about commits left on a branch whose pull request already merged
//! (pure). Whether they go to a new pull request is the tab's Auto setting, not decided here.

#[must_use]
pub fn note(commits: usize) -> Option<String> {
    let noun = if commits == 1 { "commit" } else { "commits" };
    (commits > 0).then(|| format!("{commits} {noun} after merge, not delivered"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_says_what_was_left_on_a_merged_branch() {
        let cases = [
            (0, None),
            (1, Some("1 commit after merge, not delivered")),
            (3, Some("3 commits after merge, not delivered")),
        ];
        for (commits, text) in cases {
            assert_eq!(note(commits).as_deref(), text, "{commits} commits");
        }
    }

    #[test]
    fn the_note_is_plain_ascii_on_one_line() {
        let text = note(2).unwrap();
        assert!(text.is_ascii());
        assert_eq!(text.lines().count(), 1);
    }
}
