//! Tidewatch - a window per git repo with stdusk terminal tabs on top and Kelp panels below.
//! The `eframe::App` loop lives in `app.rs`; tabs, layout math and the repo probe are pure
//! modules with unit tests. A folder that is not a git repo opens in the "Other" group.

mod agent_events;
mod agent_frame;
mod agent_jobs;
mod agent_run;
mod app;
mod catalog;
mod checkout;
mod cli;
mod close_dialog;
mod commands;
mod delivery_card;
mod delivery_view;
mod dev_agent;
mod dev_flow;
mod dev_panes;
mod dev_reader;
mod dev_screenshot;
mod dev_session;
mod dev_view;
mod dev_workflow;
mod dev_worktree;
mod diff_pane;
mod end_block_view;
mod flow;
mod flow_glue;
mod flow_jobs;
mod fonts;
mod history_pane;
mod hotkey;
mod jobs;
mod last_used;
mod layout;
mod leftover_glue;
mod leftover_jobs;
mod leftover_view;
mod leftovers;
mod list_row;
mod macos;
mod new_tab;
mod new_tab_dialog;
mod new_tab_workflow;
mod note_actions;
mod note_send;
mod notes_strip;
mod notice;
mod notify;
mod os;
mod overview;
mod overview_view;
mod palette;
mod palette_view;
mod pane;
mod pane_jobs;
mod post_confirm;
mod pr_state;
mod profile_editor;
mod pty;
mod quake;
mod reader;
mod reader_feed;
mod reader_glue;
mod reader_keys;
mod reader_rows;
mod reader_scroll;
mod reader_view;
mod repo;
mod restore_glue;
mod restore_jobs;
mod session_glue;
mod session_save;
mod session_writer;
mod settings;
mod settings_view;
mod shell;
mod shell_plan;
mod shortcuts_view;
mod single_instance;
mod status_view;
mod stepper;
mod stepper_view;
mod tab_view;
mod tabs;
#[cfg(all(test, unix))]
mod test_exec;
mod toast;
mod tray;
mod trust_glue;
mod trust_view;
mod updates;
mod updates_view;
mod window_settings;
mod window_view;
mod workflow_ask;
mod workflow_editor;
mod workflow_editor_view;
mod workflow_glue;
mod workflow_jobs;
mod workflow_run;
mod worktree_jobs;

use std::path::{Path, PathBuf};

use app::Tidewatch;
use dev_screenshot::DevScreenshot;
use repo::{Repo, Workspace};
use shell::{Setup, WINDOW_SIZE};
use shell_plan::{Env, Launch, Plan};
use single_instance::{Run, Start};
use tidewatch_core::last_workspace;
use tidewatch_core::path_env;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;

fn main() -> eframe::Result<()> {
    let args = cli::parse(std::env::args_os().skip(1));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let isolated = std::env::var_os("TIDEWATCH_HOME").is_some();
    let data_home = std::env::var_os("TIDEWATCH_HOME").map(PathBuf::from).or_else(|| home.clone());
    let paths = data_home.map(|home| TidewatchPaths::new(&home));
    if args.toggle {
        std::process::exit(single_instance::toggle_running(paths.as_ref()));
    }
    let start = last_workspace::pick(
        args.path,
        std::env::current_dir().ok(),
        paths.as_ref().and_then(last_workspace::load),
        |dir| Repo::discover(dir).is_some(),
        Path::is_dir,
    );
    let workspace = Workspace::open(&start);
    let agent_path = path_env::for_this_platform(
        std::env::var_os("PATH").as_deref(),
        home.as_deref().unwrap_or_else(|| Path::new(".")),
    );
    let screenshot = DevScreenshot::from_env();
    let background = screenshot.is_some() || std::env::var_os("TIDEWATCH_BACKGROUND").is_some();
    let env = Env::detect(background);
    let window_prefs = paths.as_ref().map(Prefs::load).unwrap_or_default().window;
    let plan = Plan::new(env, &window_prefs);

    let key = single_instance::workspace_key(workspace.dir());
    let socket = paths.as_ref().map(|paths| paths.instance_socket(&key));
    let run = match (screenshot.is_some(), background) {
        (true, _) => Run::Screenshot,
        (false, true) => Run::Background { isolated },
        (false, false) => Run::Foreground,
    };
    let mode = single_instance::mode(run, socket.is_some());
    let primary = match socket.as_deref().map(|socket| single_instance::begin(mode, socket, &key)) {
        Some(Start::Exit) => return Ok(()),
        Some(Start::Run(primary)) => primary,
        None => None,
    };
    let setup = Setup {
        env,
        prefs: window_prefs,
        label: workspace.chip_label(),
        own_socket: primary.as_ref().and(socket),
        instance_dir: paths.as_ref().map(TidewatchPaths::instance_dir),
        primary,
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Tidewatch")
        .with_inner_size(WINDOW_SIZE)
        .with_active(plan.activates);
    if plan.quake {
        viewport = viewport.with_decorations(false).with_resizable(false).with_position([0.0, 0.0]);
    }
    let mut options =
        eframe::NativeOptions { renderer: eframe::Renderer::Glow, viewport, ..Default::default() };
    match plan.launch {
        Launch::Normal => {}
        Launch::NoDockIcon => options.event_loop_builder = Some(Box::new(without_dock_icon)),
        Launch::NoDockIconNoActivation => {
            options.event_loop_builder = Some(Box::new(open_without_taking_focus));
        }
    }
    eframe::run_native(
        "Tidewatch",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Tidewatch::new(
                cc,
                workspace,
                paths,
                agent_path,
                screenshot,
                home.as_deref(),
                args.no_restore,
                setup,
            )))
        }),
    )
}

/// The quake window has no Dock icon: the menu-bar item and the hotkey are how it is reached.
fn without_dock_icon(builder: &mut winit::event_loop::EventLoopBuilder<eframe::UserEvent>) {
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = builder;
}

/// Dev and screenshot runs must not pull keyboard focus away from the terminal the developer is
/// typing in: no Dock icon and no activation on launch.
fn open_without_taking_focus(builder: &mut winit::event_loop::EventLoopBuilder<eframe::UserEvent>) {
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = builder;
}
