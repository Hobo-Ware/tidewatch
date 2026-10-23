use std::path::Path;
use std::process::Command;

use super::*;
use crate::comments::Kind;

const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -3,0 +4,2 @@ fn x() {
+    // loop over the items
+    let a = 1;
@@ -10 +12 @@ fn y() {
-    // old
+    let b = 2; // set b
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+// not code
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-// loop over the items
";

#[test]
fn added_lines_carry_their_real_line_numbers_per_file() {
    let files = added_lines(DIFF);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "src/a.rs");
    assert_eq!(
        files[0].lines,
        [
            (4, "    // loop over the items".to_owned()),
            (5, "    let a = 1;".to_owned()),
            (12, "    let b = 2; // set b".to_owned())
        ]
    );
    assert_eq!(files[1].path, "README.md");
}

#[test]
fn a_removed_line_that_looks_like_a_header_is_not_one() {
    let diff = "diff --git a/a.rb b/a.rb\n--- a/a.rb\n+++ b/a.rb\n@@ -1,2 +1,2 @@\n--- a removed ruby comment\n+# loop over a\n+++ not a header\n";
    let files = added_lines(diff);
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].lines,
        [(1, "# loop over a".to_owned()), (2, "++ not a header".to_owned())]
    );
}

#[test]
fn only_added_comments_in_code_files_are_flagged_with_real_lines() {
    let flagged = flags_of_diff(DIFF);
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0].path, "src/a.rs");
    let found: Vec<(usize, Kind)> = flagged[0].flags.iter().map(|f| (f.line, f.kind)).collect();
    assert_eq!(found, [(4, Kind::Narration), (12, Kind::Narration)]);
}

#[test]
fn runs_apart_are_scanned_apart_so_a_hunk_never_borrows_context() {
    let added = Added {
        path: "a.rs".into(),
        lines: vec![(1, "// one".into()), (2, "// two".into()), (10, "// three".into())],
    };
    assert_eq!(flags_in(&added), vec![]);
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn the_sweep_finds_committed_edited_and_new_comments_on_a_real_branch() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("old.rs"), "// loop over the old items\nfn old() {}\n").unwrap();
    git(repo, &["add", "old.rs"]);
    git(repo, &["commit", "-q", "-m", "init"]);
    git(repo, &["switch", "-q", "-c", "agent/work"]);

    std::fs::write(repo.join("committed.rs"), "fn a() {}\n// set a to one\nfn b() {}\n").unwrap();
    git(repo, &["add", "committed.rs"]);
    git(repo, &["commit", "-q", "-m", "work"]);
    std::fs::write(
        repo.join("old.rs"),
        "// loop over the old items\nfn old() {}\n// hack for now\n",
    )
    .unwrap();
    std::fs::write(repo.join("untracked.py"), "x = 1\n# initialize the list\n").unwrap();
    std::fs::write(repo.join("notes.md"), "// loop over\n").unwrap();

    let swept = sweep(repo).unwrap();
    let mut found: Vec<(String, usize, Kind)> = swept
        .iter()
        .flat_map(|file| file.flags.iter().map(|f| (file.path.clone(), f.line, f.kind)))
        .collect();
    found.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    assert_eq!(
        found,
        [
            ("committed.rs".to_owned(), 2, Kind::Narration),
            ("old.rs".to_owned(), 3, Kind::Alibi),
            ("untracked.py".to_owned(), 2, Kind::Narration),
        ]
    );
}

#[test]
fn the_base_prefers_the_default_branch_and_there_is_none_in_a_repo_without_one() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path();
    git(repo, &["init", "-q", "-b", "trunk"]);
    git(repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    assert_eq!(base(repo), None);
    assert_eq!(sweep(repo), None);
    git(repo, &["branch", "main"]);
    assert_eq!(base(repo).as_deref(), Some("main"));
}
