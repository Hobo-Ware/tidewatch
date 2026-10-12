//! Dev-only switches for the screenshot and runnable checks, read once at startup.
//!
//! - `TIDEWATCH_DEV_AGENT=<profile id>`: start that profile in a new tab, as Enter in the dialog would.
//! - `TIDEWATCH_DEV_DIALOG=<profile id>` (or `1`): open the new-tab dialog, on that profile.
//! - `TIDEWATCH_DEV_SETTINGS=1`: open the profile editor.
//! - `TIDEWATCH_DEV_TASK=<text>` and `TIDEWATCH_DEV_LEVEL=plan|ask|edits|auto|yolo`: the task and the
//!   permission level to use with either.
//!
//! Use a throwaway repo and a `TIDEWATCH_HOME` outside the real home so nothing lands in the real
//! `~/.tidewatch` or `~/.config/tidewatch`.

use tidewatch_core::agents::Level;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Start,
    Dialog,
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevAgent {
    pub(crate) mode: Mode,
    pub(crate) profile: Option<String>,
    pub(crate) task: String,
    pub(crate) level: Option<Level>,
}

impl DevAgent {
    pub(crate) fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok();
        Self::parse(
            var("TIDEWATCH_DEV_AGENT").as_deref(),
            var("TIDEWATCH_DEV_DIALOG").as_deref(),
            var("TIDEWATCH_DEV_SETTINGS").as_deref(),
            var("TIDEWATCH_DEV_TASK").as_deref(),
            var("TIDEWATCH_DEV_LEVEL").as_deref(),
        )
    }

    fn parse(
        agent: Option<&str>,
        dialog: Option<&str>,
        settings: Option<&str>,
        task: Option<&str>,
        level: Option<&str>,
    ) -> Option<Self> {
        let (mode, profile) = match (
            agent.filter(|a| !a.is_empty()),
            dialog.filter(|d| !d.is_empty()),
            settings.filter(|s| !s.is_empty()),
        ) {
            (Some(profile), _, _) => (Mode::Start, Some(profile.to_owned())),
            (None, Some("1"), _) => (Mode::Dialog, None),
            (None, Some(profile), _) => (Mode::Dialog, Some(profile.to_owned())),
            (None, None, Some(_)) => (Mode::Settings, None),
            (None, None, None) => return None,
        };
        Some(Self {
            mode,
            profile,
            task: task.unwrap_or_default().to_owned(),
            level: level.and_then(parse_level),
        })
    }
}

fn parse_level(name: &str) -> Option<Level> {
    Level::ALL.into_iter().find(|level| format!("{level:?}").eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_switch_starts_the_profile_and_beats_the_dialog_switch() {
        let dev =
            DevAgent::parse(Some("builder"), Some("1"), None, Some("hi"), Some("auto")).unwrap();
        assert_eq!(
            dev,
            DevAgent {
                mode: Mode::Start,
                profile: Some("builder".into()),
                task: "hi".into(),
                level: Some(Level::Auto)
            }
        );
    }

    #[test]
    fn the_dialog_switch_takes_a_profile_or_one_for_the_default() {
        let one = DevAgent::parse(None, Some("1"), None, None, None).unwrap();
        assert_eq!((one.mode, one.profile), (Mode::Dialog, None));
        let named = DevAgent::parse(None, Some("reviewer"), None, None, None).unwrap();
        assert_eq!((named.mode, named.profile.as_deref()), (Mode::Dialog, Some("reviewer")));
    }

    #[test]
    fn the_settings_switch_opens_the_editor() {
        let dev = DevAgent::parse(None, None, Some("1"), None, None).unwrap();
        assert_eq!(dev.mode, Mode::Settings);
    }

    #[test]
    fn no_switch_or_empty_values_mean_no_dev_agent() {
        assert_eq!(DevAgent::parse(None, None, None, Some("task"), None), None);
        assert_eq!(DevAgent::parse(Some(""), Some(""), Some(""), None, None), None);
    }

    #[test]
    fn levels_parse_by_name_and_junk_is_ignored() {
        let cases = [
            ("plan", Some(Level::Plan)),
            ("ASK", Some(Level::Ask)),
            ("edits", Some(Level::Edits)),
            ("auto", Some(Level::Auto)),
            ("yolo", Some(Level::Yolo)),
            ("max", None),
        ];
        for (name, want) in cases {
            assert_eq!(parse_level(name), want, "{name}");
        }
    }
}
