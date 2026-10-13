//! Which lines of a pull request's diff GitHub takes a review comment on (PLAN.md, D4).
//!
//! GitHub answers 422 for a comment on a line outside the diff. [`PrDiffLines::load`] reads the
//! branch's diff against its base once (`git diff base...head`, blocks on git: call it off the UI
//! thread) and [`PrDiffLines::contains`] answers per line, so the app can disable "Post to PR" with
//! a reason instead of letting gh fail. A line counts when it is inside a hunk, context included.

use std::collections::BTreeMap;
use std::path::Path;

use kelp_core::review::Side;

use crate::delivery::{compare_ref, detect_base};
use crate::tools::{Tools, finish};

type Range = (u32, u32);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FileRanges {
    new: Vec<Range>,
    old: Vec<Range>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrDiffLines {
    files: BTreeMap<String, FileRanges>,
}

impl PrDiffLines {
    /// Whether `line` of `path` on `side` is in the diff.
    #[must_use]
    pub fn contains(&self, path: &str, side: Side, line: u32) -> bool {
        let Some(file) = self.files.get(path) else { return false };
        let ranges = if side == Side::Old { &file.old } else { &file.new };
        ranges.iter().any(|&(start, end)| line >= start && line < end)
    }

    /// Reads the diff of `head` (a commit or branch) against the repository's base branch.
    pub fn load(tools: &Tools, dir: &Path, head: &str) -> Result<Self, String> {
        if head.is_empty() || head.starts_with('-') {
            return Err(format!("{head:?} is not a usable commit"));
        }
        let base =
            detect_base(tools, dir).ok_or("cannot tell which branch the pull request targets")?;
        let start = compare_ref(tools, dir, &base);
        let mut command = tools.git(dir);
        command
            .args([
                "-c",
                "core.quotepath=off",
                "diff",
                "--unified=3",
                "--no-color",
                "--no-ext-diff",
            ])
            .arg(format!("{start}...{head}"))
            .arg("--");
        let out = finish(command).map_err(|e| format!("cannot run git: {e}"))?;
        if out.success { Ok(Self::parse(&out.stdout)) } else { Err(out.stderr.trim().to_owned()) }
    }

    /// Reads unified diff text.
    #[must_use]
    pub fn parse(diff: &str) -> Self {
        let mut files: BTreeMap<String, FileRanges> = BTreeMap::new();
        let mut old_path: Option<String> = None;
        let mut current: Option<String> = None;
        for line in diff.lines() {
            if let Some(rest) = line.strip_prefix("--- ") {
                old_path = strip_prefix(rest, "a/");
                current = None;
            } else if let Some(rest) = line.strip_prefix("+++ ") {
                current = strip_prefix(rest, "b/").or_else(|| old_path.clone());
            } else if line.starts_with("@@ ")
                && let (Some(path), Some((old, new))) = (&current, hunk_ranges(line))
            {
                let entry = files.entry(path.clone()).or_default();
                entry.old.push(old);
                entry.new.push(new);
            }
        }
        Self { files }
    }
}

fn strip_prefix(path: &str, prefix: &str) -> Option<String> {
    let path = path.split('\t').next().unwrap_or(path).trim();
    if path == "/dev/null" {
        return None;
    }
    Some(path.strip_prefix(prefix).unwrap_or(path).to_owned())
}

fn hunk_ranges(header: &str) -> Option<(Range, Range)> {
    let body = header.strip_prefix("@@ ")?;
    let body = &body[..body.find(" @@")?];
    let mut parts = body.split(' ');
    let old = range(parts.next()?.strip_prefix('-')?)?;
    let new = range(parts.next()?.strip_prefix('+')?)?;
    Some((old, new))
}

fn range(text: &str) -> Option<Range> {
    let (start, count) = match text.split_once(',') {
        Some((start, count)) => (start.parse::<u32>().ok()?, count.parse::<u32>().ok()?),
        None => (text.parse::<u32>().ok()?, 1),
    };
    Some((start, start + count))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/auth.rs b/src/auth.rs
index 1..2 100644
--- a/src/auth.rs
+++ b/src/auth.rs
@@ -10,4 +10,5 @@ fn login() {
     let a = 1;
+    let b = 2;
     let c = 3;
@@ -40 +41,2 @@ fn other() {
-    old();
+    new();
+    more();
diff --git a/src/gone.rs b/src/gone.rs
deleted file mode 100644
--- a/src/gone.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-x
-y
diff --git a/new name.rs b/new name.rs
new file mode 100644
--- /dev/null
+++ b/new name.rs	
@@ -0,0 +1,3 @@
+a
+b
+c
";

    #[test]
    fn lines_inside_a_hunk_count_and_the_gaps_between_hunks_do_not() {
        let lines = PrDiffLines::parse(DIFF);
        for line in [10, 11, 14] {
            assert!(lines.contains("src/auth.rs", Side::New, line), "new {line}");
        }
        for line in [9, 15, 40, 43] {
            assert!(!lines.contains("src/auth.rs", Side::New, line), "new {line}");
        }
        assert!(lines.contains("src/auth.rs", Side::New, 41));
        assert!(lines.contains("src/auth.rs", Side::New, 42));
    }

    #[test]
    fn the_old_side_uses_the_old_numbers() {
        let lines = PrDiffLines::parse(DIFF);
        assert!(lines.contains("src/auth.rs", Side::Old, 40));
        assert!(!lines.contains("src/auth.rs", Side::Old, 41));
        assert!(lines.contains("src/gone.rs", Side::Old, 2));
    }

    #[test]
    fn deleted_and_new_files_and_odd_names_are_found() {
        let lines = PrDiffLines::parse(DIFF);
        assert!(!lines.contains("src/gone.rs", Side::New, 1));
        assert!(lines.contains("new name.rs", Side::New, 3));
        assert!(!lines.contains("new name.rs", Side::New, 4));
        assert!(!lines.contains("src/unrelated.rs", Side::New, 1));
    }

    #[test]
    fn nothing_parses_to_nothing() {
        assert_eq!(PrDiffLines::parse(""), PrDiffLines::default());
        assert_eq!(PrDiffLines::parse("@@ garbage @@"), PrDiffLines::default());
    }
}
