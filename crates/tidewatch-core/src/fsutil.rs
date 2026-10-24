//! Small file helpers shared by the settings writers.

use std::path::Path;

/// Writes a sibling temp file and renames it over `target`, so a crash never leaves half a file.
pub(crate) fn write_atomic(target: &Path, text: &str) -> std::io::Result<()> {
    write_with_mode(target, text, None)
}

/// [`write_atomic`] for a file that holds a secret: only its owner can read it (mode 0600).
pub(crate) fn write_private_atomic(target: &Path, text: &str) -> std::io::Result<()> {
    write_with_mode(target, text, Some(0o600))
}

fn write_with_mode(target: &Path, text: &str, mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp_name = target.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = target.with_file_name(tmp_name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    if let Some(mode) = mode {
        options.mode(mode);
    }
    let mut file = options.open(&tmp)?;
    if let Some(mode) = mode {
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    file.write_all(text.as_bytes())?;
    std::fs::rename(&tmp, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_parents_replaces_content_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a").join("b").join("file.toml");
        write_atomic(&target, "one").unwrap();
        write_atomic(&target, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "two");
        let names: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["file.toml"]);
    }

    #[test]
    fn a_private_file_is_readable_by_its_owner_only_even_when_it_replaces_an_open_one() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("settings.json");
        write_atomic(&target, "open").unwrap();
        write_private_atomic(&target, "secret").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "secret");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
