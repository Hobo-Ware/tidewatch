//! Dev-only switches for the screenshot and runnable checks, read once at startup.
//!
//! - `TIDEWATCH_DEV_OVERVIEW=1`: start on the Overview.
//! - `TIDEWATCH_DEV_FOCUS=1`: start in focus mode.
//! - `TIDEWATCH_DEV_STATUS_LOG=1`: print every hook and status change to stderr with a wall-clock
//!   time in milliseconds, to measure how fast a tab reacts.
//! - `TIDEWATCH_DEV_KEEP_AWAKE=1`: let a background or screenshot run hold the sleep assertion
//!   too (it never does otherwise), to check `pmset -g assertions`.
//! - `TIDEWATCH_DEV_SIDE=<fraction>`: start with the left column that wide (0.3 to 0.75 of the
//!   window), to look at the file view when it is narrow.
//! - `TIDEWATCH_DEV_WINDOW=<w>x<h>`: open the window that size (in points, 600 to 4000 each), to
//!   look at a view in a shorter or wider window.
//! - `TIDEWATCH_DEV_REMOTE_POPOVER=1`: the Remote pill's popover is open on an agent tab that has one.
//! - `TIDEWATCH_DEV_PARKED_POPOVER=1`: the Parked pill's list is open (with `TIDEWATCH_FAKE_PARKED=1`).
//! - `TIDEWATCH_DEV_FOLDER_PICKER=1`: the repo list of a folder tab's panel is open (`folder_view`).
//! - `TIDEWATCH_DEV_HOVER=<x>,<y>`: move the pointer to that point (in points) on the first frame and
//!   leave it there, to open a tooltip in a capture (use `TIDEWATCH_SCREENSHOT_WAIT=4`).
//! - `TIDEWATCH_DEV_HOVER_FILES=1`: report a file dragged over the window on every frame, to see the
//!   reply box's drop highlight in a capture.
//! - `TIDEWATCH_DEV_KEYS=<text>` after `TIDEWATCH_DEV_KEYS_AFTER=<seconds>` (default 8): type the text
//!   into the newest agent tab once, to answer a prompt a check cannot avoid (`\r`, `\n`, `\e` and
//!   `\\` are escapes; `\e[B\r` is Down then Enter).
//! - `TIDEWATCH_DEV_SCROLL=<lines>` after `TIDEWATCH_DEV_SCROLL_AFTER=<seconds>` (default 10): one wheel
//!   event of that many lines (positive is up, into the scrollback) over the active terminal.
//! - `TIDEWATCH_DEV_COPY=<seconds>`: after that long, select everything in the active terminal and press
//!   Cmd+C, as egui delivers it, to read the clipboard afterwards.
//! - `TIDEWATCH_DEV_PASTE=<text>`: after the same delay, one Cmd+V of that text (the escapes above apply),
//!   as egui delivers it, to whatever has the focus: the newest tab's terminal, or a text field.

use crate::layout;
use crate::remote_pill::Popover;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevKeys {
    pub(crate) bytes: Vec<u8>,
    pub(crate) after: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevPaste {
    pub(crate) text: String,
    pub(crate) after: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevCopy {
    pub(crate) after: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevScroll {
    pub(crate) lines: f32,
    pub(crate) after: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevView {
    pub(crate) overview: bool,
    pub(crate) focus: bool,
    pub(crate) status_log: bool,
    pub(crate) keys: Option<DevKeys>,
    pub(crate) paste: Option<DevPaste>,
    pub(crate) scroll: Option<DevScroll>,
    pub(crate) copy: Option<DevCopy>,
    pub(crate) side: Option<f32>,
    pub(crate) hover: Option<egui::Pos2>,
    pub(crate) hover_files: Option<std::path::PathBuf>,
    pub(crate) remote_popover: Popover,
    pub(crate) parked_popover: Popover,
}

const DEFAULT_KEYS_AFTER_SECS: f64 = 8.0;
const DEFAULT_SCROLL_AFTER_SECS: f64 = 10.0;
const WINDOW_EDGE: std::ops::RangeInclusive<f32> = 600.0..=4000.0;

/// The size the window opens at: `TIDEWATCH_DEV_WINDOW` when it holds `<w>x<h>`, else `default`.
pub(crate) fn window_size(default: [f32; 2]) -> [f32; 2] {
    let value = std::env::var("TIDEWATCH_DEV_WINDOW").ok();
    value.as_deref().and_then(parse_window).unwrap_or(default)
}

fn parse_window(text: &str) -> Option<[f32; 2]> {
    let (width, height) = text.split_once('x')?;
    let size = [width.parse::<f32>().ok()?, height.parse::<f32>().ok()?];
    size.iter().all(|edge| WINDOW_EDGE.contains(edge)).then_some(size)
}

impl DevView {
    pub(crate) fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self::parse(
            var("TIDEWATCH_DEV_OVERVIEW").as_deref(),
            var("TIDEWATCH_DEV_FOCUS").as_deref(),
            var("TIDEWATCH_DEV_STATUS_LOG").as_deref(),
            var("TIDEWATCH_DEV_KEYS").as_deref(),
            var("TIDEWATCH_DEV_KEYS_AFTER").as_deref(),
        )
        .with_side(var("TIDEWATCH_DEV_SIDE").as_deref())
        .with_remote_popover(var("TIDEWATCH_DEV_REMOTE_POPOVER").as_deref())
        .with_parked_popover(var("TIDEWATCH_DEV_PARKED_POPOVER").as_deref())
        .with_paste(
            var("TIDEWATCH_DEV_PASTE").as_deref(),
            var("TIDEWATCH_DEV_KEYS_AFTER").as_deref(),
        )
        .with_scroll(
            var("TIDEWATCH_DEV_SCROLL").as_deref(),
            var("TIDEWATCH_DEV_SCROLL_AFTER").as_deref(),
        )
        .with_copy(var("TIDEWATCH_DEV_COPY").as_deref())
        .with_hover(var("TIDEWATCH_DEV_HOVER").as_deref())
        .with_hover_files(var("TIDEWATCH_DEV_HOVER_FILES").as_deref())
    }

    fn with_hover_files(self, value: Option<&str>) -> Self {
        let hover_files = value
            .is_some_and(|v| !v.is_empty() && v != "0")
            .then(|| std::path::PathBuf::from("/work/shot.png"));
        Self { hover_files, ..self }
    }

    fn with_hover(self, value: Option<&str>) -> Self {
        let hover = value.and_then(|text| {
            let (x, y) = text.split_once(',')?;
            let at = egui::pos2(x.trim().parse().ok()?, y.trim().parse().ok()?);
            (at.x.is_finite() && at.y.is_finite()).then_some(at)
        });
        Self { hover, ..self }
    }

    fn with_copy(self, after: Option<&str>) -> Self {
        let copy = after
            .and_then(|after| after.parse::<f64>().ok())
            .filter(|after| after.is_finite() && *after >= 0.0)
            .map(|after| DevCopy { after });
        Self { copy, ..self }
    }

    fn with_scroll(self, lines: Option<&str>, after: Option<&str>) -> Self {
        let scroll = lines.and_then(|lines| lines.parse::<f32>().ok()).map(|lines| DevScroll {
            lines,
            after: after.and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_SCROLL_AFTER_SECS),
        });
        Self { scroll, ..self }
    }

    fn with_paste(self, text: Option<&str>, after: Option<&str>) -> Self {
        let paste = text.filter(|text| !text.is_empty()).map(|text| DevPaste {
            text: String::from_utf8_lossy(&unescape(text)).into_owned(),
            after: after.and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_KEYS_AFTER_SECS),
        });
        Self { paste, ..self }
    }

    fn with_remote_popover(self, value: Option<&str>) -> Self {
        let open = value.is_some_and(|v| !v.is_empty() && v != "0");
        Self { remote_popover: if open { Popover::Open } else { Popover::OnClick }, ..self }
    }

    fn with_parked_popover(self, value: Option<&str>) -> Self {
        let open = value.is_some_and(|v| !v.is_empty() && v != "0");
        Self { parked_popover: if open { Popover::Open } else { Popover::OnClick }, ..self }
    }

    fn with_side(self, side: Option<&str>) -> Self {
        let side = side
            .and_then(|fraction| fraction.parse::<f32>().ok())
            .filter(|fraction| layout::SIDE_RANGE.contains(fraction));
        Self { side, ..self }
    }

    fn parse(
        overview: Option<&str>,
        focus: Option<&str>,
        status_log: Option<&str>,
        keys: Option<&str>,
        keys_after: Option<&str>,
    ) -> Self {
        let on = |value: Option<&str>| value.is_some_and(|v| !v.is_empty() && v != "0");
        Self {
            overview: on(overview),
            focus: on(focus),
            status_log: on(status_log),
            keys: keys.filter(|k| !k.is_empty()).map(|k| DevKeys {
                bytes: unescape(k),
                after: keys_after.and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_KEYS_AFTER_SECS),
            }),
            paste: None,
            scroll: None,
            copy: None,
            side: None,
            hover: None,
            hover_files: None,
            remote_popover: Popover::OnClick,
            parked_popover: Popover::OnClick,
        }
    }
}

/// `\r`, `\n`, `\e` and `\\` as the bytes they name; anything else is kept as typed.
fn unescape(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('r') => out.push(b'\r'),
            Some('n') => out.push(b'\n'),
            Some('e') => out.push(0x1b),
            Some('\\') | None => out.push(b'\\'),
            Some(other) => {
                out.push(b'\\');
                let mut buf = [0; 4];
                out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hover_point_is_two_numbers_and_anything_else_is_ignored() {
        let cases = [
            (Some("400,22"), Some(egui::pos2(400.0, 22.0))),
            (Some(" 400.5 , 22 "), Some(egui::pos2(400.5, 22.0))),
            (Some("400"), None),
            (Some("a,b"), None),
            (Some("inf,3"), None),
            (None, None),
        ];
        for (value, want) in cases {
            assert_eq!(DevView::default().with_hover(value).hover, want, "{value:?}");
        }
    }

    #[test]
    fn files_hover_the_window_for_any_value_but_empty_or_zero() {
        for (value, want) in
            [(Some("1"), true), (Some("0"), false), (Some(""), false), (None, false)]
        {
            let got = DevView::default().with_hover_files(value).hover_files;
            assert_eq!(got.is_some(), want, "{value:?}");
        }
    }

    #[test]
    fn a_switch_is_on_for_any_value_but_empty_or_zero() {
        let view = DevView::parse(Some("1"), None, Some("yes"), None, None);
        assert_eq!(view, DevView { overview: true, status_log: true, ..DevView::default() });
        for (value, want) in
            [(Some("1"), Popover::Open), (Some("0"), Popover::OnClick), (None, Popover::OnClick)]
        {
            assert_eq!(DevView::default().with_remote_popover(value).remote_popover, want);
        }
        for (value, want) in
            [(Some("1"), Popover::Open), (Some("0"), Popover::OnClick), (None, Popover::OnClick)]
        {
            assert_eq!(DevView::default().with_parked_popover(value).parked_popover, want);
        }
        assert!(DevView::parse(None, Some("1"), None, None, None).focus);
        assert_eq!(
            DevView::parse(Some(""), Some("0"), Some("0"), Some(""), None),
            DevView::default()
        );
        assert_eq!(DevView::parse(None, None, None, None, None), DevView::default());
    }

    #[test]
    fn the_paste_switch_takes_escaped_text_and_the_keys_delay() {
        let paste = DevView::default().with_paste(Some("a\\r\\nb"), Some("3")).paste;
        assert_eq!(paste, Some(DevPaste { text: "a\r\nb".into(), after: 3.0 }));
        let late = DevView::default().with_paste(Some("x"), None).paste;
        assert_eq!(late.map(|paste| paste.after), Some(DEFAULT_KEYS_AFTER_SECS));
        assert_eq!(DevView::default().with_paste(Some(""), None).paste, None);
        assert_eq!(DevView::default().with_paste(None, None).paste, None);
    }

    #[test]
    fn the_copy_switch_takes_a_delay() {
        assert_eq!(DevView::default().with_copy(Some("4")).copy, Some(DevCopy { after: 4.0 }));
        assert_eq!(DevView::default().with_copy(Some("soon")).copy, None);
        assert_eq!(DevView::default().with_copy(Some("-1")).copy, None);
        assert_eq!(DevView::default().with_copy(None).copy, None);
    }

    #[test]
    fn the_scroll_switch_takes_a_line_count_and_a_delay() {
        let scroll = DevView::default().with_scroll(Some("-12"), Some("4")).scroll;
        assert_eq!(scroll, Some(DevScroll { lines: -12.0, after: 4.0 }));
        let late = DevView::default().with_scroll(Some("30"), None).scroll;
        assert_eq!(late.map(|scroll| scroll.after), Some(DEFAULT_SCROLL_AFTER_SECS));
        assert_eq!(DevView::default().with_scroll(Some("up"), None).scroll, None);
        assert_eq!(DevView::default().with_scroll(None, None).scroll, None);
    }

    #[test]
    fn the_window_switch_takes_two_sane_edges() {
        let cases = [
            ("1280x720", Some([1280.0, 720.0])),
            ("1600x760", Some([1600.0, 760.0])),
            ("1280", None),
            ("axb", None),
            ("100x100", None),
            ("9000x800", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(parse_window(text), want, "{text}");
        }
    }

    #[test]
    fn the_side_fraction_must_be_one_the_divider_could_reach() {
        let cases = [
            (Some("0.3"), Some(0.3)),
            (Some("0.5"), Some(0.5)),
            (Some("0.2"), None),
            (Some("0.9"), None),
            (Some("wide"), None),
            (None, None),
        ];
        for (text, want) in cases {
            assert_eq!(DevView::default().with_side(text).side, want, "{text:?}");
        }
    }

    #[test]
    fn keys_wait_a_default_or_a_given_number_of_seconds() {
        let default = DevView::parse(None, None, None, Some("1"), None).keys.unwrap();
        assert_eq!(default, DevKeys { bytes: b"1".to_vec(), after: 8.0 });
        let given = DevView::parse(None, None, None, Some("1"), Some("3.5")).keys.unwrap();
        assert_eq!(given.after, 3.5);
        let junk = DevView::parse(None, None, None, Some("1"), Some("soon")).keys.unwrap();
        assert_eq!(junk.after, 8.0);
    }

    #[test]
    fn escapes_become_the_bytes_they_name() {
        assert_eq!(unescape("\\e[B\\r"), b"\x1b[B\r");
        assert_eq!(unescape("a\\nb"), b"a\nb");
        assert_eq!(unescape("back\\\\slash"), b"back\\slash");
        assert_eq!(unescape("\\q and trailing \\"), b"\\q and trailing \\");
        assert_eq!(unescape("caf\u{e9}"), "caf\u{e9}".as_bytes());
    }
}
