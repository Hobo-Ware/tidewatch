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

use crate::delivery_card::{Auto, Delivery};
use crate::flow_jobs::{self, Done, Which};
use crate::jobs::Jobs;
use crate::note_send::{AutoDelivery, EnterQueue, ReviewGate, gate, gate_reason, review_gate};
use crate::notes_strip::{self, NoteRow};
use crate::outbox::Outbox;
use crate::post_confirm::{Phase, Plan, PostConfirm};
use crate::pr_state::{Chip, PrState, Why, repo_of};

/// What one frame sees of an agent worktree tab.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // independent facts of one frame, not a mode
pub(crate) struct TabFacts {
    pub(crate) id: u64,
    pub(crate) active: bool,
    pub(crate) status: AgentStatus,
    /// The work is finished and can be delivered: the agent is done, or a workflow run ended.
    pub(crate) deliverable: bool,
    pub(crate) agent_id: String,
    pub(crate) dir: PathBuf,
    pub(crate) branch: String,
    pub(crate) notes_file: Option<PathBuf>,
    pub(crate) pending: usize,
    pub(crate) stage_summary: Option<String>,
    pub(crate) ledger: Option<LedgerNote>,
    /// Auto may push the branch and open a draft pull request on its own for this tab.
    pub(crate) auto_push: bool,
    /// The tab runs on its own and approves the card of a fix it was told to make (not a Meeseeks,
    /// which hands off through its own check).
    pub(crate) fixes_push: bool,
    /// The head the branch's pull request merged with: what came after it goes to a new branch.
    pub(crate) merged_head: Option<String>,
    pub(crate) gate: Gate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LedgerNote {
    pub(crate) goal: String,
    pub(crate) verified: Vec<String>,
}

/// Whether the tab may deliver: shut once its pull request merged (`merged_branch`), so no card
/// opens and no pull request follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Gate {
    #[default]
    Open,
    Shut,
}

/// What the app applies after a job finished.
#[derive(Debug)]
pub(crate) enum Out {
    /// A review to type into the agent; `ids` are the notes marked sent for it, `auto` that it was
    /// held for an idle agent.
    Paste {
        tab: u64,
        text: String,
        ids: Vec<u64>,
        auto: bool,
    },
    Notes {
        tab: u64,
        notes: Box<Notes>,
    },
    Toast(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Want {
    Probe,
    AutoSend,
    FindPr(Why),
    LoadPrLines,
}

/// Where Auto is with pushing a tab's finished work. One go per stretch of work, so a push that
/// failed waits for the owner instead of being retried in a loop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AutoPush {
    #[default]
    Idle,
    Telling {
        commits: usize,
    },
    Running,
    Spent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tell {
    pub(crate) tab: u64,
    pub(crate) commits: usize,
    pub(crate) branch: String,
}

impl Tell {
    pub(crate) fn text(&self) -> String {
        let noun = if self.commits == 1 { "commit" } else { "commits" };
        format!(
            "You're done and have {} {noun}. Tidewatch is pushing {} and opening a draft pull request with your summary. Don't push or open one yourself.",
            self.commits, self.branch
        )
    }
}

#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // independent marks of one tab's stretch of work, not a mode
pub(crate) struct TabFlow {
    /// Enabled while a review the user sent waits for the agent to be idle.
    pub(crate) auto: AutoDelivery,
    held_message: String,
    pub(crate) delivery: Delivery,
    pub(crate) pr: PrState,
    target: Option<TabFacts>,
    was_done: bool,
    probed_after: Option<String>,
    pub(crate) push: AutoPush,
    /// The card in front is the push of a fix the agent was told to make: a tab that runs on its own
    /// approves it itself, once.
    fix_push: bool,
    was_active: bool,
    lines_loading: bool,
}

impl TabFlow {
    pub(crate) fn next_fix_step(&self) -> Auto {
        if self.fix_push { self.delivery.auto_step() } else { Auto::Wait }
    }

    pub(crate) fn mark_fix_push(&mut self) {
        self.fix_push = true;
    }
}

/// What to start for one tab this frame. Marks the state of everything it asks for as started, so
/// calling it again before the answers arrive asks for nothing twice.
pub(crate) fn decide(flow: &mut TabFlow, facts: &TabFacts, now: f64) -> Vec<Want> {
    let mut wants = Vec::new();
    flow.auto.observe(&facts.status);
    let done = facts.deliverable && facts.gate == Gate::Open;
    let merge_news = flow.was_done && flow.probed_after != facts.merged_head;
    if done && (!flow.was_done || merge_news) && flow.pr.pr().is_none() {
        flow.probed_after.clone_from(&facts.merged_head);
        flow.delivery.probe_started();
        wants.push(Want::Probe);
    }
    if !done {
        flow.delivery.leave_done();
        if flow.push != AutoPush::Running {
            flow.push = AutoPush::Idle;
        }
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
    if facts.pending == 0 {
        flow.auto.enabled = false;
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
    pub(crate) outbox: Outbox,
    tools: Tools,
    pub(crate) confirm: Option<PostConfirm>,
}

/// What the Diff pane draws for the active tab beyond the diff itself.
pub(crate) struct PaneExtras {
    pub(crate) rows: Vec<NoteRow>,
    /// The notes in the draft review.
    pub(crate) pending: usize,
    pub(crate) held: bool,
    pub(crate) send_review: Result<(), &'static str>,
    pub(crate) chip: Option<Chip>,
    pub(crate) pulling: bool,
}

impl Flow {
    pub(crate) fn new(tools: Tools) -> Self {
        Self {
            jobs: Jobs::new(),
            tabs: HashMap::new(),
            enters: EnterQueue::default(),
            outbox: Outbox::default(),
            tools,
            confirm: None,
        }
    }

    pub(crate) fn tools(&self) -> Tools {
        self.tools.clone()
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
        self.outbox.forget(tab);
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
        if let Some(at) = self.outbox.next_check(now) {
            next = Some(next.map_or(at, |soonest| soonest.min(at)));
        }
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
            self.deliver_on_its_own(fact, ctx);
        }
        next
    }

    /// Auto's next step for a tab: ready to be told, or the next step of a fix's card.
    fn deliver_on_its_own(&mut self, facts: &TabFacts, ctx: &egui::Context) {
        let tab = facts.id;
        let Some(state) = self.tabs.get_mut(&tab) else { return };
        if facts.auto_push
            && state.push == AutoPush::Idle
            && let Delivery::Ready { commits } = state.delivery
        {
            state.push = AutoPush::Telling { commits };
        }
        if !facts.fixes_push {
            return;
        }
        match state.next_fix_step() {
            Auto::Approve => self.approve_card(tab, ctx),
            Auto::Close => {
                state.delivery.close_success();
                state.fix_push = false;
            }
            Auto::Wait => {}
        }
    }

    /// The tabs whose agent Auto has to tell before it pushes, with what to say.
    pub(crate) fn telling(&self) -> Vec<Tell> {
        self.tabs
            .iter()
            .filter_map(|(tab, state)| {
                let AutoPush::Telling { commits } = state.push else { return None };
                let branch = state.target.as_ref()?.branch.clone();
                Some(Tell { tab: *tab, commits, branch })
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn ready_for_push(&mut self, facts: &TabFacts, commits: usize) {
        let mut state = TabFlow { target: Some(facts.clone()), ..TabFlow::default() };
        state.delivery.probe_started();
        state.delivery.probe_done(Some(commits));
        state.push = AutoPush::Telling { commits };
        self.tabs.insert(facts.id, state);
    }

    #[cfg(test)]
    pub(crate) fn push_state(&self, tab: u64) -> Option<(AutoPush, Delivery)> {
        self.tabs.get(&tab).map(|state| (state.push, state.delivery.clone()))
    }

    /// The agent was told. Now the branch is pushed and the draft opened.
    pub(crate) fn told(&mut self, tab: u64, ctx: &egui::Context) {
        let Some(state) = self.tabs.get_mut(&tab) else { return };
        let (AutoPush::Telling { .. }, Some(facts)) = (state.push, state.target.clone()) else {
            return;
        };
        state.push = AutoPush::Running;
        state.delivery.start_auto();
        let push = flow_jobs::AutoPush {
            dir: facts.dir,
            branch: facts.branch,
            summary: facts.stage_summary,
            ledger: facts.ledger,
            merged_head: facts.merged_head,
        };
        flow_jobs::auto_deliver(&self.jobs, ctx, tab, self.tools.clone(), push);
    }

    /// The agent cannot be told, so nothing leaves the machine: the bar stays for the owner.
    pub(crate) fn not_told(&mut self, tab: u64) {
        if let Some(state) = self.tabs.get_mut(&tab) {
            state.push = AutoPush::Spent;
        }
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
                facts.merged_head.clone(),
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
                    let message = self.tabs.get(&tab).map(|t| t.held_message.clone());
                    flow_jobs::send_notes(
                        &self.jobs,
                        ctx,
                        tab,
                        true,
                        file,
                        Which::Review(message.unwrap_or_default()),
                        facts.agent_id.clone(),
                        unix,
                    );
                }
            }
        }
    }

    /// Sends one note now, or the whole draft review: at once to an idle agent, held until it is
    /// idle when it is busy. Returns a toast when the send cannot go.
    pub(crate) fn send(
        &mut self,
        tab: u64,
        which: Which,
        ctx: &egui::Context,
        unix: i64,
    ) -> Option<String> {
        let state = self.tabs.get_mut(&tab)?;
        let facts = state.target.clone()?;
        if let Which::Review(message) = &which {
            match review_gate(Some(&facts.status)) {
                ReviewGate::Refused(reason) => return Some(reason.to_owned()),
                ReviewGate::WhenIdle => {
                    self.hold_review(tab, message.clone());
                    return None;
                }
                ReviewGate::Now => {}
            }
        } else if let Some(reason) = gate_reason(gate(Some(&facts.status))) {
            return Some(reason.to_owned());
        }
        let file = facts.notes_file.clone()?;
        flow_jobs::send_notes(&self.jobs, ctx, tab, false, file, which, facts.agent_id, unix);
        None
    }

    /// Holds the tab's review until the agent is idle, then sends it with `message`.
    pub(crate) fn hold_review(&mut self, tab: u64, message: String) {
        if let Some(state) = self.tabs.get_mut(&tab) {
            state.auto.enabled = true;
            state.held_message = message;
        }
    }

    /// Whether the tab's review waits for the agent to be idle.
    pub(crate) fn review_held(&self, tab: u64) -> bool {
        self.tabs.get(&tab).is_some_and(|state| state.auto.enabled)
    }

    /// The user took the held review back: nothing is sent when the agent is idle.
    pub(crate) fn release_review(&mut self, tab: u64) {
        if let Some(state) = self.tabs.get_mut(&tab) {
            state.auto.enabled = false;
            state.held_message.clear();
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
                facts.merged_head,
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
        let rows = notes_strip::rows(notes, agent, gate(Some(status)), &state.pr);
        let pending = notes.pending(agent).len();
        let send_review = match review_gate(Some(status)) {
            ReviewGate::Refused(reason) => Err(reason),
            ReviewGate::Now | ReviewGate::WhenIdle => Ok(()),
        };
        Some(PaneExtras {
            rows,
            pending,
            held: state.auto.enabled,
            send_review,
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

    pub(crate) fn unsend(&mut self, tab: u64, ids: Vec<u64>, ctx: &egui::Context) {
        let file = self.tabs.get(&tab).and_then(|state| state.target.as_ref()?.notes_file.clone());
        if let Some(file) = file {
            flow_jobs::unsend_notes(&self.jobs, ctx, tab, file, ids);
        }
    }

    /// Presses Enter in the tab's terminal once the paste before it has landed.
    pub(crate) fn queue_enter(&mut self, tab: u64, now: f64) {
        self.enters.push(tab, now);
    }

    pub(crate) fn due_enters(&mut self, now: f64, output: impl Fn(u64) -> Option<u64>) -> Vec<u64> {
        self.enters.take_due(now, output)
    }

    fn apply(&mut self, done: Done, ctx: &egui::Context, now: f64, outs: &mut Vec<Out>) {
        match done {
            Done::Sent { tab, auto, result } => self.sent(tab, auto, result, now, outs),
            Done::Unsent { tab, result } => outs.push(match result {
                Ok(notes) => Out::Notes { tab, notes },
                Err(message) => Out::Toast(format!("Could not take notes back: {message}")),
            }),
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
                let head = result.as_ref().ok().map(|done| done.head.clone());
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.fix_push = false;
                    state.delivery.delivered(result);
                }
                self.find_after_delivery(tab, head.is_some(), ctx, now);
            }
            Done::AutoDelivered { tab, result } => {
                let found = result.is_ok();
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.push = AutoPush::Spent;
                    match result {
                        Ok(_) => state.delivery.auto_finished(Ok(())),
                        Err(failed) => {
                            let failed = *failed;
                            if failed.form.is_none() {
                                outs.push(Out::Toast(format!(
                                    "Cannot read the commits: {}",
                                    failed.error
                                )));
                            }
                            state.delivery.auto_finished(Err((failed.form, failed.error)));
                        }
                    }
                }
                self.find_after_delivery(tab, found, ctx, now);
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

    fn find_after_delivery(&mut self, tab: u64, delivered: bool, ctx: &egui::Context, now: f64) {
        let Some(state) = self.tabs.get_mut(&tab) else { return };
        if !delivered || !state.pr.should_find(Why::Delivered, now) {
            return;
        }
        state.pr.find_started();
        if let Some(facts) = state.target.clone() {
            flow_jobs::find_pr(&self.jobs, ctx, tab, self.tools.clone(), facts.dir, facts.branch);
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
                    state.auto.enabled = false;
                    state.held_message.clear();
                } else if sent {
                    state.auto.sent_by_hand(now);
                }
                outs.push(Out::Notes { tab, notes: out.notes });
                match out.text {
                    Some(text) => {
                        outs.push(Out::Paste { tab, text, ids: out.ids, auto });
                        outs.push(Out::Toast(sent_text(out.count, auto)));
                    }
                    None if !auto => outs.push(Out::Toast("Nothing new to send".to_owned())),
                    None => {}
                }
            }
            Err(message) => {
                if auto {
                    state.auto.finished(false, now);
                    state.auto.enabled = false;
                    state.held_message.clear();
                }
                outs.push(Out::Toast(format!("Could not send notes: {message}")));
            }
        }
    }
}

pub(crate) fn sent_text(count: usize, auto: bool) -> String {
    let noun = if count == 1 { "note" } else { "notes" };
    if auto {
        format!("Agent is idle: sent your review ({count} {noun})")
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
            format!("Pulled {comments} PR {c} ({threads} new {t}) into your review")
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
            deliverable: status == AgentStatus::Done,
            status,
            agent_id: "claude".into(),
            dir: PathBuf::from("/w"),
            branch: "agent/brave-otter".into(),
            notes_file: Some(PathBuf::from("/s/notes.json")),
            pending: 0,
            stage_summary: None,
            ledger: None,
            auto_push: false,
            fixes_push: false,
            merged_head: None,
            gate: Gate::Open,
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
    fn a_tab_probed_before_its_merge_was_known_is_probed_again_with_the_merged_head() {
        let mut flow = flow_with(false);
        assert_eq!(decide(&mut flow, &facts(AgentStatus::Done), 1.0), [Want::Probe]);
        flow.delivery.probe_done(Some(2));
        assert_eq!(flow.delivery, Delivery::Ready { commits: 2 });
        let merged = TabFacts { merged_head: Some("e3dba69".into()), ..facts(AgentStatus::Done) };
        assert_eq!(decide(&mut flow, &merged, 2.0), [Want::Probe]);
        flow.delivery.probe_done(Some(0));
        assert_eq!(flow.delivery, Delivery::Hidden);
        assert_eq!(decide(&mut flow, &merged, 3.0), [], "the same head is not asked twice");
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
    fn a_tab_whose_pull_request_merged_is_not_probed_and_loses_its_card() {
        let mut flow = flow_with(false);
        decide(&mut flow, &facts(AgentStatus::Done), 0.0);
        flow.delivery.probe_done(Some(1));
        assert_eq!(flow.delivery, Delivery::Ready { commits: 1 });
        let shut = TabFacts { gate: Gate::Shut, ..facts(AgentStatus::Done) };
        assert_eq!(decide(&mut flow, &shut, 1.0), []);
        assert_eq!(flow.delivery, Delivery::Hidden);
        decide(&mut flow, &facts(AgentStatus::Working), 2.0);
        assert_eq!(decide(&mut flow, &shut, 3.0), [], "a new idle moment asks for nothing");
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

    fn flow_of(status: AgentStatus) -> Flow {
        let mut flow = Flow::new(Tools::default());
        flow.tabs.insert(7, TabFlow { target: Some(facts(status)), ..TabFlow::default() });
        flow
    }

    fn send_review(flow: &mut Flow, message: &str) -> Option<String> {
        flow.send(7, Which::Review(message.into()), &egui::Context::default(), 1)
    }

    #[test]
    fn a_review_sent_to_a_busy_agent_is_held_with_its_message_and_not_pasted() {
        for status in [
            AgentStatus::Working,
            AgentStatus::NeedsYou(Reason::Permission),
            AgentStatus::NeedsYou(Reason::Quiet),
        ] {
            let mut flow = flow_of(status.clone());
            assert_eq!(send_review(&mut flow, "start with a.rs"), None, "{status:?}");
            assert!(flow.review_held(7), "{status:?}");
            assert_eq!(flow.tabs[&7].held_message, "start with a.rs");
        }
    }

    #[test]
    fn a_review_for_no_agent_is_refused_and_a_held_one_can_be_taken_back() {
        let mut flow = flow_of(AgentStatus::Shell);
        assert_eq!(
            send_review(&mut flow, "x").as_deref(),
            Some("There is no running agent in this tab")
        );
        assert!(!flow.review_held(7));
        let mut busy = flow_of(AgentStatus::Working);
        send_review(&mut busy, "x");
        busy.release_review(7);
        assert!(!busy.review_held(7));
        assert_eq!(busy.tabs[&7].held_message, "");
    }

    #[test]
    fn a_held_review_goes_when_the_agent_is_done_and_is_dropped_when_no_note_is_left() {
        let mut flow = flow_of(AgentStatus::Working);
        send_review(&mut flow, "x");
        let state = flow.tabs.get_mut(&7).unwrap();
        let mut f = facts(AgentStatus::Working);
        f.pending = 2;
        assert!(!decide(state, &f, 0.0).contains(&Want::AutoSend));
        f.status = AgentStatus::Done;
        assert!(decide(state, &f, 1.0).contains(&Want::AutoSend));
        let mut emptied = flow_of(AgentStatus::Working);
        send_review(&mut emptied, "x");
        let state = emptied.tabs.get_mut(&7).unwrap();
        f.pending = 0;
        decide(state, &f, 2.0);
        assert!(!state.auto.enabled);
    }

    #[test]
    fn a_single_note_still_goes_to_a_working_agent_and_never_to_one_waiting_on_you() {
        let ctx = egui::Context::default();
        let mut waiting = flow_of(AgentStatus::NeedsYou(Reason::Permission));
        assert!(waiting.send(7, Which::One(3), &ctx, 1).is_some());
        let mut shell = flow_of(AgentStatus::Shell);
        assert!(shell.send(7, Which::One(3), &ctx, 1).is_some());
    }

    #[test]
    fn the_toasts_count_notes_and_comments() {
        assert_eq!(sent_text(1, false), "Sent 1 note to the agent");
        assert_eq!(sent_text(3, true), "Agent is idle: sent your review (3 notes)");
        assert_eq!(pulled_text(MergeReport::default()), "No new PR comments");
        assert_eq!(
            pulled_text(MergeReport { new_threads: 2, new_comments: 3 }),
            "Pulled 3 PR comments (2 new threads) into your review"
        );
    }

    fn ready_tab(facts: TabFacts, commits: usize) -> Flow {
        let mut flow = Flow::new(Tools::default());
        let mut state = TabFlow { target: Some(facts), ..TabFlow::default() };
        state.delivery.probe_started();
        state.delivery.probe_done(Some(commits));
        flow.tabs.insert(7, state);
        flow
    }

    fn push_on() -> TabFacts {
        TabFacts { auto_push: true, fixes_push: true, ..facts(AgentStatus::Done) }
    }

    #[test]
    fn auto_tells_the_agent_first_and_pushes_nothing_until_it_was_told() {
        let ctx = egui::Context::default();
        let mut flow = ready_tab(push_on(), 2);
        flow.deliver_on_its_own(&push_on(), &ctx);
        let tells = flow.telling();
        assert_eq!(tells.len(), 1);
        assert_eq!(
            tells[0].text(),
            "You're done and have 2 commits. Tidewatch is pushing agent/brave-otter and opening a draft pull request with your summary. Don't push or open one yourself."
        );
        assert_eq!(flow.tabs[&7].delivery, Delivery::Ready { commits: 2 }, "nothing is pushed yet");
        flow.deliver_on_its_own(&push_on(), &ctx);
        assert_eq!(flow.telling().len(), 1, "asked for once");

        flow.told(7, &ctx);
        assert_eq!(flow.tabs[&7].delivery, Delivery::Working { commits: 2 });
        assert_eq!(flow.tabs[&7].push, AutoPush::Running);
        assert_eq!(flow.telling(), []);
        flow.deliver_on_its_own(&push_on(), &ctx);
        assert_eq!(flow.telling(), [], "told once per stretch of work");
    }

    #[test]
    fn the_message_counts_one_commit_in_the_singular() {
        let tell = Tell { tab: 1, commits: 1, branch: "agent/x".into() };
        assert!(
            tell.text().starts_with("You're done and have 1 commit. Tidewatch is pushing agent/x")
        );
    }

    #[test]
    fn with_the_push_tick_off_a_tab_on_its_own_pushes_nothing_and_keeps_the_bar() {
        let ctx = egui::Context::default();
        let off = TabFacts { auto_push: false, fixes_push: true, ..facts(AgentStatus::Done) };
        let mut flow = ready_tab(off.clone(), 2);
        flow.deliver_on_its_own(&off, &ctx);
        assert_eq!(flow.telling(), []);
        assert_eq!(flow.tabs[&7].delivery, Delivery::Ready { commits: 2 });
        assert_eq!(flow.tabs[&7].delivery.card_commits(), Some(2), "the bar stays as it was");
        assert_eq!(flow.tabs[&7].push, AutoPush::Idle);
    }

    #[test]
    fn an_agent_that_cannot_be_told_leaves_the_bar_for_the_owner_and_is_not_asked_again() {
        let ctx = egui::Context::default();
        let mut flow = ready_tab(push_on(), 2);
        flow.deliver_on_its_own(&push_on(), &ctx);
        flow.not_told(7);
        flow.deliver_on_its_own(&push_on(), &ctx);
        assert_eq!(flow.telling(), []);
        assert_eq!(flow.tabs[&7].delivery, Delivery::Ready { commits: 2 });
    }

    #[test]
    fn a_new_stretch_of_work_may_push_again_but_a_push_under_way_is_not_cut_off() {
        let mut flow = TabFlow::default();
        let working = facts(AgentStatus::Working);
        flow.push = AutoPush::Spent;
        decide(&mut flow, &working, 1.0);
        assert_eq!(flow.push, AutoPush::Idle);
        flow.push = AutoPush::Telling { commits: 2 };
        decide(&mut flow, &working, 2.0);
        assert_eq!(flow.push, AutoPush::Idle, "an agent that went back to work is not told");
        flow.push = AutoPush::Running;
        decide(&mut flow, &working, 3.0);
        assert_eq!(flow.push, AutoPush::Running);
    }

    #[test]
    fn the_card_of_a_fix_is_approved_without_a_click_only_when_the_fix_opened_it() {
        let mut flow = TabFlow::default();
        flow.delivery.probe_started();
        flow.delivery.probe_done(Some(1));
        flow.delivery.open();
        flow.delivery.built(Ok(tidewatch_core::delivery::DeliveryPlan {
            dir: PathBuf::from("/w"),
            branch: "agent/brave-otter".into(),
            base: "main".into(),
            head: "agent/brave-otter".into(),
            commits: vec![tidewatch_core::delivery::CommitLine {
                sha: "abc".into(),
                subject: "add helper".into(),
                body: String::new(),
            }],
            rebase: false,
            title: "Add helper".into(),
            body: String::new(),
        }));
        assert_eq!(flow.next_fix_step(), Auto::Wait, "a card the owner opened waits for the owner");
        flow.mark_fix_push();
        assert_eq!(flow.next_fix_step(), Auto::Approve);
    }
}
