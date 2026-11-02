use super::*;

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Fixture {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    notes: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let repo = root.join("web");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "one",
        ],
    );
    sh(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let state = root.join(".state").join("app-123456").join("tab-a");
    std::fs::create_dir_all(&state).unwrap();
    Fixture { _dir: dir, repo, notes: state.join("notes.json") }
}

fn args(repo: &Path, branch: &str) -> Map<String, Value> {
    json!({ "repo": repo.to_str().unwrap(), "branch": branch }).as_object().unwrap().clone()
}

#[test]
fn the_worktree_goes_in_the_tabs_folder_and_never_beside_the_repository() {
    let f = fixture();
    let (text, is_error) = add(&args(&f.repo, "fix/pwa-icon"), &f.notes);
    assert!(!is_error, "{text}");
    let path = dir_beside(&f.notes).join("web-fix-pwa-icon");
    assert!(path.join(".git").is_file(), "{text}");
    assert!(text.contains(path.to_str().unwrap()), "{text}");
    assert_eq!(sh(&path, &["branch", "--show-current"]), "fix/pwa-icon");
    let beside: Vec<_> = std::fs::read_dir(f.repo.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(beside.iter().all(|name| !name.contains("worktrees")), "{beside:?}");
    assert_eq!(listed(&f.notes), vec![path]);
}

#[test]
fn asking_twice_for_the_same_branch_points_at_the_same_folder() {
    let f = fixture();
    let _ = add(&args(&f.repo, "feat/x"), &f.notes);
    let (text, is_error) = add(&args(&f.repo, "feat/x"), &f.notes);
    assert!(!is_error && text.starts_with("Already there"), "{text}");
}

#[test]
fn bad_input_is_refused_with_the_reason() {
    let f = fixture();
    let cases = [
        (json!({ "repo": f.repo.to_str().unwrap() }), "needs repo and branch"),
        (json!({ "repo": "relative/path", "branch": "a" }), "not a folder"),
        (json!({ "repo": f.repo.to_str().unwrap(), "branch": "--force" }), "branch:"),
        (
            json!({ "repo": f.repo.to_str().unwrap(), "branch": "bad..name" }),
            "not a valid branch name",
        ),
        (
            json!({ "repo": f.notes.parent().unwrap().to_str().unwrap(), "branch": "a" }),
            "not a git repository",
        ),
    ];
    for (input, want) in cases {
        let (text, is_error) = add(input.as_object().unwrap(), &f.notes);
        assert!(is_error && text.contains(want), "{input}: {text}");
    }
}

#[test]
fn closing_removes_a_clean_worktree_and_keeps_one_with_work_in_it() {
    let f = fixture();
    let _ = add(&args(&f.repo, "clean"), &f.notes);
    let _ = add(&args(&f.repo, "dirty"), &f.notes);
    let _ = add(&args(&f.repo, "committed"), &f.notes);
    let dir = dir_beside(&f.notes);
    std::fs::write(dir.join("web-dirty").join("file.txt"), "x").unwrap();
    sh(
        &dir.join("web-committed"),
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "local",
        ],
    );
    let kept = remove_all_but_kept(&f.notes);
    assert_eq!(kept, vec![dir.join("web-committed"), dir.join("web-dirty")]);
    assert!(!dir.join("web-clean").exists());
    assert!(!sh(&f.repo, &["worktree", "list"]).contains("web-clean"));
}
