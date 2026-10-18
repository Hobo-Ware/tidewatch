//! Dev-only switches for the screenshot and runnable checks, read once at startup.
//!
//! - `TIDEWATCH_DEV_OVERVIEW=1`: start on the Overview.
//! - `TIDEWATCH_DEV_FOCUS=1`: start in focus mode.
//! - `TIDEWATCH_DEV_STATUS_LOG=1`: print every hook and status change to stderr with a wall-clock
//!   time in milliseconds, to measure how fast a tab reacts.
//! - `TIDEWATCH_DEV_SIDE=<fraction>`: start with the left column that wide (0.3 to 0.75 of the
//!   window), to look at the file view when it is narrow.
//! - `TIDEWATCH_DEV_WINDOW=<w>x<h>`: open the window that size (in points, 600 to 4000 each), to
//!   look at a view in a shorter or wider window.
//! - `TIDEWATCH_DEV_REMOTE_POPOVER=1`: the Remote pill's popover is open on an agent tab that has one.
//! - `TIDEWATCH_DEV_KEYS=<text>` after `TIDEWATCH_DEV_KEYS_AFTER=<seconds>` (default 8): type the text
//!   into the newest agent tab once, to answer a prompt a check cannot avoid (`\r`, `\n`, `\e` and
//!   `\\` are escapes; `\e[B\r` is Down then Enter).

use crate::layout;
use crate::remote_pill::Popover;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevKeys {
    pub(crate) bytes: Vec<u8>,
    pub(crate) after: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevView {
    pub(crate) overview: bool,
    pub(crate) focus: bool,
    pub(crate) status_log: bool,
    pub(crate) keys: Option<DevKeys>,
    pub(crate) side: Option<f32>,
    pub(crate) remote_popover: Popover,
}

const DEFAULT_KEYS_AFTER_SECS: f64 = 8.0;
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
    }

    fn with_remote_popover(self, value: Option<&str>) -> Self {
        let open = value.is_some_and(|v| !v.is_empty() && v != "0");
        Self { remote_popover: if open { Popover::Open } else { Popover::OnClick }, ..self }
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
            side: None,
            remote_popover: Popover::OnClick,
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
    fn a_switch_is_on_for_any_value_but_empty_or_zero() {
        let view = DevView::parse(Some("1"), None, Some("yes"), None, None);
        assert_eq!(view, DevView { overview: true, status_log: true, ..DevView::default() });
        for (value, want) in
            [(Some("1"), Popover::Open), (Some("0"), Popover::OnClick), (None, Popover::OnClick)]
        {
            assert_eq!(DevView::default().with_remote_popover(value).remote_popover, want);
        }
        assert!(DevView::parse(None, Some("1"), None, None, None).focus);
        assert_eq!(
            DevView::parse(Some(""), Some("0"), Some("0"), Some(""), None),
            DevView::default()
        );
        assert_eq!(DevView::parse(None, None, None, None, None), DevView::default());
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
