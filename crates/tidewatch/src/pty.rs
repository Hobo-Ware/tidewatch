//! Starting the ptys a tab runs in: a plain shell in a folder, or the user's shell running the
//! script that starts an agent.

use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::path::Path;

use stdusk_core::agent_pane::{AgentSetup, PROMPT_FALLBACK};
use stdusk_core::handoff::{PaneMeta, adopt_pane};
use stdusk_core::shell::ShellProfile;
use stdusk_core::terminal::{PtyTerm, SpawnOpts};
use stdusk_egui::repaint_handle;
use tidewatch_core::prepare::{Prepared, note_script};

const INITIAL_COLS: usize = 80;
const INITIAL_ROWS: usize = 24;
const SCROLLBACK_LINES: usize = 25_000;
const WORD_SEPARATORS: &str = " ()[]{}'\"";

pub(crate) fn spawn_shell(ctx: &egui::Context, dir: &Path) -> PtyTerm {
    PtyTerm::spawn(INITIAL_COLS, INITIAL_ROWS, repaint_handle(ctx), &spawn_opts(dir, None))
}

/// A terminal another window sent over, still running what it ran. Its own folder, which the
/// shell cannot report until its next prompt, stands in for `dir`.
pub(crate) fn adopt(
    ctx: &egui::Context,
    meta: &PaneMeta,
    fd: OwnedFd,
    dir: &Path,
) -> std::io::Result<PtyTerm> {
    let mut opts = spawn_opts(dir, None);
    if let Some(cwd) = &meta.cwd {
        opts.cwd = Some(cwd.clone());
    }
    adopt_pane(repaint_handle(ctx), meta, fd, &opts)
}

/// A plain shell that prints `note` first, for a restored tab whose agent could not be resumed.
pub(crate) fn spawn_shell_noted(ctx: &egui::Context, dir: &Path, note: Option<&str>) -> PtyTerm {
    let Some(note) = note else { return spawn_shell(ctx, dir) };
    let shell = user_shell(std::env::var("SHELL").ok());
    let profile = note_profile(&shell, note);
    PtyTerm::spawn(INITIAL_COLS, INITIAL_ROWS, repaint_handle(ctx), &spawn_opts(dir, Some(profile)))
}

fn note_profile(shell: &str, note: &str) -> ShellProfile {
    ShellProfile {
        shell: Some(shell.to_owned()),
        args: vec!["-c".to_owned(), note_script(note, shell)],
        cwd: None,
        env: BTreeMap::new(),
    }
}

/// The user's shell runs `prepared.script` as its command, so it reads the login profile like any
/// tab and the agent ends up as the pty's process.
pub(crate) fn spawn_agent(ctx: &egui::Context, dir: &Path, prepared: &Prepared) -> PtyTerm {
    let shell = user_shell(std::env::var("SHELL").ok());
    let profile = ShellProfile {
        shell: Some(shell.clone()),
        args: vec!["-c".to_owned(), prepared.script(&shell)],
        cwd: None,
        env: prepared.env.iter().cloned().collect::<BTreeMap<_, _>>(),
    };
    PtyTerm::spawn(INITIAL_COLS, INITIAL_ROWS, repaint_handle(ctx), &spawn_opts(dir, Some(profile)))
}

/// A plain interactive shell, with the agent's environment, that has the agent's command typed at
/// its first prompt and waits for the user to press Enter. A shell that marks no prompt gets the
/// fallback timer, which `PtyTerm::release_pending_after_wait` runs.
pub(crate) fn spawn_typed(ctx: &egui::Context, dir: &Path, prepared: &Prepared) -> PtyTerm {
    let profile = ShellProfile {
        shell: Some(user_shell(std::env::var("SHELL").ok())),
        args: Vec::new(),
        cwd: None,
        env: prepared.env.iter().cloned().collect::<BTreeMap<_, _>>(),
    };
    let mut opts = spawn_opts(dir, Some(profile));
    opts.agent_tracking = Some(AgentSetup {
        pending_input: Some(typed_line(prepared)),
        fallback_delay: PROMPT_FALLBACK,
    });
    PtyTerm::spawn(INITIAL_COLS, INITIAL_ROWS, repaint_handle(ctx), &opts)
}

fn typed_line(prepared: &Prepared) -> Vec<u8> {
    prepared.launch.shell_line().into_bytes()
}

fn spawn_opts(dir: &Path, profile: Option<ShellProfile>) -> SpawnOpts {
    SpawnOpts {
        detect_progress: true,
        shell_integration: true,
        autosuggestions: false,
        scrollback_lines: SCROLLBACK_LINES,
        word_separators: WORD_SEPARATORS.into(),
        bold_bright: true,
        cwd: Some(dir.to_string_lossy().into_owned()),
        profile,
        agent_tracking: None,
    }
}

/// `$SHELL`, or the platform's default login shell when it is unset or empty.
fn user_shell(env: Option<String>) -> String {
    env.filter(|shell| !shell.is_empty()).unwrap_or_else(|| {
        if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" }.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_noted_shell_runs_the_note_script_in_the_users_shell() {
        let profile = note_profile("/bin/zsh", "The agent was not resumed: it is gone.");
        assert_eq!(profile.shell.as_deref(), Some("/bin/zsh"));
        assert_eq!(profile.args[0], "-c");
        assert!(profile.args[1].contains("The agent was not resumed: it is gone."));
        assert!(profile.args[1].ends_with("; exec /bin/zsh -l -i"), "{}", profile.args[1]);
    }

    #[test]
    fn the_typed_line_is_the_agents_whole_command_and_never_ends_in_enter() {
        use tidewatch_core::launch::Launch;
        let prepared = Prepared {
            launch: Launch {
                program: "claude".into(),
                args: vec![
                    "--resume".into(),
                    "abc".into(),
                    "--settings".into(),
                    "{\"a\": 1}".into(),
                ],
                env: Vec::new(),
                experimental: false,
            },
            env: Vec::new(),
            setup: String::new(),
            notes: Vec::new(),
            remote: None,
        };
        let line = String::from_utf8(typed_line(&prepared)).unwrap();
        assert_eq!(line, "claude --resume abc --settings '{\"a\": 1}'");
        assert!(!line.contains(['\r', '\n']));
    }

    #[test]
    fn the_shell_comes_from_the_environment_or_the_platform_default() {
        assert_eq!(user_shell(Some("/usr/bin/fish".into())), "/usr/bin/fish");
        for unset in [None, Some(String::new())] {
            let shell = user_shell(unset);
            assert!(shell == "/bin/zsh" || shell == "/bin/bash", "{shell}");
        }
    }
}
