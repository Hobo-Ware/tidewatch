//! Dev-only switches for checking session save and restore, read once at startup.
//!
//! - `TIDEWATCH_DEV_QUIT=<seconds>`: close the window after that long, through the same close
//!   request a click on the window's close button makes, so the quit-time save runs.
//! - `TIDEWATCH_DEV_SELECT=<n>`: select tab `n` (from 1, like Cmd+n) once, after
//!   `TIDEWATCH_DEV_SELECT_AFTER` seconds (default 4).
//! - `TIDEWATCH_DEV_SHOT=<png>` after `TIDEWATCH_DEV_SHOT_AFTER=<seconds>` (default 4): save a screenshot
//!   of the window without closing it, so a run that saves its session can still be captured (the
//!   `TIDEWATCH_SCREENSHOT` harness saves nothing).
//! - `TIDEWATCH_DEV_SAVE_DEBOUNCE_MS=<ms>`: the save debounce. A long one leaves the quit-time
//!   flush as the only write, which is how a check tells that it ran.

use std::path::PathBuf;
use std::time::Duration;

use crate::dev_screenshot::save_png;
use crate::pane::Pane;
use crate::session_writer::{Debounce, MAX_WAIT, QUIET};
use crate::tabs::{Action, Tabs};

const DEFAULT_SELECT_AFTER_SECS: f64 = 4.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevSession {
    quit_after: Option<f64>,
    select: Option<(usize, f64)>,
    shot: Option<(PathBuf, f64)>,
    asked_for_shot: bool,
    debounce_ms: Option<u64>,
}

impl DevSession {
    pub(crate) fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self::parse(
            var("TIDEWATCH_DEV_QUIT").as_deref(),
            var("TIDEWATCH_DEV_SELECT").as_deref(),
            var("TIDEWATCH_DEV_SELECT_AFTER").as_deref(),
            var("TIDEWATCH_DEV_SHOT").as_deref(),
            var("TIDEWATCH_DEV_SHOT_AFTER").as_deref(),
            var("TIDEWATCH_DEV_SAVE_DEBOUNCE_MS").as_deref(),
        )
    }

    fn parse(
        quit: Option<&str>,
        select: Option<&str>,
        select_after: Option<&str>,
        shot: Option<&str>,
        shot_after: Option<&str>,
        debounce_ms: Option<&str>,
    ) -> Self {
        Self {
            quit_after: quit.and_then(|s| s.parse().ok()),
            select: select
                .and_then(|s| s.parse::<usize>().ok())
                .and_then(|n| n.checked_sub(1))
                .map(|index| {
                    let after = select_after.and_then(|s| s.parse().ok());
                    (index, after.unwrap_or(DEFAULT_SELECT_AFTER_SECS))
                }),
            shot: shot.filter(|path| !path.is_empty()).map(|path| {
                let after = shot_after.and_then(|s| s.parse().ok());
                (PathBuf::from(path), after.unwrap_or(DEFAULT_SELECT_AFTER_SECS))
            }),
            asked_for_shot: false,
            debounce_ms: debounce_ms.and_then(|s| s.parse().ok()),
        }
    }

    fn shot_tick(&mut self, ctx: &egui::Context, now: f64) {
        let Some((path, after)) = &self.shot else { return };
        let captured = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = captured {
            save_png(path, &image);
            self.shot = None;
        } else if !self.asked_for_shot && now >= *after {
            self.asked_for_shot = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    pub(crate) fn debounce(&self) -> Debounce {
        match self.debounce_ms {
            Some(ms) => Debounce::new(Duration::from_millis(ms), Duration::from_millis(ms)),
            None => Debounce::new(QUIET, MAX_WAIT),
        }
    }

    /// Selects the tab and quits when their time comes, once each, and asks for a frame at the next
    /// deadline so an idle window still gets there.
    pub(crate) fn tick<T>(&mut self, ctx: &egui::Context, tabs: &mut Tabs<Pane<T>>, now: f64) {
        if let Some((index, after)) = self.select
            && now >= after
        {
            self.select = None;
            tabs.apply(Action::Select(index), || unreachable!());
        }
        if let Some(after) = self.quit_after
            && now >= after
        {
            self.quit_after = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.shot_tick(ctx, now);
        let pending_shot = self.shot.as_ref().filter(|_| !self.asked_for_shot).map(|(_, at)| *at);
        let next = [self.select.map(|(_, after)| after), self.quit_after, pending_shot]
            .into_iter()
            .flatten()
            .reduce(f64::min);
        if let Some(at) = next {
            ctx.request_repaint_after(Duration::from_secs_f64((at - now).max(0.0) + 0.02));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_set_means_nothing_happens_and_the_default_debounce() {
        assert_eq!(DevSession::parse(None, None, None, None, None, None), DevSession::default());
    }

    #[test]
    fn the_switches_parse_and_a_tab_number_counts_from_one() {
        let dev = DevSession::parse(
            Some("12.5"),
            Some("2"),
            Some("6"),
            Some("/tmp/x.png"),
            Some("9"),
            Some("90000"),
        );
        assert_eq!(
            dev,
            DevSession {
                quit_after: Some(12.5),
                select: Some((1, 6.0)),
                shot: Some((PathBuf::from("/tmp/x.png"), 9.0)),
                asked_for_shot: false,
                debounce_ms: Some(90_000)
            }
        );
        let late = DevSession::parse(None, Some("3"), None, None, None, None);
        assert_eq!(late.select, Some((2, DEFAULT_SELECT_AFTER_SECS)));
    }

    #[test]
    fn junk_and_tab_zero_are_ignored() {
        assert_eq!(
            DevSession::parse(Some("soon"), Some("0"), None, Some(""), None, Some("x")),
            DevSession::default()
        );
        assert_eq!(
            DevSession::parse(None, Some("two"), None, None, None, None),
            DevSession::default()
        );
    }

    #[test]
    fn the_select_happens_once_when_its_time_comes() {
        let mut dev = DevSession::parse(None, Some("2"), Some("1"), None, None, None);
        let mut tabs = Tabs::new(Pane::shell(0));
        tabs.push("a".into(), Pane::shell(1));
        let ctx = egui::Context::default();
        tabs.apply(Action::Select(0), || unreachable!());
        dev.tick(&ctx, &mut tabs, 0.5);
        assert_eq!(tabs.active(), 0, "too early");
        dev.tick(&ctx, &mut tabs, 1.2);
        assert_eq!(tabs.active(), 1);
        tabs.apply(Action::Select(0), || unreachable!());
        dev.tick(&ctx, &mut tabs, 2.0);
        assert_eq!(tabs.active(), 0, "it does not select again");
    }
}
