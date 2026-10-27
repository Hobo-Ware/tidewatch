use std::path::PathBuf;

use super::*;

fn facts() -> Facts {
    Facts { changed: Ok(0), unmerged: Ok(0), holders: Ok(Vec::new()), session_age: None }
}

fn hours(n: u64) -> Duration {
    Duration::from_secs(n * 3600)
}

#[test]
fn what_was_read_decides_between_safe_unsafe_and_unknown() {
    let running = Holder { pid: 9, command: "claude".into() };
    let cases = [
        (facts(), Safety::Safe),
        (Facts { changed: Ok(2), ..facts() }, Safety::Unsafe(vec![Risk::Uncommitted { files: 2 }])),
        (Facts { unmerged: Ok(3), ..facts() }, Safety::Unsafe(vec![Risk::Unpushed { commits: 3 }])),
        (
            Facts { holders: Ok(vec![running.clone()]), ..facts() },
            Safety::Unsafe(vec![Risk::Running { pid: 9, command: "claude".into() }]),
        ),
        (
            Facts { session_age: Some(hours(2)), ..facts() },
            Safety::Unsafe(vec![Risk::RecentSession { minutes: 120 }]),
        ),
        (Facts { session_age: Some(hours(25)), ..facts() }, Safety::Safe),
        (
            Facts { changed: Ok(1), unmerged: Ok(1), holders: Ok(vec![running]), ..facts() },
            Safety::Unsafe(vec![
                Risk::Uncommitted { files: 1 },
                Risk::Unpushed { commits: 1 },
                Risk::Running { pid: 9, command: "claude".into() },
            ]),
        ),
        (
            Facts { changed: Err("not a repo".into()), ..facts() },
            Safety::Unknown("not a repo".into()),
        ),
        (
            Facts { unmerged: Err("bad ref".into()), changed: Ok(5), ..facts() },
            Safety::Unknown("bad ref".into()),
        ),
        (Facts { holders: Err("no lsof".into()), ..facts() }, Safety::Unknown("no lsof".into())),
        (
            Facts { changed: Err("a".into()), unmerged: Err("b".into()), ..facts() },
            Safety::Unknown("a; b".into()),
        ),
    ];
    for (given, want) in cases {
        assert_eq!(assess(given.clone()), want, "{given:?}");
    }
}

#[test]
fn only_a_safe_answer_is_safe_and_the_folder_ignores_what_only_the_branch_holds() {
    let cases = [
        (Safety::Safe, true, true),
        (Safety::Unsafe(vec![Risk::Unpushed { commits: 1 }]), false, true),
        (
            Safety::Unsafe(vec![Risk::Unpushed { commits: 1 }, Risk::Uncommitted { files: 1 }]),
            false,
            false,
        ),
        (Safety::Unsafe(vec![Risk::RecentSession { minutes: 3 }]), false, false),
        (Safety::Unknown("x".into()), false, false),
    ];
    for (safety, safe, folder) in cases {
        assert_eq!((safety.is_safe(), safety.folder_is_safe()), (safe, folder), "{safety:?}");
    }
}

#[test]
fn every_reason_reads_as_a_sentence() {
    let safety = Safety::Unsafe(vec![
        Risk::Uncommitted { files: 1 },
        Risk::Unpushed { commits: 2 },
        Risk::Running { pid: 7, command: "zsh".into() },
        Risk::RecentSession { minutes: 5 },
        Risk::RecentSession { minutes: 600 },
    ]);
    assert_eq!(
        safety.reasons(),
        [
            "1 file with uncommitted changes",
            "2 commits found nowhere else (on no remote branch, not merged)",
            "zsh (pid 7) is running in it",
            "an agent session wrote in it 5 min ago",
            "an agent session wrote in it 10 h ago",
        ]
    );
    assert_eq!(Safety::Unknown("boom".into()).reasons(), ["could not check it: boom"]);
    assert_eq!(Safety::Safe.reasons(), Vec::<String>::new());
}

struct Machine {
    holders: Option<Vec<u32>>,
    ps: Option<String>,
}

impl System for Machine {
    fn ps(&self) -> Option<String> {
        self.ps.clone()
    }
    fn cwd_of(&self, _: u32) -> Option<PathBuf> {
        None
    }
    fn git_root(&self, _: &Path) -> Option<PathBuf> {
        None
    }
    fn cwd_holders(&self, _: &Path) -> Option<Vec<u32>> {
        self.holders.clone()
    }
    fn terminate(&self, _: u32) -> bool {
        false
    }
    fn alive(&self, _: u32) -> Option<String> {
        None
    }
}

const OWN: u32 = 500;
const PS: &str = "  PID  PPID STARTED                      ARGS
  500     1 Wed Oct  7 09:29:20 2026     /Applications/Tidewatch.app/Contents/MacOS/tidewatch
  600   500 Fri Oct 16 13:20:00 2026     /bin/zsh
  601   600 Fri Oct 16 13:23:31 2026     claude --permission-mode acceptEdits
  700     1 Wed Oct 14 09:59:03 2026     claude --permission-mode bypassPermissions
";

fn watch<'a>(system: &'a Machine, owner: Owner, home: &Path) -> SystemWatch<'a> {
    let env = ScanEnv {
        home: home.to_path_buf(),
        claude_dir: home.join(".claude"),
        codex_home: None,
        copilot_home: None,
        own_pid: OWN,
    };
    SystemWatch::new(system, env, owner)
}

#[test]
fn a_leftover_counts_every_process_in_its_folder_orphans_of_a_crashed_run_included() {
    let home = tempfile::tempdir().unwrap();
    let machine = Machine { holders: Some(vec![601, 700]), ps: Some(PS.into()) };
    let found = watch(&machine, Owner::Nobody, home.path()).holders(Path::new("/w")).unwrap();
    assert_eq!(
        found,
        [
            Holder { pid: 601, command: "claude --permission-mode acceptEdits".into() },
            Holder { pid: 700, command: "claude --permission-mode bypassPermissions".into() },
        ]
    );
}

#[test]
fn a_tab_being_closed_ignores_its_own_processes_but_not_a_stranger() {
    let home = tempfile::tempdir().unwrap();
    let machine = Machine { holders: Some(vec![600, 601, 700]), ps: Some(PS.into()) };
    let found = watch(&machine, Owner::ThisWindow, home.path()).holders(Path::new("/w")).unwrap();
    assert_eq!(found.iter().map(|h| h.pid).collect::<Vec<_>>(), [700]);
}

#[test]
fn processes_that_cannot_be_listed_are_an_error_not_an_empty_list() {
    let home = tempfile::tempdir().unwrap();
    let machine = Machine { holders: None, ps: Some(PS.into()) };
    for owner in [Owner::Nobody, Owner::ThisWindow] {
        assert!(watch(&machine, owner, home.path()).holders(Path::new("/w")).is_err());
    }
}

#[test]
fn a_holder_whose_row_is_missing_still_counts_and_a_tab_cannot_prove_it_is_ours_without_ps() {
    let home = tempfile::tempdir().unwrap();
    let machine = Machine { holders: Some(vec![601]), ps: None };
    let found = watch(&machine, Owner::ThisWindow, home.path()).holders(Path::new("/w")).unwrap();
    assert_eq!(found, [Holder { pid: 601, command: "a process".into() }]);
}

#[test]
fn a_long_command_is_cut_short() {
    let long = "x".repeat(200);
    let cut = short(&long);
    assert_eq!(cut.chars().count(), COMMAND_CHARS + 3);
    assert!(cut.ends_with("..."));
}

#[test]
fn a_session_file_for_the_folder_gives_its_age_to_a_leftover_and_never_to_a_tab_being_closed() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join(".tidewatch").join("web").join("calm-comet");
    std::fs::create_dir_all(&folder).unwrap();
    let projects = home
        .path()
        .join(".claude")
        .join("projects")
        .join(crate::transcript_locate::claude_project_dir_name(&folder));
    std::fs::create_dir_all(&projects).unwrap();
    let id = "7d198e16-6157-439f-a389-34a4951f5dd8";
    let file = projects.join(format!("{id}.jsonl"));
    std::fs::write(&file, "{}\n").unwrap();
    let wrote = SystemTime::now() - Duration::from_hours(3);
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(wrote).unwrap();

    let machine = Machine { holders: Some(Vec::new()), ps: None };
    let age = watch(&machine, Owner::Nobody, home.path()).session_age(&folder).unwrap();
    assert!(age >= hours(3) && age < hours(4), "{age:?}");
    assert_eq!(watch(&machine, Owner::ThisWindow, home.path()).session_age(&folder), None);
    let elsewhere = home.path().join("other");
    assert_eq!(watch(&machine, Owner::Nobody, home.path()).session_age(&elsewhere), None);
}
