//! Dev-only switch for the screenshot runs: `TIDEWATCH_DEV_WORKTREE` opens a worktree tab at
//! startup so a capture shows one without a click.
//!
//! - `open`: open one worktree tab.
//! - `ask`: open one, then ask to close it, so the capture shows the close dialog.
//! - `delete`: open one, close it and answer "Delete branch", so the capture shows it gone.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevWorktree {
    Open,
    Ask,
    Delete,
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
            _ => None,
        }
    }

    pub(crate) fn closes_after_opening(self) -> bool {
        self != Self::Open
    }

    pub(crate) fn answers_the_dialog(self) -> bool {
        self == Self::Delete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_modes_and_nothing_else() {
        assert_eq!(DevWorktree::parse("open"), Some(DevWorktree::Open));
        assert_eq!(DevWorktree::parse("ask"), Some(DevWorktree::Ask));
        assert_eq!(DevWorktree::parse("delete"), Some(DevWorktree::Delete));
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
}
