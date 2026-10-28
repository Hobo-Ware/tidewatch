//! The tests gate against real throwaway repos: the main checkout sits on an old branch that still
//! has `deno.json`, the tab's worktree is on a branch with `bun.lock`, and only a fake `bun` is on
//! `PATH`. It is the only test in this file because it changes the process `PATH`.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{
    commit_file, git_available, ok, path_with_only_git, repos, write_executable_from_a_child,
};
use tidewatch_core::meeseeks_check::tests;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::worktree;

fn fake_bun(bin: &Path, ran_in: &Path) {
    let script = format!(
        "#!/bin/sh\npwd -P > '{}'\necho \"bun $*\" >> '{}'\n",
        ran_in.display(),
        ran_in.display()
    );
    write_executable_from_a_child(&bin.join("bun"), &script);
    assert!(bin.join("bun").metadata().unwrap().permissions().mode() & 0o111 != 0);
}

#[test]
#[allow(unsafe_code)]
fn the_gate_runs_the_runtime_of_the_tabs_worktree_not_the_main_checkouts() {
    if !git_available() {
        return;
    }
    let f = repos();
    let main = f.work.clone();
    let home = f.root.path().join("home");
    let paths = TidewatchPaths::new(&home);
    let tab = worktree::add_named(&paths, &main, "tab", Some("origin/main")).unwrap();

    ok(&main, &["checkout", "-q", "-b", "old"]);
    commit_file(&main, "deno.json", r#"{"tasks":{"test":"deno test"}}"#, "chore: old tooling");
    commit_file(&tab.path, "package.json", r#"{"scripts":{"test":"bun test"}}"#, "feat: bun");
    commit_file(&tab.path, "bun.lock", "", "feat: lockfile");

    let bin = f.root.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ran_in = f.root.path().join("ran-in");
    fake_bun(&bin, &ran_in);
    let original = std::env::var_os("PATH").unwrap_or_default();
    let (_git_only, git_path) = path_with_only_git();
    let narrowed =
        std::env::join_paths([bin.as_path(), Path::new(&git_path), Path::new("/bin")]).unwrap();
    unsafe { std::env::set_var("PATH", &narrowed) };

    let said = tests(&main, &tab.path).unwrap().detail;
    assert_eq!(said, "bun run test passed");
    let log = std::fs::read_to_string(&ran_in).unwrap();
    assert_eq!(log.lines().next().unwrap(), tab.path.canonicalize().unwrap().to_str().unwrap());
    assert_eq!(log.lines().nth(1), Some("bun run test"));

    std::fs::remove_file(bin.join("bun")).unwrap();
    let skipped = tests(&main, &tab.path).unwrap().detail;
    assert_eq!(skipped, "skipped: no runnable test command (bun not installed)");

    unsafe { std::env::set_var("PATH", original) };
}
