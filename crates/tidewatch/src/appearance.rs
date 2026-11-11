//! Which palette is on screen: the Appearance setting and the system's dark mode in, one `Mode`
//! out, put on the whole app the frame it changes.

use egui::Theme;
use tidewatch_core::prefs::Appearance;

use crate::brand::{self, Mode};

/// System follows the OS and is dark when the OS does not say; Dark and Light hold.
pub(crate) fn resolve(appearance: Appearance, system: Option<Theme>) -> Mode {
    match (appearance, system) {
        (Appearance::Dark, _) | (Appearance::System, Some(Theme::Dark) | None) => Mode::Dark,
        (Appearance::Light, _) | (Appearance::System, Some(Theme::Light)) => Mode::Light,
    }
}

/// Puts the setting's palette on the app when it is not the one on screen; free when it is.
pub(crate) fn sync(ctx: &egui::Context, appearance: Appearance) {
    let want = resolve(appearance, ctx.system_theme());
    if want != brand::mode() {
        brand::apply(ctx, want);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use stdusk_egui::colors;

    use super::*;

    static GLOBAL_THEME: Mutex<()> = Mutex::new(());

    fn frame(ctx: &egui::Context, system: Option<Theme>, appearance: Appearance) {
        let raw = egui::RawInput { system_theme: system, ..egui::RawInput::default() };
        let _ = ctx.run_ui(raw, |_ui| sync(ctx, appearance));
    }

    #[test]
    fn the_setting_and_the_system_pick_the_mode() {
        let cases = [
            (Appearance::System, Some(Theme::Dark), Mode::Dark),
            (Appearance::System, Some(Theme::Light), Mode::Light),
            (Appearance::System, None, Mode::Dark),
            (Appearance::Dark, Some(Theme::Light), Mode::Dark),
            (Appearance::Light, Some(Theme::Dark), Mode::Light),
            (Appearance::Light, None, Mode::Light),
        ];
        for (appearance, system, want) in cases {
            assert_eq!(resolve(appearance, system), want, "{appearance:?} on {system:?}");
        }
    }

    #[test]
    fn choosing_light_repaints_the_brand_the_terminal_and_the_visuals_next_frame() {
        let _guard = GLOBAL_THEME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let ctx = egui::Context::default();
        frame(&ctx, Some(Theme::Dark), Appearance::Dark);
        assert_eq!(brand::text(), egui::Color32::from_rgb(0xe6, 0xe8, 0xec));
        let dark_ground = brand::ground();

        frame(&ctx, Some(Theme::Dark), Appearance::Light);
        assert_eq!(brand::mode(), Mode::Light);
        assert_eq!(brand::text(), egui::Color32::from_rgb(0x1b, 0x24, 0x30));
        assert_eq!(colors::theme().bg, brand::ground());
        assert_ne!(brand::ground(), dark_ground);
        assert!(!colors::is_dark());
        assert_eq!(ctx.global_style().visuals.override_text_color, Some(brand::text()));

        frame(&ctx, Some(Theme::Dark), Appearance::Dark);
        assert_eq!(brand::ground(), dark_ground);
        assert_eq!(colors::theme().bg, dark_ground);
        assert!(colors::is_dark());
    }

    #[test]
    fn system_switches_live_when_the_mac_does() {
        let _guard = GLOBAL_THEME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let ctx = egui::Context::default();
        frame(&ctx, Some(Theme::Dark), Appearance::System);
        assert_eq!(brand::mode(), Mode::Dark);
        frame(&ctx, Some(Theme::Light), Appearance::System);
        assert_eq!(brand::mode(), Mode::Light);
        frame(&ctx, Some(Theme::Dark), Appearance::System);
        assert_eq!(brand::mode(), Mode::Dark);
    }
}
