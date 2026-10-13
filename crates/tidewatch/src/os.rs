//! Everything the window shell asks of the operating system, behind one trait. The real one
//! registers the global hotkey, builds the menu-bar item and calls the macOS window primitives;
//! the tests use a recorder, which is how "a background run touches nothing" is checked.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::Rect;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::hotkey::Chord;
use crate::macos;
use crate::tray::{self, Click, Tray};

pub(crate) trait Os {
    fn register_hotkey(&mut self, chord: Chord) -> Result<(), String>;
    fn unregister_hotkey(&mut self);
    /// Builds the menu-bar item (named for the window's folder) or removes it. Returns whether an
    /// item exists afterwards.
    fn show_tray(&mut self, on: bool, label: &str) -> bool;
    fn tray_clicks(&mut self) -> Vec<Click>;
    fn set_dock_icon(&mut self, visible: bool);
    fn set_all_spaces(&mut self, on: bool);
    fn set_alpha(&mut self, alpha: f64);
    fn app_active(&self) -> bool;
    fn cursor_screen(&self) -> Option<Rect>;
    fn window_screen(&self) -> Option<Rect>;
    fn primary_screen(&self) -> Option<Rect>;
    /// macOS parks a hidden window on a 2 point sliver; elsewhere hiding unmaps it.
    fn parks_when_hidden(&self) -> bool;
}

/// Called from the hotkey thread on every press.
pub(crate) type OnPress = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct RealOs {
    on_press: OnPress,
    wake: Arc<dyn Fn() + Send + Sync>,
    manager: Option<GlobalHotKeyManager>,
    registered: Option<HotKey>,
    tray: Option<Tray>,
    clicks_tx: Sender<Click>,
    clicks_rx: Receiver<Click>,
}

impl RealOs {
    pub(crate) fn new(
        on_press: impl Fn() + Send + Sync + 'static,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (clicks_tx, clicks_rx) = channel();
        Self {
            on_press: Arc::new(on_press),
            wake: Arc::new(wake),
            manager: None,
            registered: None,
            tray: None,
            clicks_tx,
            clicks_rx,
        }
    }

    fn manager(&mut self) -> Result<&GlobalHotKeyManager, String> {
        if self.manager.is_none() {
            let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
            let on_press = Arc::clone(&self.on_press);
            std::thread::spawn(move || {
                let events = GlobalHotKeyEvent::receiver();
                while let Ok(event) = events.recv() {
                    if event.state == HotKeyState::Pressed {
                        on_press();
                    }
                }
            });
            self.manager = Some(manager);
        }
        self.manager.as_ref().ok_or_else(|| "no hotkey backend".to_owned())
    }
}

impl Os for RealOs {
    fn register_hotkey(&mut self, chord: Chord) -> Result<(), String> {
        let hotkey = chord.hotkey();
        self.manager()?.register(hotkey).map_err(|e| e.to_string())?;
        self.registered = Some(hotkey);
        Ok(())
    }

    fn unregister_hotkey(&mut self) {
        if let (Some(hotkey), Some(manager)) = (self.registered.take(), &self.manager) {
            let _ = manager.unregister(hotkey);
        }
    }

    fn show_tray(&mut self, on: bool, label: &str) -> bool {
        if !on {
            self.tray = None;
        } else if self.tray.is_none() {
            let wake = Arc::clone(&self.wake);
            self.tray = tray::build(label, self.clicks_tx.clone(), move || wake());
        }
        self.tray.is_some()
    }

    fn tray_clicks(&mut self) -> Vec<Click> {
        self.clicks_rx.try_iter().collect()
    }

    fn set_dock_icon(&mut self, visible: bool) {
        macos::set_dock_icon(visible);
    }

    fn set_all_spaces(&mut self, on: bool) {
        macos::set_space_behavior(on);
    }

    fn set_alpha(&mut self, alpha: f64) {
        macos::set_window_alpha(alpha);
    }

    fn app_active(&self) -> bool {
        macos::app_is_active()
    }

    fn cursor_screen(&self) -> Option<Rect> {
        macos::cursor_screen()
    }

    fn window_screen(&self) -> Option<Rect> {
        macos::window_screen()
    }

    fn primary_screen(&self) -> Option<Rect> {
        macos::primary_screen()
    }

    fn parks_when_hidden(&self) -> bool {
        cfg!(target_os = "macos")
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    pub(crate) enum Call {
        Register(Chord),
        Unregister,
        Tray(bool),
        DockIcon(bool),
        AllSpaces(bool),
        Alpha(f64),
    }

    #[derive(Default)]
    pub(crate) struct Recorder {
        pub(crate) calls: Vec<Call>,
        pub(crate) clicks: Vec<Click>,
        pub(crate) refuse_hotkey: Option<String>,
        pub(crate) parks: bool,
        pub(crate) active: bool,
        pub(crate) cursor: Option<Rect>,
        pub(crate) window: Option<Rect>,
        pub(crate) primary: Option<Rect>,
    }

    impl Os for Recorder {
        fn register_hotkey(&mut self, chord: Chord) -> Result<(), String> {
            self.calls.push(Call::Register(chord));
            self.refuse_hotkey.clone().map_or(Ok(()), Err)
        }

        fn unregister_hotkey(&mut self) {
            self.calls.push(Call::Unregister);
        }

        fn show_tray(&mut self, on: bool, _label: &str) -> bool {
            self.calls.push(Call::Tray(on));
            on
        }

        fn tray_clicks(&mut self) -> Vec<Click> {
            std::mem::take(&mut self.clicks)
        }

        fn set_dock_icon(&mut self, visible: bool) {
            self.calls.push(Call::DockIcon(visible));
        }

        fn set_all_spaces(&mut self, on: bool) {
            self.calls.push(Call::AllSpaces(on));
        }

        fn set_alpha(&mut self, alpha: f64) {
            self.calls.push(Call::Alpha(alpha));
        }

        fn app_active(&self) -> bool {
            self.active
        }

        fn cursor_screen(&self) -> Option<Rect> {
            self.cursor
        }

        fn window_screen(&self) -> Option<Rect> {
            self.window
        }

        fn primary_screen(&self) -> Option<Rect> {
            self.primary
        }

        fn parks_when_hidden(&self) -> bool {
            self.parks
        }
    }
}
