//! The app's side of the running agents of each Claude tab: a look at the session's `subagents`
//! folder on a `Jobs` thread every 2 s while the tab's agent works or a listed agent has not
//! finished, one look when the transcript is first known, and no timer otherwise.
//! `TIDEWATCH_FAKE_SUBAGENTS=1` lists four running agents and one finished for every agent tab,
//! reads and writes nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tidewatch_core::subagents::{Agent, Phase, Watch, dir_of, needs_poll, next_leave_ms};

use crate::jobs::Jobs;

const POLL_MS: i64 = 2_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub(crate) id: u64,
    pub(crate) transcript: Option<&'a Path>,
    pub(crate) working: bool,
}

struct Looked {
    tab: u64,
    dir: PathBuf,
    watch: Watch,
    agents: Vec<Agent>,
}

#[derive(Default)]
struct Tab {
    dir: Option<PathBuf>,
    watch: Option<Watch>,
    agents: Vec<Agent>,
    looked_ms: Option<i64>,
    in_flight: bool,
}

pub(crate) struct Host {
    tabs: HashMap<u64, Tab>,
    jobs: Jobs<Looked>,
    fake: bool,
    #[cfg(test)]
    looks: u32,
}

fn fake_agents(now_ms: i64) -> Vec<Agent> {
    let running = [
        ("Reading panel_host.rs sections", 253_000, 281_400),
        ("Grepping sent::now_ms usages across crates", 241_000, 281_100),
        ("Checking retarget.rs and single_instance.rs", 238_000, 272_200),
        ("Reading status_view.rs and progress_ring.rs", 232_000, 283_500),
    ];
    let make = |id: usize, what: &str, ago: i64, tokens: u64, phase: Phase| Agent {
        id: format!("fake-{id}"),
        description: what.to_owned(),
        model: Some("sonnet".into()),
        effort: Some("high".into()),
        started_ms: now_ms - ago,
        phase,
        now: what.to_owned(),
        tokens,
    };
    let mut agents: Vec<Agent> = running
        .iter()
        .enumerate()
        .map(|(id, (what, ago, tokens))| make(id, what, *ago, *tokens, Phase::Running))
        .collect();
    agents.push(make(4, "Assess app chunk 1", 245_000, 190_000, Phase::Finished(now_ms - 4_000)));
    agents
}

fn soonest(wake: Option<i64>, at: i64) -> i64 {
    wake.map_or(at, |earlier| earlier.min(at))
}

fn fake_switch() -> Option<String> {
    std::env::var("TIDEWATCH_FAKE_SUBAGENTS").ok().filter(|v| !v.is_empty() && v != "0")
}

/// Whether `TIDEWATCH_FAKE_SUBAGENTS=folded` starts every list folded, for a capture.
pub(crate) fn fake_folded() -> bool {
    fake_switch().is_some_and(|value| value == "folded")
}

impl Host {
    pub(crate) fn from_env() -> Self {
        Self::new(fake_switch().is_some())
    }

    pub(crate) fn new(fake: bool) -> Self {
        Self {
            tabs: HashMap::new(),
            jobs: Jobs::new(),
            fake,
            #[cfg(test)]
            looks: 0,
        }
    }

    pub(crate) fn agents(&self, tab: u64) -> &[Agent] {
        self.tabs.get(&tab).map_or(&[][..], |tab| tab.agents.as_slice())
    }

    pub(crate) fn running(&self, tab: u64) -> usize {
        self.agents(tab).iter().filter(|agent| agent.is_running()).count()
    }

    /// Takes the finished looks, starts the ones that are due and returns when the next frame is
    /// wanted (a look is due, or a finished agent leaves the list).
    pub(crate) fn tick(
        &mut self,
        facts: &[Facts<'_>],
        ctx: &egui::Context,
        now_ms: i64,
    ) -> Option<i64> {
        self.tabs.retain(|id, _| facts.iter().any(|fact| fact.id == *id));
        for done in self.jobs.drain() {
            let Some(tab) = self.tabs.get_mut(&done.tab) else { continue };
            tab.in_flight = false;
            if tab.dir.as_ref() == Some(&done.dir) {
                tab.watch = Some(done.watch);
                tab.agents = done.agents;
            }
        }
        let mut wake: Option<i64> = None;
        for fact in facts {
            let tab = self.tabs.entry(fact.id).or_default();
            if self.fake {
                tab.agents = fake_agents(now_ms);
                continue;
            }
            let Some(transcript) = fact.transcript else { continue };
            let dir = dir_of(transcript);
            if tab.dir.as_ref() != Some(&dir) {
                *tab = Tab { dir: Some(dir.clone()), ..Tab::default() };
            }
            let due = tab.looked_ms.is_none() || needs_poll(&tab.agents, fact.working);
            let next = tab.looked_ms.map_or(now_ms, |at| at + POLL_MS);
            if due && !tab.in_flight {
                if now_ms >= next {
                    tab.in_flight = true;
                    tab.looked_ms = Some(now_ms);
                    let mut watch = tab.watch.take().unwrap_or_default();
                    let tab_id = fact.id;
                    #[cfg(test)]
                    {
                        self.looks += 1;
                    }
                    self.jobs.spawn(ctx, move || {
                        let agents = watch.look(&dir, now_ms);
                        Looked { tab: tab_id, dir, watch, agents }
                    });
                } else {
                    wake = Some(soonest(wake, next));
                }
            }
            if let Some(leave) = next_leave_ms(&tab.agents, now_ms) {
                wake = Some(soonest(wake, leave));
            }
        }
        wake
    }
}

#[cfg(test)]
mod tests;
