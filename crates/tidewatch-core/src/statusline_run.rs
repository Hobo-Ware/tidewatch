//! `tidewatch --statusline <tab state dir>`: the status line command of every Claude tab. It saves
//! the numbers in Claude Code's input (see `statusline`), then runs the owner's own status line
//! command with the same input and working folder and hands its output and exit code back, so the
//! footer in the terminal looks as it always did. With no command of the owner's it prints nothing.
//! Nothing here fails the footer: any trouble still runs the owner's command.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::claude_memory::config_dir;
use crate::own_hooks::own_status_line;
use crate::paths::{home_dir, plan_limits_file_in};
use crate::statusline::{self, Reading};

pub const STATUSLINE_FLAG: &str = "--statusline";
const NESTED_ENV: &str = "TIDEWATCH_STATUSLINE_RUNNING";

#[derive(Clone, Debug)]
pub struct Env {
    pub claude_dir: PathBuf,
    pub cwd: Option<PathBuf>,
    /// This run was started by another `--statusline` run: the owner's command is not run again.
    pub nested: bool,
    pub now_ms: i64,
}

impl Env {
    #[must_use]
    pub fn from_process() -> Self {
        let claude_dir = home_dir()
            .map(|home| config_dir(&home, std::env::var_os("CLAUDE_CONFIG_DIR")))
            .unwrap_or_default();
        Self {
            claude_dir,
            cwd: std::env::current_dir().ok(),
            nested: std::env::var_os(NESTED_ENV).is_some(),
            now_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_millis() as i64),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub stdout: Vec<u8>,
    pub code: i32,
}

impl Outcome {
    const SILENT: Self = Self { stdout: Vec::new(), code: 0 };
}

fn save_reading(tab_dir: &Path, input: &str, now_ms: i64) {
    let Some(reading) = statusline::parse(input, now_ms).filter(Reading::has_data) else { return };
    let file = plan_limits_file_in(tab_dir);
    let _ = statusline::save(&file, &reading.over(statusline::load(&file)));
}

fn project_dir(input: &str) -> Option<PathBuf> {
    let root: Value = serde_json::from_str(input).ok()?;
    ["/workspace/project_dir", "/workspace/current_dir", "/cwd"]
        .into_iter()
        .find_map(|pointer| root.pointer(pointer)?.as_str().filter(|dir| !dir.is_empty()))
        .map(PathBuf::from)
}

fn pass_through(command: &str, input: &[u8]) -> Outcome {
    let Ok(mut child) = Command::new("sh")
        .arg("-c")
        .arg(command)
        .env(NESTED_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    else {
        return Outcome::SILENT;
    };
    let feed = child.stdin.take().map(|mut stdin| {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let Ok(output) = child.wait_with_output() else { return Outcome::SILENT };
    if let Some(feed) = feed {
        let _ = feed.join();
    }
    Outcome { stdout: output.stdout, code: output.status.code().unwrap_or(1) }
}

#[must_use]
pub fn run(tab_dir: &Path, input: &[u8], env: &Env) -> Outcome {
    let text = String::from_utf8_lossy(input);
    save_reading(tab_dir, &text, env.now_ms);
    if env.nested {
        return Outcome::SILENT;
    }
    let dir = project_dir(&text).or_else(|| env.cwd.clone()).unwrap_or_default();
    match own_status_line(&env.claude_dir, &dir) {
        Some(own) => pass_through(&own.command, input),
        None => Outcome::SILENT,
    }
}

#[cfg(test)]
mod tests;
