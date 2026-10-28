use super::*;

const ID: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";

struct World {
    _dir: tempfile::TempDir,
    claude: PathBuf,
    old: PathBuf,
    new: PathBuf,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let claude = home.join(".claude");
    let old = home.join("ws");
    let new = home.join("wt").join("calm-comet");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::create_dir_all(&new).unwrap();
    let folder = claude.join("projects").join(claude_project_dir_name(&old));
    std::fs::create_dir_all(folder.join(ID).join("subagents")).unwrap();
    std::fs::write(folder.join(format!("{ID}.jsonl")), "{\"type\":\"user\"}\n").unwrap();
    std::fs::write(folder.join(ID).join("subagents").join("agent-1.jsonl"), "{\"a\":1}\n").unwrap();
    std::fs::write(folder.join(ID).join("tool-results.txt"), "result").unwrap();
    World { _dir: dir, claude, old, new }
}

fn project(w: &World, cwd: &Path) -> PathBuf {
    w.claude.join("projects").join(claude_project_dir_name(cwd))
}

#[test]
fn the_session_is_found_in_its_own_project_folder_and_else_in_any() {
    let w = world();
    let file = project(&w, &w.old).join(format!("{ID}.jsonl"));
    assert_eq!(find_session(&w.claude, &w.old, ID), Some(file.clone()));
    assert_eq!(find_session(&w.claude, &w.new, ID), Some(file), "found from another folder");
    assert_eq!(find_session(&w.claude, &w.old, "00000000-0000-0000-0000-000000000000"), None);
}

#[test]
fn the_copy_is_byte_identical_includes_the_sidecar_folder_and_leaves_the_source_alone() {
    let w = world();
    let source = find_session(&w.claude, &w.old, ID).unwrap();
    copy_session(&w.claude, &source, &w.new).unwrap();
    let target = project(&w, &w.new);
    for relative in [
        format!("{ID}.jsonl"),
        format!("{ID}/subagents/agent-1.jsonl"),
        format!("{ID}/tool-results.txt"),
    ] {
        let (from, to) = (project(&w, &w.old).join(&relative), target.join(&relative));
        assert_eq!(std::fs::read(&from).unwrap(), std::fs::read(&to).unwrap(), "{relative}");
    }
}

#[test]
fn a_second_copy_of_the_same_session_is_fine_and_a_different_file_is_never_overwritten() {
    let w = world();
    let source = find_session(&w.claude, &w.old, ID).unwrap();
    copy_session(&w.claude, &source, &w.new).unwrap();
    copy_session(&w.claude, &source, &w.new).unwrap();
    let held = project(&w, &w.new).join(format!("{ID}.jsonl"));
    std::fs::write(&held, "something else\n").unwrap();
    let error = copy_session(&w.claude, &source, &w.new).unwrap_err();
    assert!(error.contains("already holds a different file"), "{error}");
    assert_eq!(std::fs::read_to_string(&held).unwrap(), "something else\n");
}

#[test]
fn a_folder_name_too_long_for_claude_codes_project_folder_is_an_error_not_a_guess() {
    let w = world();
    let source = find_session(&w.claude, &w.old, ID).unwrap();
    let long = w.new.join("x".repeat(210));
    let error = copy_session(&w.claude, &source, &long).unwrap_err();
    assert!(error.contains("too long"), "{error}");
}
