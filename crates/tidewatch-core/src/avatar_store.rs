//! A tab's avatar on disk: `<tab state>/avatar.json`, so it comes back with the tab.

use std::path::Path;

use crate::avatar::{Avatar, clean};
use crate::fsutil::{read_json, remove_file_if_present, save_json, write_atomic};

#[must_use]
pub fn load(file: &Path) -> Option<Avatar> {
    let saved: Avatar = read_json(file)?;
    clean(&saved.svg, &saved.reason).ok()
}

pub fn save(file: &Path, avatar: &Avatar) -> std::io::Result<()> {
    save_json(file, avatar)
}

pub fn clear(file: &Path) -> std::io::Result<()> {
    remove_file_if_present(file)
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
