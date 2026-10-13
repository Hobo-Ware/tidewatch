//! Everything that happens between an agent tab and the outside after the agent wrote code: notes
//! sent to the agent, the delivery card, the pull request link and the confirm before posting.
//! `Flow` keeps one `TabFlow` per agent worktree tab and turns what a frame sees (`TabFacts`) into
//! background jobs; finished jobs come back as `Out` values for the app to apply. It never touches a
//! terminal, a view or a thread directly, so the decisions are unit tests.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::notes::Notes;
use tidewatch_core::pr::MergeReport;
use tidewatch_core::status::AgentStatus;
use tidewatch_core::tools::Tools;

use crate::delivery_card::Delivery;
use crate::flow_jobs::{self, Done, Which};
use crate::jobs::Jobs;
use crate::note_send::{AutoDelivery, EnterQueue, Gate, gate, gate_reason};
use crate::notes_strip::{self, NoteRow};
use crate::post_confirm::{Phase, Plan, PostConfirm};
use crate::pr_state::{Chip, PrState, Why, repo_of};

/// What one frame sees of an agent worktree tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabFacts {
    pub(crate) id: u64,
    pub(crate) active: bool,
    pub(crate) status: AgentStatus,
    pub(crate) agent_id: String,
    pub(crate) dir: PathBuf,
    pub(crate) branch: String,
    pub(crate) notes_file: Option<PathBuf>,
    pub(crate) pending: usize,
    pub(crate) stage_summary: Option<String>,
}

/// What the app applies after a job finished.
#[derive(Debug)]
pub(crate) enum Out {
    Paste { tab: u64, text: String },
    Notes { tab: u64, notes: Box<Notes> },
    Toast(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Want {
    Probe,
    AutoSend,
    FindPr(Why),
    LoadPrLines,
}

#[derive(Default)]
pub(crate) struct TabFlow {
    pub(crate) auto: AutoDelivery,
    pub(crate) delivery: Delivery,
    pub(crate) pr: PrState,
    target: Option<TabFacts>,
    was_done: bool,
    was_active: bool,
    lines_loading: bool,
}

/// What to start for one tab this frame. Marks the state of everything it asks for as started, so
/// calling it again before the answers arrive asks for nothing twice.
pub(crate) fn decide(flow: &mut TabFlow, facts: &TabFacts, now: f64) -> Vec<Want> {
    let mut wants = Vec::new();
    flow.auto.observe(&facts.status);
    let done = facts.status == AgentStatus::Done;
    if done && !flow.was_done && flow.pr.pr().is_none() {
        flow.delivery.probe_started();
        wants.push(Want::Probe);
    }
    if !done {
        flow.delivery.leave_done();
    }
    flow.was_done = done;
    if facts.active && !flow.was_active && flow.pr.should_find(Why::Focus, now) {
        flow.pr.find_started();
        wants.push(Want::FindPr(Why::Focus));
    }
    flow.was_active = facts.active;
    if flow.pr.needs_lines() && !flow.lines_loading {
        flow.lines_loading = true;
        wants.push(Want::LoadPrLines);
    }
    if facts.notes_file.is_some() && flow.auto.should_send(&facts.status, facts.pending, now) {
        flow.auto.begin();
        wants.push(Want::AutoSend);
    }
    wants
}

pub(crate) struct Flow {
    jobs: Jobs<Done>,
    tabs: HashMap<u64, TabFlow>,
    enters: EnterQueue,
    tools: Tools,
    pub(crate) confirm: Option<PostConfirm>,
}

/// What the Diff pane draws for the active tab beyond the diff itself.
pub(crate) struct PaneExtras {
    pub(crate) rows: Vec<NoteRow>,
    pub(crate) pending: usize,
    pub(crate) auto: bool,
    pub(crate) send_all: Result<(), &'static str>,
    pub(crate) chip: Option<Chip>,
    pub(crate) pulling: bool,
}

impl Flow {
    pub(crate) fn new(tools: Tools) -> Self {
        Self {
            jobs: Jobs::new(),
            tabs: HashMap::new(),
            enters: EnterQueue::default(),
            tools,
            confirm: None,
        }
    }

    pub(crate) fn tab(&self, id: u64) -> Option<&TabFlow> {
        self.tabs.get(&id)
    }

    pub(crate) fn tab_mut(&mut self, id: u64) -> Option<&mut TabFlow> {
        self.tabs.get_mut(&id)
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.tabs.remove(&tab);
        self.enters.forget(tab);
        if self.confirm.as_ref().is_some_and(|c| c.tab == tab) {
            self.confirm = None;
        }
    }

    /// A dialog of the flow is open over the window.
    pub(crate) fn modal_open(&self, active: u64) -> bool {
        self.confirm.is_some() || self.tabs.get(&active).is_some_and(|t| t.delivery.is_modal())
    }

    /// Looks at every agent worktree tab and starts what each needs. Returns the next time a frame is
    /// needed with no other news (a hold ending, an Enter coming due).
    pub(crate) fn tick(
        &mut self,
        facts: &[TabFacts],
        ctx: &egui::Context,
        now: f64,
        unix: i64,
    ) -> Option<f64> {
        let open: Vec<u64> = facts.iter().map(|f| f.id).collect();
        self.tabs.retain(|id, _| open.contains(id));
        let mut next: Option<f64> = self.enters.next_due();
        for fact in facts {
            let state = self.tabs.entry(fact.id).or_default();
            state.target = Some(fact.clone());
            let wants = decide(state, fact, now);
            if let Some(at) = state.auto.next_check(now) {
                next = Some(next.map_or(at, |soonest| soonest.min(at)));
            }
            for want in wants {
                self.start(fact, want, ctx, unix);
            }
        }
        next
    }

    fn start(&self, facts: &TabFacts, want: Want, ctx: &egui::Context, unix: i64) {
        let tab = facts.id;
        let tools = self.tools.clone();
        let dir = facts.dir.clone();
        let branch = facts.branch.clone();
        match want {
            Want::Probe => flow_jobs::probe(
                &self.jobs,
                ctx,
                tab,
                tools,
                dir,
                branch,
                facts.stage_summary.clone(),
            ),
            Want::FindPr(_) => flow_jobs::find_pr(&self.jobs, ctx, tab, tools, dir, branch),
            Want::LoadPrLines => {
                let head = self
                    .tabs
                    .get(&tab)
                    .and_then(|t| t.pr.pr().map(|pr| pr.head_sha.clone()))
                    .unwrap_or_default();
                flow_jobs::load_pr_lines(&self.jobs, ctx, tab, tools, dir, head);
            }
            Want::AutoSend => {
                if let Some(file) = facts.notes_file.clone() {
                    flow_jobs::send_notes(
                        &self.jobs,
                        ctx,
                        tab,
                        true,
                        file,
                        Which::AllPending,
                        facts.agent_id.clone(),
                        unix,
                    );
                }
            }
        }
    }

    /// The Send buttons. `which` is one note or every pending one. Returns a toast when the send
    /// cannot go.
    pub(crate) fn send(
        &mut self,
        tab: u64,
        which: Which,
        ctx: &egui::Context,
        unix: i64,
    ) -> Option<String> {
        let state = self.tabs.get(&tab)?;
        let facts = state.target.clone()?;
        if let Some(reason) = gate_reason(gate(Some(&facts.status))) {
            return Some(reason.to_owned());
        }
        let file = facts.notes_file.clone()?;
        flow_jobs::send_notes(&self.jobs, ctx, tab, false, file, which, facts.agent_id, unix);
        None
    }

    pub(crate) fn toggle_auto(&mut self, tab: u64) {
        if let Some(state) = self.tabs.get_mut(&tab) {
            state.auto.enabled = !state.auto.enabled;
        }
    }

    pub(crate) fn open_card(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(state) = self.tabs.get_mut(&tab) else { return };
        let Some(facts) = state.target.clone() else { return };
        if state.delivery.open() {
            flow_jobs::build_plan(
                &self.jobs,
                ctx,
                tab,
                self.tools.clone(),
                facts.dir,
                facts.branch,
                facts.stage_summary,
            );
        }
    }

    pub(crate) fn approve_card(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(state) = self.tabs.get_mut(&tab) else { return };
        if let Some(form) = state.delivery.approve() {
            flow_jobs::deliver_plan(&self.jobs, ctx, tab, self.tools.clone(), form);
        }
    }

    pub(crate) fn pull_comments(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(state) = self.tabs.get_mut(&tab) else { return };
        let (Some(facts), Some(pr)) = (state.target.clone(), state.pr.pr().cloned()) else {
            return;
        };
        let Some(file) = facts.notes_file else { return };
        if state.pr.pulling {
            return;
        }
        state.pr.pulling = true;
        flow_jobs::pull_comments(&self.jobs, ctx, tab, self.tools.clone(), facts.dir, file, pr);
    }

    /// Opens the confirm dialog for posting `thread`. Nothing is sent yet.
    pub(crate) fn ask_post(&mut self, tab: u64, plan: Option<Plan>) {
        let Some(pr) = self.tabs.get(&tab).and_then(|t| t.pr.pr().cloned()) else { return };
        let Some(plan) = plan else { return };
        let repo = repo_of(&pr.url).unwrap_or_default();
        self.confirm = Some(PostConfirm { tab, pr, repo, plan, phase: Phase::Asking });
    }

    pub(crate) fn approve_post(&mut self, ctx: &egui::Context) {
        let Some(confirm) = &mut self.confirm else { return };
        if confirm.phase != Phase::Asking {
            return;
        }
        confirm.phase = Phase::Posting;
        let tab = confirm.tab;
        let Some(facts) = self.tabs.get(&tab).and_then(|t| t.target.clone()) else {
            self.confirm = None;
            return;
        };
        let Some(file) = facts.notes_file else {
            self.confirm = None;
            return;
        };
        flow_jobs::post(
            &self.jobs,
            ctx,
            tab,
            self.tools.clone(),
            facts.dir,
            file,
            confirm.pr.clone(),
            confirm.plan.clone(),
        );
    }

    /// Esc, Cancel or Close on the confirm dialog. A post already on its way cannot be recalled.
    pub(crate) fn cancel_post(&mut self) {
        if self.confirm.as_ref().is_some_and(|c| c.phase != Phase::Posting) {
            self.confirm = None;
        }
    }

    pub(crate) fn extras(
        &self,
        tab: u64,
        notes: &Notes,
        status: &AgentStatus,
    ) -> Option<PaneExtras> {
        let state = self.tabs.get(&tab)?;
        let agent = state.target.as_ref()?.agent_id.as_str();
        let gate: Gate = gate(Some(status));
        let rows = notes_strip::rows(notes, agent, gate, &state.pr);
        let pending = notes.pending(agent).len();
        Some(PaneExtras {
            rows,
            pending,
            auto: state.auto.enabled,
            send_all: gate_reason(gate).map_or(Ok(()), Err),
            chip: state.pr.chip(),
            pulling: state.pr.pulling,
        })
    }

    /// Applies finished jobs to the flow's state and says what the app must do about them.
    pub(crate) fn drain(&mut self, ctx: &egui::Context, now: f64) -> Vec<Out> {
        let mut outs = Vec::new();
        for done in self.jobs.drain() {
            self.apply(done, ctx, now, &mut outs);
        }
        outs
    }

    /// Presses Enter in the tab's terminal once the paste before it has landed.
    pub(crate) fn queue_enter(&mut self, tab: u64, now: f64) {
        self.enters.push(tab, now);
    }

    pub(crate) fn due_enters(&mut self, now: f64) -> Vec<u64> {
        self.enters.take_due(now)
    }

    fn apply(&mut self, done: Done, ctx: &egui::Context, now: f64, outs: &mut Vec<Out>) {
        match done {
            Done::Sent { tab, auto, result } => self.sent(tab, auto, result, now, outs),
            Done::Probed { tab, commits } => {
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.delivery.probe_done(commits);
                }
            }
            Done::Built { tab, result } => {
                if let Some(state) = self.tabs.get_mut(&tab) {
                    if let Err(error) = &result {
                        outs.push(Out::Toast(format!("Cannot read the commits: {error}")));
                    }
                    state.delivery.built(result.map(|plan| *plan));
                }
            }
            Done::Delivered { tab, result } => {
                let ok = result.is_ok();
                let mut find = false;
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.delivery.delivered(result);
                    find = ok && state.pr.should_find(Why::Delivered, now);
                    if find {
                        state.pr.find_started();
                    }
                }
                if find && let Some(facts) = self.tabs.get(&tab).and_then(|t| t.target.clone()) {
                    flow_jobs::find_pr(
                        &self.jobs,
                        ctx,
                        tab,
                        self.tools.clone(),
                        facts.dir,
                        facts.branch,
                    );
                }
            }
            Done::PrFound { tab, result } => {
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.pr.found(result, now);
                    state.lines_loading = false;
                }
            }
            Done::PrLines { tab, lines } => {
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.pr.lines_loaded(lines.map(|boxed| *boxed));
                    state.lines_loading = false;
                }
            }
            Done::Pulled { tab, result } => {
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.pr.pulling = false;
                }
                match result {
                    Ok((notes, report)) => {
                        outs.push(Out::Notes { tab, notes });
                        outs.push(Out::Toast(pulled_text(report)));
                    }
                    Err(error) => {
                        outs.push(Out::Toast(format!("Could not pull PR comments: {error}")));
                    }
                }
            }
            Done::Posted { tab, result } => match result {
                Ok((posted, notes)) => {
                    self.confirm = None;
                    outs.push(Out::Notes { tab, notes });
                    outs.push(Out::Toast(format!("Posted to the pull request: {}", posted.url)));
                }
                Err(error) => {
                    if let Some(confirm) = &mut self.confirm {
                        confirm.phase = Phase::Failed(error.to_string());
                    }
                }
            },
        }
    }

    fn sent(
        &mut self,
        tab: u64,
        auto: bool,
        result: Result<flow_jobs::SentOut, String>,
        now: f64,
        outs: &mut Vec<Out>,
    ) {
        let Some(state) = self.tabs.get_mut(&tab) else { return };
        match result {
            Ok(out) => {
                let sent = out.text.is_some();
                if auto {
                    state.auto.finished(sent, now);
                } else if sent {
                    state.auto.sent_by_hand(now);
                }
                outs.push(Out::Notes { tab, notes: out.notes });
                match out.text {
                    Some(text) => {
                        self.enters.push(tab, now);
                        outs.push(Out::Paste { tab, text });
                        outs.push(Out::Toast(sent_text(out.count, auto)));
                    }
                    None if !auto => outs.push(Out::Toast("Nothing new to send".to_owned())),
                    None => {}
                }
            }
            Err(message) => {
                if auto {
                    state.auto.finished(false, now);
                }
                outs.push(Out::Toast(format!("Could not send notes: {message}")));
            }
        }
    }
}

pub(crate) fn sent_text(count: usize, auto: bool) -> String {
    let noun = if count == 1 { "note" } else { "notes" };
    if auto {
        format!("Agent is idle: sent {count} queued {noun}")
    } else {
        format!("Sent {count} {noun} to the agent")
    }
}

pub(crate) fn pulled_text(report: MergeReport) -> String {
    match (report.new_threads, report.new_comments) {
        (0, 0) => "No new PR comments".to_owned(),
        (threads, comments) => {
            let t = if threads == 1 { "thread" } else { "threads" };
            let c = if comments == 1 { "comment" } else { "comments" };
            format!("Pulled {comments} PR {c} ({threads} new {t})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidewatch_core::pr::PullRequest;
    use tidewatch_core::status::Reason;

    fn facts(status: AgentStatus) -> TabFacts {
        TabFacts {
            id: 7,
            active: false,
            status,
            agent_id: "claude".into(),
            dir: PathBuf::from("/w"),
            branch: "agent/brave-otter".into(),
            notes_file: Some(PathBuf::from("/s/notes.json")),
            pending: 0,
            stage_summary: None,
        }
    }

    fn flow_with(auto: bool) -> TabFlow {
        let mut flow = TabFlow::default();
        flow.auto.enabled = auto;
        flow
    }

    #[test]
    fn a_tab_that_turns_done_is_probed_once_per_idle_moment() {
        let mut flow = flow_with(false);
        assert_eq!(decide(&mut flow, &facts(AgentStatus::Working), 0.0), []);
        assert_eq!(decide(&mut flow, &facts(AgentStatus::Done), 1.0), [Want::Probe]);
        assert_eq!(decide(&mut flow, &facts(AgentStatus::Done), 2.0), []);
        decide(&mut flow, &facts(AgentStatus::Working), 3.0);
        assert_eq!(decide(&mut flow, &facts(AgentStatus::Done), 4.0), [Want::Probe]);
    }

    #[test]
    fn a_tab_with_an_open_pull_request_is_not_probed_for_delivery() {
        let mut flow = flow_with(false);
        flow.pr
            .found(Ok(Some(PullRequest { number: 1, url: "u".into(), head_sha: "h".into() })), 0.0);
        flow.lines_loading = true;
        assert_eq!(decide(&mut flow, &facts(AgentStatus::Done), 1.0), []);
    }

    #[test]
    fn the_card_goes_away_while_the_agent_works_again() {
        let mut flow = flow_with(false);
        decide(&mut flow, &facts(AgentStatus::Done), 0.0);
        flow.delivery.probe_done(Some(2));
        assert_eq!(flow.delivery, Delivery::Ready { commits: 2 });
        decide(&mut flow, &facts(AgentStatus::Working), 1.0);
        assert_eq!(flow.delivery, Delivery::Hidden);
    }

    #[test]
    fn the_pull_request_is_looked_up_when_the_tab_gets_focus_not_every_frame() {
        let mut flow = flow_with(false);
        let mut active = facts(AgentStatus::Working);
        assert_eq!(decide(&mut flow, &active, 0.0), []);
        active.active = true;
        assert_eq!(decide(&mut flow, &active, 1.0), [Want::FindPr(Why::Focus)]);
        assert_eq!(decide(&mut flow, &active, 2.0), []);
        flow.pr.found(Ok(None), 3.0);
        assert_eq!(decide(&mut flow, &active, 4.0), []);
        active.active = false;
        decide(&mut flow, &active, 5.0);
        active.active = true;
        assert_eq!(decide(&mut flow, &active, 6.0), []);
        active.active = false;
        decide(&mut flow, &active, 7.0);
        active.active = true;
        assert_eq!(
            decide(&mut flow, &active, 3.0 + crate::pr_state::REFRESH_SECS + 1.0),
            [Want::FindPr(Why::Focus)]
        );
    }

    #[test]
    fn the_diff_lines_load_once_a_pull_request_is_known() {
        let mut flow = flow_with(false);
        flow.pr
            .found(Ok(Some(PullRequest { number: 1, url: "u".into(), head_sha: "h".into() })), 0.0);
        assert_eq!(decide(&mut flow, &facts(AgentStatus::Working), 1.0), [Want::LoadPrLines]);
        assert_eq!(decide(&mut flow, &facts(AgentStatus::Working), 2.0), []);
    }

    #[test]
    fn deliver_when_idle_sends_queued_notes_when_done_and_never_while_working_or_waiting() {
        for (status, sends) in [
            (AgentStatus::Working, false),
            (AgentStatus::NeedsYou(Reason::Permission), false),
            (AgentStatus::NeedsYou(Reason::Quiet), false),
            (AgentStatus::Done, true),
        ] {
            let mut flow = flow_with(true);
            flow.was_done = true;
            let mut f = facts(status.clone());
            f.pending = 2;
            let wants = decide(&mut flow, &f, 0.0);
            assert_eq!(wants.contains(&Want::AutoSend), sends, "{status:?}");
        }
    }

    #[test]
    fn the_queue_goes_out_when_the_status_changes_to_done_and_not_again_for_the_same_idle_moment() {
        let mut flow = flow_with(true);
        let mut f = facts(AgentStatus::Working);
        f.pending = 1;
        assert_eq!(decide(&mut flow, &f, 0.0), []);
        f.status = AgentStatus::Done;
        assert!(decide(&mut flow, &f, 1.0).contains(&Want::AutoSend));
        assert!(!decide(&mut flow, &f, 1.1).contains(&Want::AutoSend));
        flow.auto.finished(true, 1.5);
        f.pending = 1;
        assert!(!decide(&mut flow, &f, 2.0).contains(&Want::AutoSend));
        f.status = AgentStatus::Working;
        decide(&mut flow, &f, 3.0);
        f.status = AgentStatus::Done;
        assert!(decide(&mut flow, &f, 4.0).contains(&Want::AutoSend));
    }

    #[test]
    fn a_tab_without_a_notes_file_or_with_the_switch_off_never_auto_sends() {
        let mut flow = flow_with(true);
        let mut f = facts(AgentStatus::Done);
        f.pending = 1;
        f.notes_file = None;
        assert!(!decide(&mut flow, &f, 0.0).contains(&Want::AutoSend));
        let mut off = flow_with(false);
        f.notes_file = Some(PathBuf::from("/s/notes.json"));
        assert!(!decide(&mut off, &f, 0.0).contains(&Want::AutoSend));
    }

    #[test]
    fn the_toasts_count_notes_and_comments() {
        assert_eq!(sent_text(1, false), "Sent 1 note to the agent");
        assert_eq!(sent_text(3, true), "Agent is idle: sent 3 queued notes");
        assert_eq!(pulled_text(MergeReport::default()), "No new PR comments");
        assert_eq!(
            pulled_text(MergeReport { new_threads: 2, new_comments: 3 }),
            "Pulled 3 PR comments (2 new threads)"
        );
    }
}
