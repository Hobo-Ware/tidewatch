//! The terminal's font: loading the family a `FontChoice` names and the egui family the grid draws
//! with. The family lives under its own name, so the chrome's monospace text keeps the bundled face.

use egui::{FontDefinitions, FontFamily, FontId};
use stdusk_egui::fonts::{BOLD_FONT_FAMILY, installed_families, resolve_bold_font, resolve_font};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::FontChoice;

use crate::jobs::Jobs;
use crate::prefs_save::update_in_background;

/// The egui family the terminal grid draws with: the picked face first, then the bundled monospace
/// chain, so a glyph the face misses still finds a fallback.
pub(crate) const FAMILY: &str = "tidewatch-terminal";

static FAMILIES: std::sync::OnceLock<&'static [String]> = std::sync::OnceLock::new();

/// The installed families once `Host::start`'s thread has read them, empty until then, so a frame
/// never waits on the system font scan.
pub(crate) fn families() -> &'static [String] {
    FAMILIES.get().copied().unwrap_or(&[])
}

/// What the terminal draws with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InUse {
    BuiltIn,
    /// An installed family; `bold` when it ships a real bold face.
    Family {
        name: String,
        bold: bool,
    },
    /// The chosen family is not installed, so the bundled face draws.
    Missing(String),
}

impl InUse {
    pub(crate) fn font_ids(&self, size: f32) -> (FontId, Option<FontId>) {
        let regular = FontId::new(size, FontFamily::Name(FAMILY.into()));
        let bold = matches!(self, Self::Family { bold: true, .. })
            .then(|| FontId::new(size, FontFamily::Name(BOLD_FONT_FAMILY.into())));
        (regular, bold)
    }
}

pub(crate) struct Loaded {
    pub(crate) fonts: FontDefinitions,
    pub(crate) in_use: InUse,
    pub(crate) choice: FontChoice,
}

/// Reads the picked face from disk: run it before the first frame or on a job thread, never inside
/// a frame.
pub(crate) fn load(choice: FontChoice) -> Loaded {
    let resolved = match &choice {
        FontChoice::BuiltIn => None,
        FontChoice::Family(name) => {
            resolve_font(name).map(|regular| (regular, resolve_bold_font(name)))
        }
    };
    let in_use = match (&choice, &resolved) {
        (FontChoice::BuiltIn, _) => InUse::BuiltIn,
        (FontChoice::Family(name), Some((_, bold))) => {
            InUse::Family { name: name.clone(), bold: bold.is_some() }
        }
        (FontChoice::Family(name), None) => InUse::Missing(name.clone()),
    };
    let fonts = match resolved {
        Some((regular, bold)) => crate::fonts::assemble(Some(regular), bold),
        None => crate::fonts::assemble(None, None),
    };
    Loaded { fonts, in_use, choice }
}

/// Moves `picked` (the key `build_fonts` put first in Monospace) out of Monospace into `FAMILY`
/// ahead of the rest of the chain. Without a pick `FAMILY` is the plain Monospace chain, so the
/// grid's `FontId` always names a registered family.
pub(crate) fn split(fonts: &mut FontDefinitions, picked: Option<&str>) {
    let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
    monospace.retain(|key| Some(key.as_str()) != picked);
    let terminal: Vec<String> =
        picked.map(str::to_owned).into_iter().chain(monospace.iter().cloned()).collect();
    fonts.families.insert(FontFamily::Name(FAMILY.into()), terminal);
}

/// The app's side: the font in use, and a pick from Settings loaded on a job thread and applied
/// when it is back (a newer pick wins over an older one still loading).
pub(crate) struct Host {
    choice: FontChoice,
    in_use: InUse,
    jobs: Jobs<Loaded>,
    loading: bool,
}

impl Host {
    /// Loads the saved font, and reads the system font list on a thread for Settings.
    pub(crate) fn start(ctx: &egui::Context, choice: FontChoice) -> Self {
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = FAMILIES.set(installed_families());
            repaint.request_repaint();
        });
        let loaded = load(choice);
        ctx.set_fonts(loaded.fonts);
        Self { choice: loaded.choice, in_use: loaded.in_use, jobs: Jobs::new(), loading: false }
    }

    pub(crate) fn choose(
        &mut self,
        ctx: &egui::Context,
        paths: Option<&TidewatchPaths>,
        choice: FontChoice,
    ) {
        if choice == self.choice {
            return;
        }
        if let Some(paths) = paths.cloned() {
            let saved = choice.clone();
            update_in_background(paths, move |p| p.terminal.font = saved);
        }
        self.choice = choice.clone();
        self.loading = true;
        self.jobs.spawn(ctx, move || load(choice));
    }

    pub(crate) fn drain(&mut self, ctx: &egui::Context) {
        for loaded in self.jobs.drain() {
            if loaded.choice == self.choice {
                ctx.set_fonts(loaded.fonts);
                self.in_use = loaded.in_use;
                self.loading = false;
            }
        }
    }

    pub(crate) fn choice(&self) -> &FontChoice {
        &self.choice
    }

    pub(crate) fn in_use(&self) -> &InUse {
        &self.in_use
    }

    pub(crate) fn status(&self) -> Option<Status> {
        status(&self.choice, &self.in_use, self.loading)
    }
}

/// The line under the Font row, when there is something to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Fine(String),
    Warn(String),
}

pub(crate) fn status(choice: &FontChoice, in_use: &InUse, loading: bool) -> Option<Status> {
    if loading {
        return Some(Status::Fine("Loading the font...".to_owned()));
    }
    match (choice, in_use) {
        (_, InUse::Missing(name)) => {
            Some(Status::Warn(format!("{name} is not installed, so the built-in font draws.")))
        }
        _ => None,
    }
}

/// The text a choice shows on its button.
pub(crate) fn label(choice: &FontChoice) -> &str {
    match choice {
        FontChoice::BuiltIn => "Built-in",
        FontChoice::Family(name) => name,
    }
}

/// Indices of the families whose name contains `query`, case-insensitive.
pub(crate) fn filter(families: &[String], query: &str) -> Vec<usize> {
    let query = query.trim().to_ascii_lowercase();
    families
        .iter()
        .enumerate()
        .filter(|(_, name)| query.is_empty() || name.to_ascii_lowercase().contains(&query))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_terminal_family_starts_with_the_pick_and_monospace_loses_it() {
        let mut fonts = FontDefinitions::default();
        let plain = fonts.families[&FontFamily::Monospace].clone();
        fonts.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "user-font".to_owned());
        split(&mut fonts, Some("user-font"));
        assert_eq!(fonts.families[&FontFamily::Monospace], plain);
        let terminal = &fonts.families[&FontFamily::Name(FAMILY.into())];
        assert_eq!(terminal[0], "user-font");
        assert_eq!(terminal[1..], plain[..]);
    }

    #[test]
    fn without_a_pick_the_terminal_family_is_the_monospace_chain() {
        let mut fonts = FontDefinitions::default();
        split(&mut fonts, None);
        assert_eq!(
            fonts.families[&FontFamily::Name(FAMILY.into())],
            fonts.families[&FontFamily::Monospace]
        );
    }

    #[test]
    fn only_a_family_with_a_real_bold_face_gets_a_bold_font_id() {
        let cases = [
            (InUse::BuiltIn, false),
            (InUse::Missing("Nope".to_owned()), false),
            (InUse::Family { name: "A".to_owned(), bold: false }, false),
            (InUse::Family { name: "A".to_owned(), bold: true }, true),
        ];
        for (in_use, bold) in cases {
            let (regular, bold_id) = in_use.font_ids(13.0);
            assert_eq!(regular.family, FontFamily::Name(FAMILY.into()));
            assert_eq!(bold_id.is_some(), bold, "{in_use:?}");
        }
    }

    #[test]
    fn a_real_installed_family_draws_the_terminal_and_leaves_the_chrome_on_the_bundled_face() {
        let plain = crate::fonts::definitions().families[&FontFamily::Monospace].clone();
        let Some(name) = ["Menlo", "DejaVu Sans Mono", "Liberation Mono"]
            .into_iter()
            .find(|name| installed_families().iter().any(|family| family == name))
        else {
            return;
        };
        let loaded = load(FontChoice::Family(name.to_owned()));
        assert!(matches!(&loaded.in_use, InUse::Family { name: used, .. } if used == name));
        let terminal = &loaded.fonts.families[&FontFamily::Name(FAMILY.into())];
        assert!(!plain.contains(&terminal[0]), "{terminal:?}");
        let monospace = &loaded.fonts.families[&FontFamily::Monospace];
        assert!(!monospace.contains(&terminal[0]), "{monospace:?}");
        let missing = load(FontChoice::Family("No Such Font Family".to_owned()));
        assert_eq!(missing.in_use, InUse::Missing("No Such Font Family".to_owned()));
    }

    #[test]
    fn the_status_line_shows_only_while_loading_or_when_the_family_is_missing() {
        let family = FontChoice::Family("Nope".to_owned());
        let hack = InUse::Family { name: "Nope".to_owned(), bold: true };
        let cases = [
            (FontChoice::BuiltIn, InUse::BuiltIn, false, None),
            (family.clone(), hack.clone(), false, None),
            (family.clone(), hack, true, Some(Status::Fine("Loading the font...".to_owned()))),
            (
                family.clone(),
                InUse::Missing("Nope".to_owned()),
                false,
                Some(Status::Warn("Nope is not installed, so the built-in font draws.".to_owned())),
            ),
        ];
        for (choice, in_use, loading, want) in cases {
            assert_eq!(status(&choice, &in_use, loading), want, "{choice:?} {in_use:?}");
        }
        assert_eq!(label(&FontChoice::BuiltIn), "Built-in");
        assert_eq!(label(&family), "Nope");
    }

    #[test]
    fn the_filter_matches_any_part_of_the_name_ignoring_case() {
        let families = names(&["Hack Nerd Font Mono", "MesloLGS NF", "Noto Sans"]);
        assert_eq!(filter(&families, ""), [0, 1, 2]);
        assert_eq!(filter(&families, " nf"), [1]);
        assert_eq!(filter(&families, "NERD"), [0]);
        assert_eq!(filter(&families, "zzz"), Vec::<usize>::new());
    }
}
