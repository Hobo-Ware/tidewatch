//! Small file helpers shared by the settings writers.

use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Writes a sibling temp file and renames it over `target`, so a crash never leaves half a file.
pub(crate) fn write_atomic(target: &Path, text: &str) -> std::io::Result<()> {
    write_with_mode(target, text, None)
}

/// [`write_atomic`] for a file that holds a secret: only its owner can read it (mode 0600).
pub(crate) fn write_private_atomic(target: &Path, text: &str) -> std::io::Result<()> {
    write_with_mode(target, text, Some(0o600))
}

pub(crate) fn load_toml_or_default<T: DeserializeOwned + Default>(
    file: &Path,
    consequence: &str,
) -> T {
    let Ok(text) = std::fs::read_to_string(file) else { return T::default() };
    toml::from_str(&text).unwrap_or_else(|e| {
        eprintln!("tidewatch: {} is invalid ({e}); {consequence}", file.display());
        T::default()
    })
}

pub(crate) fn save_toml<T: Serialize>(file: &Path, value: &T) -> std::io::Result<()> {
    write_text(file, toml::to_string(value))
}

pub(crate) fn save_toml_pretty<T: Serialize>(file: &Path, value: &T) -> std::io::Result<()> {
    write_text(file, toml::to_string_pretty(value))
}

pub(crate) fn read_json<T: DeserializeOwned>(file: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

pub(crate) fn save_json<T: Serialize + ?Sized>(file: &Path, value: &T) -> std::io::Result<()> {
    write_text(file, serde_json::to_string(value))
}

pub(crate) fn save_json_pretty<T: Serialize + ?Sized>(
    file: &Path,
    value: &T,
) -> std::io::Result<()> {
    write_text(file, serde_json::to_string_pretty(value))
}

fn write_text<E>(file: &Path, text: Result<String, E>) -> std::io::Result<()>
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    write_atomic(file, &text.map_err(std::io::Error::other)?)
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

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    fn sample() -> Sample {
        Sample { name: "orders".into(), count: 3 }
    }

    #[test]
    fn toml_round_trips_through_either_save() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("s.toml");
        save_toml(&file, &sample()).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), toml::to_string(&sample()).unwrap());
        assert_eq!(load_toml_or_default::<Sample>(&file, "x"), sample());
        save_toml_pretty(&file, &Sample::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            toml::to_string_pretty(&Sample::default()).unwrap()
        );
        assert_eq!(load_toml_or_default::<Sample>(&file, "x"), Sample::default());
    }

    #[test]
    fn a_missing_or_broken_toml_file_loads_as_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.toml");
        assert_eq!(load_toml_or_default::<Sample>(&file, "x"), Sample::default());
        std::fs::write(&file, "name = [[[").unwrap();
        assert_eq!(load_toml_or_default::<Sample>(&file, "x"), Sample::default());
    }

    #[test]
    fn json_round_trips_and_pretty_differs_only_in_layout() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("s.json");
        save_json(&file, &sample()).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"{"name":"orders","count":3}"#);
        assert_eq!(read_json::<Sample>(&file), Some(sample()));
        save_json_pretty(&file, &sample()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "{\n  \"name\": \"orders\",\n  \"count\": 3\n}"
        );
        assert_eq!(read_json::<Sample>(&file), Some(sample()));
    }

    #[test]
    fn a_missing_or_broken_json_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.json");
        assert_eq!(read_json::<Sample>(&file), None);
        std::fs::write(&file, "{not json").unwrap();
        assert_eq!(read_json::<Sample>(&file), None);
    }
}
