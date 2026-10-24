//! The `eframe::App`: tab strip on top, the active tab's terminal below it, then the History and
//! Diff panes. Frame work collects `tabs::Action`s and applies them after drawing.
//! Git work for worktree tabs and for the panes runs on `jobs` threads; its results are applied at
//! the top of the next frame.

mod agents;
mod amphetamine;
mod bar;
mod build_workflow;
mod central;
mod comments;
mod dev;
mod done_check;
mod draft;
mod flow;
mod frame;
mod image_paste;
mod keep_awake;
mod meeseeks;
mod open_link;
mod overlays;
mod palette;
mod panes;
mod parking;
mod pr;
mod progress;
mod remote;
mod reopen;
mod repos;
mod restart;
mod review;
mod settings;
mod stdusk;
mod strip;
mod tabs;
mod takeover;
mod terminal;
mod workspaces;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kelp_ui::avatars::AvatarStore;
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::colors;
use stdusk_egui::ui::apply_theme;
use tidewatch_core::guards::GuardPrefs;
use tidewatch_core::notes::NotesWriter;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::{NewTabPrefs, Prefs};
use tidewatch_core::restore::RealDisk;
use tidewatch_core::run_marker;
use tidewatch_core::tools::Tools;
use tidewatch_core::update::fake_release;
use tidewatch_core::worktree::{Inspection, Worktree};

use self::open_link::LinkDone;
use self::repos::Picker;
use crate::agent_events::{self, Hooks};
use crate::agent_jobs;
use crate::amphetamine_glue;
use crate::amphetamine_view;
use crate::avatar_glue;
use crate::brand;
use crate::build_workflow::Asked;
use crate::catalog::Catalog;
use crate::comment_glue;
use crate::dev_agent::{DevAgent, DevDraft};
use crate::dev_flow::DevFlow;
use crate::dev_link::DevLink;
use crate::dev_panes::DevPanes;
use crate::dev_reader::DevReader;
use crate::dev_screenshot::DevScreenshot;
use crate::dev_session::DevSession;
use crate::dev_takeover::DevTakeover;
use crate::dev_view::{DevKeys, DevPaste, DevScroll, DevView};
use crate::dev_workflow::DevWorkflow;
use crate::dev_worktree::DevWorktree;
use crate::done_check_glue;
use crate::due;
use crate::flow::Flow;
use crate::handoff;
use crate::image_paste as picture_paste;
use crate::job_glue;
use crate::jobs::Jobs;
use crate::keep_awake_glue;
use crate::keep_awake_hold::Hold;
use crate::keep_awake_hold::RealHold;
use crate::last_used::LastUsed;
use crate::layout;
use crate::leftover_glue;
use crate::meeseeks_glue;
use crate::new_tab::Request;
use crate::notify::Notifier;
use crate::open_link::OpenLink;
use crate::os::RealOs;
use crate::overview_checklist;
use crate::palette::Palette;
use crate::pane::Pane;
use crate::pane_jobs;
use crate::pr_glue;
use crate::progress_glue;
use crate::progress_view;
use crate::pty::spawn_shell;
use crate::remote_pill::Popover;
use crate::repo::Workspace;
use crate::repo_base::Bases;
use crate::repo_close::RepoClose;
use crate::restore_glue::{self};
use crate::session_glue::Saver;
use crate::settings::Settings;
use crate::shell::{self, Shell};
use crate::tab_view::TabView;
use crate::tabs::Tabs;
use crate::takeover::Takeover;
use crate::takeover_jobs;
use crate::terminal_font;
use crate::toast::Toast;
use crate::trust_glue::Trust;
use crate::updates::{Updates, fake_state};
use crate::welcome::{Recent, recent_repos};
use crate::window_chrome;
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
    terminal_font: terminal_font::Host,
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
    takeover: Takeover,
    takeover_jobs: Jobs<takeover_jobs::Done>,
    dev_takeover: Option<DevTakeover>,
    picker: Picker,
    open_link: Option<OpenLink>,
    link_jobs: Jobs<LinkDone>,
    dev_link: Option<DevLink>,
    repo_close: Option<RepoClose>,
    notes_writer: NotesWriter,
    avatars: AvatarStore,
    toast: Option<Toast>,
    catalog: Catalog,
    agent_jobs: Jobs<agent_jobs::Done>,
    settings: Option<Settings>,
    palette: Option<Palette>,
    last_profile: Option<String>,
    new_tab_prefs: NewTabPrefs,
    guards: GuardPrefs,
    welcome: Option<Vec<Recent>>,
    bases: Bases,
    hooks: Hooks,
    flow: Flow,
    pr: pr_glue::Host,
    comments: comment_glue::Host,
    progress: progress_glue::Host,
    job_live: job_glue::Host,
    avatar_live: avatar_glue::Host,
    meeseeks: meeseeks_glue::Host,
    done_check: done_check_glue::Host,
    keep_awake: keep_awake_glue::Host<RealHold>,
    amphetamine: amphetamine_glue::Host<Box<dyn Hold>>,
    amphetamine_view: amphetamine_view::Memory,
    progress_ui: progress_view::Memory,
    checklist: overview_checklist::Host,
    snoozes: due::Snoozes,
    dev_open_scheduled: Option<String>,
    told: due::Told,
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
    dev_remote_popover: Popover,
    dev_parked_popover: Popover,
    parking: parking::Parking,
    dev_keys: Option<DevKeys>,
    dev_paste: Option<DevPaste>,
    image_paste: picture_paste::Host,
    dev_scroll: Option<DevScroll>,
    dev_reader: DevReader,
    trust: Trust,
    saver: Saver,
    last_used: LastUsed,
    dev_session: DevSession,
    restore: restore_glue::Host,
    notice: Option<String>,
    updates: Updates,
    shell: Shell<RealOs>,
    desktop: window_chrome::Desktop,
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
        let prefs = paths.as_ref().map(Prefs::load).unwrap_or_default();
        let terminal_font = terminal_font::Host::start(&cc.egui_ctx, prefs.terminal.font);
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
        let loaded = paths.as_ref().map(Prefs::load).unwrap_or_default();
        let (new_tab_prefs, park_after) = (loaded.new_tab, loaded.tabs.park_after);
        let guards = loaded.guards;
        let dev_flow = DevFlow::from_env();
        let flow = Flow::new(Tools::with_path(dev_flow.tool_path(agent_path.clone())));
        let pr = pr_glue::Host::new(
            flow.tools(),
            paths.as_ref().map(Prefs::load).unwrap_or_default().github,
            pr_glue::Dev::from_env(),
        );
        let progress = progress_glue::Host::new(
            paths.as_ref().filter(|_| screenshot.is_none()),
            progress_glue::Dev::from_env(),
        );
        let job_live = job_glue::Host::from_env(paths.is_some() && screenshot.is_none());
        let avatar_live = avatar_glue::Host::from_env(paths.is_some() && screenshot.is_none());
        let meeseeks = meeseeks_glue::Host::from_env(paths.as_ref(), screenshot.is_none());
        let done_check = done_check_glue::Host::from_env(loaded.done_check.enabled);
        let keep_awake = keep_awake_glue::Host::new(
            RealHold::default(),
            loaded.keep_awake.enabled,
            !shell.env.background
                || keep_awake_glue::dev_asks(
                    std::env::var("TIDEWATCH_DEV_KEEP_AWAKE").ok().as_deref(),
                ),
        );
        let amphetamine = amphetamine::host(
            paths.as_ref(),
            screenshot.is_some(),
            shell.env.background,
            &cc.egui_ctx,
        );
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
            prefs.updates,
            fake_release(std::env::var("TIDEWATCH_FAKE_UPDATE").ok().as_deref()),
            screenshot.is_some(),
            paths.as_ref().map_or_else(std::env::temp_dir, TidewatchPaths::state_dir),
        )
        .with_fake_state(fake_state(std::env::var("TIDEWATCH_FAKE_UPDATE_STATE").ok().as_deref()));
        let mut app = Self {
            terminal_font,
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
            takeover: Takeover::with_recent_open(DevTakeover::recent_open_from_env()),
            takeover_jobs: Jobs::new(),
            dev_takeover: DevTakeover::from_env(),
            picker: Picker::new(),
            open_link: None,
            link_jobs: Jobs::new(),
            dev_link: DevLink::from_env(),
            repo_close: None,
            notes_writer: NotesWriter::new(),
            avatars: AvatarStore::offline(cc.egui_ctx.clone()),
            toast: None,
            catalog: Catalog::new(agent_path),
            agent_jobs: Jobs::new(),
            settings: None,
            palette: None,
            last_profile: None,
            new_tab_prefs: new_tab_prefs.clone(),
            guards,
            welcome: recent,
            bases: Bases::new(),
            hooks,
            flow,
            pr,
            comments: comment_glue::Host::new(),
            progress,
            job_live,
            avatar_live,
            meeseeks,
            done_check,
            keep_awake,
            amphetamine,
            amphetamine_view: amphetamine_view::Memory::from_env(),
            progress_ui: progress_view::Memory::default(),
            checklist: overview_checklist::Host::new(),
            snoozes: due::Snoozes::default(),
            dev_open_scheduled: std::env::var("TIDEWATCH_DEV_OPEN_SCHEDULED")
                .ok()
                .filter(|name| !name.is_empty()),
            told: due::Told::default(),
            runs: workflow_glue::Host::new(),
            run_asks: Vec::new(),
            build_asks: Vec::new(),
            dev_workflow: DevWorkflow::from_env(),
            dev_flow,
            notifier: Notifier::desktop(),
            overview: dev_view.overview,
            focus: dev_view.focus || focus,
            side_fraction: dev_view.side.unwrap_or(layout::SIDE_FRACTION),
            status_log: dev_view.status_log,
            dev_remote_popover: dev_view.remote_popover,
            dev_parked_popover: dev_view.parked_popover,
            parking: parking::Parking::new(park_after),
            dev_keys: dev_view.keys,
            dev_paste: dev_view.paste,
            image_paste: picture_paste::Host::new(&cc.egui_ctx),
            dev_scroll: dev_view.scroll,
            dev_reader: DevReader::from_env(),
            trust,
            saver,
            last_used,
            dev_session,
            restore: restore_glue::Host::new(no_restore, app_crashed, RealDisk::from_env()),
            notice: handoff::notice(&notes, kept_agents, hooks_kept),
            updates,
            desktop: shell.desktop.clone(),
            shell: shell::start(&cc.egui_ctx, shell),
        };
        for (group, workspace) in app.workspaces.iter().enumerate().filter(|_| !welcome) {
            if !restored {
                app.restore.enqueue(group, workspace);
            }
            app.bases.refresh(&cc.egui_ctx, workspace.dir(), app.paths.as_ref());
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
        self.drive_image_paste(&ctx, now, &intent);
        self.show_tab_strip(ui, now, &mut intent);
        self.show_status_bar(ui, intent.active_id);
        self.show_central_panel(ui, &ctx, now, &mut intent);
        let active_id = intent.active_id;
        self.apply_frame_intent(&ctx, intent);
        self.show_dialogs(&ctx, active_id);
        if self.shell.chrome() == window_chrome::Chrome::Drawn {
            window_chrome::edges(&ctx);
        }
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
        self.amphetamine.shutdown();
        if let Some(paths) = self.paths.as_ref().filter(|_| self.screenshot.is_none()) {
            run_marker::end(paths);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let bg = colors::bg();
        egui::Rgba::from(bg).to_array()
    }
}
