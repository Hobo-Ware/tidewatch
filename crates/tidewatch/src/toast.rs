//! A single short-lived message at the bottom of the window. Time comes from egui's clock, so the
//! fade is testable.

use stdusk_egui::ui::toast_alpha;

pub(crate) const LIFETIME_S: f64 = 3.5;
const FADE_WINDOW_S: f64 = 0.35;
const FRAME_S: f64 = 1.0 / 60.0;

#[derive(Debug, PartialEq)]
pub(crate) struct Toast {
    text: String,
    expires: f64,
}

impl Toast {
    pub(crate) fn new(text: impl Into<String>, now: f64) -> Self {
        Self { text: text.into(), expires: now + LIFETIME_S }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn remaining(&self, now: f64) -> f64 {
        (self.expires - now).max(0.0)
    }

    pub(crate) fn expired(&self, now: f64) -> bool {
        self.remaining(now) <= 0.0
    }

    /// Seconds until the toast next needs a frame: straight to the start of the fade, then one
    /// frame at a time until it is gone.
    pub(crate) fn next_frame_in(&self, now: f64) -> f64 {
        let until_fade = self.remaining(now) - FADE_WINDOW_S;
        if until_fade > 0.0 { until_fade } else { FRAME_S }
    }

    pub(crate) fn fade(&self, now: f64) -> f32 {
        toast_alpha(self.remaining(now), FADE_WINDOW_S)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lives_for_its_lifetime_then_expires() {
        let toast = Toast::new("hi", 10.0);
        assert!(!toast.expired(10.0));
        assert!(!toast.expired(10.0 + LIFETIME_S - 0.01));
        assert!(toast.expired(10.0 + LIFETIME_S));
        assert_eq!(toast.remaining(99.0), 0.0);
    }

    #[test]
    fn stays_opaque_then_fades_over_the_last_moments() {
        let toast = Toast::new("hi", 0.0);
        assert_eq!(toast.fade(1.0), 1.0);
        let fading = toast.fade(LIFETIME_S - FADE_WINDOW_S / 2.0);
        assert!(fading > 0.0 && fading < 1.0, "fade {fading}");
        assert_eq!(toast.fade(LIFETIME_S), 0.0);
    }

    #[test]
    fn sleeps_until_the_fade_then_wakes_every_frame() {
        let toast = Toast::new("hi", 0.0);
        assert!((toast.next_frame_in(0.0) - (LIFETIME_S - FADE_WINDOW_S)).abs() < 1e-9);
        assert_eq!(toast.next_frame_in(LIFETIME_S - 0.1), FRAME_S);
    }

    #[test]
    fn keeps_its_text() {
        assert_eq!(Toast::new(String::from("checkout is off"), 0.0).text(), "checkout is off");
    }
}
