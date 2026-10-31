//! The window's side of the "taking long" line and its Cancel; `TIDEWATCH_DEV_SLOW_CALL=14x3`
//! lists three stand-in calls already 14 s old.

use std::sync::Once;
use std::time::{Duration, Instant};

use tidewatch_core::limited;

use crate::status_bar::slow::{self, Line};

static ARMED: Once = Once::new();

pub(super) fn line(ctx: &egui::Context) -> Option<Line> {
    ARMED.call_once(|| {
        let ctx = ctx.clone();
        limited::set_waker(move || ctx.request_repaint());
    });
    let calls = slow::calls(&limited::running(), Instant::now());
    if let Some(wait) = slow::next_look(&calls) {
        ctx.request_repaint_after(wait);
    }
    slow::pick(&calls)
}

pub(super) fn cancel(id: u64) {
    limited::cancel_call(id);
}

const FAKE_LABELS: [&str; 4] = ["git worktree add", "gh pr view", "git fetch", "git status"];

/// `14` is one call, `14x3` three.
fn parse_dev(value: &str) -> Option<(Duration, usize)> {
    let (secs, count) = value.split_once('x').unwrap_or((value, "1"));
    let secs: u64 = secs.trim().parse().ok()?;
    let count: usize =
        count.trim().parse().ok().filter(|count| (1..=FAKE_LABELS.len()).contains(count))?;
    Some((Duration::from_secs(secs), count))
}

pub(super) fn start_dev() {
    let Some((age, count)) =
        std::env::var("TIDEWATCH_DEV_SLOW_CALL").ok().as_deref().and_then(parse_dev)
    else {
        return;
    };
    let mut held: Vec<_> = FAKE_LABELS
        .iter()
        .take(count)
        .enumerate()
        .map(|(n, label)| limited::hold(label, age + Duration::from_secs(2 * (count - n) as u64)))
        .collect();
    std::thread::spawn(move || {
        while !held.is_empty() {
            held.retain(|call| !call.cancel().is_cancelled());
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dev_switch_reads_seconds_and_an_optional_count() {
        let cases = [
            ("14", Some((Duration::from_secs(14), 1))),
            ("30x3", Some((Duration::from_secs(30), 3))),
            ("30x4", Some((Duration::from_secs(30), 4))),
            ("30x0", None),
            ("30x5", None),
            ("soon", None),
            ("", None),
        ];
        for (value, want) in cases {
            assert_eq!(parse_dev(value), want, "{value:?}");
        }
    }

    #[test]
    fn the_line_names_the_oldest_call_and_its_cancel_reaches_that_call() {
        let ctx = egui::Context::default();
        let call = limited::hold("zz-hour-old-call", Duration::from_hours(1));
        let shown = line(&ctx).unwrap();
        assert!(
            shown.text.starts_with("zz-hour-old-call is taking long \u{b7} 60 min "),
            "{shown:?}"
        );
        assert!(!call.cancel().is_cancelled());
        cancel(shown.id);
        assert!(call.cancel().is_cancelled());
        drop(call);
        assert!(line(&ctx).is_none_or(|now| now.id != shown.id));
    }
}
