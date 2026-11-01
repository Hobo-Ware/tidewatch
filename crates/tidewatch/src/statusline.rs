//! `tidewatch --statusline <tab state dir>`: the status line command of an agent tab. It runs before
//! any window code and ends the process, handing back what the owner's own status line printed.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;

use tidewatch_core::statusline_run::{self, Env};

pub(crate) use tidewatch_core::statusline_run::STATUSLINE_FLAG as FLAG;

pub(crate) fn run(args: &[OsString]) -> i32 {
    let Some(tab_dir) = args.first() else {
        eprintln!("tidewatch: --statusline needs the tab's state folder");
        return 2;
    };
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    let outcome = statusline_run::run(Path::new(tab_dir), &input, &Env::from_process());
    let mut out = std::io::stdout();
    let _ = out.write_all(&outcome.stdout);
    let _ = out.flush();
    outcome.code
}
