//! Taking over sessions that run elsewhere, the window's side: scanning while the Overview shows
//! (or while a watch waits for idle), the card and the confirm dialog, starting a move on a job and
//! opening the tab through the restore path when it lands. A session in stdusk only gets a hint.

use std::path::Path;
use std::time::Duration;

use tidewatch_core::agents::AgentTable;
use tidewatch_core::prepare::Prepared;
use tidewatch_core::profiles::Profiles;

use super::{Tidewatch, WAKE_SLACK_S};
use crate::catalog::Availability;
use crate::pty::{spawn_agent, spawn_shell_noted, spawn_typed};
use crate::restore_glue::Env as RunEnv;
use crate::restore_glue::Spawn;
use crate::takeover::{
    CardAction, CardRows, END_WAIT_S, Effect, Env, MoveSpec, Scanned, Takeover, moved_text,
    window_plan,
};
use crate::takeover_group::{Ident, group_roots};
use crate::takeover_jobs::{self, Done, Landed, MoveJob, ScanSpec};
use crate::takeover_view::{self, Card, DialogOutcome};

const STDUSK_HINT: &str = "That session runs in stdusk. Use stdusk's Move tabs to Tidewatch: it moves the live terminal, nothing restarts.";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Tidewatch {
    pub(super) fn agent_table(&self) -> AgentTable {
        self.catalog.profiles().map_or_else(AgentTable::builtin, Profiles::agent_table)
    }

    fn with_takeover<R>(&mut self, f: impl FnOnce(&mut Takeover, &Env) -> R) -> R {
        let agents = self.agent_table();
        let catalog = &self.catalog;
        let installed = |id: &str| matches!(catalog.availability(id), Availability::Found(_));
        let home = self.paths.as_ref().and_then(|paths| paths.root().parent());
        let env = Env {
            agents: &agents,
            profiles: catalog.profiles(),
            installed: &installed,
            now_ms: now_ms(),
            home,
        };
        f(&mut self.takeover, &env)
    }

    fn takeover_shown(&self) -> bool {
        (self.overview && self.settings.is_none() || self.welcome.is_some())
            && !self.shell.window_hidden()
    }

    pub(super) fn drive_takeover(&mut self, ctx: &egui::Context, now: f64) {
        for done in self.takeover_jobs.drain() {
            self.apply_takeover_done(done, ctx);
        }
        self.drive_dev_takeover(ctx);
        self.note_open_sessions();
        if self.paths.is_none() || !self.catalog.is_loaded() {
            return;
        }
        let shown = self.takeover_shown();
        if self.takeover.begin_scan(shown, now) {
            self.start_takeover_scan(ctx);
        }
        if let Some(after) = self.takeover.next_scan_in(shown, now) {
            ctx.request_repaint_after(Duration::from_secs_f64(after + WAKE_SLACK_S));
        }
    }

    fn note_open_sessions(&mut self) {
        for agent in self.tabs.items().iter().filter_map(|tab| tab.pane.agent.as_ref()) {
            let id = agent.launched.as_ref().and_then(|launched| launched.session_id.as_deref());
            self.takeover.note_open(id, agent.reader.transcript());
        }
    }

    fn start_takeover_scan(&mut self, ctx: &egui::Context) {
        let folders = if self.welcome.is_some() {
            Vec::new()
        } else {
            self.workspaces.iter().map(|workspace| workspace.dir().to_path_buf()).collect()
        };
        let spec = ScanSpec {
            agents: self.agent_table(),
            folders,
            held: self.takeover.held().clone(),
            titles: self.takeover.titles().clone(),
            roots: self.takeover.known_roots(),
            idents: self.takeover.known_idents(),
        };
        takeover_jobs::scan(&self.takeover_jobs, ctx, spec);
    }

    fn apply_takeover_done(&mut self, done: Done, ctx: &egui::Context) {
        match done {
            Done::Scanned(scanned) => self.scanned(*scanned, ctx),
            Done::ScanFailed => self.takeover.scan_failed(),
            Done::Moved(landed) => self.place_taken(*landed, ctx),
            Done::Failed(failure) => {
                if let Some(key) = &failure.key {
                    self.takeover.moved(key);
                }
                self.notice = Some(format!("{} was not moved: {}.", failure.name, failure.why));
            }
        }
    }

    fn scanned(&mut self, scanned: Scanned, ctx: &egui::Context) {
        let ready = self.takeover.finish_scan(scanned);
        self.start_takeover_moves(ready, ctx);
    }

    fn start_takeover_moves(&mut self, specs: Vec<MoveSpec>, ctx: &egui::Context) {
        let open: Vec<Ident> = if self.welcome.is_some() {
            Vec::new()
        } else {
            self.workspaces
                .iter()
                .filter_map(|workspace| self.takeover.ident_of(workspace.dir()).cloned())
                .collect()
        };
        let movers: Vec<Option<Ident>> = specs
            .iter()
            .map(|spec| {
                let own_group = !spec.worktree;
                own_group.then(|| self.takeover.ident_of(&spec.session.cwd).cloned()).flatten()
            })
            .collect();
        let groups = group_roots(&open, &movers);
        for (spec, group) in specs.into_iter().zip(groups) {
            let root = self.takeover.root_of(&spec.session).map(Path::to_path_buf);
            let wait = Duration::from_secs(END_WAIT_S);
            let agents = self.agent_table();
            let job = MoveJob { spec, agents, paths: self.paths.clone(), root, group, wait };
            takeover_jobs::start_move(&self.takeover_jobs, ctx, job);
        }
    }

    fn place_taken(&mut self, landed: Landed, ctx: &egui::Context) {
        let Landed { spec, workspace, worktree } = landed;
        if let Some(key) = spec.session.key() {
            self.takeover.moved(&key);
        }
        self.takeover.note_open(spec.session.session_id.as_deref(), None);
        let dir = workspace.dir().to_path_buf();
        let other = matches!(workspace, crate::repo::Workspace::Other { .. });
        let plan = window_plan(
            &self.agent_table(),
            self.catalog.profiles(),
            &spec,
            (&dir, other),
            worktree,
        );
        let Some(plan) = plan else {
            self.notice = Some(
                "The session was ended but its CLI is not configured, so no tab opened.".to_owned(),
            );
            return;
        };
        let group = match self.group_of_dir(&dir) {
            Some(group) => group,
            None => self.add_workspace(workspace, ctx),
        };
        let now = ctx.input(|i| i.time);
        self.apply_taken_plan(&plan, group, ctx, now);
        self.show_toast(moved_text(&spec.session, now_ms()), ctx);
    }

    pub(super) fn apply_taken_plan(
        &mut self,
        plan: &tidewatch_core::restore::WindowPlan,
        group: usize,
        ctx: &egui::Context,
        now: f64,
    ) {
        let Some(paths) = &self.paths else { return };
        let env = RunEnv {
            paths,
            catalog: &self.catalog,
            listener: self.hooks.listener(),
            trust: self.trust.grant(),
        };
        let agent = |dir: &Path, prepared: &Prepared| spawn_agent(ctx, dir, prepared);
        let typed = |dir: &Path, prepared: &Prepared| spawn_typed(ctx, dir, prepared);
        let shell = |dir: &Path, note: Option<&str>| spawn_shell_noted(ctx, dir, note);
        let spawn = Spawn { agent: &agent, typed: &typed, shell: &shell };
        let report = self.restore.apply(plan, group, ctx, &mut self.tabs, &env, &spawn, now);
        for text in report.toasts {
            self.show_toast(text, ctx);
        }
        self.notice = report.notice.or(self.notice.take());
    }

    pub(super) fn takeover_card(&mut self) -> Option<Card> {
        self.with_takeover(|takeover, env| {
            let CardRows { running: rows, ended } = takeover.card_rows(env)?;
            let (running, idle) = takeover.counts();
            let noun = if running == 1 { "session" } else { "sessions" };
            Some(Card {
                summary: format!("{running} {noun} \u{b7} {idle} idle"),
                can_move_all: !takeover.idle_movable(env).is_empty(),
                rows,
                ended,
                ended_open: takeover.recent_open(),
            })
        })
    }

    pub(super) fn takeover_click(&mut self, action: CardAction, ctx: &egui::Context) {
        match action {
            CardAction::Press(key) => {
                if self.with_takeover(|takeover, env| takeover.press(env, &key))
                    == Effect::StdduskHint
                {
                    self.notice = Some(STDUSK_HINT.to_owned());
                }
            }
            CardAction::MoveAllIdle => self.open_batch(ctx),
            CardAction::ToggleRecent => self.takeover.toggle_recent(),
            CardAction::ToggleLine => self.takeover.toggle_line(),
        }
    }

    pub(super) fn open_batch(&mut self, ctx: &egui::Context) {
        if !self.with_takeover(Takeover::open_batch) {
            self.show_toast("No idle session can be moved", ctx);
        }
    }

    /// The palette command: the Overview with the elsewhere line opened and scrolled to, or the dialog for every idle session from the
    /// welcome screen, which has no Overview.
    pub(super) fn run_move_sessions(&mut self, ctx: &egui::Context) {
        if self.welcome.is_some() {
            self.open_batch(ctx);
            return;
        }
        self.settings = None;
        self.overview = true;
        self.takeover.ask_to_reveal();
        ctx.request_repaint();
    }

    pub(super) fn show_takeover_dialog(&mut self, ctx: &egui::Context) {
        if !self.takeover.dialog_open() {
            return;
        }
        let Some(view) = self.with_takeover(|takeover, env| takeover.dialog_view(env)) else {
            return;
        };
        let mut worktree = view.worktree;
        let outcome = takeover_view::dialog(ctx, &view, &mut worktree);
        if let Some(confirm) = self.takeover.confirm_mut() {
            confirm.worktree = worktree;
        }
        match outcome {
            Some(DialogOutcome::Cancel) => self.takeover.cancel(),
            Some(DialogOutcome::Accept) => {
                let accepted = self.takeover.accept();
                self.start_takeover_moves(accepted, ctx);
                ctx.request_repaint();
            }
            None => {}
        }
    }

    /// `TIDEWATCH_DEV_TAKEOVER`: opens the dialog for the running session with that pid once a scan
    /// has it, and presses Move unless the switch only asked for the dialog.
    fn drive_dev_takeover(&mut self, ctx: &egui::Context) {
        let Some(dev) = self.dev_takeover else { return };
        let key = self
            .takeover
            .sessions()
            .iter()
            .find(|session| session.running().is_some_and(|running| running.pid == dev.pid))
            .and_then(tidewatch_core::elsewhere::Session::key);
        let Some(key) = key else { return };
        self.with_takeover(|takeover, env| takeover.press(env, &key));
        if !self.takeover.dialog_open() {
            return;
        }
        self.dev_takeover = None;
        if let Some(confirm) = self.takeover.confirm_mut() {
            confirm.worktree = dev.worktree;
        }
        if dev.press {
            let accepted = self.takeover.accept();
            self.start_takeover_moves(accepted, ctx);
        }
    }

    pub(super) fn takeover_welcome_line(&self) -> Option<(usize, usize)> {
        let (running, idle) = self.takeover.counts();
        (running > 0).then_some((running, idle))
    }
}
