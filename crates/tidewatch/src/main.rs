//! Tidewatch - a window per git repo with stdusk terminal tabs on top and Kelp panels below.
//! The `eframe::App` loop lives in `app.rs`; tabs, layout math and the repo probe are pure
//! modules with unit tests. A folder that is not a git repo opens in the "Other" group.

mod agent_jobs;
mod app;
mod catalog;
mod checkout;
mod close_dialog;
mod dev_agent;
mod dev_panes;
mod dev_screenshot;
mod dev_worktree;
mod diff_pane;
mod fonts;
mod history_pane;
mod jobs;
mod layout;
mod new_tab;
mod new_tab_dialog;
mod note_actions;
mod pane;
mod pane_jobs;
mod repo;
mod tab_view;
mod tabs;
mod toast;
mod worktree_jobs;

use std::path::{Path, PathBuf};

use app::Tidewatch;
use dev_screenshot::DevScreenshot;
use repo::Workspace;
use tidewatch_core::path_env;
use tidewatch_core::paths::TidewatchPaths;

const WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];

fn main() -> eframe::Result<()> {
    let start = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let workspace = Workspace::open(&start);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("TIDEWATCH_HOME").map(PathBuf::from).or_else(|| home.clone());
    let paths = data_home.map(|home| TidewatchPaths::new(&home));
    let agent_path = path_env::for_this_platform(
        std::env::var_os("PATH").as_deref(),
        home.as_deref().unwrap_or_else(|| Path::new(".")),
    );
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
        Box::new(move |cc| {
            Ok(Box::new(Tidewatch::new(cc, workspace, paths, agent_path, screenshot)))
        }),
    )
}
