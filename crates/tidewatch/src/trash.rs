//! Moves a folder to the system Trash, the recoverable way out for a leftover folder git cannot
//! read. Never deletes: where the Trash cannot be reached it says so and the folder stays.

use std::path::Path;

#[cfg(target_os = "macos")]
pub(crate) fn move_to_trash(path: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, None).map_err(
        |error| format!("could not move it to the Trash: {}", error.localizedDescription()),
    )
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn move_to_trash(path: &Path) -> Result<(), String> {
    let out = std::process::Command::new("gio")
        .arg("trash")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|e| format!("could not move it to the Trash: gio is not available ({e})"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "could not move it to the Trash: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "puts a folder in the real Trash; run by hand"]
    fn a_folder_moves_to_the_trash_and_a_missing_one_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("tidewatch-trash-probe-7f3a");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("notes.txt"), "mine").unwrap();
        move_to_trash(&folder).unwrap();
        assert!(!folder.exists());
        let err = move_to_trash(&folder).unwrap_err();
        assert!(err.starts_with("could not move it to the Trash"), "{err}");
    }
}
