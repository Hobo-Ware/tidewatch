//! The Window section of General: quake on or off, the hotkey and the menu-bar item. The two
//! switches apply at once; the hotkey is recorded from a key press, checked at once and applied
//! when it parses. Pure state; `window_view` draws it.

use tidewatch_core::prefs::{Panels, WindowPrefs};

use crate::hotkey::{self, Problem};

pub(crate) struct Editor {
    saved: WindowPrefs,
    pub(crate) hotkey: String,
    pub(crate) recording: bool,
}

impl Editor {
    pub(crate) fn new(prefs: &WindowPrefs) -> Self {
        Self { saved: prefs.clone(), hotkey: prefs.hotkey.clone(), recording: false }
    }

    pub(crate) fn quake(&self) -> bool {
        self.saved.quake
    }

    pub(crate) fn menu_bar(&self) -> bool {
        self.saved.menu_bar
    }

    pub(crate) fn panels(&self) -> Panels {
        self.saved.panels
    }

    /// What is wrong with the text in the field, if anything.
    pub(crate) fn problem(&self) -> Option<Problem> {
        hotkey::parse(&self.hotkey).err()
    }

    pub(crate) fn set_quake(&mut self, on: bool) -> WindowPrefs {
        self.saved.quake = on;
        self.saved.clone()
    }

    pub(crate) fn set_menu_bar(&mut self, on: bool) -> WindowPrefs {
        self.saved.menu_bar = on;
        self.saved.clone()
    }

    pub(crate) fn set_panels(&mut self, panels: Panels) -> WindowPrefs {
        self.saved.panels = panels;
        self.saved.clone()
    }

    pub(crate) fn record(&mut self, chord: String) -> Option<WindowPrefs> {
        self.recording = false;
        self.hotkey = chord;
        self.leave_hotkey()
    }

    /// The field was left: a hotkey that parses and differs from the saved one is applied, one that
    /// does not stays in the field with its problem showing.
    pub(crate) fn leave_hotkey(&mut self) -> Option<WindowPrefs> {
        let text = self.hotkey.trim();
        if self.problem().is_some() || text == self.saved.hotkey {
            return None;
        }
        self.saved.hotkey = text.to_owned();
        self.hotkey.clone_from(&self.saved.hotkey);
        Some(self.saved.clone())
    }

    pub(crate) fn revert_hotkey(&mut self) {
        self.recording = false;
        self.hotkey.clone_from(&self.saved.hotkey);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        Editor::new(&WindowPrefs::default())
    }

    #[test]
    fn it_starts_on_the_saved_settings_with_nothing_wrong() {
        let editor = editor();
        assert!(editor.quake() && editor.menu_bar());
        assert_eq!(editor.hotkey, "Ctrl+Alt+Grave");
        assert_eq!(editor.problem(), None);
    }

    #[test]
    fn each_switch_returns_the_whole_window_settings_with_only_itself_changed() {
        let mut editor = editor();
        let off = editor.set_quake(false);
        assert_eq!(off, WindowPrefs { quake: false, ..WindowPrefs::default() });
        let both = editor.set_menu_bar(false);
        assert_eq!(both, WindowPrefs { quake: false, menu_bar: false, ..WindowPrefs::default() });
        assert!(!editor.quake() && !editor.menu_bar());
    }

    #[test]
    fn the_panels_choice_returns_the_whole_window_settings_with_only_itself_changed() {
        let mut editor = editor();
        assert_eq!(editor.panels(), Panels::Side);
        let below = editor.set_panels(Panels::Below);
        assert_eq!(below, WindowPrefs { panels: Panels::Below, ..WindowPrefs::default() });
        assert_eq!(editor.panels(), Panels::Below);
    }

    #[test]
    fn a_valid_hotkey_is_applied_when_the_field_is_left_and_trimmed() {
        let mut editor = editor();
        editor.hotkey = "  Cmd+Shift+T ".to_owned();
        assert_eq!(editor.problem(), None);
        let applied = editor.leave_hotkey().unwrap();
        assert_eq!(applied.hotkey, "Cmd+Shift+T");
        assert_eq!(editor.hotkey, "Cmd+Shift+T");
        assert_eq!(editor.leave_hotkey(), None, "nothing new the second time");
    }

    #[test]
    fn a_hotkey_that_does_not_parse_is_neither_applied_nor_thrown_away() {
        let mut editor = editor();
        editor.hotkey = "Ctrl+Banana".to_owned();
        assert_eq!(editor.problem(), Some(Problem::UnknownKey("Banana".into())));
        assert_eq!(editor.leave_hotkey(), None);
        assert_eq!(editor.hotkey, "Ctrl+Banana", "the field keeps what was typed");
        editor.hotkey = String::new();
        assert_eq!(editor.problem(), Some(Problem::Empty));
        assert_eq!(editor.leave_hotkey(), None);
    }

    #[test]
    fn escape_goes_back_to_the_hotkey_in_use() {
        let mut editor = editor();
        editor.hotkey = "Ctrl+Banana".to_owned();
        editor.revert_hotkey();
        assert_eq!(editor.hotkey, "Ctrl+Alt+Grave");
        assert_eq!(editor.problem(), None);
    }

    #[test]
    fn a_switch_carries_the_applied_hotkey_not_one_still_being_typed() {
        let mut editor = editor();
        editor.hotkey = "Ctrl+Banana".to_owned();
        assert_eq!(editor.set_quake(false).hotkey, "Ctrl+Alt+Grave");
    }

    #[test]
    fn a_recorded_chord_is_applied_at_once_when_it_parses() {
        let mut editor = editor();
        editor.recording = true;
        let applied = editor.record("Cmd+Shift+T".to_owned()).unwrap();
        assert_eq!(applied.hotkey, "Cmd+Shift+T");
        assert!(!editor.recording);
        editor.recording = true;
        assert_eq!(editor.record("Cmd+Shift+T".to_owned()), None, "the same chord changes nothing");
    }

    #[test]
    fn a_recorded_chord_that_does_not_parse_shows_its_problem_until_reverted() {
        let mut editor = editor();
        editor.recording = true;
        assert_eq!(editor.record("T".to_owned()), None);
        assert_eq!(editor.hotkey, "T");
        assert_eq!(editor.problem(), Some(Problem::NeedsModifier));
        editor.recording = true;
        editor.revert_hotkey();
        assert!(!editor.recording);
        assert_eq!(editor.hotkey, "Ctrl+Alt+Grave");
    }
}
