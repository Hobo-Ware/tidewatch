//! Screenshot harness: with TIDEWATCH_SCREENSHOT set to a PNG path, render a few frames, save the
//! window and quit. TIDEWATCH_SCREENSHOT_WAIT (seconds) waits longer first, for a slow shell.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{UserData, ViewportCommand};

const SETTLE_FRAMES: u32 = 3;
const POLL: Duration = Duration::from_millis(50);
const DEFAULT_WAIT: Duration = Duration::from_millis(1500);

pub(crate) struct DevScreenshot {
    path: PathBuf,
    frames: u32,
    requested: bool,
    wait: Duration,
    started: Option<Instant>,
}

impl DevScreenshot {
    pub(crate) fn from_env() -> Option<Self> {
        let path = std::env::var_os("TIDEWATCH_SCREENSHOT")?;
        let wait = std::env::var("TIDEWATCH_SCREENSHOT_WAIT")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .map_or(DEFAULT_WAIT, Duration::from_secs_f32);
        Some(Self { path: path.into(), frames: 0, requested: false, wait, started: None })
    }

    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
        let captured = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = captured {
            let [w, h] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(egui::Color32::to_array).collect();
            if let Err(e) =
                image::save_buffer(&self.path, &rgba, w as u32, h as u32, image::ColorType::Rgba8)
            {
                eprintln!("tidewatch: screenshot failed: {e}");
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if !self.requested {
            self.frames += 1;
            let started = *self.started.get_or_insert_with(Instant::now);
            if self.frames > SETTLE_FRAMES && started.elapsed() >= self.wait {
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
                self.requested = true;
            }
        }
        ctx.request_repaint_after(POLL);
    }
}
