//! Desktop notifications for an agent that needs you, finished or failed while the window is hidden
//! or unfocused. The same mechanism stdusk uses (macOS `osascript`, Linux `notify-send`), run on a
//! thread of its own so the UI never waits on it. A tab gets at most one notification per kind of
//! change in `MIN_GAP_SECS`, so a flapping status cannot spam.

use std::collections::HashMap;
use std::process::Command;

use crate::agent_run::Change;
use crate::status_view::{Tone, reason_label};
use tidewatch_core::status::AgentStatus;

const APP_NAME: &str = "Tidewatch";
pub(crate) const MIN_GAP_SECS: f64 = 60.0;
const DETAIL_CHARS: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notification {
    pub(crate) title: String,
    pub(crate) body: String,
}

/// Lets a tab through once per `gap` for each kind of change.
pub(crate) struct RateLimiter {
    gap: f64,
    last: HashMap<(u64, Tone), f64>,
}

impl RateLimiter {
    pub(crate) fn new(gap: f64) -> Self {
        Self { gap, last: HashMap::new() }
    }

    pub(crate) fn allow(&mut self, tab: u64, tone: Tone, now: f64) -> bool {
        match self.last.get(&(tab, tone)) {
            Some(at) if now - at < self.gap => false,
            _ => {
                self.last.insert((tab, tone), now);
                true
            }
        }
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.last.retain(|(id, _), _| *id != tab);
    }
}

/// True when nobody is looking at the window: it lost focus or is minimized. `None` means the
/// platform has not said, which counts as looking.
pub(crate) fn window_is_away(focused: Option<bool>, minimized: Option<bool>) -> bool {
    focused == Some(false) || minimized == Some(true)
}

/// What to tell the user about a status change, if it is worth a notification: a tab that now needs
/// you, finished or failed.
pub(crate) fn for_change(
    tab_title: &str,
    change: &Change,
    detail: Option<&str>,
) -> Option<(Tone, Notification)> {
    let tone = crate::status_view::tone(&change.to);
    if tone == crate::status_view::tone(&change.from) {
        return None;
    }
    let what = match &change.to {
        AgentStatus::NeedsYou(reason) => format!("needs you ({})", reason_label(*reason)),
        AgentStatus::Done => "is done".to_owned(),
        AgentStatus::Failed(why) => format!("failed: {why}"),
        _ => return None,
    };
    let mut body = format!("{tab_title} {what}");
    if let Some(line) = detail.and_then(|d| d.lines().find(|l| !l.trim().is_empty())) {
        body.push('\n');
        body.push_str(&clip(line.trim(), DETAIL_CHARS));
    }
    Some((tone, Notification { title: APP_NAME.to_owned(), body }))
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(3)).collect();
    clipped.push_str("...");
    clipped
}

pub(crate) struct Notifier {
    limiter: RateLimiter,
    send: Box<dyn Fn(Notification) + Send + Sync>,
}

impl Notifier {
    pub(crate) fn new(send: impl Fn(Notification) + Send + Sync + 'static) -> Self {
        Self { limiter: RateLimiter::new(MIN_GAP_SECS), send: Box::new(send) }
    }

    pub(crate) fn desktop() -> Self {
        Self::new(|notification| post(&notification))
    }

    /// Sends the notification for `change` when the window is `away` and the tab is not over its
    /// rate limit. Returns whether it was sent.
    pub(crate) fn offer(
        &mut self,
        tab: u64,
        tab_title: &str,
        change: &Change,
        detail: Option<&str>,
        away: bool,
        now: f64,
    ) -> bool {
        if !away {
            return false;
        }
        let Some((tone, notification)) = for_change(tab_title, change, detail) else {
            return false;
        };
        if !self.limiter.allow(tab, tone, now) {
            return false;
        }
        (self.send)(notification);
        true
    }

    pub(crate) fn forget(&mut self, tab: u64) {
        self.limiter.forget(tab);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Macos,
    Linux,
    Other,
}

impl Platform {
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// A string as an AppleScript literal: quoted, with backslashes and quotes escaped and line breaks
/// turned into spaces.
pub(crate) fn applescript_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The program and arguments that show `notification`, or `None` where there is no known way.
pub(crate) fn command(
    platform: Platform,
    notification: &Notification,
) -> Option<(&'static str, Vec<String>)> {
    match platform {
        Platform::Macos => {
            let script = format!(
                "display notification {} with title {}",
                applescript_string(&notification.body),
                applescript_string(&notification.title)
            );
            Some(("osascript", vec!["-e".to_owned(), script]))
        }
        Platform::Linux => {
            Some(("notify-send", vec![notification.title.clone(), notification.body.clone()]))
        }
        Platform::Other => None,
    }
}

/// Shows the notification from a thread of its own. A missing `notify-send` is not an error.
pub(crate) fn post(notification: &Notification) {
    let Some((program, args)) = command(Platform::current(), notification) else { return };
    std::thread::spawn(move || {
        let _ = Command::new(program).args(args).status();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tidewatch_core::status::Reason;

    fn change(from: AgentStatus, to: AgentStatus) -> Change {
        Change { from, to }
    }

    fn needs() -> Change {
        change(AgentStatus::Working, AgentStatus::NeedsYou(Reason::Permission))
    }

    #[test]
    fn the_limiter_lets_a_kind_of_change_through_once_per_gap() {
        let mut limiter = RateLimiter::new(60.0);
        assert!(limiter.allow(1, Tone::Needs, 0.0));
        assert!(!limiter.allow(1, Tone::Needs, 59.9));
        assert!(limiter.allow(1, Tone::Needs, 60.0));
        assert!(!limiter.allow(1, Tone::Needs, 61.0));
    }

    #[test]
    fn the_limiter_counts_tabs_and_kinds_apart() {
        let mut limiter = RateLimiter::new(60.0);
        assert!(limiter.allow(1, Tone::Needs, 0.0));
        assert!(limiter.allow(2, Tone::Needs, 1.0));
        assert!(limiter.allow(1, Tone::Done, 2.0));
        limiter.forget(1);
        assert!(limiter.allow(1, Tone::Needs, 3.0));
        assert!(!limiter.allow(2, Tone::Needs, 3.0));
    }

    #[test]
    fn a_window_is_away_when_unfocused_or_minimized() {
        let cases = [
            (Some(true), Some(false), false),
            (Some(false), Some(false), true),
            (Some(true), Some(true), true),
            (None, None, false),
            (Some(false), None, true),
        ];
        for (focused, minimized, want) in cases {
            assert_eq!(window_is_away(focused, minimized), want, "{focused:?} {minimized:?}");
        }
    }

    #[test]
    fn needs_you_done_and_failed_are_worth_a_notification() {
        let (tone, n) = for_change("brave-otter", &needs(), Some("Bash: touch hello.txt")).unwrap();
        assert_eq!(tone, Tone::Needs);
        assert_eq!(n.title, "Tidewatch");
        assert_eq!(n.body, "brave-otter needs you (wants permission)\nBash: touch hello.txt");

        let done = change(AgentStatus::Working, AgentStatus::Done);
        assert_eq!(for_change("t", &done, None).unwrap().1.body, "t is done");

        let failed = change(AgentStatus::Working, AgentStatus::Failed("exited with code 2".into()));
        assert_eq!(for_change("t", &failed, None).unwrap().1.body, "t failed: exited with code 2");
    }

    #[test]
    fn other_changes_stay_quiet() {
        let quiet = [
            change(AgentStatus::SettingUp, AgentStatus::Working),
            change(AgentStatus::NeedsYou(Reason::Permission), AgentStatus::Working),
            change(AgentStatus::NeedsYou(Reason::Permission), AgentStatus::NeedsYou(Reason::Quiet)),
            change(AgentStatus::Done, AgentStatus::Working),
        ];
        for c in quiet {
            assert_eq!(for_change("t", &c, None), None, "{c:?}");
        }
    }

    #[test]
    fn a_long_detail_is_cut_to_its_first_line() {
        let detail = "\n  first line\nsecond line".to_owned();
        let (_, n) = for_change("t", &needs(), Some(&detail)).unwrap();
        assert!(n.body.ends_with("\nfirst line"), "{}", n.body);
        let long = "x".repeat(300);
        let (_, n) = for_change("t", &needs(), Some(&long)).unwrap();
        assert_eq!(n.body.lines().nth(1).unwrap().chars().count(), DETAIL_CHARS);
        assert!(n.body.ends_with("..."));
    }

    #[test]
    fn the_notifier_sends_only_when_away_and_under_the_limit() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&sent);
        let mut notifier = Notifier::new(move |n| sink.lock().unwrap().push(n.body));

        assert!(!notifier.offer(1, "t", &needs(), None, false, 0.0));
        assert!(notifier.offer(1, "t", &needs(), None, true, 1.0));
        assert!(!notifier.offer(1, "t", &needs(), None, true, 2.0));
        let done = change(AgentStatus::Working, AgentStatus::Done);
        assert!(notifier.offer(1, "t", &done, None, true, 3.0));
        let working = change(AgentStatus::Done, AgentStatus::Working);
        assert!(!notifier.offer(1, "t", &working, None, true, 4.0));
        assert!(notifier.offer(2, "u", &needs(), None, true, 5.0));
        assert_eq!(sent.lock().unwrap().len(), 3);
    }

    #[test]
    fn applescript_literals_survive_quotes_backslashes_and_line_breaks() {
        assert_eq!(applescript_string("plain"), "\"plain\"");
        assert_eq!(applescript_string(r#"say "hi" \ now"#), r#""say \"hi\" \\ now""#);
        assert_eq!(applescript_string("a\nb\tc"), "\"a b c\"");
        assert_eq!(applescript_string("caf\u{e9} \u{1f600}"), "\"caf\u{e9} \u{1f600}\"");
    }

    #[test]
    fn each_platform_gets_its_own_command() {
        let n = Notification { title: "Tidewatch".into(), body: "t needs you".into() };
        let (program, args) = command(Platform::Macos, &n).unwrap();
        assert_eq!(program, "osascript");
        assert_eq!(args[0], "-e");
        assert_eq!(args[1], "display notification \"t needs you\" with title \"Tidewatch\"");
        let (program, args) = command(Platform::Linux, &n).unwrap();
        assert_eq!(program, "notify-send");
        assert_eq!(args, ["Tidewatch", "t needs you"]);
        assert_eq!(command(Platform::Other, &n), None);
    }
}
