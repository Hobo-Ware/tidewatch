use std::collections::BTreeMap;
use std::os::fd::BorrowedFd;
use std::time::{Duration, Instant};

use stdusk_core::handoff::export::ExportPane;
use stdusk_core::handoff::{PaneMeta, accept_within, capture_pane, listen, send_session};
use stdusk_core::repaint::Repaint;
use stdusk_core::shell::ShellProfile;
use stdusk_core::terminal::SpawnOpts;

use super::*;

type Case<'a> = (&'a str, &'a [(&'a str, bool)], &'a [&'a str], Plan);

fn landing(key: &str, focused: bool) -> Landing {
    Landing { key: PathBuf::from(key), focused }
}

fn held(keys: &[&str]) -> Vec<PathBuf> {
    keys.iter().map(PathBuf::from).collect()
}

#[test]
fn panes_land_in_the_group_of_their_repo_or_in_a_new_one() {
    let cases: [Case; 6] = [
        ("no panes", &[], &["/w/web"], Plan { new_groups: vec![], targets: vec![], focus: None }),
        (
            "a repo the window holds",
            &[("/w/web", false)],
            &["/w/api", "/w/web"],
            Plan { new_groups: vec![], targets: vec![Target::Existing(1)], focus: None },
        ),
        (
            "a repo it does not hold",
            &[("/w/cli", false)],
            &["/w/web"],
            Plan { new_groups: vec![0], targets: vec![Target::New(0)], focus: None },
        ),
        (
            "two panes of one new repo share its group",
            &[("/w/cli", false), ("/w/cli", false)],
            &["/w/web"],
            Plan {
                new_groups: vec![0],
                targets: vec![Target::New(0), Target::New(0)],
                focus: None,
            },
        ),
        (
            "a mix, groups made in the order the panes first need them",
            &[("/w/web", false), ("/w/cli", false), ("/tmp/x", false), ("/w/cli", false)],
            &["/w/web"],
            Plan {
                new_groups: vec![1, 2],
                targets: vec![Target::Existing(0), Target::New(0), Target::New(1), Target::New(0)],
                focus: None,
            },
        ),
        (
            "the welcome screen holds nothing, so everything is new",
            &[("/w/web", false), ("/w/web", false)],
            &[],
            Plan {
                new_groups: vec![0],
                targets: vec![Target::New(0), Target::New(0)],
                focus: None,
            },
        ),
    ];
    for (name, panes, held_keys, want) in cases {
        let panes: Vec<Landing> = panes.iter().map(|(key, f)| landing(key, *f)).collect();
        assert_eq!(plan(&panes, &held(held_keys)), want, "{name}");
    }
}

#[test]
fn the_focused_pane_is_the_one_that_comes_to_the_front() {
    let panes = [landing("/w/web", false), landing("/w/cli", true), landing("/w/web", false)];
    assert_eq!(plan(&panes, &held(&["/w/web"])).focus, Some(1));
    let none = [landing("/w/web", false)];
    assert_eq!(plan(&none, &held(&["/w/web"])).focus, None);
    let two = [landing("/a", true), landing("/b", true)];
    assert_eq!(plan(&two, &[]).focus, Some(0), "the first one wins if a sender sets two");
}

#[test]
fn a_tab_is_named_by_stdusk_first_then_by_its_folder_then_plainly() {
    let cwd = Some(Path::new("/w/web/app"));
    let cases = [
        (Some("build"), cwd, "build"),
        (Some("  build  "), cwd, "build"),
        (Some(""), cwd, "app"),
        (Some("   "), cwd, "app"),
        (None, cwd, "app"),
        (Some("vim - notes"), None, "vim - notes"),
        (None, None, "shell"),
        (None, Some(Path::new("/")), "shell"),
    ];
    for (title, cwd, want) in cases {
        assert_eq!(name_of(title, cwd), want, "{title:?} {cwd:?}");
    }
}

/// A shell that writes what it reads to `log`, a file, so that the old window's reader of the same
/// pty, which only a real exit stops, cannot take the answer.
fn echo_opts(cwd: &Path, log: &Path) -> SpawnOpts {
    SpawnOpts {
        detect_progress: false,
        shell_integration: false,
        autosuggestions: false,
        scrollback_lines: 200,
        word_separators: " ".into(),
        bold_bright: false,
        agent_tracking: None,
        cwd: Some(cwd.to_string_lossy().into_owned()),
        profile: Some(ShellProfile {
            shell: Some("/bin/sh".into()),
            args: vec![
                "-c".into(),
                format!(
                    "while read l; do echo got:$(echo $l | tr -dc a-z) >> '{}'; done",
                    log.display()
                ),
            ],
            cwd: None,
            env: BTreeMap::new(),
        }),
    }
}

fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The adopted shell is nudged to repaint with a Ctrl-L that lands in its next line, so the
/// shell keeps only letters of what it reads.
fn answers(term: &mut PtyTerm, log: &Path, word: &str) -> bool {
    term.send(format!("{word}\n").as_bytes());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if std::fs::read_to_string(log).is_ok_and(|text| text.contains(&format!("got:{word}"))) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn git_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let init = std::process::Command::new("git").arg("-C").arg(dir).args(["init", "-q"]).status();
    assert!(init.unwrap().success());
}

#[test]
fn live_ptys_from_stdusk_cross_a_real_socket_and_keep_answering_in_the_right_places() {
    let work = tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap();
    let repo = work.path().join("web");
    let inside = repo.join("src");
    git_repo(&repo);
    std::fs::create_dir_all(&inside).unwrap();
    let loose = work.path().join("loose");
    std::fs::create_dir(&loose).unwrap();

    let logs: Vec<PathBuf> = (0..3).map(|n| work.path().join(format!("log-{n}"))).collect();
    let mut sent: Vec<PtyTerm> = [&inside, &loose, &repo]
        .into_iter()
        .zip(&logs)
        .map(|(cwd, log)| PtyTerm::spawn(80, 24, Repaint::none(), &echo_opts(cwd, log)))
        .collect();
    let sent_pids: Vec<u32> = sent.iter().filter_map(PtyTerm::shell_pid).collect();
    assert_eq!(sent_pids.len(), 3);
    let export = Export {
        panes: vec![
            ExportPane {
                title: Some("server".into()),
                cwd: Some(inside.to_string_lossy().into_owned()),
                focused: false,
            },
            ExportPane {
                title: None,
                cwd: Some(loose.to_string_lossy().into_owned()),
                focused: true,
            },
            ExportPane { title: None, cwd: None, focused: false },
        ],
    };

    let socket = work.path().join("s.sock");
    let listener = listen(&socket).unwrap();
    let sender = std::thread::spawn(move || {
        let stream = accept_within(&listener, Duration::from_secs(10)).unwrap();
        let panes: Vec<(PaneMeta, BorrowedFd<'_>)> =
            sent.iter().map(|term| capture_pane(term).unwrap()).collect();
        send_session(&stream, &export, &panes).expect("the window confirmed");
        drop(panes);
        for term in &mut sent {
            term.mark_handed_off();
        }
        sent
    });

    let ctx = egui::Context::default();
    let mut got = receive(&ctx, &socket, &repo).expect("the panes are taken over");
    let sender_side = sender.join().unwrap();

    assert_eq!(got.len(), 3);
    assert_eq!(
        got.iter().map(|pane| pane.name.as_str()).collect::<Vec<_>>(),
        ["server", "loose", "web"]
    );
    assert_eq!(got.iter().map(|pane| pane.focused).collect::<Vec<_>>(), [false, true, false]);
    let repo_key = workspace_key(&repo);
    let loose_key = workspace_key(&loose);
    assert_eq!(
        got.iter().map(|pane| pane.key.clone()).collect::<Vec<_>>(),
        [repo_key.clone(), loose_key, repo_key]
    );
    assert!(matches!(got[0].workspace, Workspace::Repo(_)));
    assert!(matches!(got[1].workspace, Workspace::Other { .. }));
    assert_eq!(got.iter().filter_map(|pane| pane.term.shell_pid()).collect::<Vec<_>>(), sent_pids);

    drop(sender_side);
    for pid in &sent_pids {
        assert!(process_alive(*pid), "pid {pid} outlives the window that handed it over");
    }
    for (pane, log) in got.iter_mut().zip(&logs) {
        assert!(answers(&mut pane.term, log, "ping"), "the adopted shell still answers");
    }
    for pane in &mut got {
        pane.term.kill();
    }
}

#[test]
fn a_socket_nobody_listens_on_adopts_nothing_and_says_why() {
    let work = tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap();
    let ctx = egui::Context::default();
    let failed = receive(&ctx, &work.path().join("gone.sock"), work.path());
    assert!(failed.is_err_and(|reason| reason.starts_with("stdusk did not hand its tabs over")));
}

#[test]
fn a_sender_that_hangs_up_before_the_panes_adopts_nothing() {
    let work = tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap();
    let socket = work.path().join("s.sock");
    let listener = listen(&socket).unwrap();
    let hangup = std::thread::spawn(move || {
        drop(accept_within(&listener, Duration::from_secs(10)).unwrap());
    });
    let ctx = egui::Context::default();
    let failed = receive(&ctx, &socket, work.path());
    hangup.join().unwrap();
    assert!(failed.is_err());
}
