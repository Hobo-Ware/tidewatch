//! Tidewatch - a window per git repo with stdusk terminal tabs on top and Kelp panels below.
//! The `eframe::App` loop lives in `app.rs`; tabs, layout math and the repo probe are pure
//! modules with unit tests. A folder that is not a git repo opens in the "Other" group.

mod agenda;
mod agent_avatar;
mod agent_events;
mod agent_frame;
mod agent_jobs;
mod agent_process;
mod agent_ready;
mod agent_run;
mod agent_status_line;
mod amphetamine_glue;
mod amphetamine_icon;
mod amphetamine_motion;
mod amphetamine_pill;
mod amphetamine_view;
mod answer_chips;
mod app;
mod ask_model;
mod ask_view;
mod at_files;
mod at_files_view;
mod attachments;
mod autofetch_glue;
mod avatar_glue;
mod avatar_redraw;
mod brand;
mod bubble_toolbar;
mod budget_glue;
mod bullets;
mod catalog;
mod change_tab;
mod change_tab_view;
mod changes_view;
mod checkout;
mod cli;
mod close_dialog;
mod closed_jobs;
mod closed_state;
mod closed_view;
mod code_block;
mod coming_up;
mod commands;
mod comment_glue;
mod comment_model;
mod comment_strip;
mod commit_files;
mod commit_search;
mod composer;
mod delivery_card;
mod delivery_view;
mod dev_agent;
mod dev_flow;
mod dev_link;
mod dev_panes;
mod dev_reader;
mod dev_screenshot;
mod dev_see;
mod dev_session;
mod dev_takeover;
mod dev_view;
mod dev_worktree;
mod diff_pane;
mod done_check_glue;
mod done_check_trigger;
mod done_check_view;
mod due;
mod due_view;
mod emoji_motion;
mod emoji_pack;
mod emoji_paint;
mod emoji_pick;
mod emoji_text;
mod emoji_view;
mod fake_tab_names;
mod file_drop;
mod file_nav;
mod file_view;
mod files_glue;
mod fix_dialog;
mod flow;
mod flow_glue;
mod flow_jobs;
mod folder_panel;
mod folder_view;
mod font_view;
mod fonts;
mod from_stdusk;
mod general_view;
mod github_view;
mod guard;
mod guards_view;
mod handoff;
mod held_view;
mod history_pane;
mod hotkey;
mod house_style_scan;
mod house_style_view;
mod image_paste;
mod job_glue;
mod job_model;
mod job_view;
mod jobs;
mod keep_awake_glue;
mod keep_awake_hold;
mod last_turn;
mod last_turn_view;
mod last_used;
mod layout;
mod leftover_glue;
mod leftover_jobs;
mod leftover_view;
mod leftovers;
mod link_filter;
mod link_glue;
mod link_model;
mod link_text;
mod link_view;
mod links;
mod login_item;
mod macos;
mod mascot;
mod mascot_motion;
mod md_blocks;
mod md_view;
mod me_face;
mod me_glue;
mod meeseeks_glue;
mod meeseeks_model;
mod meeseeks_summary;
mod meeseeks_summary_view;
mod meeseeks_view;
mod merge_cleanup;
mod merge_groups;
mod middle_cut;
mod new_tab;
mod new_tab_done;
mod new_tab_fields;
mod new_tab_form;
mod new_tab_lane;
mod new_tab_layout;
mod new_tab_meeseeks;
mod new_tab_prefs_view;
mod new_tab_remote;
mod note_actions;
mod note_send;
mod notes_strip;
mod notice;
mod notify;
mod open_link;
mod open_link_view;
mod os;
mod outbox;
mod overview;
mod overview_checklist;
mod overview_fake;
mod overview_rail;
mod overview_view;
mod palette;
mod palette_view;
mod pane;
mod pane_jobs;
mod panel_blocks;
mod panel_fake;
mod panel_host;
mod park;
mod pictures;
mod post_confirm;
mod pr_card_model;
mod pr_cards_view;
mod pr_glue;
mod pr_jobs;
mod pr_state;
mod pr_strip;
mod pr_timeline;
mod pr_watch;
mod prefs_save;
mod profile_editor;
mod profiles_view;
mod progress_blocks;
mod progress_glue;
mod progress_model;
mod progress_ring;
mod progress_state;
mod progress_view;
mod prompt_ghost;
mod pty;
mod quake;
mod question_auto;
mod question_bulk;
mod question_card;
mod question_screen;
mod question_view;
mod quick_reply;
mod quiet_notes;
mod reader;
mod reader_chat_view;
mod reader_feed;
mod reader_glue;
mod reader_keys;
mod reader_markdown;
mod reader_rows;
mod reader_scroll;
mod reader_sent;
mod reader_turns;
mod reader_view;
mod recall;
mod remote_pill;
mod remote_session;
mod reopen;
mod reply_reason;
mod reply_store;
mod repo;
mod repo_base;
mod repo_close;
mod restore_glue;
mod restore_jobs;
mod retarget;
mod retarget_copy;
mod retarget_glue;
mod retarget_jobs;
mod retarget_repo;
mod review;
mod review_row_view;
mod review_view;
mod screen_kit;
mod search_bar;
mod segmented;
mod selectable;
mod send_line;
mod services_glue;
mod services_view;
mod session_glue;
mod session_save;
mod session_writer;
mod settings;
mod settings_icons;
mod settings_ui;
mod settings_view;
mod setup_editor;
mod shell;
mod shell_agents;
mod shell_plan;
mod shortcuts_view;
mod single_instance;
mod slash;
mod slash_glue;
mod slash_view;
mod status_bar;
mod status_view;
mod step_tick;
mod stopped_view;
mod sync_glue;
mod sync_view;
mod tab_title;
mod tab_view;
mod tabs;
mod takeover;
mod takeover_group;
mod takeover_jobs;
mod takeover_view;
mod terminal_font;
#[cfg(all(test, unix))]
mod test_exec;
mod tidewatch_view;
mod toast;
mod token_budget;
mod token_view;
mod tokens;
mod touch_id;
mod touch_id_view;
mod trash;
mod tray;
mod trust_glue;
mod trust_view;
mod update_chip;
mod updates;
mod updates_view;
mod waiting_notes;
mod welcome;
mod welcome_view;
mod window_chrome;
mod window_settings;
mod window_view;
mod workflow_glue;
mod workflow_view;
mod worktree_jobs;
mod worktree_setup_view;

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use app::Tidewatch;
use dev_screenshot::DevScreenshot;
use repo::{Repo, Workspace};
use shell::{Setup, WINDOW_SIZE};
use shell_plan::{Env, Launch, Plan};
use single_instance::{Run, Start};
use tidewatch_core::claude_memory;
use tidewatch_core::instance::Command;
use tidewatch_core::last_workspace;
use tidewatch_core::path_env;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;
use tidewatch_core::session::Session;
use window_chrome::Desktop;

/// The Wayland app id and X11 class: the installed `tidewatch.desktop`, whose icon the desktop shows
/// for the window. eframe would use the app name, `Tidewatch`, which matches no desktop entry.
const APP_ID: &str = "tidewatch";

/// The window icon where the window manager reads it from the window (X11). macOS takes the
/// bundle's icon, which a runtime icon would replace.
const LINUX_ICON: &[u8] = include_bytes!("../../../assets/icon-512.png");

fn main() -> eframe::Result<()> {
    if std::env::args_os().nth(1).is_some_and(|flag| flag == guard::FLAG) {
        std::process::exit(guard::run(&std::env::args_os().skip(2).collect::<Vec<_>>()));
    }
    let args = cli::parse(std::env::args_os().skip(1));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let isolated = std::env::var_os("TIDEWATCH_HOME").is_some();
    let data_home = std::env::var_os("TIDEWATCH_HOME").map(PathBuf::from).or_else(|| home.clone());
    let claude_dir = home
        .as_deref()
        .map(|home| claude_memory::config_dir(home, std::env::var_os("CLAUDE_CONFIG_DIR")));
    let paths = data_home.map(|data| {
        let paths = TidewatchPaths::new(&data);
        match claude_dir {
            Some(dir) => paths.with_claude_dir(dir),
            None => paths,
        }
    });
    if args.toggle {
        std::process::exit(single_instance::toggle_running(paths.as_ref()));
    }
    let prefs = paths.as_ref().map(Prefs::load).unwrap_or_default();
    let socket = paths.as_ref().map(TidewatchPaths::app_socket);
    let received = args.handoff.as_deref().map(handoff::Incoming::receive);
    let boot = handoff::boot(received, || {
        socket.as_deref().is_some_and(|socket| UnixStream::connect(socket).is_ok())
    });
    if matches!(boot, handoff::Boot::GiveWay) {
        eprintln!(
            "tidewatch: the handover failed and the old window is still running; leaving it be"
        );
        return Ok(());
    }
    let incoming = match boot {
        handoff::Boot::Adopt(incoming) => Some(incoming),
        handoff::Boot::Normal | handoff::Boot::GiveWay => None,
    };
    let has_path_arg = args.path.is_some();
    let opening = last_workspace::pick(
        args.path,
        std::env::current_dir().ok(),
        paths.as_ref().and_then(last_workspace::load),
        |dir| Repo::discover(dir).is_some(),
        Path::is_dir,
    );
    let welcome = incoming.as_ref().map_or_else(
        || last_workspace::is_blank_launch(has_path_arg, &opening),
        |incoming| incoming.session().welcome,
    );
    let restoring = !args.no_restore && prefs.restore_session;
    let workspaces = incoming.as_ref().map_or_else(
        || open_workspaces(&opening, restoring),
        |incoming| incoming.session().groups.iter().map(|g| Workspace::open(&g.dir)).collect(),
    );
    let workspace = &workspaces[0];
    let agent_path = path_env::for_this_platform(
        std::env::var_os("PATH").as_deref(),
        home.as_deref().unwrap_or_else(|| Path::new(".")),
    );
    let screenshot = DevScreenshot::from_env();
    let background = screenshot.is_some() || std::env::var_os("TIDEWATCH_BACKGROUND").is_some();
    let env = Env::detect(background);
    let window_prefs = prefs.window;
    let plan = Plan::new(env, &window_prefs);

    let key = single_instance::workspace_key(workspace.dir());
    let run = match (screenshot.is_some(), background) {
        (true, _) => Run::Screenshot,
        (false, true) => Run::Background { isolated },
        (false, false) => Run::Foreground,
    };
    let mode = single_instance::mode(run, socket.is_some());
    let replacing = incoming.is_some() && mode != single_instance::Mode::Off;
    let hand_over =
        args.from_stdusk.clone().map_or_else(|| Command::Open(key.clone()), Command::AdoptStdusk);
    let primary = match socket
        .as_deref()
        .filter(|_| !replacing)
        .map(|socket| single_instance::begin(mode, socket, &hand_over))
    {
        Some(Start::Exit) => return Ok(()),
        Some(Start::Run(primary)) => primary,
        None => None,
    };
    if incoming.is_none()
        && restoring
        && screenshot.is_none()
        && let Some(paths) = &paths
    {
        merge_saved_windows(paths);
    }
    let setup = Setup {
        env,
        prefs: window_prefs,
        label: workspace.chip_label(),
        own_socket: primary.as_ref().and(socket.clone()),
        instance_dir: paths.as_ref().map(TidewatchPaths::instance_dir),
        primary,
        replacing: socket.filter(|_| replacing),
        desktop: if env.own_frame { Desktop::read() } else { Desktop::default() },
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Tidewatch")
        .with_app_id(APP_ID)
        .with_inner_size(dev_view::window_size(WINDOW_SIZE))
        .with_min_inner_size(shell::min_inner_size(plan.quake))
        .with_active(plan.activates)
        .with_decorations(plan.chrome.decorations());
    if cfg!(target_os = "linux")
        && let Ok(icon) = eframe::icon_data::from_png_bytes(LINUX_ICON)
    {
        viewport = viewport.with_icon(icon);
    }
    if plan.quake {
        viewport = viewport.with_resizable(false).with_position([0.0, 0.0]);
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
                workspaces,
                paths,
                agent_path,
                screenshot,
                home.as_deref(),
                args.no_restore,
                welcome,
                setup,
                incoming,
                args.from_stdusk,
            )))
        }),
    )
}

/// One saved window per repo: a window whose folder is a linked worktree of another saved window's
/// repo is folded into it, so a session from before worktrees grouped comes back as one group.
fn merge_saved_windows(paths: &TidewatchPaths) {
    let mut session = Session::load(paths);
    let main_of = |dir: &Path| {
        let repo = dir.is_dir().then(|| Repo::discover_main(dir)).flatten()?;
        Some(repo.root)
    };
    if session.merge_linked(main_of) {
        session
            .save(paths)
            .unwrap_or_else(|e| eprintln!("tidewatch: cannot save the session: {e}"));
    }
}

/// The repo in front, then the others a launch brings back, each once however its folder is spelled.
fn open_workspaces(opening: &last_workspace::Opening, restoring: bool) -> Vec<Workspace> {
    let also = opening.also.iter().filter(|_| restoring);
    let mut workspaces = vec![Workspace::open_in_main_repo(&opening.dir)];
    for dir in also {
        let workspace = Workspace::open_in_main_repo(dir);
        let key = single_instance::workspace_key(workspace.dir());
        let known = workspaces.iter().any(|w| single_instance::workspace_key(w.dir()) == key);
        if !known {
            workspaces.push(workspace);
        }
    }
    workspaces
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_id_names_the_shipped_desktop_entry_and_its_window_class() {
        let packaging = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging");
        let entry = std::fs::read_to_string(packaging.join(format!("{APP_ID}.desktop"))).unwrap();
        assert!(entry.lines().any(|line| line == format!("StartupWMClass={APP_ID}")), "{entry}");
        assert!(entry.lines().any(|line| line == format!("Icon={APP_ID}")), "{entry}");
    }

    #[test]
    fn the_linux_window_icon_decodes() {
        let icon = eframe::icon_data::from_png_bytes(LINUX_ICON).unwrap();
        assert_eq!((icon.width, icon.height), (512, 512));
    }
}
