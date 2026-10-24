//! macOS window primitives for the quake drop-down: the activation policy, Spaces, window alpha and
//! the screen lookups. Every entry point has a no-op stub off macOS, so the rest of the app calls
//! them unconditionally. The quake rules these serve are in `.agents/rules/platform.md`.

use egui::Rect;

/// Dock icon and app-switcher entry on (Regular) or off (Accessory).
#[cfg(target_os = "macos")]
pub(crate) fn set_dock_icon(visible: bool) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        let policy = if visible {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        };
        NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn set_dock_icon(_visible: bool) {}

/// Joins every Space and drops over full-screen apps, so summoning lands on the desktop in use.
#[cfg(target_os = "macos")]
pub(crate) fn set_space_behavior(all_spaces: bool) {
    use objc2_app_kit::{NSApplication, NSWindowCollectionBehavior};
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        let behavior = if all_spaces {
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
        } else {
            NSWindowCollectionBehavior::Default
        };
        let windows = NSApplication::sharedApplication(mtm).windows();
        for i in 0..windows.count() {
            windows.objectAtIndex(i).setCollectionBehavior(behavior);
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn set_space_behavior(_all_spaces: bool) {}

/// 0.0 hides the parked sliver without ordering the window out, which would park the run loop.
#[cfg(target_os = "macos")]
pub(crate) fn set_window_alpha(alpha: f64) {
    use objc2_app_kit::NSApplication;
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let windows = NSApplication::sharedApplication(mtm).windows();
    for i in 0..windows.count() {
        windows.objectAtIndex(i).setAlphaValue(alpha);
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn set_window_alpha(_alpha: f64) {}

/// Whether the whole app is frontmost. It stays true while a system panel (the emoji viewer) holds
/// the key window, which is why hide-on-blur asks this and not the window's own focus.
#[cfg(target_os = "macos")]
pub(crate) fn app_is_active() -> bool {
    objc2::MainThreadMarker::new()
        .is_some_and(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isActive())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn app_is_active() -> bool {
    false
}

/// Where the "Open at login" item stands. `login_item_status` is the only source of truth: the
/// user can also change the item in System Settings, so Tidewatch stores nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoginItem {
    Off,
    On,
    NeedsApproval,
    NotFound,
    Unavailable,
}

/// Maps `SMAppServiceStatus` (0 not registered, 1 enabled, 2 requires approval, 3 not found).
/// Not found is recoverable by registering; any other value is unavailable.
pub(crate) fn login_item_from_raw(raw: isize) -> LoginItem {
    match raw {
        0 => LoginItem::Off,
        1 => LoginItem::On,
        2 => LoginItem::NeedsApproval,
        3 => LoginItem::NotFound,
        _ => LoginItem::Unavailable,
    }
}

pub(crate) const LOGIN_ITEM_OFFERED: bool = cfg!(target_os = "macos");

pub(crate) fn login_item_status() -> LoginItem {
    login_item_raw().map_or(LoginItem::Unavailable, login_item_from_raw)
}

/// Whether `exe` sits in an `.app` bundle, the only thing macOS can register as a login item.
#[cfg(any(target_os = "macos", test))]
fn in_app_bundle(exe: &std::path::Path) -> bool {
    exe.ancestors().nth(3).and_then(std::path::Path::extension).is_some_and(|ext| ext == "app")
}

/// `SMAppService` arrived in macOS 13 and objc2's class cache assumes the class exists, so every
/// entry point asks the runtime first. Outside an `.app` bundle there is nothing to register.
#[cfg(target_os = "macos")]
fn login_item_available() -> bool {
    std::env::current_exe().is_ok_and(|exe| in_app_bundle(&exe))
        && objc2::runtime::AnyClass::get(c"SMAppService").is_some()
}

/// The ObjC methods are `unsafe fn` only because every generated binding is: this one takes no
/// pointers and has no preconditions.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn login_item_raw() -> Option<isize> {
    use objc2_service_management::SMAppService;
    login_item_available().then(|| unsafe { SMAppService::mainAppService().status().0 })
}
#[cfg(not(target_os = "macos"))]
fn login_item_raw() -> Option<isize> {
    None
}

/// Registers (`true`) or unregisters the app as a login item. The error is the OS text, for a toast.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub(crate) fn set_login_item(enabled: bool) -> Result<(), String> {
    use objc2_service_management::SMAppService;
    if !login_item_available() {
        return Err("Open at login needs the installed app".to_owned());
    }
    let result = unsafe {
        let service = SMAppService::mainAppService();
        if enabled { service.registerAndReturnError() } else { service.unregisterAndReturnError() }
    };
    result.map_err(|e| e.localizedDescription().to_string())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn set_login_item(_enabled: bool) -> Result<(), String> {
    Err("Open at login needs macOS".to_owned())
}

/// Opens System Settings > General > Login Items, where the user approves a pending item.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub(crate) fn open_login_items_settings() {
    if login_item_available() {
        unsafe { objc2_service_management::SMAppService::openSystemSettingsLoginItems() };
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn open_login_items_settings() {}

#[cfg(target_os = "macos")]
fn screen_frames(mtm: objc2::MainThreadMarker) -> Vec<Rect> {
    let screens = objc2_app_kit::NSScreen::screens(mtm);
    (0..screens.count()).map(|i| ns_rect(screens.objectAtIndex(i).frame())).collect()
}

#[cfg(target_os = "macos")]
fn ns_rect(r: objc2_foundation::NSRect) -> Rect {
    Rect::from_min_size(
        egui::pos2(r.origin.x as f32, r.origin.y as f32),
        egui::vec2(r.size.width as f32, r.size.height as f32),
    )
}

/// The screen under the mouse, in the space `OuterPosition` takes.
#[cfg(target_os = "macos")]
pub(crate) fn cursor_screen() -> Option<Rect> {
    let mtm = objc2::MainThreadMarker::new()?;
    let frames = screen_frames(mtm);
    let mouse = objc2_app_kit::NSEvent::mouseLocation();
    let at = screen_index_at(&frames, egui::pos2(mouse.x as f32, mouse.y as f32))?;
    Some(cocoa_to_top_left(frames[at], frames.first()?.height()))
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn cursor_screen() -> Option<Rect> {
    None
}

/// The primary screen: the last resort, so the hidden sliver still lands on a live screen.
#[cfg(target_os = "macos")]
pub(crate) fn primary_screen() -> Option<Rect> {
    let frame = *screen_frames(objc2::MainThreadMarker::new()?).first()?;
    Some(cocoa_to_top_left(frame, frame.height()))
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn primary_screen() -> Option<Rect> {
    None
}

/// The screen the window is on, so hiding parks the sliver where the window already is.
#[cfg(target_os = "macos")]
pub(crate) fn window_screen() -> Option<Rect> {
    use objc2_app_kit::NSApplication;
    let mtm = objc2::MainThreadMarker::new()?;
    let windows = NSApplication::sharedApplication(mtm).windows();
    let window = (0..windows.count())
        .map(|i| windows.objectAtIndex(i))
        .find(|window| window.canBecomeKeyWindow())?;
    let frame = ns_rect(window.screen()?.frame());
    Some(cocoa_to_top_left(frame, screen_frames(mtm).first()?.height()))
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn window_screen() -> Option<Rect> {
    None
}

/// Index of the screen holding `point`. Both are in AppKit space (bottom-left origin, y up).
/// Half-open, so a point on a shared edge belongs to the screen it is the first column of.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn screen_index_at(frames: &[Rect], point: egui::Pos2) -> Option<usize> {
    frames.iter().position(|f| {
        f.min.x <= point.x && point.x < f.max.x && f.min.y <= point.y && point.y < f.max.y
    })
}

/// An AppKit frame (bottom-left origin, anchored to the primary screen) in the top-left space that
/// `OuterPosition` uses. `primary_h` is the primary screen's height.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn cocoa_to_top_left(frame: Rect, primary_h: f32) -> Rect {
    Rect::from_min_size(egui::pos2(frame.min.x, primary_h - frame.max.y), frame.size())
}

use std::path::PathBuf;

use tidewatch_core::image_paste::{Clip, clip_of};

use crate::image_paste::{Clipboard, Picture};

#[cfg(target_os = "macos")]
fn pasteboard_types() -> Vec<String> {
    let board = objc2_app_kit::NSPasteboard::generalPasteboard();
    board.types().map(|types| types.iter().map(|t| t.to_string()).collect()).unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn has_any(types: &[String], wanted: &[&str]) -> bool {
    types.iter().any(|t| wanted.contains(&t.as_str()))
}

/// The general pasteboard. `clip` looks at the type list only, so it is cheap enough for the key
/// monitor on the main thread; the picture itself is decoded later on a job thread.
pub(crate) struct SystemClipboard;

#[cfg(target_os = "macos")]
impl Clipboard for SystemClipboard {
    fn clip(&mut self) -> Clip {
        let types = pasteboard_types();
        let files =
            if has_any(&types, &["public.file-url"]) { self.image_files().len() } else { 0 };
        let text = has_any(&types, &["public.utf8-plain-text", "NSStringPboardType"]);
        let image = has_any(&types, &["public.png", "public.tiff", "public.jpeg"]);
        clip_of(text, image, files)
    }

    fn picture(&mut self) -> Option<Picture> {
        let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
        Some(Picture {
            width: u32::try_from(image.width).ok()?,
            height: u32::try_from(image.height).ok()?,
            rgba: image.bytes.into_owned(),
        })
    }

    fn image_files(&mut self) -> Vec<PathBuf> {
        let Ok(files) = arboard::Clipboard::new().and_then(|mut board| board.get().file_list())
        else {
            return Vec::new();
        };
        files.into_iter().filter(|path| tidewatch_core::image_paste::is_image_file(path)).collect()
    }
}

#[cfg(not(target_os = "macos"))]
impl Clipboard for SystemClipboard {
    fn clip(&mut self) -> Clip {
        clip_of(false, false, 0)
    }

    fn picture(&mut self) -> Option<Picture> {
        None
    }

    fn image_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
}

pub(crate) fn clip_has_picture(clipboard: &mut dyn Clipboard) -> bool {
    matches!(clipboard.clip(), Clip::Image | Clip::ImageFiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    #[test]
    fn the_screen_under_the_cursor_is_found_and_a_shared_edge_belongs_to_the_right_one() {
        let frames = [rect(0.0, 0.0, 1440.0, 900.0), rect(1440.0, -180.0, 1920.0, 1080.0)];
        assert_eq!(screen_index_at(&frames, pos2(100.0, 100.0)), Some(0));
        assert_eq!(screen_index_at(&frames, pos2(2000.0, 500.0)), Some(1));
        assert_eq!(screen_index_at(&frames, pos2(1440.0, 100.0)), Some(1));
        assert_eq!(screen_index_at(&frames, pos2(5000.0, 100.0)), None);
    }

    #[test]
    fn cocoa_frames_become_top_left_frames_anchored_to_the_primary_screen() {
        let primary = rect(0.0, 0.0, 1440.0, 900.0);
        assert_eq!(cocoa_to_top_left(primary, 900.0).min, pos2(0.0, 0.0));
        let right = rect(1440.0, -180.0, 1920.0, 1080.0);
        let moved = cocoa_to_top_left(right, 900.0);
        assert_eq!((moved.min, moved.size()), (pos2(1440.0, 0.0), vec2(1920.0, 1080.0)));
        let left = rect(-1920.0, 0.0, 1920.0, 1080.0);
        assert_eq!(cocoa_to_top_left(left, 900.0).min, pos2(-1920.0, -180.0));
        let above = rect(0.0, 900.0, 1440.0, 900.0);
        assert_eq!(cocoa_to_top_left(above, 900.0).min, pos2(0.0, -900.0));
    }

    #[test]
    fn login_item_status_maps_every_service_management_value() {
        let cases = [
            (0, LoginItem::Off),
            (1, LoginItem::On),
            (2, LoginItem::NeedsApproval),
            (3, LoginItem::NotFound),
            (4, LoginItem::Unavailable),
            (99, LoginItem::Unavailable),
            (-1, LoginItem::Unavailable),
        ];
        for (raw, want) in cases {
            assert_eq!(login_item_from_raw(raw), want, "raw {raw}");
        }
    }

    #[test]
    fn only_an_app_bundle_can_be_a_login_item() {
        let cases = [
            ("/Applications/Tidewatch.app/Contents/MacOS/tidewatch", true),
            ("/Users/me/Applications/Tidewatch.app/Contents/MacOS/tidewatch", true),
            ("/work/tidewatch/target/debug/tidewatch", false),
            ("/usr/local/bin/tidewatch", false),
            ("tidewatch", false),
            ("/Applications/Tidewatch.app/tidewatch", false),
        ];
        for (exe, want) in cases {
            assert_eq!(in_app_bundle(std::path::Path::new(exe)), want, "exe {exe}");
        }
    }
}
