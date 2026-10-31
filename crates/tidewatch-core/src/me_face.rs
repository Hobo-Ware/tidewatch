//! The owner's own face for the Reader chat: the GitHub picture of the account `gh` is signed in
//! to, saved once under `.state/me/` and fetched again at most once a day, else a letter from
//! `git config user.name`. Every call that runs `gh`, `curl` or `git` blocks, so call it from a
//! background thread. Never Gravatar.

use std::path::Path;
use std::time::Duration;

use crate::fsutil;
use crate::limited;
use crate::paths::TidewatchPaths;
use crate::tools::{Tools, run_gh};

pub const REFRESH_EVERY: Duration = Duration::from_hours(24);
pub const PICTURE_PX: u32 = 96;
const FALLBACK_LETTER: &str = "Y";
const CONNECT_SECS: &str = "8";
const TOTAL_SECS: &str = "20";

/// Whether the picture is due again: no stamp, a day gone by, or a stamp from the future (a clock
/// that was set back).
#[must_use]
pub fn refresh_due(stamp: Option<u64>, now: u64) -> bool {
    match stamp {
        None => true,
        Some(at) if at > now => true,
        Some(at) => now - at >= REFRESH_EVERY.as_secs(),
    }
}

#[must_use]
pub fn read_stamp(paths: &TidewatchPaths) -> Option<u64> {
    std::fs::read_to_string(paths.me_avatar_stamp_file()).ok()?.trim().parse().ok()
}

/// The first letter or digit of `name`, upper case; `Y` (for You) when there is none.
#[must_use]
pub fn letter_of(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| FALLBACK_LETTER.to_owned(), |c| c.to_uppercase().to_string())
}

/// GitHub's avatar address asked for at `PICTURE_PX` square.
#[must_use]
pub fn sized_url(url: &str) -> String {
    let joint = if url.contains('?') { '&' } else { '?' };
    format!("{url}{joint}s={PICTURE_PX}")
}

#[must_use]
pub fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || bytes.starts_with(&[0x89, b'P', b'N', b'G'])
}

#[must_use]
pub fn git_name(tools: &Tools, dir: &Path) -> Option<String> {
    let mut command = tools.git(dir);
    command.args(["config", "user.name"]);
    let finished = tools.finish(command).ok().filter(|finished| finished.success)?;
    let name = finished.stdout.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Saves the signed-in account's picture and stamps `now`; the old picture stays unless this one is.
pub fn refresh(tools: &Tools, paths: &TidewatchPaths, now: u64) -> Result<(), String> {
    let dir = paths.me_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let url = run_gh(tools, &dir, &["api", "user", "--jq", ".avatar_url"])
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    if !url.starts_with("https://") {
        return Err("gh gave no avatar address".to_owned());
    }
    let part = dir.join("avatar.part");
    let mut command = tools.command("curl", &dir);
    command
        .args(["-sfL", "--connect-timeout", CONNECT_SECS, "--max-time", TOTAL_SECS, "-o"])
        .arg(&part)
        .arg(sized_url(&url));
    let finished = tools.finish(command).map_err(|e| limited::failure("curl", &e))?;
    if !finished.success {
        let _ = std::fs::remove_file(&part);
        return Err("the picture download failed".to_owned());
    }
    let bytes = std::fs::read(&part).map_err(|e| format!("cannot read the download: {e}"))?;
    if !looks_like_image(&bytes) {
        let _ = std::fs::remove_file(&part);
        return Err("the download is not a picture".to_owned());
    }
    std::fs::rename(&part, paths.me_avatar_file())
        .map_err(|e| format!("cannot save the picture: {e}"))?;
    fsutil::write_atomic(&paths.me_avatar_stamp_file(), &now.to_string())
        .map_err(|e| format!("cannot save the stamp: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    #[test]
    fn refresh_is_due_after_a_day_without_a_stamp_or_with_one_from_the_future() {
        let now = 10 * DAY;
        let cases = [
            (None, true),
            (Some(now), false),
            (Some(now - 1), false),
            (Some(now - DAY + 1), false),
            (Some(now - DAY), true),
            (Some(now - 3 * DAY), true),
            (Some(now + 1), true),
        ];
        for (stamp, want) in cases {
            assert_eq!(refresh_due(stamp, now), want, "stamp {stamp:?}");
        }
    }

    #[test]
    fn the_letter_is_the_first_letter_or_digit_in_upper_case() {
        let cases = [
            ("Vlad Jerca", "V"),
            ("vlad", "V"),
            ("  ada", "A"),
            ("\u{201c}ned\u{201d}", "N"),
            ("\u{e9}mile", "\u{c9}"),
            ("42nd", "4"),
            ("", "Y"),
            ("  ---", "Y"),
        ];
        for (name, want) in cases {
            assert_eq!(letter_of(name), want, "name {name:?}");
        }
    }

    #[test]
    fn the_size_joins_the_address_with_the_right_separator() {
        let cases = [
            (
                "https://avatars.githubusercontent.com/u/6339681?v=4",
                "https://avatars.githubusercontent.com/u/6339681?v=4&s=96",
            ),
            (
                "https://avatars.githubusercontent.com/u/6339681",
                "https://avatars.githubusercontent.com/u/6339681?s=96",
            ),
        ];
        for (url, want) in cases {
            assert_eq!(sized_url(url), want);
        }
    }

    #[test]
    fn only_jpeg_and_png_bytes_count_as_a_picture() {
        let cases: [(&[u8], bool); 5] = [
            (&[0xFF, 0xD8, 0xFF, 0xE0], true),
            (&[0x89, b'P', b'N', b'G', 0x0D], true),
            (b"<html>not found</html>", false),
            (b"", false),
            (&[0xFF, 0xD8], false),
        ];
        for (bytes, want) in cases {
            assert_eq!(looks_like_image(bytes), want, "bytes {bytes:?}");
        }
    }

    #[test]
    fn the_stamp_reads_back_and_a_bad_one_is_none() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        assert_eq!(read_stamp(&paths), None);
        std::fs::create_dir_all(paths.me_dir()).unwrap();
        std::fs::write(paths.me_avatar_stamp_file(), "1700000000\n").unwrap();
        assert_eq!(read_stamp(&paths), Some(1_700_000_000));
        std::fs::write(paths.me_avatar_stamp_file(), "soon").unwrap();
        assert_eq!(read_stamp(&paths), None);
    }

    #[test]
    fn refresh_without_gh_fails_and_saves_nothing() {
        let home = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let tools = Tools::with_path(bare.path());
        assert!(refresh(&tools, &paths, 5).is_err());
        assert!(!paths.me_avatar_file().exists());
        assert!(!paths.me_avatar_stamp_file().exists());
    }
}
