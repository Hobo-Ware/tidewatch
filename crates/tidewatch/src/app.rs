//! The `eframe::App`: tab strip on top, the active tab's terminal below it, then the History and
//! Diff panes. Frame work collects `tabs::Action`s and applies them after drawing.
//! Git work for worktree tabs and for the panes runs on `jobs` threads; its results are applied at
//! the top of the next frame.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use egui::{CornerRadius, FontId, Frame, Key, Margin, Modifiers, RichText};
use kelp_core::refs::RefLabel;
use kelp_ui::avatars::AvatarStore;
use kelp_ui::graph::{GraphAction, Selection};
use kelp_ui::theme as kelp_theme;
use stdusk_core::progress::Progress;
use stdusk_core::shell::ShellProfile;
use stdusk_core::terminal::{CmdState, PtyTerm, SpawnOpts};
use stdusk_egui::ui::{
    CursorStyle, GridStyle, TAB_H, apply_theme, collect_input, draw_tab, draw_toast, icons,
    render_grid,
};
use stdusk_egui::widgets::{chip, icon_button};
use stdusk_egui::{colors, repaint_handle};
use tidewatch_core::notes::NotesWriter;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prepare::Prepared;
use tidewatch_core::worktree::{BranchOutcome, Inspection, Leftover, Worktree};

use crate::agent_jobs;
use crate::catalog::Catalog;
use crate::checkout::{self, Plan};
use crate::close_dialog::{self, Answer, Choice};
use crate::dev_agent::{DevAgent, Mode as DevMode};
use crate::dev_panes::{self, DevPanes};
use crate::dev_screenshot::DevScreenshot;
use crate::dev_worktree::DevWorktree;
use crate::jobs::Jobs;
use crate::new_tab::{Group, NewTab, Request};
use crate::new_tab_dialog::{self, Outcome};
use crate::pane::{
    Body, CREATING_TITLE, ClosePolicy, Pane, STARTING_TITLE, agent_tabs_in_folder, finish_open,
};
use crate::pane_jobs;
use crate::profile_editor::Editor;
use crate::repo::Workspace;
use crate::settings_view;
use crate::tab_view::{TabView, ViewSpec};
use crate::tabs::{Action, Tabs};
use crate::toast::Toast;
use crate::worktree_jobs::{self, Done, OpenSpec, Target};
use crate::{diff_pane, fonts, history_pane, layout, note_actions};

const DEFAULT_THEME: &str = "one-half-dark";
const FONT_SIZE: f32 = 13.0;
const INITIAL_COLS: usize = 80;
const INITIAL_ROWS: usize = 24;
const SCROLLBACK_LINES: usize = 25_000;
const WORD_SEPARATORS: &str = " ()[]{}'\"";
const MIN_CONTRAST: f32 = 4.0;
const STRIP_TOP_MARGIN: i8 = 6;
const STRIP_SIDE_MARGIN: i8 = 8;
const STRIP_GAP: f32 = 4.0;
const STRIP_CORNER: u8 = 10;

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
    leftovers: Vec<Leftover>,
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
    settings: Option<Editor>,
    last_profile: Option<String>,
}

impl Tidewatch {
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        workspace: Workspace,
        paths: Option<TidewatchPaths>,
        agent_path: OsString,
        screenshot: Option<DevScreenshot>,
    ) -> Self {
        colors::init(colors::by_name(DEFAULT_THEME));
        fonts::install(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);
        if screenshot.is_some() {
            cc.egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
        }
        let main = spawn_shell(&cc.egui_ctx, workspace.dir());
        let mut app = Self {
            tabs: Tabs::new(Pane::shell(main)),
            jobs: Jobs::new(),
            closing: None,
            leftovers: Vec::new(),
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
        };
        if let Some(paths) = &app.paths {
            agent_jobs::load(
                &app.agent_jobs,
                &cc.egui_ctx,
                paths.clone(),
                app.catalog.path.clone(),
            );
        }
        if let (Workspace::Repo(repo), Some(paths)) = (&app.workspace, &app.paths) {
            worktree_jobs::startup(&app.jobs, &cc.egui_ctx, paths.clone(), repo.root.clone());
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
            DevMode::Settings => {}
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
        };
        match (&self.workspace, &self.paths) {
            (Workspace::Repo(repo), Some(paths)) => {
                let tab = self.tabs.push(CREATING_TITLE.to_owned(), Pane::creating());
                let target = Target::Worktree { repo: repo.root.clone() };
                let path_var = self.catalog.path.clone();
                let spec = OpenSpec { paths: paths.clone(), target, agent, path_var, tab };
                worktree_jobs::open(&self.jobs, ctx, spec);
            }
            (workspace, Some(paths)) if agent.is_some() => {
                let tab = self.tabs.push(STARTING_TITLE.to_owned(), Pane::creating());
                let target = Target::Folder { dir: workspace.dir().to_path_buf() };
                let path_var = self.catalog.path.clone();
                let spec = OpenSpec { paths: paths.clone(), target, agent, path_var, tab };
                worktree_jobs::open(&self.jobs, ctx, spec);
            }
            _ => {
                let dir = self.workspace.dir().to_path_buf();
                self.tabs.apply(Action::New, || Pane::shell(spawn_shell(ctx, &dir)));
            }
        }
    }

    fn apply_agent_done(&mut self, done: agent_jobs::Done, ctx: &egui::Context) {
        match done {
            agent_jobs::Done::Loaded(profiles) => {
                let probes = self.catalog.set_profiles(profiles);
                self.probe_agents(probes, ctx);
                self.run_dev_agent(ctx);
            }
            agent_jobs::Done::Probed { agent, availability } => {
                self.catalog.set_availability(&agent, availability);
            }
            agent_jobs::Done::Saved { result, profiles } => match result {
                Ok(()) => {
                    let probes = self.catalog.set_profiles(profiles.clone());
                    self.probe_agents(probes, ctx);
                    if let Some(editor) = &mut self.settings {
                        editor.saved(profiles);
                    }
                    self.show_toast("Profiles saved", ctx);
                }
                Err(message) => {
                    if let Some(editor) = &mut self.settings {
                        editor.save_failed(format!("Could not save: {message}"));
                    }
                }
            },
        }
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
        if let Some(profiles) = self.catalog.profiles() {
            self.settings = Some(Editor::new(profiles, self.catalog.agent_names()));
        }
    }

    fn save_settings(&mut self, ctx: &egui::Context) {
        let (Some(editor), Some(paths)) = (&mut self.settings, &self.paths) else { return };
        match editor.build() {
            Ok(profiles) => {
                editor.saving = true;
                agent_jobs::save(&self.agent_jobs, ctx, paths.clone(), profiles);
            }
            Err(message) => editor.error = Some(message),
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
            ClosePolicy::Now => self.tabs.apply(Action::Close(index), || unreachable!()),
            ClosePolicy::Wait => {}
            ClosePolicy::Confirm(worktree) => {
                if self.closing.is_none() {
                    self.closing = Some(Closing::Inspecting(id));
                    worktree_jobs::inspect(&self.jobs, ctx, id, worktree);
                }
            }
        }
    }

    fn answer_close(&mut self, answer: Answer, ctx: &egui::Context) {
        let Some(Closing::Asking { tab, .. }) = self.closing.take() else { return };
        match answer {
            Answer::Cancel => {}
            Answer::Pick(Choice::KeepWorktree) => self.tabs.remove_id(tab),
            Answer::Pick(Choice::Remove { branch, mode }) => {
                let Some(paths) = self.paths.clone() else { return };
                let Some((shell, worktree)) =
                    self.tabs.get_mut(tab).and_then(|t| t.pane.begin_remove())
                else {
                    return;
                };
                worktree_jobs::remove(&self.jobs, ctx, paths, tab, shell, worktree, branch, mode);
            }
        }
    }

    fn apply_done(&mut self, done: Done, ctx: &egui::Context) {
        match done {
            Done::Opened { tab, result } => {
                let created = result.is_ok();
                let notes =
                    finish_open(&mut self.tabs, tab, result, |dir, prepared| match prepared {
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
            Done::Startup { repaired, leftovers } => {
                if repaired > 0 {
                    eprintln!("tidewatch: repaired {repaired} worktree(s) after the repo moved");
                }
                self.leftovers = worktree_jobs::without_open(leftovers, &self.tabs);
            }
        }
    }

    /// Keeps one view per tab that has panes: only the visible tab's view is loaded and watched.
    fn drive_views(&mut self) {
        let open: HashSet<u64> = self.tabs.items().iter().map(|t| t.id).collect();
        self.views.retain(|id, _| open.contains(id));
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
        let graph_action = history_pane::show(ui, areas.history, view, &mut self.avatars);
        let notes_changed = diff_pane::show(ui, areas.diff, view);
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

    fn dispatch(&mut self, action: Action, ctx: &egui::Context) {
        self.settings = None;
        match action {
            Action::New => self.new_tab(ctx),
            Action::CloseActive => self.close_tab(self.tabs.active(), ctx),
            Action::Close(index) => self.close_tab(index, ctx),
            Action::Select(_) => self.tabs.apply(action, || unreachable!()),
        }
    }
}

fn spawn_shell(ctx: &egui::Context, dir: &Path) -> PtyTerm {
    PtyTerm::spawn(INITIAL_COLS, INITIAL_ROWS, repaint_handle(ctx), &spawn_opts(dir, None))
}

/// The user's shell runs `prepared.script` as its command, so it reads the login profile like any
/// tab and the agent ends up as the pty's process.
fn spawn_agent(ctx: &egui::Context, dir: &Path, prepared: &Prepared) -> PtyTerm {
    let shell = user_shell(std::env::var("SHELL").ok());
    let profile = ShellProfile {
        shell: Some(shell.clone()),
        args: vec!["-c".to_owned(), prepared.script(&shell)],
        cwd: None,
        env: prepared.env.iter().cloned().collect::<BTreeMap<_, _>>(),
    };
    PtyTerm::spawn(INITIAL_COLS, INITIAL_ROWS, repaint_handle(ctx), &spawn_opts(dir, Some(profile)))
}

fn spawn_opts(dir: &Path, profile: Option<ShellProfile>) -> SpawnOpts {
    SpawnOpts {
        detect_progress: true,
        shell_integration: true,
        autosuggestions: false,
        scrollback_lines: SCROLLBACK_LINES,
        word_separators: WORD_SEPARATORS.into(),
        bold_bright: true,
        cwd: Some(dir.to_string_lossy().into_owned()),
        profile,
        agent_tracking: None,
    }
}

/// `$SHELL`, or the platform's default login shell when it is unset or empty.
fn user_shell(env: Option<String>) -> String {
    env.filter(|shell| !shell.is_empty()).unwrap_or_else(|| {
        if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" }.to_owned()
    })
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

fn leftover_tip(leftovers: &[Leftover]) -> String {
    let mut lines = vec!["Worktrees from earlier runs with no open tab:".to_owned()];
    lines.extend(leftovers.iter().map(|l| match &l.branch {
        Some(branch) => format!("{}  ({branch})", l.path.display()),
        None => l.path.display().to_string(),
    }));
    lines.join("\n")
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
        self.drive_views();
        let typing_in_a_panel = ctx.text_edit_focused();
        let modal_open =
            self.closing.is_some() || self.new_tab.is_some() || self.settings.is_some();
        if self.closing.is_none()
            && self.new_tab.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Comma))
        {
            self.toggle_settings();
        }
        let mut actions = if modal_open { Vec::new() } else { shortcut_actions(&ctx) };
        let mut toggle_settings = false;
        let mut settings_action = None;

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
                    if !self.leftovers.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("{} leftover", self.leftovers.len()))
                                .monospace()
                                .size(12.0)
                                .color(colors::dim()),
                        )
                        .on_hover_text(leftover_tip(&self.leftovers));
                        ui.add_space(8.0);
                    }
                    ui.add_space(8.0);
                    let width = layout::tab_width(
                        ui.available_width() - stdusk_egui::widgets::ICON_BTN_W * 2.0 - 18.0,
                        self.tabs.items().len(),
                        STRIP_GAP,
                    );
                    for (i, tab) in self.tabs.items().iter().enumerate() {
                        let (progress, cmd) = tab_signals(&tab.pane);
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
                            None,
                            Some(width),
                        );
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
            if let Some(editor) = &mut self.settings {
                let file = self
                    .paths
                    .as_ref()
                    .map_or_else(String::new, |p| p.profiles_file().display().to_string());
                settings_action = settings_view::show(ui, area, editor, &file);
                return;
            }
            let panels = self.workspace_has_panels();
            let mut areas = layout::split(area);
            if !panels {
                areas.terminal = layout::terminal_only(area);
            }
            let font = egui::FontId::monospace(FONT_SIZE);
            let glyph = ui.painter().layout_no_wrap("M".to_owned(), font.clone(), colors::fg());
            let (cell_w, cell_h) = (glyph.size().x, glyph.size().y);
            let (cols, rows) = layout::grid_dims(areas.terminal, cell_w, cell_h);

            let agent_name = self.tabs.items()[self.tabs.active()]
                .pane
                .agent
                .as_ref()
                .map(|agent| agent.profile_name.clone());
            match &mut self.tabs.active_pane_mut().body {
                Body::Shell(term) => {
                    term.resize(cols, rows);
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

            if panels {
                self.show_panes(ui, &areas, &ctx);
            }
            self.show_toast_pill(ui);
        });

        for action in actions {
            self.dispatch(action, &ctx);
        }
        match settings_action {
            Some(settings_view::Action::Close) => self.settings = None,
            Some(settings_view::Action::Save) => self.save_settings(&ctx),
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
        if let Some(state) = &mut self.new_tab
            && let Some(outcome) =
                new_tab_dialog::show(&ctx, state, &self.catalog, self.paths.as_ref())
        {
            self.answer_new_tab(outcome, &ctx);
        }
        if let Some(shot) = &mut self.screenshot {
            shot.tick(&ctx);
        }
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
    fn the_shell_comes_from_the_environment_or_the_platform_default() {
        assert_eq!(user_shell(Some("/usr/bin/fish".into())), "/usr/bin/fish");
        for unset in [None, Some(String::new())] {
            let shell = user_shell(unset);
            assert!(shell == "/bin/zsh" || shell == "/bin/bash", "{shell}");
        }
    }

    #[test]
    fn the_exit_notice_names_the_profile_and_the_code() {
        assert_eq!(
            exit_notice_text("Builder", 130),
            "Builder exited with code 130. Cmd+W closes the tab."
        );
    }
}
