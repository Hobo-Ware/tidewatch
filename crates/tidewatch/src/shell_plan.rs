//! What the window shell is allowed to do, decided once from the settings and where we run.
//!
//! The one rule that matters lives here: a background run (a screenshot, `TIDEWATCH_BACKGROUND`)
//! is a plain window that registers no global hotkey, builds no menu-bar item and never activates
//! the app, whatever the settings say. Everything that reaches the OS asks the plan first.

use tidewatch_core::prefs::WindowPrefs;

use crate::hotkey::{self, Chord, Problem};

/// Where and how this process runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Env {
    pub(crate) background: bool,
    pub(crate) wayland: bool,
    pub(crate) tray_supported: bool,
}

impl Env {
    pub(crate) fn detect(background: bool) -> Self {
        Self {
            background,
            wayland: wayland_from(|name| std::env::var(name).ok()),
            tray_supported: cfg!(target_os = "macos"),
        }
    }
}

/// A Wayland session has no global hotkeys, so `tidewatch --toggle` bound to a desktop shortcut is
/// the way in there. XWayland sets `DISPLAY` too, but an X11 grab does not see keys pressed in a
/// native Wayland window.
pub(crate) fn wayland_from(var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| var(name).is_some_and(|value| !value.is_empty());
    set("WAYLAND_DISPLAY") || var("XDG_SESSION_TYPE").is_some_and(|kind| kind == "wayland")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Hotkey {
    Off,
    Register(Chord),
    Invalid(Problem),
    NoGlobalHotkeys,
}

/// How the process starts, which on macOS is its activation policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Launch {
    Normal,
    NoDockIcon,
    NoDockIconNoActivation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) quake: bool,
    pub(crate) hotkey: Hotkey,
    pub(crate) tray: bool,
    pub(crate) activates: bool,
    pub(crate) launch: Launch,
}

impl Plan {
    pub(crate) fn new(env: Env, prefs: &WindowPrefs) -> Self {
        if env.background {
            return Self {
                quake: false,
                hotkey: Hotkey::Off,
                tray: false,
                activates: false,
                launch: Launch::NoDockIconNoActivation,
            };
        }
        let hotkey = match (prefs.quake, env.wayland) {
            (false, _) => Hotkey::Off,
            (true, true) => Hotkey::NoGlobalHotkeys,
            (true, false) => {
                hotkey::parse(&prefs.hotkey).map_or_else(Hotkey::Invalid, Hotkey::Register)
            }
        };
        Self {
            quake: prefs.quake,
            hotkey,
            tray: prefs.menu_bar && env.tray_supported,
            activates: true,
            launch: if prefs.quake { Launch::NoDockIcon } else { Launch::Normal },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers};

    const X11: Env = Env { background: false, wayland: false, tray_supported: true };

    fn prefs(quake: bool, hotkey: &str, menu_bar: bool) -> WindowPrefs {
        WindowPrefs { quake, hotkey: hotkey.to_owned(), menu_bar, ..WindowPrefs::default() }
    }

    #[test]
    fn a_background_run_gets_no_hotkey_no_menu_bar_item_and_no_activation_whatever_is_saved() {
        let loud = prefs(true, "Ctrl+Alt+Grave", true);
        let plan = Plan::new(Env { background: true, ..X11 }, &loud);
        assert_eq!(
            plan,
            Plan {
                quake: false,
                hotkey: Hotkey::Off,
                tray: false,
                activates: false,
                launch: Launch::NoDockIconNoActivation,
            }
        );
    }

    #[test]
    fn the_background_gate_holds_for_every_setting_and_every_platform() {
        for quake in [true, false] {
            for menu_bar in [true, false] {
                for hotkey in ["Ctrl+Alt+Grave", "nonsense", ""] {
                    for wayland in [true, false] {
                        for tray_supported in [true, false] {
                            let env = Env { background: true, wayland, tray_supported };
                            let plan = Plan::new(env, &prefs(quake, hotkey, menu_bar));
                            assert!(
                                !plan.quake && !plan.tray && !plan.activates,
                                "{quake} {menu_bar} {hotkey:?} {wayland} {tray_supported}"
                            );
                            assert_eq!(plan.hotkey, Hotkey::Off);
                            assert_eq!(plan.launch, Launch::NoDockIconNoActivation);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_defaults_make_a_quake_window_with_the_hotkey_and_the_menu_bar_item() {
        let plan = Plan::new(X11, &WindowPrefs::default());
        let chord = Chord { mods: Modifiers::CONTROL | Modifiers::ALT, code: Code::Backquote };
        assert_eq!(plan.hotkey, Hotkey::Register(chord));
        assert!(plan.quake && plan.tray && plan.activates);
        assert_eq!(plan.launch, Launch::NoDockIcon);
    }

    #[test]
    fn a_plain_window_registers_nothing_but_keeps_the_menu_bar_item_and_the_dock_icon() {
        let plan = Plan::new(X11, &prefs(false, "Ctrl+Alt+Grave", true));
        assert!(!plan.quake && plan.tray);
        assert_eq!(plan.hotkey, Hotkey::Off);
        assert_eq!(plan.launch, Launch::Normal);
    }

    #[test]
    fn the_menu_bar_item_follows_its_switch_and_the_platform() {
        assert!(!Plan::new(X11, &prefs(true, "F13", false)).tray);
        let linux = Env { tray_supported: false, ..X11 };
        assert!(!Plan::new(linux, &prefs(true, "F13", true)).tray);
    }

    #[test]
    fn wayland_skips_the_hotkey_and_keeps_the_quake_window_for_toggle() {
        let plan = Plan::new(Env { wayland: true, ..X11 }, &WindowPrefs::default());
        assert_eq!(plan.hotkey, Hotkey::NoGlobalHotkeys);
        assert!(plan.quake);
    }

    #[test]
    fn a_hotkey_that_does_not_parse_is_kept_as_the_problem_not_replaced() {
        let plan = Plan::new(X11, &prefs(true, "Ctrl+Banana", true));
        assert_eq!(plan.hotkey, Hotkey::Invalid(Problem::UnknownKey("Banana".into())));
        assert!(plan.quake);
    }

    #[test]
    fn wayland_is_told_by_the_session() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
            }
        };
        assert!(wayland_from(env(&[("WAYLAND_DISPLAY", "wayland-0")])));
        assert!(wayland_from(env(&[("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0")])));
        assert!(!wayland_from(env(&[("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":0")])));
        assert!(!wayland_from(env(&[("WAYLAND_DISPLAY", "")])));
        assert!(!wayland_from(env(&[])));
    }
}
