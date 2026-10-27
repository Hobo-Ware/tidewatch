//! Dev-only switches for the screenshot and runnable checks, read once at startup.
//!
//! - `TIDEWATCH_DEV_AGENT=<profile id>`: start that profile in a new tab, as Enter in the dialog would.
//! - `TIDEWATCH_DEV_DIALOG=choose|agent|<profile id>` (or `1`): open the new-tab form on that kind or profile (with `TIDEWATCH_DEV_START=1`, then press Start).
//! - `TIDEWATCH_DEV_FORM_MESSAGE=<text>`: typed into the form's first message box.
//! - `TIDEWATCH_DEV_SETTINGS=1`: open the settings; `general`, `trust`, `shortcuts`, `house` or `worktree` open that page, `dirty` the profiles with an unsaved edit.
//! - `TIDEWATCH_DEV_PALETTE=<query>` (or `1`): open the command palette, filled with that query.
//! - `TIDEWATCH_DEV_TRUST=yes|no|undo`: answer the folder trust question once the CLIs are known (see `trust_glue`).
//! - `TIDEWATCH_DEV_TASK=<text>`: typed into the new tab's composer; with `TIDEWATCH_DEV_START=1` it is
//!   also sent once the tab is ready.
//! - `TIDEWATCH_DEV_LEVEL=plan|ask|edits|auto|yolo`: the permission level to use with it.
//!
//! Use a throwaway repo and a `TIDEWATCH_HOME` outside the real home so nothing lands in the real
//! `~/.tidewatch` or `~/.config/tidewatch`.

use tidewatch_core::agents::Level;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Start,
    Dialog,
    Settings,
    /// Open the command palette; the profile field holds the query.
    Palette,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevAgent {
    pub(crate) mode: Mode,
    /// A profile id, a dialog screen, a settings page or a palette query, by mode.
    pub(crate) profile: Option<String>,
    pub(crate) task: String,
    pub(crate) message: String,
    pub(crate) level: Option<Level>,
    pub(crate) start: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevDraft {
    pub(crate) text: String,
    pub(crate) start: bool,
}

/// The raw switch values.
#[derive(Clone, Copy, Debug, Default)]
struct Vars<'a> {
    agent: Option<&'a str>,
    dialog: Option<&'a str>,
    settings: Option<&'a str>,
    palette: Option<&'a str>,
    task: Option<&'a str>,
    form_message: Option<&'a str>,
    level: Option<&'a str>,
    start: Option<&'a str>,
}

impl DevAgent {
    pub(crate) fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok();
        let agent = var("TIDEWATCH_DEV_AGENT");
        let (dialog, settings, palette, task, form_message, level, start) = (
            var("TIDEWATCH_DEV_DIALOG"),
            var("TIDEWATCH_DEV_SETTINGS"),
            var("TIDEWATCH_DEV_PALETTE"),
            var("TIDEWATCH_DEV_TASK"),
            var("TIDEWATCH_DEV_FORM_MESSAGE"),
            var("TIDEWATCH_DEV_LEVEL"),
            var("TIDEWATCH_DEV_START"),
        );
        Self::parse(Vars {
            agent: agent.as_deref(),
            dialog: dialog.as_deref(),
            settings: settings.as_deref(),
            palette: palette.as_deref(),
            task: task.as_deref(),
            form_message: form_message.as_deref(),
            level: level.as_deref(),
            start: start.as_deref(),
        })
    }

    fn parse(vars: Vars) -> Option<Self> {
        fn set(value: Option<&str>) -> Option<&str> {
            value.filter(|v| !v.is_empty())
        }
        let (mode, profile) =
            match (set(vars.agent), set(vars.dialog), set(vars.settings), set(vars.palette)) {
                (Some(profile), ..) => (Mode::Start, Some(profile.to_owned())),
                (None, Some("1" | "choose"), ..) => (Mode::Dialog, None),
                (None, Some(profile), ..) => (Mode::Dialog, Some(profile.to_owned())),
                (None, None, Some(page), _) => (Mode::Settings, Some(page.to_owned())),
                (None, None, None, Some(query)) => {
                    (Mode::Palette, (query != "1").then(|| query.to_owned()))
                }
                (None, None, None, None) => return None,
            };
        Some(Self {
            mode,
            profile,
            task: vars.task.unwrap_or_default().to_owned(),
            message: vars.form_message.unwrap_or_default().to_owned(),
            level: vars.level.and_then(parse_level),
            start: vars.start == Some("1"),
        })
    }
}

fn parse_level(name: &str) -> Option<Level> {
    Level::ALL.into_iter().find(|level| format!("{level:?}").eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars<'static> {
        Vars::default()
    }

    #[test]
    fn an_agent_switch_starts_the_profile_and_beats_the_dialog_switch() {
        let dev = DevAgent::parse(Vars {
            agent: Some("builder"),
            dialog: Some("1"),
            task: Some("hi"),
            level: Some("auto"),
            start: Some("1"),
            ..vars()
        })
        .unwrap();
        assert_eq!(
            dev,
            DevAgent {
                mode: Mode::Start,
                profile: Some("builder".into()),
                task: "hi".into(),
                message: String::new(),
                level: Some(Level::Auto),
                start: true
            }
        );
    }

    #[test]
    fn the_dialog_switch_also_names_the_screens() {
        let choose = DevAgent::parse(Vars { dialog: Some("choose"), ..vars() }).unwrap();
        assert_eq!((choose.mode, choose.profile), (Mode::Dialog, None));
        let agent = DevAgent::parse(Vars { dialog: Some("agent"), ..vars() }).unwrap();
        assert_eq!((agent.mode, agent.profile.as_deref()), (Mode::Dialog, Some("agent")));
        assert!(!agent.start);
    }

    #[test]
    fn the_form_message_switch_fills_the_form_and_start_presses_its_button() {
        let dev = DevAgent::parse(Vars {
            dialog: Some("agent"),
            form_message: Some("add saved searches"),
            start: Some("1"),
            ..vars()
        })
        .unwrap();
        assert_eq!(
            (dev.mode, dev.message.as_str(), dev.start),
            (Mode::Dialog, "add saved searches", true)
        );
        assert_eq!(DevAgent::parse(Vars { form_message: Some("x"), ..vars() }), None);
    }

    #[test]
    fn the_dialog_switch_takes_a_profile_or_one_for_the_default() {
        let one = DevAgent::parse(Vars { dialog: Some("1"), ..vars() }).unwrap();
        assert_eq!((one.mode, one.profile), (Mode::Dialog, None));
        let named = DevAgent::parse(Vars { dialog: Some("reviewer"), ..vars() }).unwrap();
        assert_eq!((named.mode, named.profile.as_deref()), (Mode::Dialog, Some("reviewer")));
    }

    #[test]
    fn the_settings_switch_opens_the_editor() {
        let dev = DevAgent::parse(Vars { settings: Some("1"), ..vars() }).unwrap();
        assert_eq!(dev.mode, Mode::Settings);
        let page = DevAgent::parse(Vars { settings: Some("trust"), ..vars() }).unwrap();
        assert_eq!((page.mode, page.profile.as_deref()), (Mode::Settings, Some("trust")));
    }

    #[test]
    fn the_palette_switch_takes_a_query_or_one_for_an_empty_palette() {
        let one = DevAgent::parse(Vars { palette: Some("1"), ..vars() }).unwrap();
        assert_eq!((one.mode, one.profile), (Mode::Palette, None));
        let query = DevAgent::parse(Vars { palette: Some("tab"), ..vars() }).unwrap();
        assert_eq!((query.mode, query.profile.as_deref()), (Mode::Palette, Some("tab")));
        let settings = DevAgent::parse(Vars { settings: Some("1"), palette: Some("1"), ..vars() });
        assert_eq!(settings.unwrap().mode, Mode::Settings);
    }

    #[test]
    fn no_switch_or_empty_values_mean_no_dev_agent() {
        assert_eq!(DevAgent::parse(Vars { task: Some("task"), ..vars() }), None);
        let empty = Vars {
            agent: Some(""),
            dialog: Some(""),
            settings: Some(""),
            palette: Some(""),
            ..vars()
        };
        assert_eq!(DevAgent::parse(empty), None);
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
