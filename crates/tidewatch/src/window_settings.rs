//! The Window section of General: quake on or off, the hotkey and the menu-bar item. The two
//! switches apply at once; the hotkey text is checked as you type and applied when you leave the
//! field, and only when it parses. Pure state; `window_view` draws it.

use tidewatch_core::prefs::WindowPrefs;

use crate::hotkey::{self, Problem};

pub(crate) struct Editor {
    saved: WindowPrefs,
    pub(crate) hotkey: String,
}

impl Editor {
    pub(crate) fn new(prefs: &WindowPrefs) -> Self {
        Self { saved: prefs.clone(), hotkey: prefs.hotkey.clone() }
    }

    pub(crate) fn quake(&self) -> bool {
        self.saved.quake
    }

    pub(crate) fn menu_bar(&self) -> bool {
        self.saved.menu_bar
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

    /// Escape: the field goes back to the hotkey in use.
    pub(crate) fn revert_hotkey(&mut self) {
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
}
