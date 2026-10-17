//! Dev-only switches for the screenshot and runnable checks, read once at startup.
//!
//! - `TIDEWATCH_DEV_AGENT=<profile id>`: start that profile in a new tab, as Enter in the dialog would.
//! - `TIDEWATCH_DEV_DIALOG=choose|agent|workflow|<profile id>|workflow:<id>` (or `1`): open the new-tab form on that kind, profile or workflow (with `TIDEWATCH_DEV_START=1`, then press Start).
//! - `TIDEWATCH_DEV_FORM_MESSAGE=<text>`: typed into the form's first message or task box.
//! - `TIDEWATCH_DEV_WORKFLOW=<workflow id>`: open a tab for that workflow, which then asks for its task.
//! - `TIDEWATCH_DEV_RESUME=<worktree name>`: open the tab of a saved workflow run and carry on where it was.
//! - `TIDEWATCH_DEV_SETTINGS=1`: open the settings; `general`, `workflows`, `trust`, `shortcuts` or `house` open that page, `dirty` the profiles with an unsaved edit.
//! - `TIDEWATCH_DEV_PROPOSE=<toml file>`: open that workflow (a `workflows.toml` table or one bare workflow) as a
//!   proposal from a stand-in tab, the way an agent's answer opens: Settings, Workflows with an unsaved draft.
//! - `TIDEWATCH_DEV_PALETTE=<query>` (or `1`): open the command palette, filled with that query.
//! - `TIDEWATCH_DEV_TRUST=yes|no|undo`: answer the folder trust question once the CLIs are known (see `trust_glue`).
//! - `TIDEWATCH_DEV_TASK=<text>`: typed into the new tab's composer (an agent) or task box (a workflow); with
//!   `TIDEWATCH_DEV_START=1` it is also sent, or the run started, once the tab is ready.
//! - `TIDEWATCH_DEV_LEVEL=plan|ask|edits|auto|yolo`: the permission level to use with either.
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
    /// Open the tab of a saved workflow run (the worktree's name) and carry on.
    Resume,
    /// Open the workflow in the toml file (the profile field) as a proposal in the editor.
    Propose,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevAgent {
    pub(crate) mode: Mode,
    /// A profile id, `workflow:<id>` for a workflow, or the worktree name to resume.
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
    workflow: Option<&'a str>,
    resume: Option<&'a str>,
    dialog: Option<&'a str>,
    settings: Option<&'a str>,
    palette: Option<&'a str>,
    propose: Option<&'a str>,
    task: Option<&'a str>,
    form_message: Option<&'a str>,
    level: Option<&'a str>,
    start: Option<&'a str>,
}

impl DevAgent {
    pub(crate) fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok();
        let (agent, workflow, resume) = (
            var("TIDEWATCH_DEV_AGENT"),
            var("TIDEWATCH_DEV_WORKFLOW"),
            var("TIDEWATCH_DEV_RESUME"),
        );
        let (dialog, settings, palette, propose, task, form_message, level, start) = (
            var("TIDEWATCH_DEV_DIALOG"),
            var("TIDEWATCH_DEV_SETTINGS"),
            var("TIDEWATCH_DEV_PALETTE"),
            var("TIDEWATCH_DEV_PROPOSE"),
            var("TIDEWATCH_DEV_TASK"),
            var("TIDEWATCH_DEV_FORM_MESSAGE"),
            var("TIDEWATCH_DEV_LEVEL"),
            var("TIDEWATCH_DEV_START"),
        );
        Self::parse(Vars {
            agent: agent.as_deref(),
            workflow: workflow.as_deref(),
            resume: resume.as_deref(),
            dialog: dialog.as_deref(),
            settings: settings.as_deref(),
            palette: palette.as_deref(),
            propose: propose.as_deref(),
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
        if let Some(file) = set(vars.propose) {
            return Some(Self {
                mode: Mode::Propose,
                profile: Some(file.to_owned()),
                task: String::new(),
                message: String::new(),
                level: None,
                start: false,
            });
        }
        let (mode, profile) = match (
            set(vars.agent),
            set(vars.workflow),
            set(vars.resume),
            set(vars.dialog),
            set(vars.settings),
            set(vars.palette),
        ) {
            (Some(profile), ..) => (Mode::Start, Some(profile.to_owned())),
            (None, Some(workflow), ..) => (Mode::Start, Some(format!("workflow:{workflow}"))),
            (None, None, Some(name), ..) => (Mode::Resume, Some(name.to_owned())),
            (None, None, None, Some("1" | "choose"), ..) => (Mode::Dialog, None),
            (None, None, None, Some(profile), ..) => (Mode::Dialog, Some(profile.to_owned())),
            (None, None, None, None, Some(page), _) => (Mode::Settings, Some(page.to_owned())),
            (None, None, None, None, None, Some(query)) => {
                (Mode::Palette, (query != "1").then(|| query.to_owned()))
            }
            (None, None, None, None, None, None) => return None,
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
    fn a_workflow_switch_starts_that_workflow_with_the_task() {
        let dev = DevAgent::parse(Vars {
            workflow: Some("ship"),
            task: Some("add a retry"),
            dialog: Some("1"),
            ..vars()
        })
        .unwrap();
        assert_eq!(
            (dev.mode, dev.profile.as_deref(), dev.task.as_str()),
            (Mode::Start, Some("workflow:ship"), "add a retry")
        );
        let agent =
            DevAgent::parse(Vars { agent: Some("builder"), workflow: Some("ship"), ..vars() });
        assert_eq!(agent.unwrap().profile.as_deref(), Some("builder"));
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
            dialog: Some("workflow"),
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
    fn a_propose_switch_names_the_toml_file_and_beats_the_settings_switch() {
        let dev = DevAgent::parse(Vars {
            propose: Some("/tmp/x/mine.toml"),
            settings: Some("workflows"),
            ..vars()
        })
        .unwrap();
        assert_eq!((dev.mode, dev.profile.as_deref()), (Mode::Propose, Some("/tmp/x/mine.toml")));
        assert_eq!(DevAgent::parse(Vars { propose: Some(""), ..vars() }), None);
    }

    #[test]
    fn a_resume_switch_names_the_worktree_of_the_saved_run() {
        let dev = DevAgent::parse(Vars { resume: Some("brave-otter"), ..vars() }).unwrap();
        assert_eq!((dev.mode, dev.profile.as_deref()), (Mode::Resume, Some("brave-otter")));
    }

    #[test]
    fn the_dialog_switch_takes_a_profile_a_workflow_or_one_for_the_default() {
        let one = DevAgent::parse(Vars { dialog: Some("1"), ..vars() }).unwrap();
        assert_eq!((one.mode, one.profile), (Mode::Dialog, None));
        let named = DevAgent::parse(Vars { dialog: Some("reviewer"), ..vars() }).unwrap();
        assert_eq!((named.mode, named.profile.as_deref()), (Mode::Dialog, Some("reviewer")));
        let flow = DevAgent::parse(Vars { dialog: Some("workflow:fix"), ..vars() }).unwrap();
        assert_eq!((flow.mode, flow.profile.as_deref()), (Mode::Dialog, Some("workflow:fix")));
    }

    #[test]
    fn the_settings_switch_opens_the_editor() {
        let dev = DevAgent::parse(Vars { settings: Some("1"), ..vars() }).unwrap();
        assert_eq!(dev.mode, Mode::Settings);
        let page = DevAgent::parse(Vars { settings: Some("workflows"), ..vars() }).unwrap();
        assert_eq!((page.mode, page.profile.as_deref()), (Mode::Settings, Some("workflows")));
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
            workflow: Some(""),
            resume: Some(""),
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
