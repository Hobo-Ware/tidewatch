//! The repaint a deadline needs: a little after the moment, so the frame lands past it.

use std::time::Duration;

pub(crate) const WAKE_SLACK_S: f64 = 0.02;

pub(crate) fn request_wake(ctx: &egui::Context, secs: f64) {
    ctx.request_repaint_after(wake_delay(secs));
}

fn wake_delay(secs: f64) -> Duration {
    Duration::from_secs_f64(secs + WAKE_SLACK_S)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delay_is_the_wait_plus_the_slack() {
        let cases = [(0.0, 20), (0.5, 520), (1.0, 1020), (29.98, 30_000)];
        for (secs, millis) in cases {
            assert_eq!(wake_delay(secs).as_millis(), millis, "wait {secs}");
        }
    }
}
