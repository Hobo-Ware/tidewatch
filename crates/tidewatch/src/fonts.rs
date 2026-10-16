//! The font set: stdusk's fonts with Kelp's bundled faces and `semibold` family merged in, and the
//! terminal's own family (`terminal_font::FAMILY`).

use egui::FontDefinitions;
use stdusk_egui::fonts::ResolvedFont;

use crate::terminal_font;

const FALLBACK_PROBE: &str = "\u{65e5}";

#[cfg(test)]
/// The bundled set without the system CJK fallback, for tests that must not read system fonts.
pub(crate) fn definitions() -> FontDefinitions {
    base(None, None)
}

fn base(regular: Option<ResolvedFont>, bold: Option<ResolvedFont>) -> FontDefinitions {
    let has_user_font = regular.is_some();
    let mut fonts = stdusk_egui::fonts::build_fonts(regular, bold);
    let picked = has_user_font
        .then(|| fonts.families.get(&egui::FontFamily::Monospace)?.first().cloned())
        .flatten();
    kelp_ui::fonts::extend(&mut fonts);
    terminal_font::split(&mut fonts, picked.as_deref());
    fonts
}

/// The whole set the app runs with: `regular` (and its `bold` face) as the terminal's font, the
/// system fallback for scripts the bundled faces miss.
pub(crate) fn assemble(
    regular: Option<ResolvedFont>,
    bold: Option<ResolvedFont>,
) -> FontDefinitions {
    let mut fonts = base(regular, bold);
    kelp_ui::fonts::extend_fallback(&mut fonts, FALLBACK_PROBE);
    fonts
}

#[cfg(test)]
pub(crate) fn install(ctx: &egui::Context) {
    ctx.set_fonts(assemble(None, None));
}

#[cfg(test)]
pub(crate) fn test_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_fonts(definitions());
    ctx
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
