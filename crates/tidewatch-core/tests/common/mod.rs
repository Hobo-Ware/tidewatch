//! Shared test helpers: real throwaway git repos and a fake `gh` that records its calls.
//! The real `gh` is never started from a test.

#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

pub fn git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .args(args)
        .output()
        .expect("git is installed because git_available() passed")
}

pub fn ok(dir: &Path, args: &[&str]) -> String {
    let out = git(dir, args);
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn commit_file(dir: &Path, name: &str, text: &str, message: &str) {
    std::fs::write(dir.join(name), text).unwrap();
    ok(dir, &["add", name]);
    ok(dir, &["commit", "-q", "-m", message]);
}

pub struct Repos {
    pub root: tempfile::TempDir,
    pub remote: PathBuf,
    pub work: PathBuf,
}

/// A bare remote and a clone of it with `main` pushed and `origin/HEAD` set.
pub fn repos() -> Repos {
    let root = tempfile::tempdir().unwrap();
    let remote = root.path().join("remote.git");
    let work = root.path().join("work");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    ok(&remote, &["init", "-q", "--bare", "-b", "main"]);
    ok(&work, &["init", "-q", "-b", "main"]);
    ok(&work, &["remote", "add", "origin", remote.to_str().unwrap()]);
    commit_file(&work, "readme.md", "hello\n", "chore: init");
    ok(&work, &["push", "-q", "-u", "origin", "main"]);
    ok(&work, &["remote", "set-head", "origin", "main"]);
    Repos { root, remote, work }
}

pub fn remote_branches(remote: &Path) -> Vec<String> {
    ok(remote, &["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .lines()
        .map(str::to_owned)
        .collect()
}

pub struct Reply {
    pub when: &'static str,
    pub code: i32,
    pub stdout: &'static str,
    pub stderr: &'static str,
}

pub fn reply(when: &'static str, code: i32, stdout: &'static str, stderr: &'static str) -> Reply {
    Reply { when, code, stdout, stderr }
}

/// A `gh` script in its own directory. It logs every call and answers by the first two arguments
/// (`pr create`, `pr view`, `api repos/...`); `--version` always succeeds.
pub struct FakeGh {
    pub dir: tempfile::TempDir,
}

impl FakeGh {
    pub fn new(replies: &[Reply]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("calls.log");
        let mut cases: Vec<String> = Vec::new();
        for (i, r) in replies.iter().enumerate() {
            std::fs::write(dir.path().join(format!("out{i}")), r.stdout).unwrap();
            std::fs::write(dir.path().join(format!("err{i}")), r.stderr).unwrap();
            cases.push(format!(
                "  '{}'*) cat '{d}/out{i}'; cat '{d}/err{i}' >&2; exit {};;\n",
                r.when,
                r.code,
                d = dir.path().display()
            ));
        }
        let script = format!(
            "#!/bin/sh\n\
             LOG='{log}'\n\
             for a in \"$@\"; do printf '%s\\036' \"$a\" >> \"$LOG\"; done\n\
             printf '\\035' >> \"$LOG\"\n\
             prev=''\n\
             for a in \"$@\"; do\n\
             \x20 if [ \"$prev\" = '--body-file' ]; then cat \"$a\" > \"$LOG.body\"; fi\n\
             \x20 prev=\"$a\"\n\
             done\n\
             [ \"$1\" = '--version' ] && {{ echo 'gh version fake'; exit 0; }}\n\
             case \"$1 $2\" in\n{cases}\
             \x20 *) echo \"unexpected gh call: $*\" >&2; exit 99;;\n\
             esac\n",
            log = log.display(),
            cases = cases.concat()
        );
        write_executable_from_a_child(&dir.path().join("gh"), &script);
        Self { dir }
    }

    /// The fake first, then the system `PATH` (for `git`, `sh`, `cat`).
    pub fn path(&self) -> OsString {
        let mut dirs = vec![self.dir.path().to_path_buf()];
        dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        std::env::join_paths(dirs).unwrap()
    }

    pub fn calls(&self) -> Vec<Vec<String>> {
        let log = std::fs::read(self.dir.path().join("calls.log")).unwrap_or_default();
        String::from_utf8_lossy(&log)
            .split('\u{1d}')
            .filter(|call| !call.is_empty())
            .map(|call| call.split('\u{1e}').filter(|a| !a.is_empty()).map(str::to_owned).collect())
            .collect()
    }

    pub fn calls_to(&self, first_two: &str) -> Vec<Vec<String>> {
        self.calls().into_iter().filter(|c| c[..c.len().min(2)].join(" ") == first_two).collect()
    }

    pub fn body_file_text(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join("calls.log.body")).ok()
    }
}

/// A `PATH` holding only `git`, so there is no `gh` to find.
pub fn path_with_only_git() -> (tempfile::TempDir, OsString) {
    let dir = tempfile::tempdir().unwrap();
    let real = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|d| d.join("git"))
        .find(|p| p.exists())
        .expect("git is on PATH because git_available() passed");
    #[cfg(unix)]
    std::os::unix::fs::symlink(real, dir.path().join("git")).unwrap();
    let path = dir.path().as_os_str().to_owned();
    (dir, path)
}

/// Linux refuses to exec a file that any process holds open for writing (ETXTBSY), and a test
/// thread that forks while this process writes the file inherits that handle, so `cp` writes it.
pub fn write_executable_from_a_child(path: &Path, text: &str) {
    let source = path.with_extension("src");
    std::fs::write(&source, text).unwrap();
    let copied = Command::new("cp").arg(&source).arg(path).status().unwrap();
    assert!(copied.success(), "cp {} failed", path.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
