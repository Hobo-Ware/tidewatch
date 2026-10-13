//! Starting the ptys a tab runs in: a plain shell in a folder, or the user's shell running the
//! script that starts an agent.

use std::collections::BTreeMap;
use std::path::Path;

use stdusk_core::shell::ShellProfile;
use stdusk_core::terminal::{PtyTerm, SpawnOpts};
use stdusk_egui::repaint_handle;
use tidewatch_core::prepare::Prepared;

const INITIAL_COLS: usize = 80;
const INITIAL_ROWS: usize = 24;
const SCROLLBACK_LINES: usize = 25_000;
const WORD_SEPARATORS: &str = " ()[]{}'\"";

pub(crate) fn spawn_shell(ctx: &egui::Context, dir: &Path) -> PtyTerm {
    PtyTerm::spawn(INITIAL_COLS, INITIAL_ROWS, repaint_handle(ctx), &spawn_opts(dir, None))
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
    fn the_shell_comes_from_the_environment_or_the_platform_default() {
        assert_eq!(user_shell(Some("/usr/bin/fish".into())), "/usr/bin/fish");
        for unset in [None, Some(String::new())] {
            let shell = user_shell(unset);
            assert!(shell == "/bin/zsh" || shell == "/bin/bash", "{shell}");
        }
    }
}
