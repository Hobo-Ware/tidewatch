//! Processes left in a closed tab's worktree that stdusk's pty teardown cannot reach: Claude's Bash
//! tool gives each command its own session, so `vite dev &` ends up orphaned with no terminal.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::elsewhere_scan::{RealSystem, System};
use crate::limited;
use crate::tools::Tools;

const TERM_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    pid: u32,
    ppid: u32,
    has_terminal: bool,
    program: String,
}

fn parse_ps(text: &str) -> Vec<Row> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid = words.next()?.parse().ok()?;
            let ppid = words.next()?.parse().ok()?;
            let has_terminal = !words.next()?.starts_with('?');
            let program = words.collect::<Vec<_>>().join(" ");
            Some(Row { pid, ppid, has_terminal, program })
        })
        .collect()
}

fn is_orphan_reaper(rows: &[Row], pid: u32) -> bool {
    pid == 1
        || rows.iter().any(|row| {
            row.pid == pid && Path::new(&row.program).file_name().is_some_and(|n| n == "systemd")
        })
}

/// The holders that escaped: orphaned and with no terminal. A process with a live parent is in some
/// tree (stdusk reaps the tab's own); one with a terminal belongs to a pty session, which may be
/// another tab's shell opened on the same folder and adopted after a restart.
fn escaped(holders: &[u32], rows: &[Row], own_pid: u32) -> Vec<u32> {
    holders
        .iter()
        .copied()
        .filter(|&pid| pid > 1 && pid != own_pid)
        .filter(|&pid| {
            rows.iter()
                .find(|row| row.pid == pid)
                .is_some_and(|row| !row.has_terminal && is_orphan_reaper(rows, row.ppid))
        })
        .collect()
}

fn run(tools: &Tools, args: &[&str]) -> Option<String> {
    let output = limited::output(
        tools.command(args[0], Path::new("/")).args(&args[1..]).stderr(Stdio::null()),
        tools.limit(),
    )
    .ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn signal(tools: &Tools, signal: &str, pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let mut args = vec!["kill".to_owned(), signal.to_owned()];
    args.extend(pids.iter().map(u32::to_string));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let _ = run(tools, &args);
}

/// Stops what escaped the tab's terminal inside `dir`: SIGTERM, a short grace, then SIGKILL for
/// what is left. Returns the pids it signalled.
#[must_use]
pub fn stop_escaped(dir: &Path) -> Vec<u32> {
    let system = RealSystem::new();
    let tools = Tools::system();
    let Some(holders) = system.cwd_holders(dir).filter(|h| !h.is_empty()) else {
        return Vec::new();
    };
    let Some(text) = run(&tools, &["ps", "-axo", "pid=,ppid=,tty=,comm="]) else {
        return Vec::new();
    };
    let targets = escaped(&holders, &parse_ps(&text), std::process::id());
    signal(&tools, "-TERM", &targets);
    let deadline = Instant::now() + TERM_GRACE;
    let mut left = targets.clone();
    while !left.is_empty() && Instant::now() < deadline {
        std::thread::sleep(POLL);
        left.retain(|&pid| system.alive(pid).is_some());
    }
    signal(&tools, "-KILL", &left);
    targets
}

#[cfg(test)]
mod tests;
