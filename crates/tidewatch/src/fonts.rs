//! The font set: stdusk's fonts plus the `semibold` family Kelp's widgets draw with.

use egui::{FontDefinitions, FontFamily};

const FALLBACK_PROBE: &str = "\u{65e5}";

pub(crate) fn definitions() -> FontDefinitions {
    with_semibold(stdusk_egui::fonts::build_fonts(None, None))
}

/// Kelp's widgets name a `semibold` family that egui panics on when it is missing. It maps to the
/// proportional fonts, so headings are regular weight until Kelp exposes its semibold face.
fn with_semibold(mut fonts: FontDefinitions) -> FontDefinitions {
    let proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.insert(FontFamily::Name(kelp_ui::fonts::SEMIBOLD.into()), proportional);
    fonts
}

/// Kelp swaps in its own complete font set the first time it meets text outside Latin ranges,
/// which would drop stdusk's icons and terminal font. Triggering that once now marks the swap as
/// done, and the set installed after it is ours.
pub(crate) fn install(ctx: &egui::Context) {
    kelp_ui::fonts::ensure_fallback(ctx, FALLBACK_PROBE);
    ctx.set_fonts(definitions());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semibold_family_exists_and_reuses_the_proportional_fonts() {
        let fonts = definitions();
        let semibold = FontFamily::Name(kelp_ui::fonts::SEMIBOLD.into());
        assert_ne!(fonts.families[&semibold], Vec::<String>::new());
        assert_eq!(fonts.families[&semibold], fonts.families[&FontFamily::Proportional]);
    }

    #[test]
    fn the_probe_text_triggers_kelps_fallback_swap() {
        assert!(kelp_ui::fonts::needs_fallback(FALLBACK_PROBE));
    }

    #[test]
    fn kelp_widgets_can_lay_out_semibold_text_with_our_fonts() {
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions());
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label(egui::RichText::new("Heading").family(kelp_ui::theme::semibold()).size(13.0));
        });
    }
}
