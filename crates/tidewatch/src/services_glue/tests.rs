use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use stdusk_core::handoff::send_session;
use stdusk_core::repaint::Repaint;
use stdusk_core::shell::ShellProfile;
use stdusk_core::terminal::SpawnOpts;
use tidewatch_core::service_supervisor::{Phase, State};

use super::*;
use crate::handoff::{Handover, Ptys, rebuild};

fn quiet_opts(command: &str, dir: &Path) -> SpawnOpts {
    SpawnOpts {
        detect_progress: false,
        shell_integration: false,
        autosuggestions: false,
        scrollback_lines: 200,
        word_separators: " ".into(),
        bold_bright: false,
        agent_tracking: None,
        cwd: Some(dir.to_string_lossy().into_owned()),
        profile: Some(ShellProfile {
            shell: Some("/bin/sh".into()),
            args: vec!["-c".into(), command.into()],
            cwd: None,
            env: BTreeMap::new(),
        }),
    }
}

fn spawner() -> Spawner {
    Box::new(|dir, command| PtyTerm::spawn(160, 40, Repaint::none(), &quiet_opts(command, dir)))
}

struct Fixture {
    host: Host,
    paths: TidewatchPaths,
    repo: tempfile::TempDir,
    _home: tempfile::TempDir,
}

fn fixture(services: &[Service]) -> Fixture {
    fixture_with(services, true)
}

fn fixture_with(services: &[Service], autostart: bool) -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    Store::update(&paths, |store| {
        services.iter().try_for_each(|service| store.add(repo.path(), service))
    })
    .unwrap();
    let mut host = Host::new(Some(paths.clone()), spawner(), autostart, false);
    host.sync(&[repo.path()]);
    Fixture { host, paths, repo, _home: home }
}

fn service(name: &str, command: &str, autostart: bool) -> Service {
    Service::new(name, command, autostart)
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Fixture {
    fn word(&mut self, name: &str) -> Word {
        self.host.drive();
        self.host.rows(self.repo.path()).iter().find(|row| row.name == name).unwrap().word
    }

    fn pid(&mut self, name: &str) -> u32 {
        self.host.term_mut(self.repo.path(), name).and_then(|term| term.shell_pid()).unwrap()
    }

    fn says(&self, name: &str, text: &str) -> bool {
        self.host
            .log_tail(self.repo.path(), name, 50)
            .is_some_and(|lines| lines.iter().any(|line| line.contains(text)))
    }
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn a_service_with_autostart_runs_when_its_repo_opens_and_one_without_waits_to_be_started() {
    let mut fx =
        fixture(&[service("db", "echo ready; sleep 30", true), service("api", "sleep 30", false)]);
    assert_eq!(fx.word("db"), Word::Running);
    assert_eq!(fx.word("api"), Word::Stopped);
    wait_for("db's output", || fx.says("db", "ready"));
    let started = fx.host.command(fx.repo.path(), "API", Command::Start).unwrap();
    assert_eq!(started, "Started `api`.");
    assert_eq!(fx.word("api"), Word::Running);
    let again = fx.host.command(fx.repo.path(), "api", Command::Start).unwrap();
    assert_eq!(again, "`api` was already running.");
    fx.host.shutdown();
}

#[test]
fn a_screenshot_or_background_run_starts_nothing_on_its_own() {
    let mut fx = fixture_with(&[service("db", "sleep 30", true)], false);
    assert_eq!(fx.word("db"), Word::Stopped);
    assert!(fx.host.term_mut(fx.repo.path(), "db").is_none());
}

#[test]
fn a_stop_by_hand_ends_the_process_and_nothing_starts_it_again() {
    let mut fx = fixture(&[service("db", "sleep 30", true)]);
    let pid = fx.pid("db");
    assert!(alive(pid));
    let stopped = fx.host.command(fx.repo.path(), "db", Command::Stop).unwrap();
    assert!(stopped.starts_with("Stopped `db`"), "{stopped}");
    wait_for("the process to end", || !alive(pid));
    let until = Instant::now() + Duration::from_millis(1600);
    while Instant::now() < until {
        assert_eq!(fx.word("db"), Word::Stopped);
        std::thread::sleep(Duration::from_millis(50));
    }
    let row = fx.host.rows(fx.repo.path()).remove(0);
    assert!(row.line.starts_with("stopped"), "{}", row.line);
    let again = fx.host.command(fx.repo.path(), "db", Command::Stop).unwrap();
    assert_eq!(again, "`db` was not running.");
}

#[test]
fn a_hand_stop_reads_stopped_with_no_exit_code_though_the_killed_shell_reports_exit_1() {
    let mut fx = fixture(&[service("db", "sleep 30", true)]);
    let repo = fx.repo.path().to_path_buf();
    assert_eq!(fx.word("db"), Word::Running);
    fx.host.command(&repo, "db", Command::Stop).unwrap();
    wait_for("the killed shell's exit", || {
        fx.host.term_mut(&repo, "db").and_then(|term| term.exited()).is_some()
    });
    let code = fx.host.term_mut(&repo, "db").and_then(|term| term.exited()).map(|exit| exit.code);
    assert_eq!(code, Some(1), "a signal death is reported as exit 1");
    assert_eq!(fx.word("db"), Word::Stopped);
    assert_eq!(fx.host.rows(&repo)[0].line, "stopped");
    assert_eq!(fx.host.listed(&repo)[0].last_exit, None);
}

#[test]
fn the_port_comes_from_what_the_service_printed_and_goes_with_the_process() {
    let command = "echo 'Local address to connect your application: 127.0.0.1:55926 (press ctrl-c to quit)'; \
                   echo 'client 127.0.0.1:50412 connected'; sleep 30";
    let mut fx = fixture(&[service("db", command, true)]);
    let repo = fx.repo.path().to_path_buf();
    wait_for("the port to be read", || {
        fx.host.drive();
        fx.host.rows(&repo)[0].line.ends_with(" \u{b7} :55926")
    });
    assert_eq!(fx.host.facts(&repo)[0].port, Some(55926));
    assert_eq!(fx.host.listed(&repo)[0].port, Some(55926));
    fx.host.command(&repo, "db", Command::Stop).unwrap();
    assert_eq!(fx.host.facts(&repo)[0].port, None);
    fx.host.shutdown();
}

#[test]
fn a_port_saved_by_an_older_version_shows_until_the_output_names_one() {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    let file = format!(
        "[[repo.\"{}\".service]]\nname = \"db\"\ncommand = \"sleep 30\"\nport = 3309\n",
        repo.path().display()
    );
    std::fs::write(paths.services_file(), file).unwrap();
    let mut host = Host::new(Some(paths), spawner(), true, false);
    host.sync(&[repo.path()]);
    assert_eq!(host.facts(repo.path())[0].port, Some(3309));
    host.shutdown();
}

#[test]
fn a_service_that_exits_is_started_again_after_one_second_then_two() {
    let mut fx = fixture(&[service("flaky", "echo boom; exit 1", true)]);
    wait_for("the first exit to be noticed", || fx.word("flaky") == Word::Restarting);
    let row = fx.host.rows(fx.repo.path()).remove(0);
    assert!(row.line.starts_with("restarting in"), "{}", row.line);
    assert_eq!(fx.host.listed(fx.repo.path())[0].last_exit, Some(1));
    assert!(fx.says("flaky", "boom"));
    let first = fx.host.entries();
    assert!(matches!(first[0].carry.phase, Phase::Backoff { attempt: 1, .. }), "{first:?}");
    wait_for("the second exit", || {
        fx.host.drive();
        matches!(fx.host.entries()[0].carry.phase, Phase::Backoff { attempt: 2, .. })
    });
    fx.host.shutdown();
}

#[test]
fn a_service_that_gave_up_reads_crashed_needs_no_wake_up_and_a_restart_brings_it_back() {
    let mut fx = fixture(&[service("flaky", "sleep 30", false)]);
    let repo = fx.repo.path().to_path_buf();
    let gave_up = Carry {
        phase: Phase::Crashed { exit: 3, ago: 5.0 },
        streak: 5,
        exits_ago: Vec::new(),
        last_exit: Some(3),
    };
    fx.host.repos.get_mut(repo.as_path()).unwrap()[0].sup =
        Supervisor::from_carry(&gave_up, fx.host.now());
    assert_eq!(fx.word("flaky"), Word::Crashed);
    assert_eq!(fx.host.rows(&repo)[0].line, "gave up after 5 crashes, Start to try again");
    assert_eq!(fx.host.drive(), None, "a crashed service needs no wake-up");
    fx.host.command(&repo, "flaky", Command::Restart).unwrap();
    assert_eq!(fx.word("flaky"), Word::Running);
    fx.host.shutdown();
}

#[test]
fn a_restart_replaces_the_process() {
    let mut fx = fixture(&[service("db", "sleep 30", true)]);
    let old = fx.pid("db");
    fx.host.command(fx.repo.path(), "db", Command::Restart).unwrap();
    let rows = fx.host.rows(fx.repo.path());
    assert_eq!(rows[0].word, Word::Restarting, "no new process until the old one is gone");
    assert_eq!(fx.pid("db"), old);
    wait_for("the new process to run", || fx.word("db") == Word::Running);
    let new = fx.pid("db");
    assert_ne!(old, new);
    wait_for("the old process to end", || !alive(old));
    assert!(alive(new));
    fx.host.shutdown();
}

#[test]
fn a_restart_with_no_process_to_wait_for_spawns_at_once() {
    let mut fx = fixture(&[service("db", "sleep 30", true)]);
    fx.host.repos.get_mut(fx.repo.path()).unwrap()[0].term = None;
    fx.host.command(fx.repo.path(), "db", Command::Restart).unwrap();
    assert_eq!(fx.word("db"), Word::Running);
    fx.host.shutdown();
}

#[test]
fn a_service_the_owner_adds_is_saved_stopped_until_they_start_it() {
    let mut fx = fixture(&[]);
    let repo = fx.repo.path().to_path_buf();
    fx.host.add(&repo, &service("db", "sleep 30", true), Begin::Stopped).unwrap();
    assert_eq!(fx.word("db"), Word::Stopped);
    assert_eq!(
        tidewatch_core::services::services_for(&fx.paths, &repo),
        [service("db", "sleep 30", true)]
    );
    fx.host.command(&repo, "db", Command::Start).unwrap();
    assert_eq!(fx.word("db"), Word::Running);
    fx.host.shutdown();
}

#[test]
fn adding_saves_and_starts_even_without_autostart_editing_restarts_on_a_new_command_and_removing_stops_it()
 {
    let mut fx = fixture(&[]);
    let repo = fx.repo.path().to_path_buf();
    fx.host.add(&repo, &service("db", "sleep 30", false), Begin::Now).unwrap();
    assert_eq!(fx.word("db"), Word::Running);
    assert_eq!(
        tidewatch_core::services::services_for(&fx.paths, &repo),
        [service("db", "sleep 30", false)]
    );
    let refused = fx.host.add(&repo, &service("DB", "sleep 31", true), Begin::Now).unwrap_err();
    assert_eq!(refused.len(), 1, "{refused:?}");
    let before = fx.pid("db");
    fx.host.edit(&repo, "db", &Service::new("db", "sleep 30", true)).unwrap();
    assert_eq!(fx.pid("db"), before, "the same command keeps the process");
    fx.host.edit(&repo, "db", &Service::new("db", "sleep 31", true)).unwrap();
    wait_for("the new command to run", || {
        fx.host.drive();
        fx.pid("db") != before
    });
    let after = fx.pid("db");
    wait_for("the old process to end", || !alive(before));
    fx.host.remove(&repo, "db").unwrap();
    assert_eq!(fx.host.rows(&repo).len(), 0);
    assert_eq!(tidewatch_core::services::services_for(&fx.paths, &repo).len(), 0);
    fx.host.shutdown();
    wait_for("the removed service's process to end", || !alive(after));
}

#[test]
fn quitting_stops_every_service_and_closing_a_repo_stops_its_services() {
    let mut fx = fixture(&[service("db", "sleep 30", true)]);
    let pid = fx.pid("db");
    fx.host.sync(&[]);
    wait_for("the process of a closed repo to end", || !alive(pid));
    let mut other = fixture(&[service("db", "sleep 30", true)]);
    let pid = other.pid("db");
    other.host.shutdown();
    wait_for("the process to end on quit", || !alive(pid));
}

#[test]
fn a_tool_error_for_an_unknown_name_lists_the_services_there_are() {
    let mut fx = fixture(&[service("db", "sleep 30", false), service("api", "sleep 30", false)]);
    let refused = fx.host.command(fx.repo.path(), "tunnel", Command::Restart).unwrap_err();
    assert_eq!(refused, "There is no service called `tunnel`. This repo has: `db`, `api`.");
    let none = fixture(&[]).host.log_text(Path::new("/nowhere"), "x", 5).unwrap_err();
    assert!(none.contains("this repo has none"), "{none}");
    assert_eq!(
        fx.host.log_text(fx.repo.path(), "db", 5),
        Ok("`db` has printed nothing yet.".to_owned())
    );
}

#[test]
fn the_log_is_the_last_lines_the_service_printed_without_colors() {
    let mut fx =
        fixture(&[service("db", "printf '\\033[32mone\\033[0m\\ntwo\\nthree\\n'; sleep 30", true)]);
    wait_for("the output", || fx.says("db", "three"));
    assert_eq!(fx.host.log_tail(fx.repo.path(), "db", 2).unwrap(), ["two", "three"]);
    assert_eq!(fx.host.log_text(fx.repo.path(), "DB", 50).unwrap(), "one\ntwo\nthree");
    fx.host.shutdown();
}

#[test]
fn the_fake_services_are_three_stand_ins_that_spawn_and_write_nothing() {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let spawned = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = std::rc::Rc::clone(&spawned);
    let spawn: Spawner = Box::new(move |_, _| {
        counter.set(counter.get() + 1);
        unreachable!("a stand-in never spawns")
    });
    let mut host = Host::new(Some(paths.clone()), spawn, true, true);
    host.sync(&[Path::new("/work/shop")]);
    host.drive();
    let rows = host.rows(Path::new("/work/shop"));
    let seen: Vec<(&str, Word)> = rows.iter().map(|row| (row.name.as_str(), row.word)).collect();
    assert_eq!(seen, [("db", Word::Running), ("api", Word::Running), ("tunnel", Word::Crashed)]);
    assert_eq!(rows[2].line, "gave up after 5 crashes, Start to try again");
    assert_eq!(rows[0].line, "up 2h 14m \u{b7} :3309");
    assert_eq!(rows[1].line, "up 12m \u{b7} :3000");
    assert!(
        host.fake_lines(Path::new("/work/shop"), "tunnel")
            .unwrap()
            .iter()
            .any(|l| l.contains("giving up"))
    );
    assert!(host.log_tail(Path::new("/work/shop"), "db", 3).unwrap().len() <= 3);
    assert!(host.add(Path::new("/work/shop"), &service("x", "y", true), Begin::Now).is_err());
    assert!(host.command(Path::new("/work/shop"), "db", Command::Stop).is_err());
    assert_eq!(spawned.get(), 0);
    assert!(!paths.services_file().exists());
    assert_eq!(host.entries().len(), 0);
}

#[test]
fn a_tab_is_told_again_only_when_a_state_or_the_tabs_change() {
    let mut fx = fixture(&[service("db", "sleep 30", true)]);
    let repo = fx.repo.path().to_path_buf();
    let tabs = [(1u64, repo.as_path()), (2, repo.as_path())];
    let first = fx.host.lines_for(tabs.iter().copied()).unwrap();
    assert_eq!(first.len(), 2);
    assert!(first[&1].contains("db is running"), "{first:?}");
    assert_eq!(fx.host.lines_for(tabs.iter().copied()), None);
    fx.host.command(&repo, "db", Command::Stop).unwrap();
    assert!(fx.host.lines_for(tabs.iter().copied()).unwrap()[&2].contains("db is stopped"));
    let more = [(1u64, repo.as_path()), (2, repo.as_path()), (3, repo.as_path())];
    assert_eq!(fx.host.lines_for(more.iter().copied()).unwrap().len(), 3);
    let elsewhere = [(1u64, Path::new("/other"))];
    assert_eq!(fx.host.lines_for(elsewhere.iter().copied()).unwrap().len(), 0);
}

fn entry(repo: &Path, name: &str, live: bool) -> crate::handoff::ServiceEntry {
    crate::handoff::ServiceEntry {
        repo: repo.to_path_buf(),
        name: name.to_owned(),
        live,
        carry: Supervisor::new().carry(0.0),
    }
}

#[test]
fn services_travel_behind_the_tabs_and_pair_with_their_entries_by_position() {
    let repo = PathBuf::from("/work/shop");
    let handover = Handover {
        groups: vec![crate::handoff::Group {
            dir: repo.clone(),
            other: false,
            selected: 0,
            tabs: vec![crate::handoff::TabEntry {
                title: "main".into(),
                pinned: true,
                live: true,
                worktree: None,
                agent: None,
                park: None,
                keep: false,
                summary: None,
            }],
        }],
        active_group: 0,
        focus: false,
        welcome: false,
        hook_port: None,
        services: vec![
            entry(&repo, "stopped", false),
            entry(&repo, "db", true),
            entry(&repo, "api", true),
        ],
    };
    let text = serde_json::to_string(&handover).unwrap();
    let handover: Handover = serde_json::from_str(&text).unwrap();
    assert_eq!(stdusk_core::handoff::Layout::pane_count(&handover), 3);
    let mut adopt = |pane: u32, _: &Path| {
        if pane == 12 { Err(std::io::Error::other("gone")) } else { Ok(pane) }
    };
    let fresh = |_: &Path, _: Option<&str>| 0u32;
    let rebuilt =
        rebuild(&handover, vec![10, 11, 12], 0.0, Ptys { adopt: &mut adopt, fresh: &fresh })
            .unwrap();
    let seen: Vec<(&str, Option<u32>)> =
        rebuilt.services.iter().map(|s| (s.entry.name.as_str(), s.term)).collect();
    assert_eq!(seen, [("stopped", None), ("db", Some(11)), ("api", None)]);
    assert_eq!(rebuilt.notes, ["The service api was started again."]);
    let short = rebuild(&handover, vec![10, 11], 0.0, Ptys { adopt: &mut adopt, fresh: &fresh });
    assert!(short.is_err());
}

#[test]
fn a_service_pty_crosses_a_restart_still_running_with_its_uptime_and_the_old_window_never_kills_it()
{
    let mut old = fixture(&[service("db", "echo up; sleep 60", true)]);
    let repo = old.repo.path().to_path_buf();
    wait_for("output", || old.says("db", "up"));
    std::thread::sleep(Duration::from_millis(1100));
    let pid = old.pid("db");

    let mut tabs = crate::tabs::Tabs::new(crate::pane::Pane::shell(PtyTerm::spawn(
        80,
        24,
        Repaint::none(),
        &quiet_opts("sleep 60", &repo),
    )));
    let workspaces = [crate::repo::Workspace::Repo(crate::repo::Repo {
        root: repo.clone(),
        label: "shop".into(),
    })];
    let facts = crate::handoff::Facts {
        workspaces: &workspaces,
        focus: false,
        welcome: false,
        hook_port: None,
    };
    let (mut handover, order) = crate::handoff::plan(&tabs, &facts, &|_| None);
    handover.services = old.host.entries();
    assert!(handover.services[0].live);

    let dir = tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("h.sock");
    let listener = stdusk_core::handoff::listen(&socket).unwrap();
    let successor = {
        let socket = socket.clone();
        std::thread::spawn(move || {
            let mut incoming = crate::handoff::Incoming::receive(&socket).unwrap();
            let panes = incoming.take_panes();
            let handover = incoming.session().clone();
            let mut adopt = |(meta, fd): (stdusk_core::handoff::PaneMeta, std::os::fd::OwnedFd),
                             _: &Path| {
                stdusk_core::handoff::adopt_pane(
                    Repaint::none(),
                    &meta,
                    fd,
                    &quiet_opts("unused", Path::new("/")),
                )
            };
            let fresh = |_: &Path, _: Option<&str>| unreachable!("every pane is adopted");
            let rebuilt =
                rebuild(&handover, panes, 0.0, Ptys { adopt: &mut adopt, fresh: &fresh }).unwrap();
            incoming.ack().unwrap();
            rebuilt
        })
    };
    let stream = stdusk_core::handoff::accept_within(&listener, Duration::from_secs(10)).unwrap();
    let mut panes = crate::handoff::capture(&tabs, &order).unwrap();
    panes.extend(old.host.captures().unwrap());
    send_session(&stream, &handover, &panes).expect("the successor confirmed");
    drop(panes);
    crate::handoff::release(&mut tabs);
    old.host.release();
    let mut rebuilt = successor.join().unwrap();

    old.host.shutdown();
    drop(old);
    drop(tabs);
    assert!(alive(pid), "the service outlives the window that handed it over");

    let mut kept = rebuilt.services.remove(0);
    assert_eq!(kept.entry.name, "db");
    kept.term.as_mut().unwrap().arm_teardown();
    let mut new =
        Host::new(Some(TidewatchPaths::new(&dir.path().join("home"))), spawner(), true, false);
    Store::update(new.paths.as_ref().unwrap(), |store| {
        store.add(&repo, &service("db", "echo up; sleep 60", true))
    })
    .unwrap();
    new.adopt(vec![kept]);
    new.sync(&[repo.as_path()]);
    assert!(matches!(new.repos[repo.as_path()][0].sup.state(), State::Running { .. }));
    let row = new.rows(&repo).remove(0);
    assert!(row.line.starts_with("up "), "{}", row.line);
    assert!(new.entries()[0].live);
    assert_eq!(new.term_mut(&repo, "db").and_then(|term| term.shell_pid()), Some(pid));
    new.shutdown();
    wait_for("the adopted service to end on quit", || !alive(pid));
}
