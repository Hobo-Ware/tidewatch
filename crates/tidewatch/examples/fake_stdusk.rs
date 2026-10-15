//! A stand-in for stdusk's "Move tabs to Tidewatch", run by hand to check the receiving side:
//!
//! `cargo run -p tidewatch --example fake_stdusk -- <tidewatch executable> <folder>...`
//!
//! Spawns one interactive shell per folder (the last one holds focus), launches
//! `<tidewatch> --from-stdusk <socket>` with this environment (so `TIDEWATCH_HOME` picks the
//! window), sends the shells over the socket and waits for the window's confirmation. Then it
//! prints `pid <n> <folder>` per shell and `acked`, and exits without stopping any of them: they
//! belong to the window now. Run it against a `TIDEWATCH_BACKGROUND=1` window in a short throwaway
//! `TIDEWATCH_HOME`, never against the real one.

use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use stdusk_core::handoff::export::{Export, ExportPane};
use stdusk_core::handoff::send_session;
use stdusk_core::handoff::{LAUNCH_TIMEOUT, PaneMeta, accept_within, capture_pane, listen};
use stdusk_core::repaint::Repaint;
use stdusk_core::shell::ShellProfile;
use stdusk_core::terminal::{PtyTerm, SpawnOpts};

fn shell_in(dir: &str) -> PtyTerm {
    let opts = SpawnOpts {
        detect_progress: false,
        shell_integration: false,
        autosuggestions: false,
        scrollback_lines: 1000,
        word_separators: " ".into(),
        bold_bright: false,
        agent_tracking: None,
        cwd: Some(dir.to_owned()),
        profile: Some(ShellProfile {
            shell: Some("/bin/sh".into()),
            args: Vec::new(),
            cwd: None,
            env: std::collections::BTreeMap::new(),
        }),
    };
    PtyTerm::spawn(80, 24, Repaint::none(), &opts)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(exe), dirs) = (args.next(), args.collect::<Vec<_>>()) else {
        eprintln!("usage: fake_stdusk <tidewatch executable> <folder>...");
        std::process::exit(2);
    };
    if dirs.is_empty() {
        eprintln!("usage: fake_stdusk <tidewatch executable> <folder>...");
        std::process::exit(2);
    }
    let mut terms: Vec<PtyTerm> = dirs.iter().map(|dir| shell_in(dir)).collect();
    let export = Export {
        panes: dirs
            .iter()
            .enumerate()
            .map(|(at, dir)| ExportPane {
                title: Some(format!("moved {}", at + 1)),
                cwd: Some(dir.clone()),
                focused: at + 1 == dirs.len(),
            })
            .collect(),
    };

    let socket = PathBuf::from(format!("/tmp/fake-stdusk-{}.sock", std::process::id()));
    let listener = listen(&socket).expect("the handover socket");
    let mut child = Command::new(&exe)
        .arg("--from-stdusk")
        .arg(&socket)
        .stdin(Stdio::null())
        .spawn()
        .expect("tidewatch starts");
    std::thread::spawn(move || child.wait());
    let sent = accept_within(&listener, LAUNCH_TIMEOUT).and_then(|stream| {
        let panes: Vec<(PaneMeta, BorrowedFd<'_>)> =
            terms.iter().map(|term| capture_pane(term).expect("a master fd")).collect();
        send_session(&stream, &export, &panes)
    });
    let _ = std::fs::remove_file(&socket);
    if let Err(e) = sent {
        eprintln!("not taken over: {e}");
        std::process::exit(1);
    }
    for term in &mut terms {
        term.mark_handed_off();
    }
    for (term, dir) in terms.iter().zip(&dirs) {
        println!("pid {} {dir}", term.shell_pid().unwrap_or_default());
    }
    println!("acked");
}
