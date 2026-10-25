//! What a Meeseeks task is: a question or an investigation that only reads, or an edit that changes
//! the worktree. A task starts read-only and is promoted to edit, never the other way (pure).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    ReadOnly,
    #[default]
    Edit,
}

impl Kind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read-only",
            Self::Edit => "Edit",
        }
    }

    /// The word a ledger's `kind:` line holds; `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().replace([' ', '_'], "-").as_str() {
            "read-only" | "readonly" => Some(Self::ReadOnly),
            "edit" => Some(Self::Edit),
            _ => None,
        }
    }

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Edit => "edit",
        }
    }

    /// The kind after `declared` was said or seen: an edit stays an edit.
    #[must_use]
    pub fn promoted_by(self, declared: Self) -> Self {
        if self == Self::Edit || declared == Self::Edit { Self::Edit } else { Self::ReadOnly }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ledger_word_reads_back_as_its_kind_and_anything_else_is_none() {
        let cases = [
            ("read-only", Some(Kind::ReadOnly)),
            (" Read-Only ", Some(Kind::ReadOnly)),
            ("read only", Some(Kind::ReadOnly)),
            ("readonly", Some(Kind::ReadOnly)),
            ("edit", Some(Kind::Edit)),
            ("EDIT", Some(Kind::Edit)),
            ("write", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(Kind::parse(text), want, "{text:?}");
        }
        for kind in [Kind::ReadOnly, Kind::Edit] {
            assert_eq!(Kind::parse(kind.word()), Some(kind));
        }
    }

    #[test]
    fn only_an_edit_declaration_changes_a_kind_and_an_edit_never_goes_back() {
        let cases = [
            (Kind::ReadOnly, Kind::ReadOnly, Kind::ReadOnly),
            (Kind::ReadOnly, Kind::Edit, Kind::Edit),
            (Kind::Edit, Kind::ReadOnly, Kind::Edit),
            (Kind::Edit, Kind::Edit, Kind::Edit),
        ];
        for (now, declared, want) in cases {
            assert_eq!(now.promoted_by(declared), want, "{now:?} {declared:?}");
        }
    }
}
