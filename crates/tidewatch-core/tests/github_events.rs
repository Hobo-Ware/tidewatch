//! The webhook forwarder host end to end with a fake `gh` (the real one never runs) and a real
//! listener: a live forwarder, signed deliveries, a missing extension, a stale hook and no admin.

mod common;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tidewatch_core::github_events::{Host, News, record_hook, recorded_hook};
use tidewatch_core::github_forwarder::Why;
use tidewatch_core::github_hook::signature_header;
use tidewatch_core::listener::Listener;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::tools::Tools;

const SCRIPT: &str = r#"#!/bin/sh
D='@DIR@'
echo "$*" >> "$D/calls.log"
[ "$1" = '--version' ] && { echo 'gh version fake'; exit 0; }
case "$1 $2" in
  'webhook forward')
    echo "$*" >> "$D/forward.log"
    mode=$(cat "$D/mode")
    case "$mode" in
      live)
        trap 'echo terminated >> "$D/term.log"; exit 0' TERM
        sleep 600 &
        wait;;
      missing)
        [ -f "$D/installed" ] || { echo 'unknown command "webhook" for "gh"' >&2; exit 1; }
        trap 'echo terminated >> "$D/term.log"; exit 0' TERM
        sleep 600 &
        wait;;
      taken)
        [ -f "$D/deleted" ] || { echo 'error creating webhook: HTTP 422: Validation Failed' >&2; echo 'Hook already exists on this repository' >&2; exit 1; }
        trap 'echo terminated >> "$D/term.log"; exit 0' TERM
        sleep 600 &
        wait;;
      admin) echo 'HTTP 403: Must have admin rights to Repository.' >&2; exit 1;;
    esac;;
  'extension install') touch "$D/installed"; exit 0;;
  'api repos/Acme/Web/hooks')
    cat "$D/hooks.json"; exit 0;;
  'api -X') [ "$3" = 'DELETE' ] && touch "$D/deleted"; exit 0;;
  ?*) echo "unexpected gh call: $*" >&2; exit 99;;
esac
"#;

struct Fixture {
    dir: tempfile::TempDir,
    _home: tempfile::TempDir,
    repo: tempfile::TempDir,
    paths: TidewatchPaths,
    listener: Listener,
    host: Host,
}

fn fixture(mode: &str) -> Fixture {
    assert!(common::git_available());
    let dir = tempfile::tempdir().unwrap();
    let script = SCRIPT.replace("@DIR@", dir.path().to_str().unwrap());
    common::write_executable_from_a_child(&dir.path().join("gh"), &script);
    std::fs::write(dir.path().join("mode"), mode).unwrap();
    std::fs::write(
        dir.path().join("hooks.json"),
        r#"[{"id": 12, "name": "cli", "config": {"url": "https://webhook-forwarder.github.com/hook"}}]"#,
    )
    .unwrap();
    let mut path_dirs = vec![dir.path().to_path_buf()];
    path_dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    let path: OsString = std::env::join_paths(path_dirs).unwrap();
    let repo = tempfile::tempdir().unwrap();
    common::ok(repo.path(), &["init", "-q", "-b", "main"]);
    common::ok(repo.path(), &["remote", "add", "origin", "https://github.com/Acme/Web.git"]);
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let (listener, _inbox) = Listener::start(paths.clone(), Box::new(|| {})).unwrap();
    let host = Host::new(Tools::with_path(path), Some(paths.clone()), Arc::new(|| {}), true);
    Fixture { dir, _home: home, repo, paths, listener, host }
}

impl Fixture {
    fn repos(&self) -> Vec<PathBuf> {
        vec![self.repo.path().to_path_buf()]
    }

    fn drive(&mut self) -> Vec<News> {
        let repos = self.repos();
        self.host.drive(&repos, Some(&self.listener)).news
    }

    fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self, &[News]) -> bool) -> Vec<News> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut all = Vec::new();
        loop {
            let news = self.drive();
            all.extend(news);
            if done(self, &all) {
                return all;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn log(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(name)).unwrap_or_default()
    }

    fn forward_args(&self) -> String {
        self.log("forward.log").lines().next().unwrap_or_default().to_owned()
    }

    fn url_and_secret(&self) -> (String, String) {
        let args = self.forward_args();
        let take = |flag: &str| {
            args.split(' ')
                .find_map(|word| word.strip_prefix(flag))
                .unwrap_or_else(|| panic!("{flag} in {args}"))
                .to_owned()
        };
        (take("--url="), take("--secret="))
    }

    fn deliver(&self, event: &str, body: &str, secret: &str) -> u16 {
        let (url, _) = self.url_and_secret();
        post(&url, event, Some(&signature_header(secret, body.as_bytes())), body)
    }
}

fn post(url: &str, event: &str, signature: Option<&str>, body: &str) -> u16 {
    let rest = url.strip_prefix("http://").unwrap();
    let (addr, path) = rest.split_once('/').unwrap();
    let mut stream = TcpStream::connect(addr).unwrap();
    let mut head = format!(
        "POST /{path} HTTP/1.1\r\nHost: {addr}\r\nX-GitHub-Event: {event}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(signature) = signature {
        let _ = write!(head, "X-Hub-Signature-256: {signature}\r\n");
    }
    stream.write_all(format!("{head}\r\n{body}").as_bytes()).unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    reply.split(' ').nth(1).unwrap().parse().unwrap()
}

const PING: &str = r#"{"zen": "Keep it logically awesome.", "hook_id": 630}"#;

fn calls_to(f: &Fixture, prefix: &str) -> usize {
    f.log("calls.log").lines().filter(|line| line.starts_with(prefix)).count()
}

#[test]
fn a_forwarder_goes_live_on_its_signed_ping_then_delivers_signed_events_and_stops_on_sigterm() {
    let mut f = fixture("live");
    f.until("the forwarder to start", |f, _| !f.forward_args().is_empty());
    let args = f.forward_args();
    assert!(args.contains("webhook forward --repo=Acme/Web --events=pull_request,"), "{args}");
    assert!(args.contains("--url=http://127.0.0.1:"), "{args}");
    let (_, secret) = f.url_and_secret();
    assert!(f.host.live_repos().is_empty());

    assert_eq!(f.deliver("ping", PING, "wrong"), 401, "a bad signature is refused");
    assert_eq!(f.deliver("ping", PING, &secret), 200);
    f.until("live", |f, _| f.host.live_repos().contains(f.repo.path()));
    f.until("the record", |f, _| recorded_hook(&f.paths, "acme/web") == Some(630));

    let body = r#"{"check_run": {"check_suite": {"head_branch": "agent/calm"}, "pull_requests": [{"number": 7}]}}"#;
    assert_eq!(f.deliver("check_run", body, "wrong"), 401);
    assert_eq!(f.deliver("check_run", body, &secret), 200);
    assert_eq!(f.deliver("star", "{}", &secret), 200);
    let news = f.until("the event", |_, news| !news.is_empty());
    assert_eq!(news.len(), 1);
    assert_eq!(news[0].repos, [f.repo.path().to_path_buf()]);
    assert_eq!(
        (news[0].hit.numbers.clone(), news[0].hit.branches.clone()),
        (vec![7], vec!["agent/calm".to_owned()])
    );

    f.host.shutdown(Some(&f.listener));
    assert_eq!(f.log("term.log").trim(), "terminated", "SIGTERM reached the forwarder");
    assert!(f.host.live_repos().is_empty());
}

#[test]
fn a_missing_extension_is_installed_once_and_the_forwarder_starts_again() {
    let mut f = fixture("missing");
    f.until("the second start", |f, _| f.log("forward.log").lines().count() >= 2);
    assert_eq!(calls_to(&f, "extension install cli/gh-webhook"), 1);
    f.host.shutdown(Some(&f.listener));
}

#[test]
fn a_missing_extension_that_cannot_be_installed_leaves_the_repo_polled() {
    let mut f = fixture("missing");
    let script = std::fs::read_to_string(f.dir.path().join("gh")).unwrap();
    let failing = script.replace("touch \"$D/installed\"; exit 0;;", "exit 1;;");
    common::write_executable_from_a_child(&f.dir.path().join("gh.new"), &failing);
    std::fs::rename(f.dir.path().join("gh.new"), f.dir.path().join("gh")).unwrap();
    let repo = f.repo.path().to_path_buf();
    f.until("a fallback", |f, _| f.host.why_polled(&repo) == Some(Why::InstallFailed));
    assert_eq!(calls_to(&f, "extension install"), 1);
}

#[test]
fn a_stale_hook_we_recorded_is_deleted_and_the_forwarder_starts_again() {
    let mut f = fixture("taken");
    record_hook(&f.paths, "acme/web", 12);
    f.until("the restart", |f, _| f.log("forward.log").lines().count() >= 2);
    assert_eq!(calls_to(&f, "api -X DELETE repos/Acme/Web/hooks/12"), 1);
    f.host.shutdown(Some(&f.listener));
}

#[test]
fn a_hook_we_never_recorded_is_left_alone_and_the_repo_is_polled() {
    let mut f = fixture("taken");
    record_hook(&f.paths, "acme/web", 99);
    let repo = f.repo.path().to_path_buf();
    f.until("a fallback", |f, _| f.host.why_polled(&repo) == Some(Why::HookTaken));
    assert_eq!(calls_to(&f, "api -X DELETE"), 0);
    assert_eq!(f.log("forward.log").lines().count(), 1);
}

#[test]
fn a_repo_without_admin_rights_is_polled_and_says_why() {
    let mut f = fixture("admin");
    let repo = f.repo.path().to_path_buf();
    f.until("a fallback", |f, _| f.host.why_polled(&repo) == Some(Why::NoAdmin));
    assert!(f.host.live_repos().is_empty());
}

#[test]
fn a_repo_that_leaves_the_window_stops_its_forwarder_and_a_host_that_is_off_starts_nothing() {
    let mut f = fixture("live");
    f.until("the forwarder to start", |f, _| !f.forward_args().is_empty());
    let none: Vec<PathBuf> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while f.log("term.log").is_empty() {
        f.host.drive(&none, Some(&f.listener));
        assert!(Instant::now() < deadline, "the forwarder was never stopped");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(f.log("term.log").trim(), "terminated");

    let mut off = fixture("live");
    off.host = Host::new(
        Tools::with_path(std::env::var_os("PATH").unwrap()),
        None,
        Arc::new(|| {}),
        false,
    );
    let repos = off.repos();
    for _ in 0..5 {
        off.host.drive(&repos, Some(&off.listener));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(off.log("forward.log"), "");
}
