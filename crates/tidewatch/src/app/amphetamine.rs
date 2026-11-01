//! The window's side of Amphetamine: the host built for this run, its clock and toasts once per
//! frame, the palette row, and what the button's menu asked for.

use std::sync::Arc;
use std::time::Duration;

use tidewatch_core::amphetamine_system::{FakeSystem, RealSystem, System};
use tidewatch_core::paths::TidewatchPaths;

use super::{Tidewatch, wall_ms};
use crate::amphetamine_glue::{Dev, Files, Host};
use crate::amphetamine_view::Action;
use crate::keep_awake_hold::{Hold, RealHold};
use crate::toast::Toast;
use crate::wake::request_wake;

struct NoHold;

impl Hold for NoHold {
    fn take(&mut self, _reason: &str) -> Result<(), String> {
        Ok(())
    }
    fn release(&mut self) {}
    fn lost(&mut self) -> bool {
        false
    }
}

#[cfg(target_os = "macos")]
fn real_system() -> Arc<dyn System> {
    Arc::new(RealSystem::with_authenticator(
        crate::touch_id::authenticate,
        crate::touch_id::biometrics_available,
    ))
}

#[cfg(not(target_os = "macos"))]
fn real_system() -> Arc<dyn System> {
    Arc::new(RealSystem)
}

pub(super) fn wall_secs() -> f64 {
    wall_ms() as f64 / 1000.0
}

/// The host for this run. A dev switch, a screenshot or a background run gets the recording fake
/// and no hold, so none of them can reach `pmset` or the password prompt.
pub(super) fn host(
    paths: Option<&TidewatchPaths>,
    screenshot: bool,
    background: bool,
    ctx: &egui::Context,
) -> Host<Box<dyn Hold>> {
    let dev = Dev::parse(std::env::var("TIDEWATCH_FAKE_AMPHETAMINE").ok().as_deref());
    let lid = cfg!(target_os = "macos");
    if let Some(dev) = dev {
        let system = match dev {
            Dev::TouchId | Dev::Start => FakeSystem::with_touch_id(),
            Dev::Setup => FakeSystem::with_biometrics(),
            Dev::Pending => FakeSystem::with_biometrics().holding(Duration::from_secs(3600)),
            _ => FakeSystem::new(),
        };
        return Host::dev(&dev, Arc::new(system), Box::new(NoHold), lid, wall_secs());
    }
    if background || screenshot {
        let files = paths.filter(|_| !screenshot).map(Files::of);
        return Host::new(Arc::new(FakeSystem::new()), Box::new(NoHold), files, lid, false);
    }
    let system = real_system();
    let hold: Box<dyn Hold> = Box::new(RealHold::default());
    let host = Host::new(system, hold, paths.map(Files::of), lid, true);
    host.check_last_run(ctx);
    host
}

impl Tidewatch {
    pub(super) fn drive_amphetamine(&mut self, ctx: &egui::Context, now: f64) {
        let wake = self.amphetamine.tick(wall_secs(), now, ctx);
        for voice in self.amphetamine.take_voices() {
            self.toast = Some(Toast::voice(voice.lines(), now));
            ctx.request_repaint();
        }
        if let Some(seconds) = wake {
            request_wake(ctx, seconds);
        }
    }

    pub(super) fn run_amphetamine(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.amphetamine.toggle(wall_secs(), now, ctx);
    }

    pub(super) fn apply_amphetamine(&mut self, action: Action, ctx: &egui::Context) {
        let (wall, now) = (wall_secs(), ctx.input(|i| i.time));
        match action {
            Action::Pick(choice) => self.amphetamine.pick(choice, wall, now, ctx),
            Action::AddHour => self.amphetamine.add_hour(wall, now, ctx),
            Action::TurnOff => self.amphetamine.turn_off(wall, now, ctx),
        }
    }
}
