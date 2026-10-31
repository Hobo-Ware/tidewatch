//! The `eframe::App`: tab strip on top, the active tab's terminal below it, then the History and
//! Diff panes. Frame work collects `tabs::Action`s and applies them after drawing.
//! Git work for worktree tabs and for the panes runs on `jobs` threads; its results are applied at
//! the top of the next frame.

mod agents;
mod amphetamine;
mod autofetch;
mod bar;
mod budget;
mod central;
mod closed;
mod comments;
mod dev;
mod done_check;
mod draft;
mod flow;
mod frame;
mod github_events;
mod image_paste;
mod keep_awake;
mod meeseeks;
mod merge_groups;
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
mod retarget;
mod review;
mod services;
mod settings;
mod slow;
mod stdusk;
pub(crate) mod strip;
mod tabs;
mod takeover;
mod terminal;
mod workflow;
mod workspaces;

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use kelp_ui::avatars::AvatarStore;
use stdusk_core::config::TerminalConfig;
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::colors;
use stdusk_egui::ui::{BellFlash, apply_theme};
use tidewatch_core::guards::GuardPrefs;
use tidewatch_core::notes::NotesWriter;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::{ChatPrefs, CloseEnter, NewTabPrefs, Prefs, TerminalPrefs};
use tidewatch_core::restore::RealDisk;
use tidewatch_core::run_marker;
use tidewatch_core::tools::Tools;
use tidewatch_core::update::fake_release;

use self::open_link::LinkDone;
use self::repos::Picker;
use crate::agent_events::{self, Hooks};
use crate::agent_jobs;
use crate::amphetamine_glue;
use crate::amphetamine_view;
use crate::autofetch_glue;
use crate::avatar_glue;
use crate::brand;
use crate::budget_glue;
use crate::catalog::Catalog;
use crate::close_dialog::Closing;
use crate::comment_glue;
use crate::dev_agent::{DevAgent, DevDraft};
use crate::dev_flow::DevFlow;
use crate::dev_link::DevLink;
use crate::dev_panes::DevPanes;
use crate::dev_reader::DevReader;
use crate::dev_screenshot::DevScreenshot;
use crate::dev_session::DevSession;
use crate::dev_takeover::DevTakeover;
use crate::dev_view::{DevCopy, DevKeys, DevPaste, DevScroll, DevView};
use crate::dev_worktree::DevWorktree;
use crate::done_check_glue;
use crate::due;
use crate::emoji_motion;
use crate::flow::Flow;
use crate::folder_panel::Folders;
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
use crate::link_glue;
use crate::me_glue;
use crate::meeseeks_glue;
use crate::new_tab::Request;
use crate::notify::Notifier;
use crate::open_link::OpenLink;
use crate::os::RealOs;
use crate::overview_checklist;
use crate::palette::Palette;
use crate::pane::Pane;
use crate::pane_jobs;
use crate::panel_host;
use crate::pr_glue;
use crate::progress_glue;
use crate::progress_view;
use crate::pty::spawn_shell;
use crate::remote_pill::Popover;
use crate::repo::Workspace;
use crate::repo_base::Bases;
use crate::repo_close::RepoClose;
use crate::restore_glue::{self};
use crate::services_glue;
use crate::services_view;
use crate::session_glue::Saver;
use crate::settings::Settings;
use crate::shell::{self, Shell};
use crate::slash_glue;
use crate::sync_glue;
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
use crate::workflow_glue;
use crate::worktree_jobs::Done;
use crate::{closed_jobs, closed_state};

const FONT_SIZE: f32 = 13.0;
const ANIMATION_FRAME_MS: u64 = 40;
const WAKE_SLACK_S: f64 = 0.02;

pub(crate) struct Tidewatch {
    workspaces: Vec<Workspace>,
    terminal_font: terminal_font::Host,
    terminal_prefs: TerminalPrefs,
    chat_prefs: ChatPrefs,
    reduce_motion: emoji_motion::ReduceMotion,
    terminal_config: TerminalConfig,
    pending_pastes: VecDeque<(u64, String)>,
    bell: BellFlash,
    paths: Option<TidewatchPaths>,
    tabs: Tabs<Pane<PtyTerm>>,
    jobs: Jobs<Done>,
    closing: Option<Closing>,
    leftovers: leftover_glue::Host,
    dev: Option<DevWorktree>,
    dev_agent: Option<DevAgent>,
    dev_draft: Option<DevDraft>,
    dev_form_start: Option<(u64, Request)>,
    dev_panes: DevPanes,
    screenshot: Option<DevScreenshot>,
    views: HashMap<u64, TabView>,
    pane_jobs: Jobs<pane_jobs::Done>,
    folders: Folders,
    workspace_jobs: Jobs<Workspace>,
    group_merge: merge_groups::Host,
    stdusk_jobs: Jobs<stdusk::Done>,
    takeover: Takeover,
    takeover_jobs: Jobs<takeover_jobs::Done>,
    retarget: crate::retarget_glue::Host,
    closed: closed_state::State,
    closed_jobs: Jobs<closed_jobs::Done>,
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
    github_events: tidewatch_core::github_events::Host,
    comments: comment_glue::Host,
    progress: progress_glue::Host,
    job_live: job_glue::Host,
    panel: panel_host::Host,
    link_live: link_glue::Host,
    me: me_glue::Host,
    avatar_live: avatar_glue::Host,
    avatar_redraws: crate::avatar_redraw::Waiting,
    meeseeks: meeseeks_glue::Host,
    done_check: done_check_glue::Host,
    keep_awake: keep_awake_glue::Host<RealHold>,
    budget: budget_glue::Host,
    autofetch: autofetch_glue::Host,
    amphetamine: amphetamine_glue::Host<Box<dyn Hold>>,
    amphetamine_view: amphetamine_view::Memory,
    progress_ui: progress_view::Memory,
    checklist: overview_checklist::Host,
    overview_state: crate::overview_view::State,
    ask_given: HashMap<u64, crate::ask_model::Answered>,
    slash: slash_glue::Host,
    snoozes: due::Snoozes,
    dev_open_scheduled: Option<String>,
    told: due::Told,
    started: due::Told,
    started_loaded: HashSet<u64>,
    dev_flow: DevFlow,
    notifier: Notifier,
    overview: bool,
    focus: bool,
    side_fraction: f32,
    status_log: bool,
    dev_remote_popover: Popover,
    dev_parked_popover: Popover,
    parking: parking::Parking,
    close_enter: CloseEnter,
    dev_keys: Option<DevKeys>,
    dev_paste: Option<DevPaste>,
    image_paste: picture_paste::Host,
    dev_scroll: Option<DevScroll>,
    dev_hover: Option<egui::Pos2>,
    dev_hover_files: Option<std::path::PathBuf>,
    dev_copy: Option<DevCopy>,
    dev_reader: DevReader,
    trust: Trust,
    saver: Saver,
    last_used: LastUsed,
    dev_session: DevSession,
    restore: restore_glue::Host,
    notice: Option<String>,
    updates: Updates,
    settings_sync: sync_glue::Host,
    shell: Shell<RealOs>,
    desktop: window_chrome::Desktop,
    services: services_glue::Host,
    services_ui: services_view::Memory,
    workflow: workflow_glue::Host,
    house_style_scan: crate::house_style_scan::Host,
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
        slow::start_dev();
        colors::init(brand::theme());
        colors::set_chrome(brand::chrome());
        let prefs = paths.as_ref().map(Prefs::load).unwrap_or_default();
        let terminal_prefs = prefs.terminal.clone();
        cc.egui_ctx.options_mut(|options| options.zoom_with_keyboard = false);
        cc.egui_ctx.set_zoom_factor(tidewatch_core::ui_zoom::render_factor(prefs.window.ui_zoom));
        let terminal_font = terminal_font::Host::start(&cc.egui_ctx, prefs.terminal.font);
        apply_theme(&cc.egui_ctx);
        brand::tint_visuals(&cc.egui_ctx);
        if screenshot.is_some() {
            cc.egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
        }
        let adopted = incoming.map(|incoming| take_over(&cc.egui_ctx, incoming, &mut shell));
        let hooks_port = adopted.as_ref().and_then(|adopted| adopted.hook_port);
        let (tabs, credentials, notes, focus, kept_services) = match adopted {
            Some(adopted) => {
                (adopted.tabs, adopted.credentials, adopted.notes, adopted.focus, adopted.services)
            }
            None => {
                (fresh_tabs(&cc.egui_ctx, &workspaces), Vec::new(), Vec::new(), false, Vec::new())
            }
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
        let notice =
            [handoff::notice(&notes, kept_agents, hooks_kept), hooks.problem().map(str::to_owned)]
                .into_iter()
                .flatten()
                .reduce(|first, second| format!("{first}\n{second}"));
        let restored = hooks_port.is_some();
        let loaded = paths.as_ref().map(Prefs::load).unwrap_or_default();
        let (new_tab_prefs, park_after, close_enter) =
            (loaded.new_tab, loaded.tabs.park_after, loaded.tabs.close_enter);
        let guards = loaded.guards;
        let dev_flow = DevFlow::from_env();
        let flow = Flow::new(Tools::with_path(dev_flow.tool_path(agent_path.clone())));
        let pr = pr_glue::Host::new(
            flow.tools(),
            paths.as_ref().map(Prefs::load).unwrap_or_default().github,
            pr_glue::Dev::from_env(),
        );
        let github_events = github_events_host(
            flow.tools(),
            paths.clone(),
            &cc.egui_ctx,
            screenshot.is_some() || shell.env.background,
        );
        let progress = progress_glue::Host::new(
            paths.as_ref().filter(|_| screenshot.is_none()),
            progress_glue::Dev::from_env(),
        );
        let job_live = job_glue::Host::from_env(paths.is_some() && screenshot.is_none());
        let link_live = link_glue::Host::from_env(paths.is_some() && screenshot.is_none());
        let avatar_live = avatar_glue::Host::from_env(paths.is_some() && screenshot.is_none());
        let me = me_glue::Host::start(
            &cc.egui_ctx,
            paths.as_ref(),
            flow.tools(),
            screenshot.is_some() || shell.env.background,
        );
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
        let budget = budget_glue::Host::new(loaded.budget.save_mode, wall_ms() as i64);
        let autofetch = autofetch_glue::Host::new(
            loaded.git,
            screenshot.is_none()
                && (!shell.env.background
                    || keep_awake_glue::dev_asks(
                        std::env::var("TIDEWATCH_DEV_AUTOFETCH").ok().as_deref(),
                    )),
        );
        let amphetamine = amphetamine::host(
            paths.as_ref(),
            screenshot.is_some(),
            shell.env.background,
            &cc.egui_ctx,
        );
        let dev_view = DevView::from_env();
        let panel = panel_host::Host::from_env(
            paths.is_some() && screenshot.is_none(),
            dev_view.status_log,
            paths.as_ref().map(TidewatchPaths::panel_ontime_file),
        );
        let trust = Trust::new(paths.as_ref(), cli_home);
        let dev_session = DevSession::from_env();
        let saver = Saver::new(paths.as_ref(), screenshot.is_none(), dev_session.debounce());
        let crash_loop =
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
        let settings_sync =
            sync_glue::Host::new(paths.as_ref(), flow.tools(), screenshot.is_none(), &cc.egui_ctx);
        let mut services = services_glue::Host::new(
            paths.clone(),
            services_spawner(&cc.egui_ctx),
            services_autostart(screenshot.is_some() || shell.env.background),
            std::env::var(services_glue::FAKE_VAR).is_ok_and(|value| value == "1"),
        );
        services.adopt(kept_services);
        let workflow = workflow_glue::Host::new(
            paths.clone(),
            workflow_glue::Host::mode_from_env(screenshot.is_some() || shell.env.background),
        );
        let mut app = Self {
            terminal_font,
            terminal_config: terminal::terminal_config(terminal_prefs.copy_on_select),
            terminal_prefs,
            chat_prefs: loaded.chat,
            reduce_motion: emoji_motion::ReduceMotion::default(),
            pending_pastes: VecDeque::new(),
            bell: BellFlash::default(),
            tabs,
            jobs: Jobs::new(),
            closing: None,
            leftovers: leftover_glue::Host::new(paths.clone()),
            dev: DevWorktree::from_env(),
            dev_agent: DevAgent::from_env(),
            dev_draft: None,
            dev_form_start: None,
            dev_panes: DevPanes::from_env(),
            workspaces,
            paths,
            screenshot,
            views: HashMap::new(),
            pane_jobs: Jobs::new(),
            folders: Folders::new(),
            workspace_jobs: Jobs::new(),
            group_merge: merge_groups::Host::new(),
            stdusk_jobs: Jobs::new(),
            takeover: Takeover::with_recent_open(DevTakeover::recent_open_from_env())
                .with_line_open(DevTakeover::line_open_from_env()),
            takeover_jobs: Jobs::new(),
            retarget: crate::retarget_glue::Host::from_env(),
            closed: closed_state::State::new(closed_state::fake_requested()),
            closed_jobs: Jobs::new(),
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
            panel,
            link_live,
            me,
            avatar_live,
            avatar_redraws: crate::avatar_redraw::Waiting::default(),
            meeseeks,
            done_check,
            keep_awake,
            budget,
            autofetch,
            amphetamine,
            amphetamine_view: amphetamine_view::Memory::from_env(),
            progress_ui: progress_view::Memory::default(),
            checklist: overview_checklist::Host::new(),
            overview_state: crate::overview_fake::initial_state(),
            ask_given: HashMap::new(),
            slash: slash_glue::Host::new(),
            snoozes: due::Snoozes::default(),
            dev_open_scheduled: std::env::var("TIDEWATCH_DEV_OPEN_SCHEDULED")
                .ok()
                .filter(|name| !name.is_empty()),
            told: due::Told::default(),
            started: due::Told::default(),
            started_loaded: HashSet::new(),
            dev_flow,
            notifier: Notifier::desktop(),
            overview: dev_view.overview,
            focus: dev_view.focus || focus,
            side_fraction: dev_view.side.unwrap_or(layout::SIDE_FRACTION),
            status_log: dev_view.status_log,
            dev_remote_popover: dev_view.remote_popover,
            dev_parked_popover: dev_view.parked_popover,
            parking: parking::Parking::new(park_after),
            close_enter,
            dev_keys: dev_view.keys,
            dev_paste: dev_view.paste,
            image_paste: picture_paste::Host::new(&cc.egui_ctx),
            dev_scroll: dev_view.scroll,
            dev_hover: dev_view.hover,
            dev_hover_files: dev_view.hover_files.clone(),
            dev_copy: dev_view.copy,
            dev_reader: DevReader::from_env(),
            trust,
            saver,
            last_used,
            dev_session,
            restore: restore_glue::Host::new(no_restore, crash_loop, RealDisk::from_env()),
            notice,
            updates,
            settings_sync,
            desktop: shell.desktop.clone(),
            shell: shell::start(&cc.egui_ctx, shell),
            services,
            github_events,
            services_ui: services_view::Memory::from_env(),
            workflow,
            house_style_scan: crate::house_style_scan::Host::new(),
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
    services: Vec<handoff::ResumedService<PtyTerm>>,
    tabs: Tabs<Pane<PtyTerm>>,
    credentials: Vec<(u64, tidewatch_core::listener::Credential)>,
    notes: Vec<String>,
    focus: bool,
    hook_port: Option<u16>,
}

/// A screenshot, background or fake-gh run starts no forwarder, so it never makes a webhook on a
/// real repo.
fn github_events_host(
    tools: Tools,
    paths: Option<TidewatchPaths>,
    ctx: &egui::Context,
    throwaway: bool,
) -> tidewatch_core::github_events::Host {
    let fake_gh = std::env::var_os("TIDEWATCH_DEV_GH_DIR").is_some_and(|dir| !dir.is_empty());
    let wake = ctx.clone();
    tidewatch_core::github_events::Host::new(
        tools,
        paths,
        std::sync::Arc::new(move || wake.request_repaint()),
        !throwaway && !fake_gh,
    )
}

fn services_spawner(ctx: &egui::Context) -> services_glue::Spawner {
    let ctx = ctx.clone();
    Box::new(move |repo, command| {
        crate::pty::spawn_service(stdusk_egui::repaint_handle(&ctx), repo, command)
    })
}

/// A screenshot or background run starts no saved service on its own, so a throwaway run in the
/// real home never starts what the owner's repos define, unless `TIDEWATCH_DEV_SERVICES_AUTOSTART=1`.
fn services_autostart(throwaway: bool) -> bool {
    !throwaway || std::env::var("TIDEWATCH_DEV_SERVICES_AUTOSTART").is_ok_and(|v| v == "1")
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
    let handoff::Rebuilt { mut tabs, credentials, notes, mut services } = rebuilt;
    if let Err(e) = handoff::confirm(&incoming, &mut tabs, &mut services) {
        eprintln!("tidewatch: could not confirm the handover ({e})");
    }
    shell.claim_from_old_window();
    let handover = incoming.session();
    Adopted {
        tabs,
        credentials,
        notes,
        focus: handover.focus,
        hook_port: handover.hook_port,
        services,
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
        self.drive_image_paste(&ctx, now, &intent);
        self.drive_file_drop(&ctx, &intent);
        self.show_tab_strip(ui, now, &mut intent);
        self.show_status_bar(ui, intent.active_id);
        self.show_central_panel(ui, &ctx, now, &mut intent);
        self.apply_frame_intent(&ctx, intent);
        self.show_dialogs(&ctx);
        if self.shell.chrome() == window_chrome::Chrome::Drawn {
            window_chrome::edges(&ctx);
        }
        self.finish_frame(&ctx, now);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(at) = self.dev_hover.take() {
            raw_input.events.push(egui::Event::PointerMoved(at));
            ctx.request_repaint();
        }
        if let Some(path) = &self.dev_hover_files {
            raw_input.hovered_files = vec![egui::HoveredFile {
                path: Some(path.clone()),
                ..egui::HoveredFile::default()
            }];
        }
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
        self.services.shutdown();
        self.github_events.shutdown(self.hooks.listener().as_deref());
        if let Some(paths) = self.paths.as_ref().filter(|_| self.screenshot.is_none()) {
            run_marker::end(paths);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let bg = colors::bg();
        egui::Rgba::from(bg).to_array()
    }
}
