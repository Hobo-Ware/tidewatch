use super::*;

fn record(n: u64) -> Record {
    Record {
        at_ms: n,
        origin: Origin::Edit,
        file: "src/a.rs".into(),
        line: n as usize,
        kind: "narration".into(),
        text: format!("note {n}"),
    }
}

#[test]
fn records_append_as_lines_and_load_back_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let files = TabFiles::new(&dir.path().join("tab"));
    assert_eq!(files.load(), vec![]);
    files.append(&[record(1), record(2)]).unwrap();
    files.append(&[record(3)]).unwrap();
    assert_eq!(files.load(), [record(1), record(2), record(3)]);
    assert_eq!(std::fs::read_to_string(files.flags_file()).unwrap().lines().count(), 3);
}

#[test]
fn a_bad_line_is_skipped_and_nothing_appended_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let files = TabFiles::new(dir.path());
    files.append(&[]).unwrap();
    assert!(!files.flags_file().exists());
    files.append(&[record(1)]).unwrap();
    let mut text = std::fs::read_to_string(files.flags_file()).unwrap();
    text.push_str("not json\n{\"at_ms\":1}\n");
    std::fs::write(files.flags_file(), text).unwrap();
    files.append(&[record(2)]).unwrap();
    assert_eq!(files.load(), [record(1), record(2)]);
}

#[test]
fn a_file_past_the_cap_is_cut_back_to_the_newest_records() {
    let dir = tempfile::tempdir().unwrap();
    let files = TabFiles::new(dir.path());
    let big: Vec<Record> =
        (0..400).map(|n| Record { text: "x".repeat(300), ..record(n) }).collect();
    files.append(&big).unwrap();
    let kept = files.load();
    assert_eq!(kept.len(), 200);
    assert_eq!(kept[0].at_ms, 200);
    assert_eq!(kept[199].at_ms, 399);
    assert_eq!(std::fs::read_to_string(files.flags_file()).unwrap().lines().count(), 200);
}

#[test]
fn the_off_switch_is_a_marker_file_and_turning_it_off_twice_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let files = TabFiles::new(&dir.path().join("tab"));
    assert!(!files.is_off());
    files.set_off(true).unwrap();
    assert!(files.is_off());
    files.set_off(false).unwrap();
    files.set_off(false).unwrap();
    assert!(!files.is_off());
}

#[test]
fn the_sweep_digest_is_remembered_and_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let files = TabFiles::new(dir.path());
    assert_eq!(files.swept(), None);
    files.set_swept(Some("abc123")).unwrap();
    assert_eq!(files.swept().as_deref(), Some("abc123"));
    files.set_swept(None).unwrap();
    files.set_swept(None).unwrap();
    assert_eq!(files.swept(), None);
}
