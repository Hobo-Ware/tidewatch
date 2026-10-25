use super::*;

#[test]
fn a_path_becomes_the_folder_name_claude_code_uses() {
    let table = [
        ("/Users/x/Git/Hoboware/kelp", "-Users-x-Git-Hoboware-kelp"),
        (
            "/Users/x/.tidewatch/kelp-aa771b/jolly-tide",
            "-Users-x--tidewatch-kelp-aa771b-jolly-tide",
        ),
        ("/work/my repo/a_b", "-work-my-repo-a-b"),
        ("/work/caf\u{e9}", "-work-caf-"),
        ("/work/\u{1F980}", "-work---"),
    ];
    for (path, want) in table {
        assert_eq!(project_dir_name(Path::new(path)).as_deref(), Some(want), "{path}");
    }
}

#[test]
fn a_name_of_exactly_the_limit_is_kept_and_one_more_is_not_named() {
    let at_limit = format!("/{}", "a".repeat(MAX_NAME_UTF16 - 1));
    assert_eq!(project_dir_name(Path::new(&at_limit)).map(|n| n.len()), Some(MAX_NAME_UTF16));
    let over = format!("/{}", "a".repeat(MAX_NAME_UTF16));
    assert_eq!(project_dir_name(Path::new(&over)), None);
}

#[test]
fn the_memory_folder_sits_under_projects_in_the_config_dir() {
    assert_eq!(
        memory_dir(Path::new("/c"), Path::new("/Users/x/Git/kelp")),
        Some(PathBuf::from("/c/projects/-Users-x-Git-kelp/memory")),
    );
    assert_eq!(memory_dir(Path::new("/c"), Path::new(&format!("/{}", "a".repeat(300)))), None);
}

#[test]
fn the_config_dir_follows_the_variable_and_ignores_an_empty_one() {
    let home = Path::new("/Users/x");
    assert_eq!(config_dir(home, None), Path::new("/Users/x/.claude"));
    assert_eq!(config_dir(home, Some(OsString::new())), Path::new("/Users/x/.claude"));
    assert_eq!(config_dir(home, Some("/elsewhere".into())), Path::new("/elsewhere"));
}
