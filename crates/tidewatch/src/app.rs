//! The `eframe::App`: tab strip on top, the active tab's terminal below it, then the History and
//! Diff panes. Frame work collects `tabs::Action`s and applies them after drawing.
//! Git work for worktree tabs and for the panes runs on `jobs` threads; its results are applied at
//! the top of the next frame.

mod agents;
mod bar;
mod build_workflow;
mod central;
mod dev;
mod flow;
mod frame;
mod overlays;
mod palette;
mod panes;
mod repos;
mod restart;
mod review;
mod settings;
mod stdusk;
mod strip;
mod tabs;
mod terminal;
mod workspaces;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kelp_ui::avatars::AvatarStore;
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::colors;
use stdusk_egui::ui::apply_theme;
use tidewatch_core::notes::NotesWriter;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::Prefs;
use tidewatch_core::restore::RealDisk;
use tidewatch_core::run_marker;
use tidewatch_core::tools::Tools;
use tidewatch_core::update::fake_release;
use tidewatch_core::worktree::{Inspection, Worktree};

use self::repos::Picker;
use crate::agent_events::{self, Hooks};
use crate::agent_jobs;
use crate::brand;
use crate::build_workflow::Asked;
use crate::catalog::Catalog;
use crate::dev_agent::{DevAgent, DevDraft};
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
use crate::handoff;
use crate::jobs::Jobs;
use crate::last_used::LastUsed;
use crate::layout;
use crate::leftover_glue;
use crate::new_tab::{NewTab, Request};
use crate::notify::Notifier;
use crate::os::RealOs;
use crate::palette::Palette;
use crate::pane::Pane;
use crate::pane_jobs;
use crate::pty::spawn_shell;
use crate::repo::Workspace;
use crate::repo_base::Bases;
use crate::repo_close::RepoClose;
use crate::restore_glue::{self};
use crate::session_glue::Saver;
use crate::settings::Settings;
use crate::shell::{self, Shell};
use crate::tab_view::TabView;
use crate::tabs::Tabs;
use crate::toast::Toast;
use crate::trust_glue::Trust;
use crate::updates::Updates;
use crate::welcome::{Recent, recent_repos};
use crate::workflow_glue::{self};
use crate::workflow_run::Exec;
use crate::worktree_jobs::Done;

const FONT_SIZE: f32 = 13.0;
const MIN_CONTRAST: f32 = 4.0;
const ANIMATION_FRAME_MS: u64 = 40;
const WAKE_SLACK_S: f64 = 0.02;

enum Closing {
    Inspecting(u64),
    Asking { tab: u64, worktree: Worktree, inspection: Inspection },
}

pub(crate) struct Tidewatch {
    workspaces: Vec<Workspace>,
    paths: Option<TidewatchPaths>,
    tabs: Tabs<Pane<PtyTerm>>,
    jobs: Jobs<Done>,
    closing: Option<Closing>,
    leftovers: leftover_glue::Host,
    dev: Option<DevWorktree>,
    dev_agent: Option<DevAgent>,
    dev_draft: Option<DevDraft>,
    dev_panes: DevPanes,
    screenshot: Option<DevScreenshot>,
    views: HashMap<u64, TabView>,
    pane_jobs: Jobs<pane_jobs::Done>,
    workspace_jobs: Jobs<Workspace>,
    stdusk_jobs: Jobs<stdusk::Done>,
    picker: Picker,
    repo_close: Option<RepoClose>,
    notes_writer: NotesWriter,
    avatars: AvatarStore,
    toast: Option<Toast>,
    catalog: Catalog,
    agent_jobs: Jobs<agent_jobs::Done>,
    new_tab: Option<NewTab>,
    settings: Option<Settings>,
    palette: Option<Palette>,
    last_profile: Option<String>,
    welcome: Option<Vec<Recent>>,
    bases: Bases,
    hooks: Hooks,
    flow: Flow,
    runs: workflow_glue::Host,
    run_asks: Vec<(u64, Vec<Exec>)>,
    build_asks: Vec<Asked>,
    dev_workflow: DevWorkflow,
    dev_flow: DevFlow,
    notifier: Notifier,
    overview: bool,
    focus: bool,
    side_fraction: f32,
    status_log: bool,
    dev_keys: Option<DevKeys>,
    dev_reader: DevReader,
    trust: Trust,
    saver: Saver,
    last_used: LastUsed,
    dev_session: DevSession,
    restore: restore_glue::Host,
    notice: Option<String>,
    updates: Updates,
    shell: Shell<RealOs>,
}

impl Tidewatch {
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        workspaces: Vec<Workspace>,
        paths: Option<TidewatchPaths>,
        agent_path: OsString,
        screenshot: Option<DevScreenshot>,
        cli_home: Option<&Path>,
        no_restore: bool,
        welcome: bool,
        mut shell: shell::Setup,
        incoming: Option<handoff::Incoming>,
        from_stdusk: Option<PathBuf>,
    ) -> Self {
        colors::init(brand::theme());
        colors::set_chrome(brand::chrome());
        fonts::install(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);
        brand::tint_visuals(&cc.egui_ctx);
        if screenshot.is_some() {
            cc.egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
        }
        let adopted = incoming.map(|incoming| take_over(&cc.egui_ctx, incoming, &mut shell));
        let hooks_port = adopted.as_ref().and_then(|adopted| adopted.hook_port);
        let (tabs, credentials, notes, focus) = match adopted {
            Some(adopted) => (adopted.tabs, adopted.credentials, adopted.notes, adopted.focus),
            None => (fresh_tabs(&cc.egui_ctx, &workspaces), Vec::new(), Vec::new(), false),
        };
        let recent = welcome.then(|| recent_repos(paths.as_ref()));
        let (hooks, hooks_kept) = match hooks_port {
            Some(port) => Hooks::start_on_port(
                paths.as_ref(),
                &cc.egui_ctx,
                port,
                agent_events::PORT_PATIENCE,
            ),
            None => (Hooks::start(paths.as_ref(), &cc.egui_ctx), false),
        };
        let kept_agents = credentials.len();
        for (tab, credential) in credentials.iter().filter(|_| hooks_kept) {
            hooks.adopt(*tab, credential);
        }
        let restored = hooks_port.is_some();
        let dev_flow = DevFlow::from_env();
        let flow = Flow::new(Tools::with_path(dev_flow.tool_path(agent_path.clone())));
        let dev_view = DevView::from_env();
        let trust = Trust::new(paths.as_ref(), cli_home);
        let dev_session = DevSession::from_env();
        let saver = Saver::new(paths.as_ref(), screenshot.is_none(), dev_session.debounce());
        let app_crashed =
            paths.as_ref().filter(|_| screenshot.is_none()).is_some_and(run_marker::begin);
        let last_used =
            LastUsed::new(paths.as_ref(), screenshot.is_none() && !shell.env.background);
        let updates = Updates::new(
            &cc.egui_ctx,
            paths.as_ref().map(Prefs::load).unwrap_or_default().updates,
            fake_release(std::env::var("TIDEWATCH_FAKE_UPDATE").ok().as_deref()),
            screenshot.is_some(),
        );
        let mut app = Self {
            tabs,
            jobs: Jobs::new(),
            closing: None,
            leftovers: leftover_glue::Host::new(),
            dev: DevWorktree::from_env(),
            dev_agent: DevAgent::from_env(),
            dev_draft: None,
            dev_panes: DevPanes::from_env(),
            workspaces,
            paths,
            screenshot,
            views: HashMap::new(),
            pane_jobs: Jobs::new(),
            workspace_jobs: Jobs::new(),
            stdusk_jobs: Jobs::new(),
            picker: Picker::new(),
            repo_close: None,
            notes_writer: NotesWriter::new(),
            avatars: AvatarStore::offline(cc.egui_ctx.clone()),
            toast: None,
            catalog: Catalog::new(agent_path),
            agent_jobs: Jobs::new(),
            new_tab: None,
            settings: None,
            palette: None,
            last_profile: None,
            welcome: recent,
            bases: Bases::new(),
            hooks,
            flow,
            runs: workflow_glue::Host::new(),
            run_asks: Vec::new(),
            build_asks: Vec::new(),
            dev_workflow: DevWorkflow::from_env(),
            dev_flow,
            notifier: Notifier::desktop(),
            overview: dev_view.overview,
            focus: dev_view.focus || focus,
            side_fraction: layout::SIDE_FRACTION,
            status_log: dev_view.status_log,
            dev_keys: dev_view.keys,
            dev_reader: DevReader::from_env(),
            trust,
            saver,
            last_used,
            dev_session,
            restore: restore_glue::Host::new(no_restore, app_crashed, RealDisk::from_env()),
            notice: handoff::notice(&notes, kept_agents, hooks_kept),
            updates,
            shell: shell::start(&cc.egui_ctx, shell),
        };
        for (group, workspace) in app.workspaces.iter().enumerate().filter(|_| !welcome) {
            if !restored {
                app.restore.enqueue(group, workspace);
            }
            app.bases.refresh(&cc.egui_ctx, workspace.dir());
        }
        if restored {
            app.restore.mark_settled();
        }
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
        if let Some(socket) = from_stdusk {
            app.adopt_from_stdusk(socket, &cc.egui_ctx);
        }
        app
    }
}

struct Adopted {
    tabs: Tabs<Pane<PtyTerm>>,
    credentials: Vec<(u64, tidewatch_core::listener::Credential)>,
    notes: Vec<String>,
    focus: bool,
    hook_port: Option<u16>,
}

fn fresh_tabs(ctx: &egui::Context, workspaces: &[Workspace]) -> Tabs<Pane<PtyTerm>> {
    let mut shells =
        workspaces.iter().map(|workspace| Pane::shell(spawn_shell(ctx, workspace.dir())));
    let mut tabs = Tabs::new(shells.next().expect("a window opens at least one workspace"));
    for main in shells {
        tabs.add_group(main);
    }
    tabs
}

/// Builds the old window's tabs around the terminals it sent and confirms, so it lets go; then
/// claims the instance socket the old window leaves. A layout that does not line up is refused
/// before anything is confirmed, so the old window keeps everything and this process only ends.
fn take_over(
    ctx: &egui::Context,
    mut incoming: handoff::Incoming,
    shell: &mut shell::Setup,
) -> Adopted {
    let rebuilt = match handoff::adopt(ctx, &mut incoming, 0.0) {
        Ok(rebuilt) => rebuilt,
        Err(reason) => {
            eprintln!("tidewatch: the handed-over window is not usable ({reason}); leaving it be");
            std::process::exit(1);
        }
    };
    let handoff::Rebuilt { mut tabs, credentials, notes } = rebuilt;
    if let Err(e) = handoff::confirm(&incoming, &mut tabs) {
        eprintln!("tidewatch: could not confirm the handover ({e})");
    }
    shell.claim_from_old_window();
    let handover = incoming.session();
    Adopted { tabs, credentials, notes, focus: handover.focus, hook_port: handover.hook_port }
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
        self.show_status_bar(ui, intent.active_id);
        self.show_central_panel(ui, &ctx, now, &mut intent);
        let active_id = intent.active_id;
        self.apply_frame_intent(&ctx, intent);
        self.show_dialogs(&ctx, active_id);
        self.finish_frame(&ctx, now);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shell.set_closing(ctx.input(|i| i.viewport().close_requested()));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shell.set_closing(true);
        if self.welcome.is_none() {
            self.last_used.record(&workspaces::remembered(&self.workspaces, &self.tabs));
        }
        self.saver.flush(
            self.restore.settled(),
            workspaces::window_views(&self.workspaces, self.focus),
            &self.tabs,
        );
        if let Some(paths) = self.paths.as_ref().filter(|_| self.screenshot.is_none()) {
            run_marker::end(paths);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let bg = colors::bg();
        egui::Rgba::from(bg).to_array()
    }
}
