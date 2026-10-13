//! The `eframe::App`: tab strip on top, the active tab's terminal below it, then the History and
//! Diff panes. Frame work collects `tabs::Action`s and applies them after drawing.
//! Git work for worktree tabs and for the panes runs on `jobs` threads; its results are applied at
//! the top of the next frame.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use egui::{CornerRadius, FontId, Frame, Key, Margin, Modifiers};
use kelp_core::refs::RefLabel;
use kelp_ui::avatars::AvatarStore;
use kelp_ui::graph::{GraphAction, Selection};
use kelp_ui::theme as kelp_theme;
use stdusk_core::progress::Progress;
use stdusk_core::terminal::{CmdState, PtyTerm};
use stdusk_egui::colors;
use stdusk_egui::ui::{
    CursorStyle, GridStyle, TAB_H, apply_theme, collect_input, draw_tab, draw_toast, icons,
    render_grid,
};
use stdusk_egui::widgets::{chip, icon_button};
use tidewatch_core::notes::NotesWriter;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::WindowPrefs;
use tidewatch_core::prepare::Prepared;
use tidewatch_core::restore::{RealDisk, WindowPlan};
use tidewatch_core::status::AgentStatus;
use tidewatch_core::tools::Tools;
use tidewatch_core::trust::TrustCli;
use tidewatch_core::worktree::{BranchOutcome, Inspection, Worktree};

use crate::agent_events::Hooks;
use crate::agent_frame::{self, Follow, Wake};
use crate::agent_jobs;
use crate::agent_run::{TermSnapshot, progress_fraction};
use crate::catalog::Catalog;
use crate::checkout::{self, Plan};
use crate::close_dialog::{self, Answer, Choice};
use crate::delivery_card::Delivery;
use crate::delivery_view;
use crate::dev_agent::{DevAgent, Mode as DevMode};
use crate::dev_flow::DevFlow;
use crate::dev_panes::{self, DevPanes};
use crate::dev_reader::DevReader;
use crate::dev_screenshot::DevScreenshot;
use crate::dev_session::{DevSession, LeftoverStep};
use crate::dev_view::{DevKeys, DevView};
use crate::dev_workflow::DevWorkflow;
use crate::dev_worktree::DevWorktree;
use crate::end_block_view;
use crate::flow::Flow;
use crate::flow_glue;
use crate::jobs::Jobs;
use crate::leftover_glue;
use crate::new_tab::{Group, NewTab, Request};
use crate::new_tab_dialog::{self, Outcome};
use crate::notice;
use crate::notify::{self, Notifier};
use crate::os::RealOs;
use crate::overview;
use crate::overview_view;
use crate::pane::{
    Body, CREATING_TITLE, ClosePolicy, Pane, STARTING_TITLE, agent_tabs_in_folder, finish_open,
};
use crate::pane_jobs;
use crate::pty::{spawn_agent, spawn_shell, spawn_shell_noted};
use crate::reader::Shown;
use crate::reader_glue;
use crate::repo::Workspace;
use crate::restore_glue::{self, Spawn};
use crate::session_glue::Saver;
use crate::session_save::WindowView;
use crate::settings::{Section, Settings};
use crate::settings_view::{self, Files, General};
use crate::shell::{self, Event, Shell};
use crate::status_view;
use crate::stepper_view;
use crate::tab_view::{TabView, ViewSpec};
use crate::tabs::{Action, Tabs};
use crate::toast::Toast;
use crate::trust_glue::Trust;
use crate::trust_view;
use crate::window_view;
use crate::workflow_glue::{self, Env, Spawners, Start};
use crate::workflow_run::Exec;
use crate::worktree_jobs::{self, Done, OpenSpec, Target};
use crate::{diff_pane, fonts, history_pane, layout, note_actions};

const DEFAULT_THEME: &str = "one-half-dark";
const FONT_SIZE: f32 = 13.0;
const MIN_CONTRAST: f32 = 4.0;
const STRIP_TOP_MARGIN: i8 = 6;
const STRIP_SIDE_MARGIN: i8 = 8;
const STRIP_GAP: f32 = 4.0;
const STRIP_CORNER: u8 = 10;
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

    /// Opens the new-tab dialog. Without a home directory there are no profiles to pick from, so
    /// the tab opens as a plain shell.
    fn new_tab(&mut self, ctx: &egui::Context) {
        if self.new_tab.is_some() {
            return;
        }
        if self.paths.is_none() {
            self.open_tab(ctx, Request::Shell);
            return;
        }
        if !self.catalog.is_loaded() {
            self.show_toast("Profiles are still loading", ctx);
            return;
        }
        self.new_tab = Some(self.dialog_state(self.last_profile.clone().as_deref()));
    }

    fn dialog_state(&self, preselect: Option<&str>) -> NewTab {
        let (group, label, dir) = match &self.workspace {
            Workspace::Repo(repo) => (Group::Repo, repo.label.clone(), repo.root.clone()),
            Workspace::Other { dir, label } => {
                (Group::Other, format!("Other: {label}"), dir.clone())
            }
        };
        NewTab::open(group, label, dir, &self.catalog, agent_tabs_in_folder(&self.tabs), preselect)
    }

    /// Runs the `TIDEWATCH_DEV_AGENT` / `TIDEWATCH_DEV_DIALOG` switch once the profiles are loaded.
    fn run_dev_agent(&mut self, ctx: &egui::Context) {
        let Some(dev) = self.dev_agent.take() else { return };
        if dev.mode == DevMode::Settings {
            self.toggle_settings();
            if let Some(settings) = &mut self.settings {
                match dev.profile.as_deref() {
                    Some("workflows") => settings.section = Section::Workflows,
                    Some("trust") => settings.section = Section::Trust,
                    Some("general") => settings.section = Section::General,
                    _ => {}
                }
            }
            return;
        }
        if dev.mode == DevMode::Resume {
            self.dev_resume(ctx, dev.profile.as_deref().unwrap_or_default());
            return;
        }
        let mut state = self.dialog_state(dev.profile.as_deref());
        if let Some(level) = dev.level
            && !state.set_level(level, &self.catalog)
        {
            eprintln!("tidewatch: dev agent not started: {} is not open here", level.label());
            return;
        }
        state.task = dev.task;
        state.task_edited();
        match dev.mode {
            DevMode::Settings | DevMode::Resume => {}
            DevMode::Dialog => self.new_tab = Some(state),
            DevMode::Start => match state.request(&self.catalog) {
                Ok(request) => self.open_tab(ctx, request),
                Err(reason) => eprintln!("tidewatch: dev agent not started: {reason}"),
            },
        }
    }

    /// In a git repo a new tab is a fresh worktree; in the Other group it is a plain shell, or an
    /// agent in the folder itself. An agent tab is prepared on a job thread first.
    fn open_tab(&mut self, ctx: &egui::Context, request: Request) {
        let agent = match request {
            Request::Shell => None,
            Request::Agent(agent) => Some(agent),
            Request::Workflow(request) => {
                self.open_workflow(ctx, Start::New(request), None);
                return;
            }
        };
        let now = ctx.input(|i| i.time);
        let pane = match &agent {
            Some(request) => {
                Pane::creating_agent(request.profile.name.clone(), request.agent.id.clone(), now)
            }
            None => Pane::creating(),
        };
        let listener = self.hooks.listener();
        let trust = self.trust.grant();
        match (&self.workspace, &self.paths) {
            (Workspace::Repo(repo), Some(paths)) => {
                let tab = self.tabs.push(CREATING_TITLE.to_owned(), pane);
                let target = Target::Worktree { repo: repo.root.clone() };
                let path_var = self.catalog.path.clone();
                let spec = OpenSpec {
                    paths: paths.clone(),
                    target,
                    agent,
                    path_var,
                    tab,
                    listener,
                    trust,
                };
                worktree_jobs::open(&self.jobs, ctx, spec);
            }
            (workspace, Some(paths)) if agent.is_some() => {
                let tab = self.tabs.push(STARTING_TITLE.to_owned(), pane);
                let target = Target::Folder { dir: workspace.dir().to_path_buf() };
                let path_var = self.catalog.path.clone();
                let spec = OpenSpec {
                    paths: paths.clone(),
                    target,
                    agent,
                    path_var,
                    tab,
                    listener,
                    trust,
                };
                worktree_jobs::open(&self.jobs, ctx, spec);
            }
            _ => {
                let dir = self.workspace.dir().to_path_buf();
                self.tabs.apply(Action::New, || Pane::shell(spawn_shell(ctx, &dir)));
            }
        }
    }

    /// A workflow needs a worktree, so only a repo can start one. The first stage starts when the
    /// worktree is ready.
    fn open_workflow(&mut self, ctx: &egui::Context, start: Start, existing: Option<Worktree>) {
        let (Workspace::Repo(repo), Some(paths)) = (&self.workspace, &self.paths) else {
            self.show_toast("A workflow needs a git repo", ctx);
            return;
        };
        let env = Env {
            paths,
            catalog: &self.catalog,
            listener: self.hooks.listener(),
            tools: self.flow.tools(),
            trust: self.trust.grant(),
        };
        let now = ctx.input(|i| i.time);
        self.runs.open(ctx, &mut self.tabs, &env, &repo.root, existing, start, now);
    }

    fn drive_dev_leftovers(&mut self, ctx: &egui::Context) {
        let Some(step) = self.dev_session.leftovers(self.leftovers.count() > 0) else { return };
        if !self.leftovers.is_open() {
            self.leftovers.open(ctx);
            self.dev_session.leftovers_opened();
        } else if step == LeftoverStep::Clean
            && self.leftovers.press_clean_up(ctx, self.paths.as_ref())
        {
            self.dev_session.leftovers_done();
        }
    }

    /// `TIDEWATCH_DEV_RESUME`: the restart path, for the one saved run in the named worktree.
    fn dev_resume(&mut self, ctx: &egui::Context, name: &str) {
        let (Workspace::Repo(repo), Some(paths)) = (&self.workspace, &self.paths) else { return };
        let plan = restore_glue::dev_resume_plan(paths, &repo.root, name);
        self.drive_restore(ctx, ctx.input(|i| i.time), Some(plan));
    }

    /// Carries out the restore: the plan of a dev switch, or whatever the startup and reopen jobs
    /// finished.
    fn drive_restore(&mut self, ctx: &egui::Context, now: f64, plan: Option<WindowPlan>) {
        let Some(paths) = &self.paths else { return };
        let env = Env {
            paths,
            catalog: &self.catalog,
            listener: self.hooks.listener(),
            tools: self.flow.tools(),
            trust: self.trust.grant(),
        };
        let agent = |dir: &Path, prepared: &Prepared| spawn_agent(ctx, dir, prepared);
        let shell = |dir: &Path, note: Option<&str>| spawn_shell_noted(ctx, dir, note);
        let spawn = Spawn { agent: &agent, shell: &shell };
        let runs = &mut self.runs;
        let report = match plan {
            Some(plan) => self.restore.apply(&plan, ctx, &mut self.tabs, &env, &spawn, runs, now),
            None => self.restore.tick(ctx, &mut self.tabs, &env, &spawn, runs, now),
        };
        for text in report.toasts {
            self.show_toast(text, ctx);
        }
        self.notice = report.notice.or(self.notice.take());
        if let Some(leftovers) = report.leftovers {
            self.leftovers.set(leftovers);
        }
        if let Some(focus) = report.focus {
            self.focus = focus;
        }
    }

    /// Carries out what the workflow runs asked for (from messages, failed processes and clicks) and
    /// applies what their jobs finished.
    fn drive_workflows(&mut self, ctx: &egui::Context, now: f64, mut asked: Vec<(u64, Vec<Exec>)>) {
        let Some(paths) = &self.paths else { return };
        asked.append(&mut self.run_asks);
        if !self.dev_workflow.is_idle() {
            asked.extend(self.dev_workflow.drive(&mut self.tabs));
        }
        let env = Env {
            paths,
            catalog: &self.catalog,
            listener: self.hooks.listener(),
            tools: self.flow.tools(),
            trust: self.trust.grant(),
        };
        let agent = |dir: &Path, prepared: &Prepared| spawn_agent(ctx, dir, prepared);
        let shell = |dir: &Path| spawn_shell(ctx, dir);
        let spawn = Spawners { agent: &agent, shell: &shell };
        let toasts = self.runs.tick(ctx, &mut self.tabs, &env, &spawn, asked, now);
        for text in toasts {
            self.show_toast(text, ctx);
        }
    }

    fn apply_agent_done(&mut self, done: agent_jobs::Done, ctx: &egui::Context) {
        match done {
            agent_jobs::Done::Loaded { profiles, workflows } => {
                self.catalog.set_workflows(workflows);
                let probes = self.catalog.set_profiles(profiles);
                self.probe_agents(probes, ctx);
                self.refresh_trust(ctx);
                if let Some(paths) = &self.paths {
                    self.restore.begin(ctx, paths, &self.workspace, &self.catalog);
                }
                self.run_dev_agent(ctx);
            }
            agent_jobs::Done::Probed { agent, availability } => {
                self.catalog.set_availability(&agent, availability);
            }
            agent_jobs::Done::Saved { result, profiles } => match result {
                Ok(()) => {
                    let probes = self.catalog.set_profiles(profiles.clone());
                    self.probe_agents(probes, ctx);
                    if let Some(settings) = &mut self.settings {
                        settings.profiles_saved(&profiles);
                    }
                    self.show_toast("Profiles saved", ctx);
                }
                Err(message) => {
                    if let Some(settings) = &mut self.settings {
                        settings.profiles.save_failed(format!("Could not save: {message}"));
                    }
                }
            },
            agent_jobs::Done::WorkflowsSaved { result, workflows } => match result {
                Ok(()) => {
                    self.catalog.set_workflows(workflows);
                    if let Some(settings) = &mut self.settings {
                        settings.workflows.saved();
                    }
                    self.show_toast("Workflows saved", ctx);
                }
                Err(message) => {
                    if let Some(settings) = &mut self.settings {
                        settings.workflows.save_failed(format!("Could not save: {message}"));
                    }
                }
            },
        }
    }

    fn refresh_trust(&self, ctx: &egui::Context) {
        let bins = self
            .catalog
            .agent_names()
            .iter()
            .filter_map(|(id, _)| self.catalog.agent(id))
            .filter_map(|a| TrustCli::from_wiring(a.wiring).map(|cli| (cli, a.bin.clone())))
            .collect();
        self.trust.refresh(ctx, bins, self.catalog.path.clone());
    }

    fn probe_agents(&self, agents: Vec<(String, String)>, ctx: &egui::Context) {
        for (agent, bin) in agents {
            agent_jobs::probe(&self.agent_jobs, ctx, agent, bin, self.catalog.path.clone());
        }
    }

    fn toggle_settings(&mut self) {
        if self.settings.take().is_some() {
            return;
        }
        if let (Some(profiles), Some(workflows)) =
            (self.catalog.profiles(), self.catalog.workflows())
        {
            self.settings = Some(Settings::new(
                profiles,
                workflows,
                self.catalog.agent_names(),
                self.shell.prefs(),
            ));
        }
    }

    /// What another launch, the hotkey or the menu-bar item asked of the app itself.
    fn shell_event(&mut self, event: &Event, ctx: &egui::Context) {
        if self.status_log {
            eprintln!("tidewatch: shell {} at {}", shell::describe(event), wall_ms());
        }
        match event {
            Event::Overview => {
                self.overview = true;
                self.settings = None;
            }
            Event::Settings if self.settings.is_none() => self.toggle_settings(),
            Event::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Event::Settings | Event::Opened(_) | Event::Toggled => {}
        }
    }

    fn set_window_prefs(&mut self, prefs: WindowPrefs, ctx: &egui::Context) {
        if let Some(paths) = &self.paths {
            shell::save(paths.clone(), prefs.clone());
        }
        self.shell.apply_prefs(ctx, prefs);
    }

    fn save_settings(&mut self, ctx: &egui::Context) {
        let (Some(settings), Some(paths)) = (&mut self.settings, &self.paths) else { return };
        match settings.section {
            Section::Profiles => match settings.profiles.build() {
                Ok(profiles) => {
                    settings.profiles.saving = true;
                    agent_jobs::save(&self.agent_jobs, ctx, paths.clone(), profiles);
                }
                Err(message) => settings.profiles.error = Some(message),
            },
            Section::Workflows => match settings.workflows.build() {
                Ok(workflows) => {
                    settings.workflows.saving = true;
                    agent_jobs::save_workflows(&self.agent_jobs, ctx, paths.clone(), workflows);
                }
                Err(message) => settings.workflows.error = Some(message),
            },
            Section::Trust | Section::General => {}
        }
    }

    fn answer_new_tab(&mut self, outcome: Outcome, ctx: &egui::Context) {
        self.new_tab = None;
        let Outcome::Start(request) = outcome else { return };
        if let Request::Agent(agent) = &request {
            self.last_profile = Some(agent.profile_id.clone());
        }
        self.open_tab(ctx, request);
    }

    fn close_tab(&mut self, index: usize, ctx: &egui::Context) {
        let Some(tab) = self.tabs.items().get(index) else { return };
        if tab.pinned {
            return;
        }
        let id = tab.id;
        match tab.pane.close_policy() {
            ClosePolicy::Now => {
                self.tabs.apply(Action::Close(index), || unreachable!());
                self.forget_tab(id);
            }
            ClosePolicy::Wait => {}
            ClosePolicy::Confirm(worktree) => {
                if self.closing.is_none() {
                    self.closing = Some(Closing::Inspecting(id));
                    worktree_jobs::inspect(&self.jobs, ctx, id, worktree);
                }
            }
        }
    }

    fn forget_tab(&mut self, id: u64) {
        self.hooks.close_tab(id);
        self.notifier.forget(id);
        self.flow.forget(id);
        self.runs.forget(id);
    }

    fn answer_close(&mut self, answer: Answer, ctx: &egui::Context) {
        let Some(Closing::Asking { tab, .. }) = self.closing.take() else { return };
        match answer {
            Answer::Cancel => {}
            Answer::Pick(Choice::KeepWorktree) => {
                self.tabs.remove_id(tab);
                self.forget_tab(tab);
            }
            Answer::Pick(Choice::Remove { branch, mode }) => {
                let Some(paths) = self.paths.clone() else { return };
                let Some((shell, worktree)) =
                    self.tabs.get_mut(tab).and_then(|t| t.pane.begin_remove())
                else {
                    return;
                };
                let trust = self.trust.grant();
                worktree_jobs::remove(
                    &self.jobs, ctx, paths, tab, shell, worktree, branch, mode, trust,
                );
            }
        }
    }

    fn apply_done(&mut self, done: Done, ctx: &egui::Context) {
        match done {
            Done::Opened { tab, result } => {
                let created = result.is_ok();
                let now = ctx.input(|i| i.time);
                let notes =
                    finish_open(&mut self.tabs, tab, result, now, |dir, prepared| match prepared {
                        Some(prepared) => spawn_agent(ctx, dir, prepared),
                        None => spawn_shell(ctx, dir),
                    });
                if let Some(note) = notes.first() {
                    self.show_toast(format!("Started without part of the setup: {note}"), ctx);
                }
                if created
                    && self.dev.is_some_and(DevWorktree::closes_after_opening)
                    && let Some(index) = self.tabs.index_of(tab)
                {
                    self.close_tab(index, ctx);
                }
            }
            Done::Inspected { tab, worktree, inspection } => {
                if !matches!(self.closing, Some(Closing::Inspecting(id)) if id == tab) {
                    return;
                }
                self.closing = Some(Closing::Asking { tab, worktree, inspection });
                if self.dev.is_some_and(DevWorktree::answers_the_dialog)
                    && let Some(choice) = close_dialog::choices(&inspection).get(1).copied()
                {
                    self.answer_close(Answer::Pick(choice), ctx);
                }
            }
            Done::Removed { tab, result } => match result {
                Ok(outcome) => {
                    self.tabs.remove_id(tab);
                    self.forget_tab(tab);
                    if let BranchOutcome::DeleteFailed(message) = outcome {
                        eprintln!("tidewatch: worktree removed, branch kept: {message}");
                    }
                }
                Err(message) => {
                    if let Some(tab) = self.tabs.get_mut(tab) {
                        tab.pane.remove_failed(message);
                    }
                }
            },
        }
    }

    /// Keeps one view per tab that has panes: only the visible tab's view is loaded and watched.
    fn drive_views(&mut self) {
        let open: HashSet<u64> = self.tabs.items().iter().map(|t| t.id).collect();
        self.views.retain(|id, _| open.contains(id));
        if self.focus || self.overview {
            for view in self.views.values_mut().filter(|view| view.is_active()) {
                view.stop();
            }
            return;
        }
        let active = &self.tabs.items()[self.tabs.active()];
        let active_id = active.id;
        for (id, view) in &mut self.views {
            if *id != active_id && view.is_active() {
                view.stop();
            }
        }
        let Some(spec) = ViewSpec::of(&self.workspace, self.paths.as_ref(), active) else { return };
        let dev_scope = self.dev_panes.scope;
        let view = self.views.entry(active_id).or_insert_with(|| {
            let mut view = spec.into_view();
            if let Some(scope) = dev_scope {
                view.set_scope(scope);
            }
            view
        });
        if !view.is_active() {
            view.start();
        }
        view.poll();
    }

    fn apply_pane_done(&mut self, done: pane_jobs::Done, ctx: &egui::Context) {
        let tab = done.tab();
        if let pane_jobs::Done::Checkout { outcome, .. } = &done {
            self.show_toast(checkout::message(outcome), ctx);
            if let Some(view) = self.views.get_mut(&tab) {
                view.reload_after_checkout();
            }
            return;
        }
        if let Some(view) = self.views.get_mut(&tab) {
            view.apply(done);
        }
    }

    fn show_toast(&mut self, text: impl Into<String>, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.toast = Some(Toast::new(text, now));
        ctx.request_repaint();
    }

    fn double_clicked_ref(&mut self, label: &RefLabel, ctx: &egui::Context) {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let Some(view) = self.views.get(&active_id) else { return };
        match checkout::plan(view.is_main, label) {
            Plan::Ignore => {}
            Plan::Refuse(reason) => self.show_toast(reason, ctx),
            Plan::Switch(op) => {
                pane_jobs::run_checkout(&self.pane_jobs, ctx, active_id, view.dir.clone(), op);
            }
        }
    }

    fn save_notes(&self, view_id: u64) {
        let Some(view) = self.views.get(&view_id) else { return };
        if let Some(file) = view.notes_file() {
            self.notes_writer.save(file.clone(), view.notes.clone());
        }
    }

    /// Applies the `TIDEWATCH_DEV_PICK` and `TIDEWATCH_DEV_NOTE` switches once the view can take
    /// them. Returns whether a note was added.
    fn apply_dev_panes(&mut self, view_id: u64, now: i64) -> bool {
        let Some(view) = self.views.get_mut(&view_id) else { return false };
        if let Some(row) = self.dev_panes.pick_row
            && view.status_known()
            && let Some(loaded) = &view.loaded
        {
            self.dev_panes.pick_row = None;
            if row < loaded.history.len() {
                view.select(Selection::Commit(row));
            }
        }
        let Some(text) = &self.dev_panes.note else { return false };
        if !view.notes_ready || view.is_loading_source() {
            return false;
        }
        let Some(open) = &view.open else { return false };
        let Some(action) = dev_panes::note_on_first_added_line(&open.diff, text, now) else {
            return false;
        };
        let (path, commit) =
            (open.diff.path.clone(), view.wanted_commit().map(|id| id.to_string()));
        self.dev_panes.note = None;
        note_actions::apply(&mut view.notes, &path, commit.as_deref(), vec![action])
    }

    fn workspace_has_panels(&self) -> bool {
        matches!(self.workspace, Workspace::Repo(_))
    }

    fn show_panes(&mut self, ui: &mut egui::Ui, areas: &layout::Layout, ctx: &egui::Context) {
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let Some(view) = self.views.get_mut(&active_id) else {
            for rect in [areas.history, areas.diff] {
                ui.painter().rect_filled(rect, 0.0, kelp_theme::bg());
            }
            return;
        };
        let status = self.tabs.items()[self.tabs.active()]
            .pane
            .agent
            .as_ref()
            .map(|agent| agent.run.status().clone());
        let extras = status.and_then(|status| self.flow.extras(active_id, &view.notes, &status));
        let graph_action = history_pane::show(ui, areas.history, view, &mut self.avatars);
        let now_unix = flow_glue::unix_now();
        let pane = diff_pane::show(ui, areas.diff, view, extras.as_ref(), now_unix);
        let notes_changed = pane.notes_changed;
        let toasts = flow_glue::pane_actions(&mut self.flow, view, active_id, pane.actions, ctx);
        let mut clicked_ref = None;
        match graph_action {
            Some(GraphAction::Select(selection)) => {
                view.select(selection);
                ctx.request_repaint();
            }
            Some(GraphAction::ActivateRef(label)) => clicked_ref = Some(label),
            Some(_) | None => {}
        }
        view.sync(&self.pane_jobs, ctx);
        for text in toasts {
            self.show_toast(text, ctx);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if notes_changed || self.apply_dev_panes(active_id, now) {
            self.save_notes(active_id);
            ctx.request_repaint();
        }
        if let Some(label) = clicked_ref {
            self.double_clicked_ref(&label, ctx);
        }
    }

    fn show_toast_pill(&mut self, ui: &egui::Ui) {
        let Some(toast) = &self.toast else { return };
        let now = ui.ctx().input(|i| i.time);
        if toast.expired(now) {
            self.toast = None;
            return;
        }
        draw_toast(ui, toast.text(), toast.fade(now));
        ui.ctx().request_repaint_after(Duration::from_secs_f64(toast.next_frame_in(now)));
    }

    fn follow(&mut self, follow: Follow, ctx: &egui::Context) {
        match follow {
            Follow::Finding { tab, notes_file, dir, author, finding } => {
                let dir = dir.unwrap_or_else(|| self.workspace.dir().to_path_buf());
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                pane_jobs::add_finding(
                    &self.pane_jobs,
                    ctx,
                    tab,
                    notes_file,
                    dir,
                    author,
                    finding,
                    now,
                );
            }
            Follow::ReloadNotes { tab } => {
                if let Some(view) = self.views.get_mut(&tab) {
                    view.reload_notes();
                    ctx.request_repaint();
                }
            }
        }
    }

    /// Takes what agents reported, feeds terminal signals into every agent tab's status and
    /// notifies for changes while the window is away.
    fn drive_agents(&mut self, ctx: &egui::Context, now: f64) {
        let mut asked = Vec::new();
        for message in self.hooks.drain() {
            if self.status_log {
                eprintln!(
                    "tidewatch: hook tab={} gen={} {:?} at {}",
                    message.tab_id,
                    message.generation,
                    message.event,
                    wall_ms()
                );
            }
            let Some(execs) = workflow_glue::feed(&mut self.tabs, &message) else { continue };
            let tab = message.tab_id;
            if let Some(follow) = agent_frame::route(&mut self.tabs, message, now) {
                self.follow(follow, ctx);
            }
            asked.push((tab, execs));
        }
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
        let away = notify::window_is_away(focused, minimized);
        let polled =
            agent_frame::poll(&mut self.tabs, TermSnapshot::of, &mut self.notifier, away, now);
        if self.status_log {
            for (tab, change) in &polled.changes {
                eprintln!(
                    "tidewatch: status tab={tab} {:?} -> {:?} at {}",
                    change.from,
                    change.to,
                    wall_ms()
                );
            }
        }
        asked.extend(workflow_glue::failures(&mut self.tabs, &polled.changes));
        self.drive_workflows(ctx, now, asked);
        self.send_dev_keys(now);
        schedule(ctx, polled.wake, now, minimized == Some(true));
    }

    /// Starts what the flow needs for each agent tab, applies what its jobs finished and presses the
    /// Enter that follows a paste.
    fn drive_flow(&mut self, ctx: &egui::Context, now: f64) {
        let unix = flow_glue::unix_now();
        let outs = self.flow.drain(ctx, now);
        for text in flow_glue::apply_outs(outs, &mut self.tabs, &mut self.views) {
            self.show_toast(text, ctx);
        }
        for tab in self.flow.due_enters(now) {
            flow_glue::press_enter(&mut self.tabs, tab);
        }
        let active = self.tabs.items()[self.tabs.active()].id;
        let facts = flow_glue::facts(&self.tabs, &self.views, active);
        if let Some(at) = self.flow.tick(&facts, ctx, now, unix) {
            ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
        }
        if !self.dev_flow.is_idle() {
            let view = self.views.get(&active);
            flow_glue::dev_drive(&mut self.dev_flow, &mut self.flow, view, active, ctx);
        }
    }

    fn send_dev_keys(&mut self, now: f64) {
        let Some(keys) = self.dev_keys.as_ref().filter(|keys| now >= keys.after) else { return };
        let target =
            self.tabs.items_mut().iter_mut().rev().filter(|tab| tab.pane.agent.is_some()).find_map(
                |tab| match &mut tab.pane.body {
                    Body::Shell(term) => Some(term),
                    _ => None,
                },
            );
        if let Some(term) = target {
            term.send(&keys.bytes);
            self.dev_keys = None;
        }
    }

    fn overview_data(&self, now: f64) -> overview::Overview {
        let place =
            overview::collect(&self.workspace.chip_label(), self.workspace.dir(), &self.tabs, now);
        overview::build(vec![place])
    }

    fn toggle_overview(&mut self) {
        self.overview = !self.overview;
        self.settings = None;
    }

    fn toggle_reader(&mut self, ctx: &egui::Context) {
        if let Some(shown) = reader_glue::toggle(self.tabs.active_pane_mut()) {
            self.show_toast(reader_glue::toggle_text(shown), ctx);
        }
    }

    fn toggle_focus(&mut self, ctx: &egui::Context) {
        self.focus = !self.focus;
        let text = if self.focus { "Focus mode on (Cmd+.)" } else { "Focus mode off (Cmd+.)" };
        self.show_toast(text, ctx);
    }

    fn dispatch(&mut self, action: Action, ctx: &egui::Context) {
        self.settings = None;
        self.overview = false;
        match action {
            Action::New => self.new_tab(ctx),
            Action::CloseActive => self.close_tab(self.tabs.active(), ctx),
            Action::Close(index) => self.close_tab(index, ctx),
            Action::Select(_) => self.tabs.apply(action, || unreachable!()),
        }
    }
}

fn wall_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

/// Asks for the next frame agent status needs: a short tick while a tab works (its bar moves), and
/// one wake-up at the moment a status decays. A window that is minimized does not animate.
fn schedule(ctx: &egui::Context, wake: Wake, now: f64, minimized: bool) {
    if wake.animate && !minimized {
        ctx.request_repaint_after(Duration::from_millis(ANIMATION_FRAME_MS));
    }
    if let Some(at) = wake.next_change {
        ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + WAKE_SLACK_S));
    }
}

fn status_message(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    let painter = ui.painter_at(rect);
    let galley = painter.layout(
        text.to_owned(),
        FontId::monospace(FONT_SIZE),
        color,
        (rect.width() * 0.8).max(1.0),
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}

fn exit_notice_text(profile_name: &str, code: i32) -> String {
    format!("{profile_name} exited with code {code}. Cmd+W closes the tab.")
}

fn exit_notice(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    let painter = ui.painter_at(rect);
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(13.0), colors::fg());
    let pill = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - 28.0),
        galley.size() + egui::vec2(24.0, 14.0),
    );
    painter.rect_filled(pill, 8.0, colors::elevated());
    painter.rect_stroke(
        pill,
        8.0,
        egui::Stroke::new(1.0, colors::border()),
        egui::StrokeKind::Inside,
    );
    painter.galley(pill.center() - galley.size() / 2.0, galley, colors::fg());
}

fn tab_signals(pane: &Pane<PtyTerm>) -> (Progress, CmdState) {
    match &pane.body {
        Body::Shell(term) => (term.progress(), term.cmd_state()),
        _ => (Progress::default(), CmdState::default()),
    }
}

fn shortcut_actions(ctx: &egui::Context) -> Vec<Action> {
    ctx.input_mut(|i| {
        let mut actions = Vec::new();
        if i.consume_key(Modifiers::COMMAND, Key::T) {
            actions.push(Action::New);
        }
        if i.consume_key(Modifiers::COMMAND, Key::W) {
            actions.push(Action::CloseActive);
        }
        for key in [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ] {
            if let Some(idx) = layout::digit_index(key)
                && i.consume_key(Modifiers::COMMAND, key)
            {
                actions.push(Action::Select(idx));
            }
        }
        actions
    })
}

impl eframe::App for Tidewatch {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        for done in self.jobs.drain() {
            self.apply_done(done, &ctx);
        }
        for done in self.pane_jobs.drain() {
            self.apply_pane_done(done, &ctx);
        }
        for done in self.agent_jobs.drain() {
            self.apply_agent_done(done, &ctx);
        }
        for event in self.shell.tick(&ctx) {
            self.shell_event(&event, &ctx);
        }
        let toasts = [self.trust.drain(&ctx), self.leftovers.tick()];
        for text in toasts.into_iter().flatten() {
            self.show_toast(text, &ctx);
        }
        let now = ctx.input(|i| i.time);
        self.dev_reader.press_toggle(&ctx, now);
        reader_glue::start(&mut self.tabs, &ctx, &self.dev_reader);
        reader_glue::drain(&mut self.tabs);
        self.drive_agents(&ctx, now);
        self.drive_flow(&ctx, now);
        self.drive_restore(&ctx, now, None);
        self.drive_dev_leftovers(&ctx);
        self.drive_views();
        let typing_in_a_panel = ctx.text_edit_focused();
        let active_id = self.tabs.items()[self.tabs.active()].id;
        let modal_open = self.closing.is_some()
            || self.new_tab.is_some()
            || self.settings.is_some()
            || self.trust.dialog_open()
            || self.leftovers.is_open()
            || self.flow.modal_open(active_id);
        if self.closing.is_none()
            && self.new_tab.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Comma))
        {
            self.toggle_settings();
        }
        let reader_key = Modifiers::COMMAND.plus(Modifiers::SHIFT);
        if !modal_open
            && !self.overview
            && self.settings.is_none()
            && ctx.input_mut(|i| i.consume_key(reader_key, Key::T))
        {
            self.toggle_reader(&ctx);
        }
        let mut actions = if modal_open { Vec::new() } else { shortcut_actions(&ctx) };
        if self.overview {
            actions.retain(|action| *action != Action::CloseActive);
        }
        let mut toggle_settings = false;
        let mut settings_action = None;
        let mut toggle_overview = false;
        if self.closing.is_none() && self.new_tab.is_none() {
            toggle_overview = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Num0));
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Period)) {
                self.toggle_focus(&ctx);
            }
            if self.overview && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                toggle_overview = true;
            }
        }
        let needs_you = agent_frame::tones(&self.tabs)
            .into_iter()
            .filter(|tone| *tone == status_view::Tone::Needs)
            .count();

        egui::Panel::top("tabbar")
            .exact_size(TAB_H + f32::from(STRIP_TOP_MARGIN))
            .frame(
                Frame::new()
                    .fill(colors::titlebar())
                    .corner_radius(CornerRadius {
                        nw: STRIP_CORNER,
                        ne: STRIP_CORNER,
                        sw: 0,
                        se: 0,
                    })
                    .inner_margin(Margin {
                        left: STRIP_SIDE_MARGIN,
                        right: STRIP_SIDE_MARGIN,
                        top: STRIP_TOP_MARGIN,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(TAB_H);
                    ui.spacing_mut().item_spacing.x = STRIP_GAP;
                    ui.add_space(4.0);
                    chip(ui, &self.workspace.chip_label(), true)
                        .on_hover_text(self.workspace.dir().display().to_string());
                    self.leftovers.chip(ui);
                    ui.add_space(4.0);
                    if overview_view::button(ui, self.overview, needs_you).clicked() {
                        toggle_overview = true;
                    }
                    ui.add_space(8.0);
                    let width = layout::tab_width(
                        ui.available_width()
                            - stdusk_egui::widgets::ICON_BTN_W * 2.0
                            - overview_view::BUTTON_W
                            - 22.0,
                        self.tabs.items().len(),
                        STRIP_GAP,
                    );
                    for (i, tab) in self.tabs.items().iter().enumerate() {
                        let (pty_progress, cmd) = tab_signals(&tab.pane);
                        let agent = tab.pane.agent.as_ref();
                        let progress = if agent.is_some() { Progress::None } else { pty_progress };
                        let cli = agent.and_then(|a| status_view::cli_of(&a.run.agent_id));
                        let (resp, close) = draw_tab(
                            ui,
                            Some(i + 1),
                            tab.id,
                            &tab.title,
                            i == self.tabs.active(),
                            tab.pinned,
                            None,
                            progress,
                            cmd,
                            &[],
                            cli,
                            Some(width),
                        );
                        if let Some(agent) = agent {
                            let tone = status_view::tone(agent.run.status());
                            let fraction = progress_fraction(pty_progress);
                            status_view::paint_bar(ui, resp.rect, tone, now, fraction);
                        }
                        if close {
                            actions.push(Action::Close(i));
                        } else if resp.clicked() {
                            actions.push(Action::Select(i));
                        }
                    }
                    ui.add_space(6.0);
                    if icon_button(ui, icons::PLUS, "New tab (Cmd+T)").clicked() {
                        actions.push(Action::New);
                    }
                    if icon_button(ui, icons::GEAR, "Profiles (Cmd+,)").clicked() {
                        toggle_settings = true;
                    }
                });
            });

        egui::CentralPanel::default().frame(Frame::new().fill(colors::bg())).show(ui, |ui| {
            let area = ui.max_rect();
            if self.overview && self.settings.is_none() {
                let data = self.overview_data(now);
                if overview::is_ticking(&data) {
                    ctx.request_repaint_after(Duration::from_secs(1));
                }
                if let Some(tab) = overview_view::show(ui, area, &data)
                    && let Some(index) = self.tabs.index_of(tab)
                {
                    actions.push(Action::Select(index));
                }
                return;
            }
            if let Some(settings) = &mut self.settings {
                let files = self.paths.as_ref().map_or_else(
                    || Files {
                        profiles: String::new(),
                        workflows: String::new(),
                        trust: String::new(),
                        general: String::new(),
                    },
                    |p| Files {
                        profiles: p.profiles_file().display().to_string(),
                        workflows: p.workflows_file().display().to_string(),
                        trust: p.trust_file().display().to_string(),
                        general: p.prefs_file().display().to_string(),
                    },
                );
                let general = General {
                    restore: self.restore.restore_enabled(),
                    skipped_by_flag: self.restore.skipped_by_flag(),
                    window: window_view::View {
                        status: self.shell.hotkey_status(),
                        menu_bar_available: self.shell.tray_supported(),
                    },
                };
                settings_action =
                    settings_view::show(ui, area, settings, &files, &self.trust.page(), &general);
                return;
            }
            let panels = self.workspace_has_panels() && !self.focus;
            let mut areas = layout::split(area);
            if !panels {
                areas.terminal = layout::terminal_only(area);
            }
            let header = self
                .tabs
                .active_pane_mut()
                .agent
                .as_mut()
                .and_then(|agent| agent.workflow.as_mut())
                .map(|run| stepper_view::show(ui, &mut areas.terminal, run));
            if let Some(execs) = header.filter(|execs| !execs.is_empty()) {
                self.run_asks.push((active_id, execs));
                ctx.request_repaint();
            }
            let end = self.tabs.items()[self.tabs.active()].pane.agent.as_ref().and_then(|agent| {
                let waiting =
                    matches!(agent.run.status(), AgentStatus::Done | AgentStatus::NeedsYou(_));
                agent
                    .run
                    .end_block()
                    .filter(|_| waiting)
                    .map(|block| (block.clone(), agent.run.end_open))
            });
            let end_band = end.as_ref().map(|(block, open)| {
                layout::band_below(&mut areas.terminal, end_block_view::height(block, *open))
            });
            let card = self.flow.tab(active_id).and_then(|state| {
                state
                    .delivery
                    .card_commits()
                    .map(|n| (n, matches!(state.delivery, Delivery::Building { .. })))
            });
            let band = card.map(|_| layout::band_below(&mut areas.terminal, delivery_view::BAND_H));
            let font = egui::FontId::monospace(FONT_SIZE);
            let glyph = ui.painter().layout_no_wrap("M".to_owned(), font.clone(), colors::fg());
            let (cell_w, cell_h) = (glyph.size().x, glyph.size().y);
            let (cols, rows) = layout::grid_dims(areas.terminal, cell_w, cell_h);

            let agent_name = self.tabs.items()[self.tabs.active()]
                .pane
                .agent
                .as_ref()
                .map(|agent| agent.profile_name.clone());
            let shown = reader_glue::shown(self.tabs.active_pane_mut());
            let reader_on = shown == Some(Shown::Reader);
            match &mut self.tabs.active_pane_mut().body {
                Body::Shell(term) => {
                    term.resize(cols, rows);
                    if !reader_on {
                        let typed = collect_input(ui, false, false, term.app_cursor());
                        if !typed.is_empty() && !modal_open && !typing_in_a_panel {
                            term.send(&typed);
                        }
                        let snap = term.grid_snapshot();
                        render_grid(
                            ui,
                            &[],
                            areas.terminal,
                            term,
                            &snap,
                            cell_w,
                            cell_h,
                            &font,
                            None,
                            GridStyle {
                                cursor: CursorStyle::Block,
                                dimmed: false,
                                link_active: false,
                                blink: false,
                                ligatures: false,
                                min_contrast: MIN_CONTRAST,
                                app_mouse: false,
                            },
                            &[],
                        );
                    }
                    if let (Some(name), Some(exit)) = (&agent_name, term.exited()) {
                        exit_notice(ui, areas.terminal, &exit_notice_text(name, exit.code));
                    }
                }
                Body::Creating => {
                    status_message(ui, areas.terminal, "Creating worktree...", colors::dim());
                }
                Body::Removing => {
                    status_message(ui, areas.terminal, "Removing worktree...", colors::dim());
                }
                Body::Failed(message) => status_message(ui, areas.terminal, message, colors::red()),
            }
            match shown {
                Some(Shown::Reader) => {
                    let pane = self.tabs.active_pane_mut();
                    let asked =
                        reader_glue::draw(ui, areas.terminal, pane, &mut self.dev_reader, now);
                    let toast = reader_glue::apply(
                        asked,
                        &mut self.tabs,
                        active_id,
                        &mut self.flow,
                        &ctx,
                        now,
                    );
                    if let Some(text) = toast {
                        self.show_toast(text, &ctx);
                    }
                }
                Some(Shown::Terminal(Some(note))) => reader_glue::note(ui, areas.terminal, note),
                Some(Shown::Terminal(None)) | None => {}
            }

            if let (Some((block, open)), Some(band)) = (&end, end_band)
                && end_block_view::show_card(ui, band, block, *open)
                && let Some(agent) = self.tabs.active_pane_mut().agent.as_mut()
            {
                agent.run.end_open = !agent.run.end_open;
            }
            if let (Some((commits, building)), Some(band)) = (card, band)
                && delivery_view::show_card(ui, band, commits, building).is_some()
            {
                self.flow.open_card(active_id, &ctx);
            }
            if panels {
                self.show_panes(ui, &areas, &ctx);
            }
            self.show_toast_pill(ui);
        });

        for action in actions {
            self.dispatch(action, &ctx);
        }
        if toggle_overview {
            self.toggle_overview();
        }
        match settings_action {
            Some(settings_view::Action::Close) => self.settings = None,
            Some(settings_view::Action::Save) => self.save_settings(&ctx),
            Some(settings_view::Action::Trust(event)) => self.trust.send(event, &ctx),
            Some(settings_view::Action::Restore(on)) => {
                if let Some(paths) = &self.paths {
                    self.restore.set_restore_enabled(paths, on);
                }
            }
            Some(settings_view::Action::Window(prefs)) => self.set_window_prefs(prefs, &ctx),
            None => {}
        }
        if toggle_settings {
            self.toggle_settings();
        }
        let answer = match &self.closing {
            Some(Closing::Asking { worktree, inspection, .. }) => {
                close_dialog::show(&ctx, worktree, inspection)
            }
            _ => None,
        };
        if let Some(answer) = answer {
            self.answer_close(answer, &ctx);
        }
        self.leftovers.show(&ctx, self.paths.as_ref(), self.trust.grant().as_ref());
        flow_glue::dialogs(&ctx, &mut self.flow, active_id);
        if let Some(state) = &mut self.new_tab
            && let Some(outcome) =
                new_tab_dialog::show(&ctx, state, &self.catalog, self.paths.as_ref())
        {
            self.answer_new_tab(outcome, &ctx);
        }
        if self.trust.dialog_open()
            && let Some(event) = trust_view::dialog(&ctx, &self.trust.page())
        {
            self.trust.send(event, &ctx);
        }
        if let Some(text) = &self.notice
            && notice::show(&ctx, text)
        {
            self.notice = None;
        }
        self.dev_session.tick(&ctx, &mut self.tabs, now);
        self.saver.tick(
            self.restore.settled(),
            &WindowView::of(&self.workspace, self.focus),
            &self.tabs,
        );
        if let Some(shot) = &mut self.screenshot {
            shot.tick(&ctx);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exit_notice_names_the_profile_and_the_code() {
        assert_eq!(
            exit_notice_text("Builder", 130),
            "Builder exited with code 130. Cmd+W closes the tab."
        );
    }
}
