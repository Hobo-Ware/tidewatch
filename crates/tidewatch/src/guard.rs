//! `tidewatch --guard <git|gh> <args>`: the answer of an agent tab's `git` and `gh` shims. It runs
//! before any window code and ends the process, either refusing the call or running the real tool.

use std::ffi::OsString;

use tidewatch_core::guard_run::{self, GuardEnv, Tool};

pub(crate) const FLAG: &str = "--guard";

pub(crate) fn run(args: &[OsString]) -> i32 {
    let tool = args.first().and_then(|name| name.to_str()).and_then(Tool::parse);
    let env = GuardEnv::from_env(|key| std::env::var_os(key));
    if let (Some(tool), Some(env)) = (tool, env) {
        return guard_run::run(tool, &args[1..], &env);
    }
    eprintln!("tidewatch: --guard needs git or gh and an agent tab's environment");
    2
}
