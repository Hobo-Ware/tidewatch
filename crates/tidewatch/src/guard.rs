//! `tidewatch --guard <git|gh> <args>`: the answer of an agent tab's `git` and `gh` shims. It runs
//! before any window code and ends the process, either refusing the call or running the real tool.

use std::ffi::OsString;
use std::io::Read;

use tidewatch_core::guard_run::{self, GuardEnv, Tool};
use tidewatch_core::{guard_bash, guard_comments};

pub(crate) const FLAG: &str = "--guard";
const COMMENTS: &str = "comments";
const TRAILERS: &str = "trailers";
const STOP: &str = "--stop";

pub(crate) fn run(args: &[OsString]) -> i32 {
    if args.first().is_some_and(|word| word == COMMENTS) {
        return comments(args.get(1).is_some_and(|word| word == STOP));
    }
    if args.first().is_some_and(|word| word == TRAILERS) {
        return trailers();
    }
    let tool = args.first().and_then(|name| name.to_str()).and_then(Tool::parse);
    let env = GuardEnv::from_env(|key| std::env::var_os(key));
    if let (Some(tool), Some(env)) = (tool, env) {
        return guard_run::run(tool, &args[1..], &env);
    }
    eprintln!("tidewatch: --guard needs git or gh and an agent tab's environment");
    2
}

/// The comment guard's hook: the payload comes on stdin, a flag is exit 2 with the report on stderr.
/// Outside an agent tab's environment, or on any trouble reading, it does nothing.
fn comments(stop: bool) -> i32 {
    let Some(env) = GuardEnv::from_env(|key| std::env::var_os(key)) else { return 0 };
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return 0;
    }
    guard_comments::run(stop, &payload, &env)
}

/// The trailers guard's PreToolUse hook on Bash: the payload comes on stdin, a refusal is exit 2
/// with the reason on stderr. Outside an agent tab's environment, or on any trouble reading, it
/// does nothing.
fn trailers() -> i32 {
    let Some(env) = GuardEnv::from_env(|key| std::env::var_os(key)) else { return 0 };
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return 0;
    }
    guard_bash::run(&payload, &env)
}
