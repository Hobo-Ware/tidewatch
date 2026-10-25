//! The app's side of Amphetamine: the session machine in `tidewatch_core::amphetamine` driven by the
//! window, its effects carried out through the `System` seam (the password prompt, `pmset`) and the
//! `Hold` seam (the idle-sleep hold), and the safety check at launch. Calls that wait for a person
//! or a process run on `Jobs` threads. A background or fake run never reaches the machine.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tidewatch_core::amphetamine::{ADD_S, Choice, Effect, Event, State, Voice, step};
use tidewatch_core::amphetamine_lid::{
    Marker, Recovery, read_marker, recovery, remove_marker, remove_token, write_marker, write_token,
};
use tidewatch_core::amphetamine_system::{Route, System, begin_lid, end_lid, route_of};
use tidewatch_core::paths::TidewatchPaths;

use crate::amphetamine_motion::{DEACTIVATE_S, Look};
use crate::jobs::Jobs;
use crate::keep_awake_hold::Hold;
use crate::touch_id_view::{Click, TouchId};

impl Hold for Box<dyn Hold> {
    fn take(&mut self, reason: &str) -> Result<(), String> {
        (**self).take(reason)
    }
    fn release(&mut self) {
        (**self).release();
    }
    fn lost(&mut self) -> bool {
        (**self).lost()
    }
}

pub(crate) const HOLD_REASON: &str = "Tidewatch: Amphetamine";
const SETTLE_TRIES: u32 = 20;
const CONFIRM_TRIES: u32 = 6;
const SETTLE_EVERY: Duration = Duration::from_millis(if cfg!(test) { 5 } else { 500 });
const PALETTE_CHOICE: Choice = Choice::Hour;
const PINNED_ON_AGE: f64 = 30.5;
const PINNED_ACTIVATING_AGE: f64 = 0.62;
const DEV_LEFT_S: f64 = 41.5 * 60.0;
const DEV_SHORT_LEFT_S: f64 = 3.0 * 3600.0 + 12.0 * 60.0 - 30.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Dev {
    Off,
    On,
    Forever,
    Activating,
    Short,
    Notice,
    Toast(Voice),
    TouchId,
    Start,
    Setup,
    Pending,
}

impl Dev {
    pub(crate) fn parse(value: Option<&str>) -> Option<Self> {
        Some(match value? {
            "off" => Self::Off,
            "on" => Self::On,
            "forever" => Self::Forever,
            "activating" => Self::Activating,
            "short" => Self::Short,
            "notice" => Self::Notice,
            "touchid" => Self::TouchId,
            "start" => Self::Start,
            "setup" => Self::Setup,
            "pending" => Self::Pending,
            "toast" => Self::Toast(Voice::Start),
            "hour" => Self::Toast(Voice::HoursLeft(3)),
            "end" => Self::Toast(Voice::End),
            _ => return None,
        })
    }
}

/// The files Tidewatch owns for a session, in its state folder.
#[derive(Clone, Debug)]
pub(crate) struct Files {
    token: PathBuf,
    marker: PathBuf,
}

impl Files {
    pub(crate) fn of(paths: &TidewatchPaths) -> Self {
        Self { token: paths.amphetamine_token_file(), marker: paths.amphetamine_marker_file() }
    }

    fn nowhere() -> Self {
        let dir =
            std::env::temp_dir().join(format!("tidewatch-amphetamine-{}", std::process::id()));
        Self { token: dir.join("token"), marker: dir.join("marker") }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pin {
    age: f64,
    short: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    AskingPassword,
    Restoring,
    Restored,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) kind: NoticeKind,
    pub(crate) title: &'static str,
    pub(crate) body: String,
    pub(crate) foot: &'static str,
}

const FOOT: &str = "Amphetamine was still on when Tidewatch last closed, so this Mac could not sleep on battery. Nothing else changed.";

pub(crate) fn notice_for(kind: NoticeKind, why: &str) -> Notice {
    let (title, body) = match kind {
        NoticeKind::Restored => (
            "Sleep is back to normal",
            "Tidewatch found sleep turned off from last time and turned it back on.".to_owned(),
        ),
        NoticeKind::Restoring => (
            "Sleep was left off",
            "Tidewatch found sleep turned off from last time and is turning it back on.".to_owned(),
        ),
        NoticeKind::AskingPassword => (
            "Sleep was left off",
            "Tidewatch found sleep turned off from last time. macOS will ask for your password once to turn it back on."
                .to_owned(),
        ),
        NoticeKind::Failed => (
            "Sleep is still off",
            format!("Tidewatch could not turn it back on ({why}). Run sudo pmset -a disablesleep 0 in a terminal."),
        ),
    };
    Notice { kind, title, body, foot: FOOT }
}

enum Done {
    Started(Result<u32, String>),
    Stopped(u64),
    Confirmed { epoch: u64, on: bool, ok: bool },
    Checked(Recovery),
    Recovered(Result<(), String>),
    TouchIdSet(Result<(), String>),
    TouchIdRemoved(Result<(), String>),
    Routed(Route),
}

pub(crate) struct Host<H: Hold> {
    state: State,
    lid: bool,
    allowed: bool,
    system: Arc<dyn System>,
    hold: H,
    files: Files,
    wrote: bool,
    jobs: Jobs<Done>,
    voices: Vec<Voice>,
    notice: Option<Notice>,
    changed_at: f64,
    pinned: Option<Pin>,
    total: Option<f64>,
    epoch: u64,
    fading_since: Option<f64>,
    touch_id_job: Option<Click>,
    autostart: Option<Choice>,
    route: Route,
}

impl<H: Hold> Host<H> {
    pub(crate) fn new(
        system: Arc<dyn System>,
        hold: H,
        files: Option<Files>,
        lid: bool,
        allowed: bool,
    ) -> Self {
        Self {
            state: State::Off,
            lid,
            allowed,
            system,
            hold,
            wrote: files.is_some(),
            files: files.unwrap_or_else(Files::nowhere),
            jobs: Jobs::new(),
            voices: Vec::new(),
            notice: None,
            changed_at: f64::NEG_INFINITY,
            pinned: None,
            total: None,
            epoch: 0,
            fading_since: None,
            touch_id_job: None,
            autostart: None,
            route: Route::Password,
        }
    }

    /// A host showing one state for a capture, with no machine behind it.
    pub(crate) fn dev(dev: &Dev, system: Arc<dyn System>, hold: H, lid: bool, wall: f64) -> Self {
        let route = route_of(system.as_ref());
        let mut host = Self::new(system, hold, None, lid, true);
        host.route = route;
        let on = |until: Option<f64>| State::On { until, told: 0 };
        match dev {
            Dev::Off | Dev::TouchId | Dev::Setup => {}
            Dev::On | Dev::Short | Dev::Activating => {
                let left = if *dev == Dev::Short { DEV_SHORT_LEFT_S } else { DEV_LEFT_S };
                host.state = on(Some(wall + left));
                host.total = Some(if *dev == Dev::Short { 4.0 * 3600.0 } else { 3600.0 });
                let age =
                    if *dev == Dev::Activating { PINNED_ACTIVATING_AGE } else { PINNED_ON_AGE };
                host.pinned = Some(Pin { age, short: *dev == Dev::Short });
            }
            Dev::Forever => {
                host.state = on(None);
                host.pinned = Some(Pin { age: PINNED_ON_AGE, short: false });
            }
            Dev::Start | Dev::Pending => host.autostart = Some(PALETTE_CHOICE),
            Dev::Notice => host.notice = Some(notice_for(NoticeKind::Restored, "")),
            Dev::Toast(voice) => host.voices.push(voice.clone()),
        }
        host
    }

    pub(crate) fn state(&self) -> State {
        self.state
    }

    /// The Touch ID row of Settings; `None` where there is no lid mode to unlock.
    pub(crate) fn touch_id(&self) -> Option<TouchId> {
        if !self.lid {
            return None;
        }
        Some(if self.touch_id_job.is_some() {
            TouchId::Working
        } else if self.system.touch_id_installed() {
            TouchId::Installed
        } else {
            TouchId::Missing
        })
    }

    pub(crate) fn change_touch_id(&mut self, click: Click, ctx: &egui::Context) {
        if !self.allowed || !self.lid || self.touch_id_job.is_some() {
            return;
        }
        self.touch_id_job = Some(click);
        let system = Arc::clone(&self.system);
        self.jobs.spawn(ctx, move || match click {
            Click::SetUp => Done::TouchIdSet(system.install_touch_id()),
            Click::Remove => Done::TouchIdRemoved(system.remove_touch_id()),
        });
    }

    /// How the next start asks for permission, as last probed off the UI thread.
    pub(crate) fn route(&self) -> Route {
        self.route
    }

    pub(crate) fn lid(&self) -> bool {
        self.lid
    }

    pub(crate) fn pinned(&self) -> bool {
        self.pinned.is_some()
    }

    pub(crate) fn short(&self) -> bool {
        self.pinned.is_some_and(|pin| pin.short)
    }

    pub(crate) fn total(&self) -> Option<f64> {
        self.total
    }

    pub(crate) fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    pub(crate) fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    pub(crate) fn take_voices(&mut self) -> Vec<Voice> {
        std::mem::take(&mut self.voices)
    }

    /// What the tablet shows at `now` (egui's clock).
    pub(crate) fn look(&self, now: f64) -> Look {
        let age = self.pinned.map_or(now - self.changed_at, |pin| pin.age);
        match self.state {
            State::Starting { .. } => Look::Pending,
            State::On { .. } => Look::On { age },
            State::Off | State::Stopping => match self.fading_since {
                Some(since) if now - since < DEACTIVATE_S => Look::Stopping { age: now - since },
                _ => Look::Off,
            },
        }
    }

    pub(crate) fn pick(&mut self, choice: Choice, wall: f64, now: f64, ctx: &egui::Context) {
        self.event(&Event::Pick(choice), wall, now, ctx);
    }

    pub(crate) fn turn_off(&mut self, wall: f64, now: f64, ctx: &egui::Context) {
        self.event(&Event::TurnOff, wall, now, ctx);
    }

    pub(crate) fn add_hour(&mut self, wall: f64, now: f64, ctx: &egui::Context) {
        self.event(&Event::AddHour, wall, now, ctx);
    }

    /// The palette row: starts the default session, or ends the one that runs.
    pub(crate) fn toggle(&mut self, wall: f64, now: f64, ctx: &egui::Context) {
        match self.state {
            State::Off => self.pick(PALETTE_CHOICE, wall, now, ctx),
            State::On { .. } => self.turn_off(wall, now, ctx),
            State::Starting { .. } | State::Stopping => {}
        }
    }

    /// Once per frame: finished jobs, the clock, and the seconds until the next thing to show.
    pub(crate) fn tick(&mut self, wall: f64, now: f64, ctx: &egui::Context) -> Option<f64> {
        for done in self.jobs.drain() {
            self.apply_done(done, wall, now, ctx);
        }
        if let Some(choice) = self.autostart.take() {
            self.pick(choice, wall, now, ctx);
        }
        if self.pinned.is_some() {
            return None;
        }
        if self.state.is_on() && self.hold.lost() {
            eprintln!("tidewatch: the Amphetamine hold ended on its own; turning it off");
            self.event(&Event::TurnOff, wall, now, ctx);
        }
        self.event(&Event::Tick, wall, now, ctx);
        self.state.left(wall).map(|left| (left % 60.0).max(0.2))
    }

    /// The window is closing: let the helper restore sleep, which it does once the token is gone.
    pub(crate) fn shutdown(&mut self) {
        if self.pinned.is_some() || !self.state.busy() {
            return;
        }
        remove_token(&self.files.token);
        self.hold.release();
    }

    /// Only a marker this app wrote counts, so sleep turned off by hand is never touched.
    pub(crate) fn check_last_run(&self, ctx: &egui::Context) {
        if !self.allowed || !self.lid {
            return;
        }
        self.probe_route(ctx);
        let (system, marker) = (Arc::clone(&self.system), self.files.marker.clone());
        self.jobs.spawn(ctx, move || {
            let found = read_marker(&marker);
            let alive = found.as_ref().and_then(|m| m.helper).is_some_and(|pid| system.alive(pid));
            Done::Checked(recovery(found.as_ref(), system.sleep_disabled(), alive))
        });
    }

    fn probe_route(&self, ctx: &egui::Context) {
        let system = Arc::clone(&self.system);
        self.jobs.spawn(ctx, move || Done::Routed(route_of(system.as_ref())));
    }

    fn event(&mut self, event: &Event, wall: f64, now: f64, ctx: &egui::Context) {
        if !self.allowed {
            return;
        }
        if let (Event::Pick(choice), State::Off) = (event, self.state) {
            self.total = choice.seconds();
        }
        if matches!((event, self.state), (Event::AddHour, State::On { until: Some(_), .. })) {
            self.total = self.total.map(|total| total + ADD_S);
        }
        let (state, effects) = step(self.state, event, wall, self.lid);
        if std::mem::discriminant(&state) != std::mem::discriminant(&self.state) {
            self.changed_at = now;
            self.epoch += 1;
            ctx.request_repaint();
            if matches!((self.state, state), (State::On { .. }, State::Stopping)) {
                self.fading_since = Some(now);
            }
        }
        self.state = state;
        for effect in effects {
            self.apply(effect, ctx);
        }
    }

    fn apply(&mut self, effect: Effect, ctx: &egui::Context) {
        match effect {
            Effect::TakeHold => {
                if let Err(e) = self.hold.take(HOLD_REASON) {
                    eprintln!("tidewatch: Amphetamine cannot hold the computer awake ({e})");
                }
            }
            Effect::ReleaseHold => self.hold.release(),
            Effect::Say(voice) => self.voices.push(voice),
            Effect::WriteDeadline(until) => self.write_token(until),
            Effect::StartLid { until } => {
                if self.wrote {
                    write_marker(
                        &self.files.marker,
                        &Marker { pid: std::process::id(), helper: None },
                    );
                }
                self.write_token(until);
                let (system, token) = (Arc::clone(&self.system), self.files.token.clone());
                self.jobs.spawn(ctx, move || {
                    Done::Started(begin_lid(system.as_ref(), &token, std::process::id()))
                });
            }
            Effect::ConfirmOn => {
                let (system, epoch) = (Arc::clone(&self.system), self.epoch);
                self.jobs.spawn(ctx, move || Done::Confirmed {
                    epoch,
                    on: true,
                    ok: confirmed_on(system.as_ref()),
                });
            }
            Effect::Abandon => {
                remove_token(&self.files.token);
                remove_marker(&self.files.marker);
            }
            Effect::StopLid => {
                remove_token(&self.files.token);
                let (system, epoch) = (Arc::clone(&self.system), self.epoch);
                self.jobs.spawn(ctx, move || {
                    end_lid(system.as_ref());
                    Done::Stopped(epoch)
                });
            }
            Effect::ConfirmOff => {
                let (system, epoch) = (Arc::clone(&self.system), self.epoch);
                self.jobs.spawn(ctx, move || Done::Confirmed {
                    epoch,
                    on: false,
                    ok: settled(system.as_ref()),
                });
            }
        }
    }

    fn write_token(&self, until: Option<f64>) {
        if let Err(e) = write_token(&self.files.token, until) {
            eprintln!("tidewatch: cannot write {} ({e})", self.files.token.display());
        }
    }

    fn apply_done(&mut self, done: Done, wall: f64, now: f64, ctx: &egui::Context) {
        match done {
            Done::Started(Ok(helper)) => {
                if self.wrote {
                    write_marker(
                        &self.files.marker,
                        &Marker { pid: std::process::id(), helper: Some(helper) },
                    );
                }
                self.event(&Event::Started, wall, now, ctx);
                self.probe_route(ctx);
            }
            Done::Started(Err(why)) => {
                remove_token(&self.files.token);
                remove_marker(&self.files.marker);
                self.event(&Event::StartFailed(why), wall, now, ctx);
            }
            Done::Stopped(epoch) if epoch == self.epoch => {
                self.event(&Event::Stopped, wall, now, ctx);
            }
            Done::Confirmed { epoch, on, ok } if epoch == self.epoch => {
                if on {
                    if !ok {
                        self.event(&Event::Unconfirmed, wall, now, ctx);
                    }
                } else {
                    if ok {
                        remove_marker(&self.files.marker);
                    }
                    self.event(&Event::Settled(ok), wall, now, ctx);
                }
            }
            Done::Stopped(_) | Done::Confirmed { .. } => {}
            Done::Routed(route) => self.route = route,
            Done::TouchIdSet(result) => {
                self.touch_id_job = None;
                self.probe_route(ctx);
                self.voices.push(match result {
                    Ok(()) => Voice::TouchIdReady,
                    Err(why) => Voice::TouchIdFailed(why),
                });
            }
            Done::TouchIdRemoved(result) => {
                self.touch_id_job = None;
                self.probe_route(ctx);
                self.voices.push(match result {
                    Ok(()) => Voice::TouchIdGone,
                    Err(why) => Voice::TouchIdFailed(why),
                });
            }
            Done::Checked(found) => self.start_recovery(found, ctx),
            Done::Recovered(result) => {
                let notice = match result {
                    Ok(()) => notice_for(NoticeKind::Restored, ""),
                    Err(why) => {
                        eprintln!("tidewatch: could not restore sleep at launch ({why})");
                        notice_for(NoticeKind::Failed, &why)
                    }
                };
                remove_marker(&self.files.marker);
                self.notice = Some(notice);
            }
        }
    }

    fn start_recovery(&mut self, found: Recovery, ctx: &egui::Context) {
        let system = Arc::clone(&self.system);
        let token = self.files.token.clone();
        match found {
            Recovery::Nothing => {}
            Recovery::Stale => remove_marker(&self.files.marker),
            Recovery::HelperRestores => {
                self.notice = Some(notice_for(NoticeKind::Restoring, ""));
                remove_token(&token);
                self.jobs.spawn(ctx, move || {
                    Done::Recovered(
                        settled(system.as_ref())
                            .then_some(())
                            .ok_or_else(|| "the helper did not answer".to_owned()),
                    )
                });
            }
            Recovery::NeedsPassword => {
                self.notice = Some(notice_for(NoticeKind::AskingPassword, ""));
                remove_token(&token);
                self.jobs.spawn(ctx, move || Done::Recovered(system.restore_lid()));
            }
        }
    }
}

fn confirmed_on(system: &dyn System) -> bool {
    for _ in 0..CONFIRM_TRIES {
        if system.sleep_disabled() != Some(false) {
            return true;
        }
        std::thread::sleep(SETTLE_EVERY);
    }
    false
}

fn settled(system: &dyn System) -> bool {
    for _ in 0..SETTLE_TRIES {
        if system.sleep_disabled() == Some(false) {
            return true;
        }
        std::thread::sleep(SETTLE_EVERY);
    }
    false
}

#[cfg(test)]
mod tests;
