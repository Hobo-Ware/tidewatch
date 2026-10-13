//! The font set: stdusk's fonts with Kelp's bundled faces and `semibold` family merged in.

use egui::FontDefinitions;

const FALLBACK_PROBE: &str = "\u{65e5}";

pub(crate) fn definitions() -> FontDefinitions {
    let mut fonts = stdusk_egui::fonts::build_fonts(None, None);
    kelp_ui::fonts::extend(&mut fonts);
    fonts
}

pub(crate) fn install(ctx: &egui::Context) {
    let mut fonts = definitions();
    kelp_ui::fonts::extend_fallback(&mut fonts, FALLBACK_PROBE);
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::FontFamily;

    #[test]
    fn semibold_family_exists_and_falls_back_to_the_proportional_fonts() {
        let fonts = definitions();
        let semibold = &fonts.families[&FontFamily::Name(kelp_ui::fonts::SEMIBOLD.into())];
        assert_ne!(*semibold, Vec::<String>::new());
        let proportional = &fonts.families[&FontFamily::Proportional];
        let own: Vec<_> = proportional.iter().filter(|name| semibold.contains(name)).collect();
        assert_ne!(own, Vec::<&String>::new());
    }

    #[test]
    fn stdusk_fonts_survive_the_merge() {
        let plain = stdusk_egui::fonts::build_fonts(None, None);
        let merged = definitions();
        assert_eq!(merged.families[&FontFamily::Monospace], plain.families[&FontFamily::Monospace]);
        assert_eq!(
            merged.families[&FontFamily::Proportional],
            plain.families[&FontFamily::Proportional]
        );
    }

    #[test]
    fn the_probe_text_needs_a_fallback_font() {
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
