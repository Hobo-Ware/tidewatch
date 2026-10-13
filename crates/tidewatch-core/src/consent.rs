//! D14 consent: the one question Tidewatch asks before it edits any agent CLI's config file, and the
//! list of entries it added so they can be undone. Stored in `~/.config/tidewatch/trust.toml`.
//!
//! The state machine is pure: an event in, the next state and the effects to carry out out. A bad
//! file loads as "not asked yet" with a stderr note, so the worst case is asking again.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::write_atomic;
use crate::trust::{TrustCli, TrustState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Consent {
    #[default]
    Undecided,
    Granted,
    Declined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The first-launch dialog's Yes.
    Yes,
    /// The first-launch dialog's No.
    No,
    /// The settings row's apply button.
    Apply,
    /// The settings row's undo button.
    Undo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Add the Tidewatch root to every installed CLI whose entry covers it.
    TrustRoot,
    /// Remove every entry Tidewatch added.
    RevokeAll,
    Save,
}

impl Consent {
    #[must_use]
    pub fn next(self, event: Event) -> (Self, Vec<Effect>) {
        use Effect::{RevokeAll, Save, TrustRoot};
        match (self, event) {
            (Self::Undecided, Event::Yes | Event::Apply) | (Self::Declined, Event::Apply) => {
                (Self::Granted, vec![TrustRoot, Save])
            }
            (Self::Undecided, Event::No) => (Self::Declined, vec![Save]),
            (Self::Granted, Event::Apply) => (Self::Granted, vec![TrustRoot]),
            (Self::Granted, Event::Undo) => (Self::Declined, vec![RevokeAll, Save]),
            (state, _) => (state, Vec::new()),
        }
    }

    /// The first-launch dialog shows once, and only when some file would change.
    #[must_use]
    pub fn should_ask(self, state: &TrustState) -> bool {
        self == Self::Undecided && state.needs_consent()
    }
}

/// An entry Tidewatch wrote: which CLI's file and the folder it trusts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Added {
    pub cli: TrustCli,
    pub file: PathBuf,
    pub entry: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub consent: Consent,
    #[serde(default)]
    pub added: Vec<Added>,
}

impl Record {
    #[must_use]
    pub fn load(file: &Path) -> Self {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                eprintln!("tidewatch: cannot read {} ({e}); asking again", file.display());
                return Self::default();
            }
        };
        toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("tidewatch: trust.toml is invalid ({e}); asking again");
            Self::default()
        })
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        write_atomic(file, &toml::to_string(self).map_err(std::io::Error::other)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::assess;

    use Consent::{Declined, Granted, Undecided};
    use Effect::{RevokeAll, Save, TrustRoot};

    #[test]
    fn every_state_and_event() {
        let cases = [
            (Undecided, Event::Yes, Granted, vec![TrustRoot, Save]),
            (Undecided, Event::No, Declined, vec![Save]),
            (Undecided, Event::Apply, Granted, vec![TrustRoot, Save]),
            (Undecided, Event::Undo, Undecided, vec![]),
            (Declined, Event::Apply, Granted, vec![TrustRoot, Save]),
            (Declined, Event::Yes, Declined, vec![]),
            (Declined, Event::No, Declined, vec![]),
            (Declined, Event::Undo, Declined, vec![]),
            (Granted, Event::Apply, Granted, vec![TrustRoot]),
            (Granted, Event::Undo, Declined, vec![RevokeAll, Save]),
            (Granted, Event::Yes, Granted, vec![]),
            (Granted, Event::No, Granted, vec![]),
        ];
        for (from, event, to, effects) in cases {
            assert_eq!(from.next(event), (to, effects), "{from:?} + {event:?}");
        }
    }

    #[test]
    fn the_dialog_is_asked_once_and_only_when_something_would_change() {
        let needs = TrustState { clis: vec![assess(TrustCli::Claude, None, None, "/r")] };
        let nothing = TrustState { clis: vec![assess(TrustCli::Gemini, None, None, "/r")] };
        assert!(Undecided.should_ask(&needs));
        assert!(!Undecided.should_ask(&nothing));
        assert!(!Granted.should_ask(&needs));
        assert!(!Declined.should_ask(&needs));
    }

    #[test]
    fn the_record_round_trips_and_a_bad_file_means_ask_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config").join("trust.toml");
        assert_eq!(Record::load(&file), Record::default());
        let record = Record {
            consent: Granted,
            added: vec![Added {
                cli: TrustCli::Copilot,
                file: "/h/.copilot/config.json".into(),
                entry: "/h/.tidewatch".into(),
            }],
        };
        record.save(&file).unwrap();
        assert_eq!(Record::load(&file), record);
        std::fs::write(&file, "consent = [").unwrap();
        assert_eq!(Record::load(&file).consent, Undecided);
    }
}
