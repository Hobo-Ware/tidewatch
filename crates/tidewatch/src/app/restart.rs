//! Restart to update, from the window's side. The window launches its replacement, sends it the
//! layout and every live terminal (`crate::handoff` holds what is pure) and, once the replacement
//! confirms that all of them run there, closes without stopping any of them.
//!
//! Until that confirmation nothing has changed: any failure leaves the window as it was, with a
//! notice saying why, and the shells still its own.

use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Stdio;

use egui::ViewportCommand;
use stdusk_core::handoff::{LAUNCH_TIMEOUT, accept_within, listen, send_session};
use tidewatch_core::paths::TidewatchPaths;

use super::Tidewatch;
use crate::handoff::{self, Facts};
use crate::update_chip;

impl Tidewatch {
    /// The update pill's Restart. With a home folder the terminals travel to the new window; without
    /// one nothing is saved anyway, so the window just starts again.
    pub(super) fn restart_to_update(&mut self, ctx: &egui::Context) {
        if !self.updates.can_restart() {
            self.show_toast(update_chip::RESTART_WAITS, ctx);
            return;
        }
        let Some(paths) = self.paths.clone() else {
            restart_plainly(ctx);
            return;
        };
        match self.hand_off(&paths) {
            Ok(()) => ctx.send_viewport_cmd(ViewportCommand::Close),
            Err(reason) => {
                eprintln!("tidewatch: restart cancelled: {reason}");
                self.notice = Some(format!("Restart cancelled: {reason}. Nothing was closed."));
                ctx.request_repaint();
            }
        }
    }

    fn hand_off(&mut self, paths: &TidewatchPaths) -> Result<(), String> {
        if let Some(blocker) = handoff::blocker(&self.tabs) {
            return Err(format!("{}, try again in a moment", blocker.text()));
        }
        let socket = paths.handoff_socket(std::process::id());
        let listener =
            listen(&socket).map_err(|e| format!("the handover socket cannot be opened ({e})"))?;
        self.shell.set_closing(true);
        let sent = self.transfer(&listener, &socket);
        let _ = std::fs::remove_file(&socket);
        if sent.is_err() {
            self.shell.set_closing(false);
        }
        sent?;
        handoff::release(&mut self.tabs);
        Ok(())
    }

    fn transfer(&self, listener: &UnixListener, socket: &Path) -> Result<(), String> {
        let facts = Facts {
            workspaces: &self.workspaces,
            focus: self.focus,
            welcome: self.welcome.is_some(),
            hook_port: self.hooks.port(),
        };
        let (handover, order) =
            handoff::plan(&self.tabs, &facts, &|id| self.hooks.credential_of(id));
        spawn_successor(socket).map_err(|e| format!("the new window cannot start ({e})"))?;
        let stream = accept_within(listener, LAUNCH_TIMEOUT)
            .map_err(|e| format!("the new window did not connect ({e})"))?;
        let panes = handoff::capture(&self.tabs, &order)?;
        send_session(&stream, &handover, &panes)
            .map_err(|e| format!("the new window did not take over ({e})"))
    }
}

fn spawn_successor(socket: &Path) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let bundle = kelp_core::update::app_bundle();
    let mut command =
        handoff::successor_command(&exe, bundle.as_deref(), socket, |name| std::env::var_os(name));
    if bundle.is_none() {
        return command.stdin(Stdio::null()).stdout(Stdio::null()).spawn().map(drop);
    }
    let status = command.status()?;
    if status.success() {
        return Ok(());
    }
    Err(std::io::Error::other(format!("open exited with {status}")))
}

/// What the updater's own Restart does: start the app again and close this window, stopping its
/// shells.
fn restart_plainly(ctx: &egui::Context) {
    if let Some(bundle) = kelp_core::update::app_bundle() {
        let _ = std::process::Command::new("open").arg("-n").arg(bundle).spawn();
    }
    ctx.send_viewport_cmd(ViewportCommand::Close);
}
