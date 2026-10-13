//! The menu-bar item (macOS): a template icon macOS tints for light and dark bars, with Show/Hide,
//! Overview, Settings and Quit. Clicks arrive through a handler that sends them down a channel and
//! wakes the UI, so one is noticed even while the window is idle or parked.
//!
//! macOS only: `tray-icon` needs a GTK event loop on Linux, which this app does not run. Other
//! platforms get an item that is never built.

use std::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) enum Click {
    ShowHide,
    Overview,
    Settings,
    Quit,
}

#[cfg(any(target_os = "macos", test))]
const SHOW_ID: &str = "tidewatch.show";
#[cfg(any(target_os = "macos", test))]
const OVERVIEW_ID: &str = "tidewatch.overview";
#[cfg(any(target_os = "macos", test))]
const SETTINGS_ID: &str = "tidewatch.settings";
#[cfg(any(target_os = "macos", test))]
const QUIT_ID: &str = "tidewatch.quit";

#[cfg(any(target_os = "macos", test))]
fn click_for(id: &str) -> Option<Click> {
    match id {
        SHOW_ID => Some(Click::ShowHide),
        OVERVIEW_ID => Some(Click::Overview),
        SETTINGS_ID => Some(Click::Settings),
        QUIT_ID => Some(Click::Quit),
        _ => None,
    }
}

/// Dropping it removes the item from the menu bar.
#[cfg(target_os = "macos")]
pub(crate) struct Tray {
    _icon: tray_icon::TrayIcon,
}

/// Builds the item, or `None` when the platform will not have it. `clicks` and `wake` are wired in
/// once: the menu handler is process-wide and keeps the first pair it is given.
#[cfg(target_os = "macos")]
pub(crate) fn build(
    label: &str,
    clicks: Sender<Click>,
    wake: impl Fn() + Send + Sync + 'static,
) -> Option<Tray> {
    use std::sync::Mutex;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    let data =
        eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/tray.png")).ok()?;
    let icon = Icon::from_rgba(data.rgba, data.width, data.height).ok()?;
    let menu = Menu::new();
    menu.append(&MenuItem::with_id(SHOW_ID, "Show / Hide", true, None)).ok()?;
    menu.append(&MenuItem::with_id(OVERVIEW_ID, "Overview", true, None)).ok()?;
    menu.append(&MenuItem::with_id(SETTINGS_ID, "Settings...", true, None)).ok()?;
    menu.append(&PredefinedMenuItem::separator()).ok()?;
    menu.append(&MenuItem::with_id(QUIT_ID, "Quit Tidewatch", true, None)).ok()?;
    let clicks = Mutex::new(clicks);
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(click) = click_for(&event.id.0)
            && let Ok(clicks) = clicks.lock()
        {
            let _ = clicks.send(click);
            wake();
        }
    }));
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_tooltip(format!("Tidewatch: {label}"))
        .with_menu_on_left_click(true)
        .build()
        .ok()?;
    Some(Tray { _icon: icon })
}

#[cfg(not(target_os = "macos"))]
pub(crate) enum Tray {}

#[cfg(not(target_os = "macos"))]
pub(crate) fn build(
    _label: &str,
    _clicks: Sender<Click>,
    _wake: impl Fn() + Send + Sync + 'static,
) -> Option<Tray> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_menu_id_maps_to_its_click_and_a_stranger_to_none() {
        let cases = [
            (SHOW_ID, Some(Click::ShowHide)),
            (OVERVIEW_ID, Some(Click::Overview)),
            (SETTINGS_ID, Some(Click::Settings)),
            (QUIT_ID, Some(Click::Quit)),
            ("quit", None),
            ("", None),
        ];
        for (id, want) in cases {
            assert_eq!(click_for(id), want, "{id}");
        }
    }

    #[test]
    fn the_tray_icon_is_a_real_png_of_a_menu_bar_size() {
        let data =
            eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/tray.png")).unwrap();
        assert!(data.width <= 64 && data.height <= 64 && data.width > 0);
    }
}
