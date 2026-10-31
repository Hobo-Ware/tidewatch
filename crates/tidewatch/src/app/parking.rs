//! The window's side of parked tabs: the rules run on what the tabs say, parking by hand, and the
//! `TIDEWATCH_FAKE_PARKED` stand-ins for a capture.

use std::collections::HashMap;
use std::time::Duration;

use stdusk_core::terminal::PtyTerm;
use tidewatch_core::prefs::ParkAfter;

use super::strip::needs_you;
use super::{Tidewatch, wall_ms};
use crate::agent_run::AgentRun;
use crate::pane::{AgentTab, Body, Pane};
use crate::park::{
    self, Change, Facts, Fits, Line, Park, Role, Rules, Schedule, Tone, Touches, Why,
};
use crate::progress_glue;
use crate::pty::spawn_shell;
use crate::tabs::Action;

const LOOK_EVERY_MS: u64 = 1000;
const NO_LIMIT: Fits = Fits { alone: usize::MAX, with_pill: usize::MAX };

const FAKE_VISIBLE: [&str; 3] = ["backfill-orders", "fix-ci", "review-auth"];
const FAKE_PARKED: [(&str, Why, &str, Tone); 3] = [
    ("migrate-billing", Why::Job, "Running 64% \u{b7} ETA 3h 10m", Tone::Teal),
    ("flaky-tests", Why::Waiting, "Waiting until Fri 09:00", Tone::Blue),
    ("docs-sweep", Why::Idle, "Idle 6h", Tone::Muted),
];

pub(super) struct Parking {
    touches: Touches,
    after: ParkAfter,
    fits: Option<Fits>,
    last_look_ms: u64,
    seen_active: Option<u64>,
    seen_fits: Option<Fits>,
    fake: Option<Fake>,
}

enum Fake {
    Waiting { parked: bool },
    Placed(HashMap<u64, Line>),
}

impl Parking {
    pub(super) fn new(after: ParkAfter) -> Self {
        let fake = std::env::var("TIDEWATCH_FAKE_PARKED")
            .ok()
            .filter(|value| !value.is_empty() && value != "0")
            .map(|value| Fake::Waiting { parked: value != "visible" });
        Self {
            touches: Touches::default(),
            after,
            fits: None,
            last_look_ms: 0,
            seen_active: None,
            seen_fits: None,
            fake,
        }
    }

    pub(super) fn set_after(&mut self, after: ParkAfter) {
        self.after = after;
        self.last_look_ms = 0;
    }

    pub(super) fn look_again(&mut self) {
        self.last_look_ms = 0;
    }

    pub(super) fn after(&self) -> ParkAfter {
        self.after
    }

    pub(super) fn set_fits(&mut self, fits: Fits) {
        self.fits = Some(fits);
    }

    pub(super) fn fake_line(&self, tab: u64) -> Option<Line> {
        match &self.fake {
            Some(Fake::Placed(lines)) => lines.get(&tab).cloned(),
            _ => None,
        }
    }

    pub(super) fn forget(&mut self, tab: u64) {
        self.touches.forget(tab);
    }

    fn rules(&self) -> Rules {
        let after_ms = self.after.duration().map(|after| after.as_millis() as u64);
        Rules { after_ms, fits: self.fits.unwrap_or(NO_LIMIT) }
    }
}

impl Tidewatch {
    pub(super) fn drive_parking(&mut self, ctx: &egui::Context) {
        if self.welcome.is_some() {
            return;
        }
        if let Some(Fake::Waiting { parked }) = self.parking.fake {
            self.place_fake_parked(ctx, parked);
            return;
        }
        let now_ms = wall_ms() as u64;
        let active = self.tabs.items()[self.tabs.active()].id;
        self.parking.touches.observe(active, now_ms);
        if self.parking.fake.is_some() {
            return;
        }
        let moved =
            self.parking.seen_active != Some(active) || self.parking.seen_fits != self.parking.fits;
        if !moved && now_ms.saturating_sub(self.parking.last_look_ms) < LOOK_EVERY_MS {
            return;
        }
        self.parking.last_look_ms = now_ms;
        self.parking.seen_active = Some(active);
        self.parking.seen_fits = self.parking.fits;
        let facts = self.park_facts(active, now_ms);
        let rules = self.parking.rules();
        for change in park::decide(&facts, now_ms, &rules) {
            match change {
                Change::Park(id, why) => {
                    let since_ms = self.parking.touches.of(id, now_ms);
                    self.tabs.set_park(id, Park { why, since_ms });
                }
                Change::Unpark(id) => {
                    self.tabs.unpark(id);
                    self.parking.touches.set(id, now_ms);
                }
                Change::Refill(id) => self.tabs.unpark(id),
            }
        }
        if let Some(wait) = park::next_look_ms(&facts, now_ms, &rules) {
            ctx.request_repaint_after(Duration::from_millis(wait));
        }
    }

    fn park_facts(&mut self, active: u64, now_ms: u64) -> Vec<Facts> {
        let today = progress_glue::today();
        let mut facts = Vec::with_capacity(self.tabs.items().len());
        for tab in self.tabs.items() {
            let schedule = self
                .progress
                .ledger(tab.id)
                .map_or(Schedule::None, |known| park::schedule_of(&known.ledger, today));
            facts.push(Facts {
                id: tab.id,
                group: tab.group,
                active: tab.id == active,
                role: match (tab.pinned, &tab.pane.body) {
                    (true, _) => Role::Pinned,
                    (_, Body::Draft(_)) => Role::Draft,
                    _ => Role::Plain,
                },
                keep: tab.keep,
                needs_you: needs_you(&tab.pane),
                park: tab.park,
                touched_ms: self.parking.touches.of(tab.id, now_ms),
                work: park::work_of(self.job_live.board(tab.id), now_ms as i64),
                schedule,
            });
        }
        facts
    }

    pub(super) fn park_tab(&mut self, index: usize, ctx: &egui::Context) {
        let Some(tab) = self.tabs.items().get(index) else { return };
        let refusal = if tab.pinned {
            Some("The main tab stays in the strip.".to_owned())
        } else if matches!(tab.pane.body, Body::Draft(_)) {
            Some("Start or close the new tab first.".to_owned())
        } else if needs_you(&tab.pane) {
            let label = crate::tab_title::label(tab.pane.agent.as_ref(), &tab.title);
            Some(format!("{label} needs you, so it stays in the strip."))
        } else {
            None
        };
        if let Some(text) = refusal {
            self.show_toast(text, ctx);
            return;
        }
        self.tabs.park_index(index, Why::Manual, wall_ms() as u64);
    }

    pub(super) fn dispatch_parking(&mut self, action: Action, ctx: &egui::Context) -> bool {
        match action {
            Action::Park(index) => self.park_tab(index, ctx),
            Action::ParkActive => self.park_tab(self.tabs.active(), ctx),
            Action::ToggleKeep(_) => self.tabs.apply(action, || unreachable!()),
            _ => return false,
        }
        true
    }

    fn place_fake_parked(&mut self, ctx: &egui::Context, parked: bool) {
        let Some(dir) = self.workspaces.first().map(|workspace| workspace.dir().to_path_buf())
        else {
            return;
        };
        let now = ctx.input(|input| input.time);
        let now_ms = wall_ms() as u64;
        let mut lines = HashMap::new();
        let mut first = None;
        let names = FAKE_VISIBLE.into_iter().map(|name| (name, None)).chain(
            FAKE_PARKED.into_iter().map(|(name, why, text, tone)| (name, Some((why, text, tone)))),
        );
        for (name, parked_as) in names {
            let mut run = AgentRun::setting_up("claude".to_owned(), now);
            run.started(Some(dir.join("fake-parked-notes.json")), now);
            let pane = Pane {
                body: Body::Shell::<PtyTerm>(spawn_shell(ctx, &dir)),
                worktree: None,
                agent: Some(AgentTab::new("Builder".to_owned(), run)),
            };
            let id = self.tabs.push_in(self.tabs.active_group(), name.to_owned(), pane);
            first.get_or_insert(id);
            if let Some((why, text, tone)) = parked.then_some(parked_as).flatten() {
                self.tabs.set_park(id, Park { why, since_ms: now_ms });
                lines.insert(id, Line { text: text.to_owned(), tone });
            }
        }
        if let Some(id) = first {
            self.tabs.select_id(id);
        }
        self.parking.fake = parked.then_some(Fake::Placed(lines));
    }
}
