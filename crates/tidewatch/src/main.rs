//! Tidewatch - a window per git repo with stdusk terminal tabs on top and Kelp panels below.
//! The `eframe::App` loop lives in `app.rs`; tabs, layout math and the repo probe are pure
//! modules with unit tests.

mod app;
mod dev_screenshot;
mod layout;
mod panels;
mod repo;
mod tabs;

use std::path::PathBuf;

use app::Tidewatch;
use dev_screenshot::DevScreenshot;

const WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];

fn main() -> eframe::Result<()> {
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let Some(repo) = repo::Repo::discover(&start) else {
        eprintln!("tidewatch: {} is not inside a git repository", start.display());
        std::process::exit(2);
    };
    let screenshot = DevScreenshot::from_env();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Tidewatch")
            .with_inner_size(WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        "Tidewatch",
        options,
        Box::new(move |cc| Ok(Box::new(Tidewatch::new(cc, repo, screenshot)))),
    )
}
