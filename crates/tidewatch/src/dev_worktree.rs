//! Dev-only switch for the screenshot runs: `TIDEWATCH_DEV_WORKTREE` opens a worktree tab at
//! startup so a capture shows one without a click.
//!
//! - `open`: open one worktree tab.
//! - `ask`: open one, then ask to close it, so the capture shows the close dialog.
//! - `delete`: open one, close it and answer "Delete branch", so the capture shows it gone.
//! - `checking`: open one, close it and never take the answer of the worktree check, so the
//!   capture shows the dialog while it still says it is checking.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevWorktree {
    Open,
    Ask,
    Delete,
    Checking,
}

impl DevWorktree {
    pub(crate) fn from_env() -> Option<Self> {
        Self::parse(&std::env::var("TIDEWATCH_DEV_WORKTREE").ok()?)
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "ask" => Some(Self::Ask),
            "delete" => Some(Self::Delete),
            "checking" => Some(Self::Checking),
            _ => None,
        }
    }

    pub(crate) fn closes_after_opening(self) -> bool {
        self != Self::Open
    }

    pub(crate) fn answers_the_dialog(self) -> bool {
        self == Self::Delete
    }

    pub(crate) fn holds_the_check(self) -> bool {
        self == Self::Checking
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_four_modes_and_nothing_else() {
        assert_eq!(DevWorktree::parse("open"), Some(DevWorktree::Open));
        assert_eq!(DevWorktree::parse("ask"), Some(DevWorktree::Ask));
        assert_eq!(DevWorktree::parse("delete"), Some(DevWorktree::Delete));
        assert_eq!(DevWorktree::parse("checking"), Some(DevWorktree::Checking));
        assert_eq!(DevWorktree::parse(""), None);
        assert_eq!(DevWorktree::parse("OPEN"), None);
    }

    #[test]
    fn only_ask_and_delete_close_and_only_delete_answers() {
        assert!(!DevWorktree::Open.closes_after_opening());
        assert!(DevWorktree::Ask.closes_after_opening());
        assert!(DevWorktree::Delete.closes_after_opening());
        assert!(!DevWorktree::Ask.answers_the_dialog());
        assert!(DevWorktree::Delete.answers_the_dialog());
    }

    #[test]
    fn only_checking_holds_the_check_and_it_closes_the_tab() {
        assert!(DevWorktree::Checking.closes_after_opening());
        assert!(DevWorktree::Checking.holds_the_check());
        assert!(!DevWorktree::Checking.answers_the_dialog());
        for mode in [DevWorktree::Open, DevWorktree::Ask, DevWorktree::Delete] {
            assert!(!mode.holds_the_check(), "{mode:?}");
        }
    }
}
