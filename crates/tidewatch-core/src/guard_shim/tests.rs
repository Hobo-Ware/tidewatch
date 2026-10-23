use super::*;

#[test]
fn the_environment_names_the_binary_the_settings_the_repo_the_cache_and_the_shim_folder() {
    let ctx = GuardCtx {
        bin: "/app/tidewatch".into(),
        config: "/cfg/settings.toml".into(),
        repo: "/work/web".into(),
        cache_dir: "/state/guard-cache".into(),
    };
    let env = ctx.env(Path::new("/state/guard-bin"));
    let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    assert_eq!(get(BIN_ENV), Some("/app/tidewatch"));
    assert_eq!(get(CONFIG_ENV), Some("/cfg/settings.toml"));
    assert_eq!(get(REPO_ENV), Some("/work/web"));
    assert_eq!(get(CACHE_ENV), Some("/state/guard-cache"));
    assert_eq!(get(DIR_ENV), Some("/state/guard-bin"));
    assert_eq!(env.len(), 5);
}

#[test]
fn there_is_one_shim_per_tool_and_each_watches_its_own_words() {
    let shims = shims();
    let names: Vec<&str> = shims.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["git", "gh"]);
    let git = &shims[0].1;
    assert!(git.starts_with("#!/bin/sh\n") && git.contains("case \"$arg\" in commit)"));
    assert!(git.contains("--guard git"));
    let gh = &shims[1].1;
    assert!(gh.contains("case \"$arg\" in pr|issue|run)") && gh.contains("--guard gh"));
    for (_, content) in &shims {
        assert!(!content.contains("@TOOL@") && !content.contains("@WATCH@"), "{content}");
    }
}

#[test]
fn shims_are_written_once_as_executables_and_repaired_when_changed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    ensure_shims(dir.path()).unwrap();
    for (name, content) in shims() {
        let file = dir.path().join(name);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), content);
        assert_ne!(file.metadata().unwrap().permissions().mode() & 0o111, 0);
    }
    let git = dir.path().join("git");
    let stamp = git.metadata().unwrap().modified().unwrap();
    ensure_shims(dir.path()).unwrap();
    assert_eq!(git.metadata().unwrap().modified().unwrap(), stamp);

    std::fs::write(&git, "tampered").unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o644)).unwrap();
    ensure_shims(dir.path()).unwrap();
    assert_eq!(std::fs::read_to_string(&git).unwrap(), shims()[0].1);
    assert_ne!(git.metadata().unwrap().permissions().mode() & 0o111, 0);
}
