//! The one runner for a child process Tidewatch waits on. At the limit the whole process group is
//! killed, so a credential helper or pager started by a hung `git` or `gh` goes too.

use std::fmt;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const DEFAULT: Duration = Duration::from_secs(60);
/// `git worktree add` and `remove` write a whole checkout, so they get longer.
pub const WORKTREE: Duration = Duration::from_secs(180);

const POLL_FIRST: Duration = Duration::from_millis(1);
const POLL_MAX: Duration = Duration::from_millis(25);
const PIPE_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum Ended {
    Exited(Output),
    Stopped(Output),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stopped {
    pub program: String,
    pub limit: Duration,
}

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} took longer than {} and was stopped", self.program, describe(self.limit))
    }
}

impl std::error::Error for Stopped {}

#[must_use]
pub fn describe(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs == 0 {
        format!("{} ms", duration.as_millis())
    } else if secs >= 120 && secs.is_multiple_of(60) {
        format!("{} min", secs / 60)
    } else {
        format!("{secs} s")
    }
}

#[must_use]
pub fn is_stopped(error: &io::Error) -> bool {
    error.get_ref().is_some_and(<dyn std::error::Error + Send + Sync>::is::<Stopped>)
}

#[must_use]
pub fn failure(program: &str, error: &io::Error) -> String {
    if is_stopped(error) { error.to_string() } else { format!("cannot run {program}: {error}") }
}

fn program_name(command: &Command) -> String {
    let program = Path::new(command.get_program());
    program.file_name().unwrap_or(program.as_os_str()).to_string_lossy().into_owned()
}

fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        while let Ok(n) = pipe.read(&mut chunk) {
            if n == 0 || sender.send(chunk[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    receiver
}

fn collect(receiver: Option<mpsc::Receiver<Vec<u8>>>, until: Instant) -> Vec<u8> {
    let mut bytes = Vec::new();
    let Some(receiver) = receiver else { return bytes };
    while let Ok(chunk) = receiver.recv_timeout(until.saturating_duration_since(Instant::now())) {
        bytes.extend(chunk);
    }
    bytes
}

fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
}

/// # Errors
/// The program could not start, or its status could not be read.
pub fn run(command: &mut Command, limit: Duration) -> io::Result<Ended> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let deadline = Instant::now() + limit;
    let mut pause = POLL_FIRST;
    let (status, stopped): (Option<ExitStatus>, bool) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false),
            Ok(None) if Instant::now() >= deadline => {
                kill_group(child.id());
                let _ = child.kill();
                break (child.wait().ok(), true);
            }
            Ok(None) => {
                std::thread::sleep(pause.min(deadline.saturating_duration_since(Instant::now())));
                pause = (pause * 2).min(POLL_MAX);
            }
            Err(e) => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        }
    };
    let until = Instant::now() + PIPE_GRACE;
    let output = Output {
        status: status.unwrap_or_default(),
        stdout: collect(stdout, until),
        stderr: collect(stderr, until),
    };
    Ok(if stopped { Ended::Stopped(output) } else { Ended::Exited(output) })
}

/// # Errors
/// As [`run`], and the time limit.
pub fn output(command: &mut Command, limit: Duration) -> io::Result<Output> {
    let program = program_name(command);
    match run(command, limit)? {
        Ended::Exited(output) => Ok(output),
        Ended::Stopped(_) => {
            Err(io::Error::new(io::ErrorKind::TimedOut, Stopped { program, limit }))
        }
    }
}

#[cfg(test)]
mod tests;
