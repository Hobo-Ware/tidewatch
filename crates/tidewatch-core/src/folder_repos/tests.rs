use std::fs;

use super::*;
use crate::transcript::ToolCall;

fn repo(name: &str, touched_ms: i64) -> FolderRepo {
    FolderRepo {
        root: PathBuf::from("/w").join(name),
        name: name.to_owned(),
        branch: Some("main".into()),
        touched_ms,
        linked: false,
    }
}

fn paths(items: &[&str]) -> Vec<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

fn make_repo(parent: &Path, name: &str, head: &str) -> PathBuf {
    let root = parent.join(name);
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), head).unwrap();
    root
}

fn call(name: &str, summary: &str, phase: ToolPhase) -> Entry {
    Entry {
        role: Role::Tool,
        text: String::new(),
        tool: Some(ToolCall {
            name: name.into(),
            summary: summary.into(),
            id: None,
            phase,
            ask: None,
        }),
        ts: None,
        images: Vec::new(),
    }
}

#[test]
fn head_text_names_the_branch_or_the_short_id() {
    let cases = [
        ("ref: refs/heads/main\n", Some("main")),
        ("ref: refs/heads/fix/panel\n", Some("fix/panel")),
        ("ref: refs/remotes/origin/main\n", Some("refs/remotes/origin/main")),
        ("0123456789abcdef0123456789abcdef01234567\n", Some("0123456")),
        ("not a head", None),
        ("", None),
    ];
    for (text, want) in cases {
        assert_eq!(parse_head(text).as_deref(), want, "{text:?}");
    }
}

#[test]
fn a_git_file_points_at_its_folder_relative_to_the_repo() {
    let repo = Path::new("/w/app-fix");
    let cases = [
        ("gitdir: /w/app/.git/worktrees/fix\n", Some("/w/app/.git/worktrees/fix")),
        ("gitdir: ../app/.git/worktrees/fix\n", Some("/w/app-fix/../app/.git/worktrees/fix")),
        ("gitdir:\n", None),
        ("something else\n", None),
    ];
    for (text, want) in cases {
        assert_eq!(parse_gitdir(text, repo), want.map(PathBuf::from), "{text:?}");
    }
}

#[test]
fn the_default_pick_follows_the_newest_edit_then_the_newest_touch_then_the_name() {
    let repos = [repo("api", 100), repo("web", 300), repo("docs", 200)];
    let cases = [
        ("newest edit wins", paths(&["/w/api/a.rs", "/w/docs/b.md"]), Some("docs")),
        (
            "an older edit does not beat a newer one",
            paths(&["/w/docs/b.md", "/w/api/a.rs"]),
            Some("api"),
        ),
        (
            "an edit outside every repo is skipped",
            paths(&["/w/api/a.rs", "/elsewhere/x"]),
            Some("api"),
        ),
        ("no edits fall back to the newest touch", Vec::new(), Some("web")),
        ("edits that match nothing fall back to the touch", paths(&["/nowhere/x"]), Some("web")),
    ];
    for (name, edits, want) in cases {
        let got = default_pick(&repos, &edits).map(|repo| repo.name.as_str());
        assert_eq!(got, want, "{name}");
    }
    assert_eq!(default_pick(&[], &paths(&["/w/api/a.rs"])), None);
}

#[test]
fn equal_touches_pick_the_first_by_name() {
    let repos = [repo("api", 5), repo("web", 5)];
    assert_eq!(default_pick(&repos, &[]).map(|repo| repo.name.as_str()), Some("api"));
}

#[test]
fn a_nested_repo_wins_over_the_repo_around_it() {
    let mut inner = repo("api", 1);
    inner.root = PathBuf::from("/w/api/vendor/lib");
    let repos = [repo("api", 1), inner];
    let got = repo_of(&repos, Path::new("/w/api/vendor/lib/x.rs")).unwrap();
    assert_eq!(got.root, Path::new("/w/api/vendor/lib"));
}

#[test]
fn the_owners_pick_holds_until_the_repo_is_gone() {
    let repos = [repo("api", 100), repo("web", 300)];
    let edits = paths(&["/w/web/a.rs"]);
    let cases = [
        ("a hand pick beats the edit", Some("/w/api"), "api"),
        ("no hand pick follows the edit", None, "web"),
        ("a vanished hand pick falls back", Some("/w/gone"), "web"),
    ];
    for (name, hand, want) in cases {
        let got = pick(&repos, hand.map(Path::new), &edits).map(|repo| repo.name.as_str());
        assert_eq!(got, Some(want), "{name}");
    }
}

#[test]
fn a_folder_inside_a_repo_shows_that_repo_and_one_with_none_shows_the_folder() {
    let repos = vec![repo("api", 1)];
    let inside = Scan {
        folder: "/w/api/src".into(),
        inside: Some("/w/api".into()),
        repos: Vec::new(),
        recent: Vec::new(),
    };
    assert_eq!(target(&inside, None, &[]), Target::Inside(Path::new("/w/api")));
    let with_repos =
        Scan { folder: "/w".into(), inside: None, repos: repos.clone(), recent: Vec::new() };
    assert_eq!(target(&with_repos, None, &[]), Target::Repo(&repos[0]));
    let none = Scan { folder: "/w".into(), ..Scan::default() };
    assert_eq!(target(&none, None, &[]), Target::Folder);
}

#[test]
fn discovery_lists_repos_one_level_down_by_name_and_skips_hidden_and_build_folders() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path(), "web", "ref: refs/heads/main\n");
    make_repo(dir.path(), "Api", "ref: refs/heads/dev\n");
    make_repo(dir.path(), ".hidden", "ref: refs/heads/main\n");
    make_repo(dir.path(), "node_modules", "ref: refs/heads/main\n");
    make_repo(dir.path(), "target", "ref: refs/heads/main\n");
    fs::create_dir_all(dir.path().join("plain/src")).unwrap();
    fs::write(dir.path().join("note.txt"), "x").unwrap();
    let repos = discover(dir.path());
    let seen: Vec<(&str, Option<&str>)> =
        repos.iter().map(|repo| (repo.name.as_str(), repo.branch.as_deref())).collect();
    assert_eq!(seen, [("Api", Some("dev")), ("web", Some("main"))]);
    assert!(repos.iter().all(|repo| !repo.linked && repo.touched_ms > 0));
}

#[test]
fn a_linked_worktree_is_a_repo_and_reads_its_own_head() {
    let dir = tempfile::tempdir().unwrap();
    let main = make_repo(dir.path(), "app", "ref: refs/heads/main\n");
    let admin = main.join(".git/worktrees/app-fix");
    fs::create_dir_all(&admin).unwrap();
    fs::write(admin.join("HEAD"), "ref: refs/heads/fix\n").unwrap();
    let linked = dir.path().join("app-fix");
    fs::create_dir_all(&linked).unwrap();
    fs::write(linked.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    let repos = discover(dir.path());
    let seen: Vec<(&str, Option<&str>, bool)> =
        repos.iter().map(|r| (r.name.as_str(), r.branch.as_deref(), r.linked)).collect();
    assert_eq!(seen, [("app", Some("main"), false), ("app-fix", Some("fix"), true)]);
}

#[test]
fn discovery_stops_at_fifty_repos() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..60 {
        make_repo(dir.path(), &format!("r{n:02}"), "ref: refs/heads/main\n");
    }
    let repos = discover(dir.path());
    assert_eq!(repos.len(), MAX_REPOS);
    assert_eq!(repos[0].name, "r00");
}

#[test]
fn a_repo_whose_head_moved_last_is_the_most_recently_touched() {
    let dir = tempfile::tempdir().unwrap();
    let old = make_repo(dir.path(), "old", "ref: refs/heads/main\n");
    let new = make_repo(dir.path(), "new", "ref: refs/heads/main\n");
    let hour_ago = SystemTime::now() - Duration::from_hours(1);
    fs::File::options()
        .write(true)
        .open(old.join(".git/HEAD"))
        .unwrap()
        .set_modified(hour_ago)
        .unwrap();
    let repos = discover(dir.path());
    let picked = default_pick(&repos, &[]).unwrap();
    assert_eq!(picked.root, new);
}

#[test]
fn a_folder_inside_a_repo_is_not_scanned_for_repos() {
    let dir = tempfile::tempdir().unwrap();
    let root = make_repo(dir.path(), "app", "ref: refs/heads/main\n");
    fs::create_dir_all(root.join("src")).unwrap();
    let scan = scan(&root.join("src"), 0);
    assert_eq!(scan.inside, Some(root));
    assert!(scan.repos.is_empty() && scan.recent.is_empty());
}

fn now_ms() -> i64 {
    i64::try_from(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis())
        .unwrap()
}

#[test]
fn a_folder_with_no_repo_lists_the_files_changed_in_the_last_hour_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("sub/deep")).unwrap();
    fs::create_dir_all(root.join(".cache")).unwrap();
    fs::create_dir_all(root.join("node_modules/x")).unwrap();
    fs::create_dir_all(root.join("target")).unwrap();
    for file in [
        "fresh.txt",
        "sub/deep/mid.txt",
        "old.txt",
        ".cache/h.txt",
        "node_modules/x/n.js",
        "target/t.o",
    ] {
        fs::write(root.join(file), "x").unwrap();
    }
    let age = |file: &str, ago: Duration| {
        fs::File::options()
            .write(true)
            .open(root.join(file))
            .unwrap()
            .set_modified(SystemTime::now() - ago)
            .unwrap();
    };
    age("fresh.txt", Duration::from_secs(5));
    age("sub/deep/mid.txt", Duration::from_mins(20));
    age("old.txt", Duration::from_hours(3));
    let scan = scan(root, now_ms());
    assert_eq!(scan.inside, None);
    assert_eq!(scan.repos.len(), 0);
    let names: Vec<&str> = scan.recent.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(names, ["fresh.txt", "sub/deep/mid.txt"]);
}

#[test]
fn the_recent_list_stops_at_two_hundred_files() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..230 {
        fs::write(dir.path().join(format!("f{n:03}.txt")), "x").unwrap();
    }
    assert_eq!(recent_files(dir.path(), now_ms()).len(), MAX_RECENT);
}

#[test]
fn only_absolute_edit_and_write_calls_count_as_edits() {
    let entries = [
        call("Edit", "/w/api/a.rs", ToolPhase::Call),
        call("Read", "/w/api/b.rs", ToolPhase::Call),
        call("Write", "/w/web/c.rs", ToolPhase::Call),
        call("Write", "relative/d.rs", ToolPhase::Call),
        call("MultiEdit", "/w/web/long/path...", ToolPhase::Call),
        call("Edit", "/w/api/e.rs", ToolPhase::Result { error: false }),
        call("write_file", "/w/web/f.rs", ToolPhase::Call),
    ];
    assert_eq!(edited_paths(&entries), paths(&["/w/api/a.rs", "/w/web/c.rs", "/w/web/f.rs"]));
}

#[test]
fn kept_edits_run_oldest_to_newest_without_repeats_and_stay_bounded() {
    let mut kept = Vec::new();
    remember_edits(&mut kept, paths(&["/a", "/b", "/a"]));
    assert_eq!(kept, paths(&["/b", "/a"]));
    let many: Vec<PathBuf> = (0..100).map(|n| PathBuf::from(format!("/f{n}"))).collect();
    remember_edits(&mut kept, many);
    assert_eq!(kept.len(), MAX_EDITS);
    assert_eq!(kept.last(), Some(&PathBuf::from("/f99")));
}

#[test]
fn the_home_folder_is_written_as_a_tilde() {
    let home = Path::new("/Users/me");
    let cases = [
        ("/Users/me", "~"),
        ("/Users/me/Git/work", "~/Git/work"),
        ("/Users/mea/x", "/Users/mea/x"),
        ("/tmp/x", "/tmp/x"),
    ];
    for (path, want) in cases {
        assert_eq!(tilde(Path::new(path), Some(home)), want, "{path}");
    }
    assert_eq!(tilde(Path::new("/Users/me/x"), None), "/Users/me/x");
}

#[test]
fn a_file_changed_at_the_top_of_a_repo_makes_it_the_most_recently_touched() {
    let dir = tempfile::tempdir().unwrap();
    let quiet = make_repo(dir.path(), "quiet", "ref: refs/heads/main\n");
    let busy = make_repo(dir.path(), "busy", "ref: refs/heads/main\n");
    let hour_ago = SystemTime::now() - Duration::from_hours(1);
    for repo in [&quiet, &busy] {
        fs::File::options()
            .write(true)
            .open(repo.join(".git/HEAD"))
            .unwrap()
            .set_modified(hour_ago)
            .unwrap();
    }
    fs::write(busy.join("work.txt"), "edit").unwrap();
    let repos = discover(dir.path());
    assert_eq!(default_pick(&repos, &[]).unwrap().root, busy);
}
