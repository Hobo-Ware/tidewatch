//! A tab's avatar on disk: `<tab state>/avatar.json`, so it comes back with the tab.

use std::path::Path;

use crate::avatar::{Avatar, clean};
use crate::fsutil::write_atomic;

#[must_use]
pub fn load(file: &Path) -> Option<Avatar> {
    let saved: Avatar = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    clean(&saved.svg, &saved.reason).ok()
}

pub fn save(file: &Path, avatar: &Avatar) -> std::io::Result<()> {
    write_atomic(file, &serde_json::to_string(avatar).map_err(std::io::Error::other)?)
}

pub fn clear(file: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(file) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[must_use]
pub fn was_asked(marker: &Path) -> bool {
    marker.exists()
}

pub fn mark_asked(marker: &Path) -> std::io::Result<()> {
    write_atomic(marker, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = "<svg><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>";

    #[test]
    fn a_saved_avatar_loads_back_and_a_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tab").join("avatar.json");
        assert_eq!(load(&file), None);
        let avatar = clean(SVG, "a dot").unwrap();
        save(&file, &avatar).unwrap();
        assert_eq!(load(&file), Some(avatar));
    }

    #[test]
    fn a_file_edited_to_hold_something_unclean_loads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("avatar.json");
        let hostile = Avatar { svg: "<svg><script>1</script></svg>".into(), reason: "x".into() };
        save(&file, &hostile).unwrap();
        assert_eq!(load(&file), None);
        std::fs::write(&file, "nope").unwrap();
        assert_eq!(load(&file), None);
    }

    #[test]
    fn clear_removes_the_file_and_is_fine_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("avatar.json");
        clear(&file).unwrap();
        save(&file, &clean(SVG, "a dot").unwrap()).unwrap();
        clear(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn the_asked_marker_is_absent_until_marked_and_marking_twice_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("tab").join("avatar-asked");
        assert!(!was_asked(&marker));
        mark_asked(&marker).unwrap();
        assert!(was_asked(&marker));
        mark_asked(&marker).unwrap();
        assert!(was_asked(&marker));
    }
}
