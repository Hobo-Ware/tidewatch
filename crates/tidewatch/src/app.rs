//! The `eframe::App`: tab strip on top, the active tab's terminal below it, then the History and
//! Diff panes. Frame work collects `tabs::Action`s and applies them after drawing.
//! Git work for worktree tabs and for the panes runs on `jobs` threads; its results are applied at
//! the top of the next frame.

mod agents;
mod central;
mod dev;
mod flow;
mod frame;
mod overlays;
mod panes;
mod settings;
mod strip;
mod tabs;
mod terminal;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use kelp_ui::avatars::AvatarStore;
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::colors;
use stdusk_egui::ui::apply_theme;
use tidewatch_core::notes::NotesWriter;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;
use tidewatch_core::restore::RealDisk;
use tidewatch_core::tools::Tools;
use tidewatch_core::update::fake_release;
use tidewatch_core::worktree::{Inspection, Worktree};

use crate::agent_events::Hooks;
use crate::agent_jobs;
use crate::catalog::Catalog;
use crate::dev_agent::DevAgent;
use crate::dev_flow::DevFlow;
use crate::dev_panes::DevPanes;
use crate::dev_reader::DevReader;
use crate::dev_screenshot::DevScreenshot;
use crate::dev_session::DevSession;
use crate::dev_view::{DevKeys, DevView};
use crate::dev_workflow::DevWorkflow;
use crate::dev_worktree::DevWorktree;
use crate::flow::Flow;
use crate::fonts;
use crate::jobs::Jobs;
use crate::leftover_glue;
use crate::new_tab::{NewTab, Request};
use crate::notify::Notifier;
use crate::os::RealOs;
use crate::pane::Pane;
use crate::pane_jobs;
use crate::pty::spawn_shell;
use crate::repo::Workspace;
use crate::restore_glue::{self};
use crate::session_glue::Saver;
use crate::session_save::WindowView;
use crate::settings::Settings;
use crate::shell::{self, Shell};
use crate::tab_view::TabView;
use crate::tabs::Tabs;
use crate::toast::Toast;
use crate::trust_glue::Trust;
use crate::updates::Updates;
use crate::workflow_glue::{self};
use crate::workflow_run::Exec;
use crate::worktree_jobs::Done;

const DEFAULT_THEME: &str = "one-half-dark";
const FONT_SIZE: f32 = 13.0;
const MIN_CONTRAST: f32 = 4.0;
const ANIMATION_FRAME_MS: u64 = 40;
const WAKE_SLACK_S: f64 = 0.02;

enum Closing {
    Inspecting(u64),
    Asking { tab: u64, worktree: Worktree, inspection: Inspection },
}

pub(crate) struct Tidewatch {
    workspace: Workspace,
    paths: Option<TidewatchPaths>,
    tabs: Tabs<Pane<PtyTerm>>,
    jobs: Jobs<Done>,
    closing: Option<Closing>,
    leftovers: leftover_glue::Host,
    dev: Option<DevWorktree>,
    dev_agent: Option<DevAgent>,
    dev_panes: DevPanes,
    screenshot: Option<DevScreenshot>,
    views: HashMap<u64, TabView>,
    pane_jobs: Jobs<pane_jobs::Done>,
    notes_writer: NotesWriter,
    avatars: AvatarStore,
    toast: Option<Toast>,
    catalog: Catalog,
    agent_jobs: Jobs<agent_jobs::Done>,
    new_tab: Option<NewTab>,
    settings: Option<Settings>,
    last_profile: Option<String>,
    hooks: Hooks,
    flow: Flow,
    runs: workflow_glue::Host,
    run_asks: Vec<(u64, Vec<Exec>)>,
    dev_workflow: DevWorkflow,
    dev_flow: DevFlow,
    notifier: Notifier,
    overview: bool,
    focus: bool,
    status_log: bool,
    dev_keys: Option<DevKeys>,
    dev_reader: DevReader,
    trust: Trust,
    saver: Saver,
    dev_session: DevSession,
    restore: restore_glue::Host,
    notice: Option<String>,
    updates: Updates,
    shell: Shell<RealOs>,
}

impl Tidewatch {
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        workspace: Workspace,
        paths: Option<TidewatchPaths>,
        agent_path: OsString,
        screenshot: Option<DevScreenshot>,
        cli_home: Option<&Path>,
        no_restore: bool,
        shell: shell::Setup,
    ) -> Self {
        colors::init(colors::by_name(DEFAULT_THEME));
        fonts::install(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);
        if screenshot.is_some() {
            cc.egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
        }
        let main = spawn_shell(&cc.egui_ctx, workspace.dir());
        let hooks = Hooks::start(paths.as_ref(), &cc.egui_ctx);
        let dev_flow = DevFlow::from_env();
        let flow = Flow::new(Tools::with_path(dev_flow.tool_path(agent_path.clone())));
        let dev_view = DevView::from_env();
        let trust = Trust::new(paths.as_ref(), cli_home);
        let dev_session = DevSession::from_env();
        let saver = Saver::new(paths.as_ref(), screenshot.is_none(), dev_session.debounce());
        let updates = Updates::new(
            &cc.egui_ctx,
            paths.as_ref().map(Prefs::load).unwrap_or_default().updates,
            fake_release(std::env::var("TIDEWATCH_FAKE_UPDATE").ok().as_deref()),
            screenshot.is_some(),
        );
        let mut app = Self {
            tabs: Tabs::new(Pane::shell(main)),
            jobs: Jobs::new(),
            closing: None,
            leftovers: leftover_glue::Host::new(),
            dev: DevWorktree::from_env(),
            dev_agent: DevAgent::from_env(),
            dev_panes: DevPanes::from_env(),
            workspace,
            paths,
            screenshot,
            views: HashMap::new(),
            pane_jobs: Jobs::new(),
            notes_writer: NotesWriter::new(),
            avatars: AvatarStore::offline(cc.egui_ctx.clone()),
            toast: None,
            catalog: Catalog::new(agent_path),
            agent_jobs: Jobs::new(),
            new_tab: None,
            settings: None,
            last_profile: None,
            hooks,
            flow,
            runs: workflow_glue::Host::new(),
            run_asks: Vec::new(),
            dev_workflow: DevWorkflow::from_env(),
            dev_flow,
            notifier: Notifier::desktop(),
            overview: dev_view.overview,
            focus: dev_view.focus,
            status_log: dev_view.status_log,
            dev_keys: dev_view.keys,
            dev_reader: DevReader::from_env(),
            trust,
            saver,
            dev_session,
            restore: restore_glue::Host::new(no_restore, RealDisk::from_env()),
            notice: None,
            updates,
            shell: shell::start(&cc.egui_ctx, shell),
        };
        if let Some(paths) = &app.paths {
            agent_jobs::load(
                &app.agent_jobs,
                &cc.egui_ctx,
                paths.clone(),
                app.catalog.path.clone(),
            );
        }
        if app.dev.is_some() {
            app.open_tab(&cc.egui_ctx, Request::Shell);
        }
        app
    }
}

fn wall_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

impl eframe::App for Tidewatch {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_events_and_jobs(&ctx);
        let now = ctx.input(|i| i.time);
        self.drive_background_work(&ctx, now);
        let mut intent = self.read_shortcuts(&ctx);
        self.show_tab_strip(ui, now, &mut intent);
        self.show_central_panel(ui, &ctx, now, &mut intent);
        let active_id = intent.active_id;
        self.apply_frame_intent(&ctx, intent);
        self.show_dialogs(&ctx, active_id);
        self.finish_frame(&ctx, now);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.saver.flush(
            self.restore.settled(),
            &WindowView::of(&self.workspace, self.focus),
            &self.tabs,
        );
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let bg = colors::bg();
        egui::Rgba::from(bg).to_array()
    }
}
