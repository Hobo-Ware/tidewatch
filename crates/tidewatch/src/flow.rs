//! Everything that happens between an agent tab and the outside after the agent wrote code: notes
//! sent to the agent, the delivery card, the pull request link and the confirm before posting.
//! `Flow` keeps one `TabFlow` per agent worktree tab and turns what a frame sees (`TabFacts`) into
//! background jobs; finished jobs come back as `Out` values for the app to apply. It never touches a
//! terminal, a view or a thread directly, so the decisions are unit tests.

use std::collections::HashMap;
use std::path::PathBuf;

use tidewatch_core::notes::Notes;
use tidewatch_core::pr::MergeReport;
use tidewatch_core::pr_ask::{self, Opening};
use tidewatch_core::status::AgentStatus;
use tidewatch_core::tools::Tools;
use tidewatch_core::workflow_profile::Workflow;

use crate::delivery_card::Delivery;
use crate::done_check_glue::TestsGate;
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
    /// Auto may ask the agent to push the branch and open a draft pull request for this tab.
    pub(crate) auto_push: bool,
    /// The head the branch's pull request merged with: what came after it goes to a new branch.
    pub(crate) merged_head: Option<String>,
    pub(crate) gate: Gate,
    pub(crate) profile: String,
    pub(crate) workflow: Workflow,
    /// What the done check's own test run says: Auto asks for the push only once it passed.
    pub(crate) tests: TestsGate,
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
    ReviewExpired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Asker {
    Auto,
    Owner,
}

/// Where the ask for a tab's finished work is. One go per stretch of work, so an ask that was
/// refused waits for the owner instead of being retried in a loop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AutoPush {
    #[default]
    Idle,
    Asking {
        by: Asker,
    },
    Spent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ask {
    pub(crate) tab: u64,
    pub(crate) by: Asker,
    pub(crate) text: String,
}

#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // independent marks of one tab's stretch of work, not a mode
pub(crate) struct TabFlow {
    /// Enabled while a review the user sent waits for the agent to be idle.
    pub(crate) auto: AutoDelivery,
    held_message: String,
    pub(crate) delivery: Delivery,
    base: Option<String>,
    pub(crate) pr: PrState,
    target: Option<TabFacts>,
    was_done: bool,
    probed_after: Option<String>,
    pub(crate) push: AutoPush,
    was_active: bool,
    lines_loading: bool,
}

impl TabFlow {
    /// The commits the card offers came after the branch's pull request merged.
    pub(crate) fn after_merge(&self) -> bool {
        self.target.as_ref().is_some_and(|facts| facts.merged_head.is_some())
    }

    fn begin_ask(&mut self, by: Asker) {
        if self.delivery.ask_started() {
            self.push = AutoPush::Asking { by };
        }
    }

    fn ask_text(&self) -> Option<String> {
        let facts = self.target.as_ref()?;
        let (Delivery::Asked { commits, .. }, Some(base)) = (&self.delivery, &self.base) else {
            return None;
        };
        let opening = Opening {
            branch: &facts.branch,
            base,
            commits: *commits,
            merged_head: facts.merged_head.as_deref(),
        };
        Some(pr_ask::open_pull_request(&opening, facts.workflow))
    }
}

/// What to start for one tab this frame. Marks the state of everything it asks for as started, so
/// calling it again before the answers arrive asks for nothing twice.
pub(crate) fn decide(flow: &mut TabFlow, facts: &TabFacts, now: f64) -> Vec<Want> {
    let mut wants = Vec::new();
    flow.auto.observe(&facts.status, now);
    let done = facts.deliverable && facts.gate == Gate::Open;
    let merge_news = flow.was_done && flow.probed_after != facts.merged_head;
    if done && (!flow.was_done || merge_news) && flow.pr.pr().is_none() && !flow.delivery.is_asked()
    {
        flow.probed_after.clone_from(&facts.merged_head);
        flow.delivery.probe_started();
        wants.push(Want::Probe);
    }
    if facts.gate == Gate::Shut || flow.pr.pr().is_some() {
        flow.delivery.hide();
    }
    if !done {
        flow.delivery.leave_done();
        if !flow.delivery.is_asked() {
            flow.push = AutoPush::Idle;
        }
    }
    flow.was_done = done;
    if flow.delivery.watch(facts.status == AgentStatus::Done, now)
        && flow.pr.should_find(Why::Asked, now)
    {
        flow.pr.find_started();
        wants.push(Want::FindPr(Why::Asked));
    }
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
    if flow.auto.expire(now) {
        flow.held_message.clear();
        wants.push(Want::ReviewExpired);
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
    notices: Vec<String>,
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
            notices: Vec::new(),
        }
    }

    pub(crate) fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    pub(crate) fn tools(&self) -> Tools {
        self.tools.clone()
    }

    pub(crate) fn tab(&self, id: u64) -> Option<&TabFlow> {
        self.tabs.get(&id)
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
    pub(crate) fn modal_open(&self) -> bool {
        self.confirm.is_some()
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
            let wakes = [state.auto.next_check(now), state.delivery.next_wake()];
            for at in wakes.into_iter().flatten() {
                next = Some(next.map_or(at, |soonest| soonest.min(at)));
            }
            for want in wants {
                self.start(fact, want, ctx, unix);
            }
            self.ask_on_its_own(fact);
        }
        next
    }

    pub(crate) fn ask_on_its_own(&mut self, facts: &TabFacts) {
        let Some(state) = self.tabs.get_mut(&facts.id) else { return };
        if facts.auto_push && facts.tests == TestsGate::Clear && state.push == AutoPush::Idle {
            state.begin_ask(Asker::Auto);
        }
    }

    /// The tabs Auto would ask to push now if Tidewatch's own test run had come back: they need
    /// the done check to run.
    pub(crate) fn waiting_for_tests(&self, facts: &[TabFacts]) -> Vec<u64> {
        facts
            .iter()
            .filter(|fact| fact.auto_push && fact.tests == TestsGate::Pending)
            .filter(|fact| {
                self.tabs.get(&fact.id).is_some_and(|state| {
                    state.push == AutoPush::Idle
                        && matches!(
                            state.delivery,
                            Delivery::Ready { .. } | Delivery::Missing { .. }
                        )
                })
            })
            .map(|fact| fact.id)
            .collect()
    }

    pub(crate) fn ask_to_open(&mut self, tab: u64) {
        if let Some(state) = self.tabs.get_mut(&tab) {
            state.begin_ask(Asker::Owner);
        }
    }

    pub(crate) fn show_asked(&mut self, tab: u64, now: f64) {
        if let Some(state) = self.tabs.get_mut(&tab)
            && state.delivery.ask_started()
        {
            state.push = AutoPush::Spent;
            state.delivery.asked(now);
        }
    }

    pub(crate) fn show_missing(&mut self, tab: u64) {
        if let Some(state) = self.tabs.get_mut(&tab)
            && let Delivery::Ready { commits } = state.delivery
        {
            state.push = AutoPush::Spent;
            state.delivery = Delivery::Missing { commits };
        }
    }

    pub(crate) fn asking(&self) -> Vec<Ask> {
        self.tabs
            .iter()
            .filter_map(|(tab, state)| {
                let AutoPush::Asking { by } = state.push else { return None };
                Some(Ask { tab: *tab, by, text: state.ask_text()? })
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn ready_for_ask(&mut self, facts: &TabFacts, commits: usize) {
        let mut state = TabFlow {
            target: Some(facts.clone()),
            base: Some("main".to_owned()),
            ..TabFlow::default()
        };
        state.delivery.probe_started();
        state.delivery.probe_done(Some(commits));
        self.tabs.insert(facts.id, state);
    }

    #[cfg(test)]
    pub(crate) fn push_state(&self, tab: u64) -> Option<(AutoPush, Delivery)> {
        self.tabs.get(&tab).map(|state| (state.push, state.delivery.clone()))
    }

    /// The ask went through the send gate and was typed: the agent has the whole job from here.
    pub(crate) fn asked(&mut self, tab: u64, now: f64) {
        if let Some(state) = self.tabs.get_mut(&tab) {
            state.push = AutoPush::Spent;
            state.delivery.asked(now);
        }
    }

    /// The agent cannot take the ask: the card offers it again and Auto does not retry.
    pub(crate) fn not_asked(&mut self, tab: u64) {
        if let Some(state) = self.tabs.get_mut(&tab) {
            state.push = AutoPush::Spent;
            state.delivery.not_asked();
        }
    }

    fn start(&mut self, facts: &TabFacts, want: Want, ctx: &egui::Context, unix: i64) {
        let tab = facts.id;
        let tools = self.tools.clone();
        let dir = facts.dir.clone();
        let branch = facts.branch.clone();
        match want {
            Want::Probe => {
                flow_jobs::probe(
                    &self.jobs,
                    ctx,
                    tab,
                    tools,
                    dir,
                    branch,
                    facts.merged_head.clone(),
                );
            }
            Want::FindPr(_) => flow_jobs::find_pr(&self.jobs, ctx, tab, tools, dir, branch),
            Want::LoadPrLines => {
                let head = self
                    .tabs
                    .get(&tab)
                    .and_then(|t| t.pr.pr().map(|pr| pr.head_sha.clone()))
                    .unwrap_or_default();
                flow_jobs::load_pr_lines(&self.jobs, ctx, tab, tools, dir, head);
            }
            Want::ReviewExpired => {
                self.notices
                    .push(format!("Review not sent: {} stayed busy for an hour", facts.profile));
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

    /// Esc, Cancel or Close on the confirm dialog. A post already on its way cannot be recalled:
    /// its result arrives as a toast.
    pub(crate) fn cancel_post(&mut self) {
        self.confirm = None;
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
    pub(crate) fn drain(&mut self, now: f64) -> Vec<Out> {
        let mut outs = Vec::new();
        for done in self.jobs.drain() {
            self.apply(done, now, &mut outs);
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

    fn apply(&mut self, done: Done, now: f64, outs: &mut Vec<Out>) {
        match done {
            Done::Sent { tab, auto, result } => self.sent(tab, auto, result, now, outs),
            Done::Unsent { tab, result } => outs.push(match result {
                Ok(notes) => Out::Notes { tab, notes },
                Err(message) => Out::Toast(format!("Could not take notes back: {message}")),
            }),
            Done::Probed { tab, found } => {
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.base = found.as_ref().map(|probe| probe.base.clone());
                    state.delivery.probe_done(found.map(|probe| probe.commits));
                }
            }
            Done::PrFound { tab, result } => {
                if let Some(state) = self.tabs.get_mut(&tab) {
                    state.pr.found(result, now);
                    state.delivery.answered(state.pr.pr().is_some());
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
            Done::Posted { tab, result } => {
                let waiting = self
                    .confirm
                    .as_ref()
                    .is_some_and(|c| c.tab == tab && c.phase == Phase::Posting);
                match result {
                    Ok((posted, notes)) => {
                        if waiting {
                            self.confirm = None;
                        }
                        outs.push(Out::Notes { tab, notes });
                        outs.push(Out::Toast(format!(
                            "Posted to the pull request: {}",
                            posted.url
                        )));
                    }
                    Err(error) => match &mut self.confirm {
                        Some(confirm) if waiting => {
                            confirm.phase = Phase::Failed(error.to_string());
                        }
                        _ => outs.push(Out::Toast(format!(
                            "Could not post to the pull request: {error}"
                        ))),
                    },
                }
            }
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
    use tidewatch_core::tools::GhError;

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
            auto_push: false,
            merged_head: None,
            gate: Gate::Open,
            profile: "Builder".into(),
            workflow: Workflow::default(),
            tests: TestsGate::Clear,
        }
    }

    fn flow_with(auto: bool) -> TabFlow {
        let mut flow = TabFlow::default();
        flow.auto.enabled = auto;
        flow
    }

    #[test]
    fn a_review_held_for_a_busy_agent_expires_once_after_an_hour_and_sends_nothing() {
        let mut flow = flow_with(true);
        flow.held_message = "please".into();
        let busy = TabFacts { pending: 2, ..facts(AgentStatus::Working) };
        let limit = crate::note_send::HOLD_EXPIRES_SECS;
        assert_eq!(decide(&mut flow, &busy, 10.0), []);
        assert_eq!(decide(&mut flow, &busy, 10.0 + limit - 1.0), []);
        assert!(flow.auto.enabled);
        assert_eq!(decide(&mut flow, &busy, 10.0 + limit), [Want::ReviewExpired]);
        assert!(!flow.auto.enabled);
        assert_eq!(flow.held_message, "");
        assert_eq!(decide(&mut flow, &busy, 11.0 + limit), []);
        let idle = TabFacts { pending: 2, ..facts(AgentStatus::Done) };
        assert_eq!(decide(&mut flow, &idle, 12.0 + limit), [Want::Probe]);
    }

    #[test]
    fn a_review_held_for_a_busy_agent_goes_out_when_it_turns_done_before_the_hour() {
        let mut flow = flow_with(true);
        let busy = TabFacts { pending: 1, ..facts(AgentStatus::Working) };
        assert_eq!(decide(&mut flow, &busy, 0.0), []);
        let idle = TabFacts { pending: 1, ..facts(AgentStatus::Done) };
        assert_eq!(decide(&mut flow, &idle, 600.0), [Want::Probe, Want::AutoSend]);
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
    fn the_card_says_after_the_merge_only_for_commits_past_a_merged_head() {
        let flow = flow_with(false);
        assert!(!flow.after_merge(), "nothing is known before the first look");
        let plain = TabFlow { target: Some(facts(AgentStatus::Done)), ..TabFlow::default() };
        assert!(!plain.after_merge());
        let merged = TabFacts { merged_head: Some("abc".into()), ..facts(AgentStatus::Done) };
        let after = TabFlow { target: Some(merged), ..TabFlow::default() };
        assert!(after.after_merge());
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

    fn posting(flow: &mut Flow, phase: Phase) {
        flow.confirm = Some(PostConfirm {
            tab: 7,
            pr: PullRequest {
                number: 12,
                url: "https://github.com/o/r/pull/12".into(),
                head_sha: "a".into(),
            },
            repo: "o/r".into(),
            plan: Plan {
                thread: 1,
                kind: crate::post_confirm::Kind::Reply { root: 9 },
                body: "x".into(),
            },
            phase,
        });
    }

    fn posted_ok() -> Done {
        let posted =
            tidewatch_core::pr::Posted { id: 1, url: "https://github.com/o/r/pull/12#c1".into() };
        Done::Posted { tab: 7, result: Ok((posted, Box::default())) }
    }

    fn posted_err() -> Done {
        Done::Posted { tab: 7, result: Err(GhError::Failed("HTTP 502".into())) }
    }

    fn toasts(outs: &[Out]) -> Vec<&str> {
        outs.iter()
            .filter_map(|out| match out {
                Out::Toast(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn cancel_closes_the_post_dialog_in_every_phase() {
        for phase in [Phase::Asking, Phase::Posting, Phase::Failed("x".into())] {
            let mut flow = Flow::new(Tools::default());
            posting(&mut flow, phase.clone());
            flow.cancel_post();
            assert_eq!(flow.confirm, None, "{phase:?}");
        }
    }

    #[test]
    fn a_post_that_lands_after_cancel_arrives_as_a_toast() {
        let landed = |done: Done| {
            let mut flow = Flow::new(Tools::default());
            posting(&mut flow, Phase::Posting);
            flow.cancel_post();
            let mut outs = Vec::new();
            flow.apply(done, 0.0, &mut outs);
            outs.iter()
                .filter_map(|out| match out {
                    Out::Toast(text) => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            landed(posted_ok()),
            ["Posted to the pull request: https://github.com/o/r/pull/12#c1"]
        );
        let failed = landed(posted_err());
        assert_eq!(failed.len(), 1);
        assert!(failed[0].contains("HTTP 502"), "{failed:?}");
    }

    #[test]
    fn a_late_post_result_leaves_a_newer_dialog_alone() {
        let mut flow = Flow::new(Tools::default());
        posting(&mut flow, Phase::Asking);
        let mut outs = Vec::new();
        flow.apply(posted_ok(), 0.0, &mut outs);
        assert_eq!(flow.confirm.as_ref().map(|c| c.phase.clone()), Some(Phase::Asking));
        flow.apply(posted_err(), 0.0, &mut outs);
        assert_eq!(flow.confirm.as_ref().map(|c| c.phase.clone()), Some(Phase::Asking));
    }

    #[test]
    fn a_post_result_while_the_dialog_waits_closes_it_or_shows_the_failure() {
        let mut flow = Flow::new(Tools::default());
        posting(&mut flow, Phase::Posting);
        flow.apply(posted_ok(), 0.0, &mut Vec::new());
        assert_eq!(flow.confirm, None);
        posting(&mut flow, Phase::Posting);
        let mut outs = Vec::new();
        flow.apply(posted_err(), 0.0, &mut outs);
        assert_eq!(toasts(&outs), Vec::<&str>::new());
        assert!(
            matches!(flow.confirm.as_ref().map(|c| &c.phase), Some(Phase::Failed(m)) if m.contains("HTTP 502"))
        );
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

    fn ready_tab(facts: &TabFacts, commits: usize) -> Flow {
        let mut flow = Flow::new(Tools::default());
        flow.ready_for_ask(facts, commits);
        flow
    }

    fn push_on() -> TabFacts {
        TabFacts { auto_push: true, ..facts(AgentStatus::Done) }
    }

    #[test]
    fn auto_asks_the_agent_to_open_the_pull_request_and_pushes_and_opens_nothing_itself() {
        let mut flow = ready_tab(&push_on(), 2);
        flow.ask_on_its_own(&push_on());
        let asks = flow.asking();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].by, Asker::Auto);
        for needle in [
            "2 commits on `agent/brave-otter` are not in `main`",
            "open a draft pull request into `main`",
            "Write the title and body yourself",
        ] {
            assert!(asks[0].text.contains(needle), "missing {needle:?} in {}", asks[0].text);
        }
        assert_eq!(
            flow.tabs[&7].delivery,
            Delivery::Asked { commits: 2, wait: crate::delivery_card::Wait::Queued }
        );
        flow.ask_on_its_own(&push_on());
        assert_eq!(flow.asking().len(), 1, "asked for once");

        flow.asked(7, 5.0);
        assert_eq!(
            flow.tabs[&7].delivery,
            Delivery::Asked { commits: 2, wait: crate::delivery_card::Wait::Typed { at: 5.0 } }
        );
        assert_eq!(flow.asking(), []);
        flow.ask_on_its_own(&push_on());
        assert_eq!(flow.asking(), [], "asked once per stretch of work");
    }

    #[test]
    fn the_ask_names_the_base_branch_the_repo_really_has() {
        let mut flow = ready_tab(&push_on(), 1);
        flow.tabs.get_mut(&7).unwrap().base = Some("develop".into());
        flow.ask_on_its_own(&push_on());
        let text = &flow.asking()[0].text;
        assert!(text.contains("into `develop`"), "{text}");
        assert!(text.starts_with("1 commit on `agent/brave-otter` is not in `develop`"), "{text}");
    }

    #[test]
    fn the_ask_opens_the_kind_of_pull_request_the_profile_says() {
        use tidewatch_core::workflow_profile::PullRequests;
        let ready = Workflow { pull_requests: PullRequests::Ready, ..Workflow::default() };
        let facts = TabFacts { workflow: ready, ..push_on() };
        let mut flow = ready_tab(&facts, 1);
        flow.ask_on_its_own(&facts);
        let text = &flow.asking()[0].text;
        assert!(text.contains("open a pull request (not a draft) into `main`"), "{text}");
    }

    #[test]
    fn a_commit_set_after_a_merge_is_asked_for_on_a_new_branch() {
        let merged = TabFacts { merged_head: Some("e3dba69".into()), ..push_on() };
        let mut flow = ready_tab(&merged, 1);
        flow.ask_on_its_own(&merged);
        let text = &flow.asking()[0].text;
        assert!(text.contains("merged at `e3dba69`"), "{text}");
        assert!(text.contains("push it as `agent/brave-otter-2`"), "{text}");
    }

    #[test]
    fn with_the_push_tick_off_nothing_is_asked_and_the_card_waits_for_the_owner() {
        let off = TabFacts { auto_push: false, ..facts(AgentStatus::Done) };
        let mut flow = ready_tab(&off, 2);
        flow.ask_on_its_own(&off);
        assert_eq!(flow.asking(), []);
        assert_eq!(flow.tabs[&7].delivery, Delivery::Ready { commits: 2 });
        assert_eq!(flow.tabs[&7].push, AutoPush::Idle);
    }

    #[test]
    fn the_owners_click_asks_from_a_ready_card_and_again_from_a_missing_one() {
        let off = TabFacts { auto_push: false, ..facts(AgentStatus::Done) };
        let mut flow = ready_tab(&off, 2);
        flow.ask_to_open(7);
        let asks = flow.asking();
        assert_eq!((asks.len(), asks[0].by), (1, Asker::Owner));
        flow.asked(7, 1.0);
        flow.ask_to_open(7);
        assert_eq!(flow.asking(), [], "nothing to ask again while it is being asked");
        flow.tabs.get_mut(&7).unwrap().delivery = Delivery::Missing { commits: 2 };
        flow.ask_to_open(7);
        assert_eq!(flow.asking().len(), 1, "Ask again");
        let mut none = Flow::new(Tools::default());
        none.ask_to_open(99);
        assert_eq!(none.asking(), []);
    }

    #[test]
    fn an_agent_that_cannot_take_the_ask_leaves_the_button_for_the_owner_and_auto_does_not_retry() {
        let mut flow = ready_tab(&push_on(), 2);
        flow.ask_on_its_own(&push_on());
        flow.not_asked(7);
        flow.ask_on_its_own(&push_on());
        assert_eq!(flow.asking(), []);
        assert_eq!(flow.tabs[&7].delivery, Delivery::Ready { commits: 2 });
        flow.ask_to_open(7);
        assert_eq!(flow.asking().len(), 1, "the owner may still ask");
    }

    #[test]
    fn a_new_stretch_of_work_may_ask_again_but_an_ask_under_way_is_not_cut_off() {
        let mut flow = TabFlow::default();
        let working = facts(AgentStatus::Working);
        flow.push = AutoPush::Spent;
        decide(&mut flow, &working, 1.0);
        assert_eq!(flow.push, AutoPush::Idle);
        flow.push = AutoPush::Asking { by: Asker::Auto };
        decide(&mut flow, &working, 2.0);
        assert_eq!(flow.push, AutoPush::Idle, "an agent that went back to work is not asked");
        flow.delivery = Delivery::Asked { commits: 1, wait: crate::delivery_card::Wait::Working };
        flow.push = AutoPush::Spent;
        decide(&mut flow, &working, 3.0);
        assert_eq!(flow.push, AutoPush::Spent, "the ask under way keeps its mark");
    }

    fn asked_flow() -> Flow {
        let mut flow = ready_tab(&push_on(), 2);
        flow.ask_on_its_own(&push_on());
        flow.asked(7, 10.0);
        flow
    }

    fn tick(flow: &mut Flow, status: AgentStatus, now: f64) -> Vec<Want> {
        let facts = TabFacts { auto_push: true, ..facts(status) };
        let state = flow.tabs.get_mut(&7).unwrap();
        state.target = Some(facts.clone());
        decide(state, &facts, now)
    }

    fn answer(flow: &mut Flow, result: Result<Option<PullRequest>, GhError>, now: f64) {
        flow.apply(Done::PrFound { tab: 7, result }, now, &mut Vec::new());
    }

    #[test]
    fn the_card_says_asked_while_the_agent_works_and_looks_for_the_pr_when_it_goes_idle() {
        let mut flow = asked_flow();
        assert_eq!(tick(&mut flow, AgentStatus::Working, 11.0), []);
        assert_eq!(tick(&mut flow, AgentStatus::Working, 40.0), []);
        assert!(flow.tabs[&7].delivery.is_asked());
        assert_eq!(tick(&mut flow, AgentStatus::Done, 50.0), [Want::FindPr(Why::Asked)]);
        assert_eq!(tick(&mut flow, AgentStatus::Done, 51.0), [], "one lookup, its answer awaited");
    }

    #[test]
    fn no_pr_after_the_second_lookup_leaves_no_pull_request_yet_and_ask_again_works() {
        let mut flow = asked_flow();
        tick(&mut flow, AgentStatus::Working, 11.0);
        assert_eq!(tick(&mut flow, AgentStatus::Done, 50.0), [Want::FindPr(Why::Asked)]);
        answer(&mut flow, Ok(None), 50.5);
        assert_eq!(tick(&mut flow, AgentStatus::Done, 100.0), []);
        assert_eq!(tick(&mut flow, AgentStatus::Done, 170.0), [Want::FindPr(Why::Asked)]);
        answer(&mut flow, Ok(None), 170.5);
        assert_eq!(flow.tabs[&7].delivery, Delivery::Missing { commits: 2 });
        assert_eq!(tick(&mut flow, AgentStatus::Done, 400.0), [], "no third lookup");
        assert_eq!(flow.asking(), [], "Auto never asks again by itself");
        flow.ask_to_open(7);
        assert_eq!(flow.asking().len(), 1);
        assert_eq!(flow.asking()[0].by, Asker::Owner);
    }

    #[test]
    fn a_pr_found_by_the_first_lookup_ends_the_wait_and_hands_the_card_to_the_pr_strip() {
        let mut flow = asked_flow();
        tick(&mut flow, AgentStatus::Working, 11.0);
        tick(&mut flow, AgentStatus::Done, 50.0);
        let pr = PullRequest {
            number: 9,
            url: "https://github.com/o/r/pull/9".into(),
            head_sha: "h".into(),
        };
        answer(&mut flow, Ok(Some(pr)), 50.5);
        assert_eq!(flow.tabs[&7].delivery, Delivery::Hidden);
        assert_eq!(flow.tabs[&7].delivery.card(false, "Builder"), None);
        assert_eq!(flow.tabs[&7].pr.chip().map(|chip| chip.text), Some("PR #9 open".to_owned()));
    }

    #[test]
    fn a_pr_that_appears_while_the_agent_still_works_also_ends_the_wait() {
        let mut flow = asked_flow();
        tick(&mut flow, AgentStatus::Working, 11.0);
        let pr = PullRequest { number: 9, url: "u".into(), head_sha: "h".into() };
        flow.tabs.get_mut(&7).unwrap().pr.found(Ok(Some(pr)), 20.0);
        tick(&mut flow, AgentStatus::Working, 21.0);
        assert_eq!(flow.tabs[&7].delivery, Delivery::Hidden);
    }

    #[test]
    fn a_merged_branch_shuts_the_card_whatever_it_was_waiting_for() {
        let mut flow = asked_flow();
        let shut = TabFacts { gate: Gate::Shut, ..facts(AgentStatus::Done) };
        let state = flow.tabs.get_mut(&7).unwrap();
        decide(state, &shut, 20.0);
        assert_eq!(state.delivery, Delivery::Hidden);
    }

    #[test]
    fn the_wake_up_is_when_the_next_lookup_is_due() {
        let mut flow = asked_flow();
        let ctx = egui::Context::default();
        let wake = flow.tick(&[push_on()], &ctx, 11.0, 0);
        assert_eq!(wake, Some(10.0 + crate::delivery_card::TYPED_GRACE_SECS));
    }

    #[test]
    fn the_dev_switches_show_the_asked_and_missing_cards_without_an_agent() {
        let mut flow = ready_tab(&push_on(), 3);
        flow.show_asked(7, 1.0);
        assert_eq!(
            flow.tabs[&7].delivery.card(false, "Builder").map(|card| card.text),
            Some("Asked Builder to open the pull request".to_owned())
        );
        let mut missing = ready_tab(&push_on(), 3);
        missing.show_missing(7);
        let card = missing.tabs[&7].delivery.card(false, "Builder").unwrap();
        assert_eq!(
            (card.text.as_str(), card.button.as_deref()),
            ("No pull request yet", Some("Ask again"))
        );
        assert_eq!(missing.asking(), [], "forcing a card sends nothing");
    }

    fn waiting(tests: TestsGate) -> TabFacts {
        TabFacts { tests, ..push_on() }
    }

    #[test]
    fn auto_push_holds_back_until_the_tests_come_back_clear() {
        for (tests, asked) in
            [(TestsGate::Pending, 0), (TestsGate::Failed, 0), (TestsGate::Clear, 1)]
        {
            let facts = waiting(tests);
            let mut flow = ready_tab(&facts, 2);
            flow.ask_on_its_own(&facts);
            assert_eq!(flow.asking().len(), asked, "{tests:?}");
        }
    }

    #[test]
    fn only_a_ready_tab_whose_push_waits_on_the_tests_needs_a_run() {
        let pending = waiting(TestsGate::Pending);
        let flow = ready_tab(&pending, 2);
        assert_eq!(flow.waiting_for_tests(std::slice::from_ref(&pending)), [7]);
        let cases = [
            TabFacts { tests: TestsGate::Clear, ..pending.clone() },
            TabFacts { tests: TestsGate::Failed, ..pending.clone() },
            TabFacts { auto_push: false, ..pending.clone() },
            TabFacts { id: 99, ..pending.clone() },
        ];
        for facts in cases {
            assert_eq!(
                flow.waiting_for_tests(std::slice::from_ref(&facts)),
                Vec::<u64>::new(),
                "{facts:?}"
            );
        }
        let nothing_to_push = Flow::new(Tools::default());
        assert_eq!(nothing_to_push.waiting_for_tests(&[pending]), Vec::<u64>::new());
    }
}
