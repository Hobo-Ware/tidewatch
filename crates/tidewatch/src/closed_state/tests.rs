use tidewatch_core::agents::Level;

use super::*;

const NOW: u64 = 40 * 86_400;

fn record(session: Option<&str>, title: &str, age: u64) -> Closed {
    Closed {
        at: NOW - age,
        repo: "/work/web".into(),
        path: format!("/root/web/{title}").into(),
        branch: format!("agent/{title}"),
        sha: Some("abc".into()),
        head: None,
        profile: "builder".into(),
        level: Level::Edits,
        agent: "claude".into(),
        session_id: session.map(str::to_owned),
        title: title.into(),
        branch_kept: false,
        label: String::new(),
        chosen_name: None,
        summary: None,
    }
}

fn web() -> &'static Path {
    Path::new("/work/web")
}

fn loaded(records: Vec<Closed>) -> State {
    let mut state = State::default();
    assert!(state.begin_load(true));
    state.finish_load(records);
    state
}

#[test]
fn rows_say_what_closed_and_when_and_skip_what_cannot_come_back() {
    let state = loaded(vec![
        record(Some("a"), "heron", 2 * 3600),
        record(None, "no-session", 60),
        record(Some("old"), "ancient", 15 * 86_400),
        record(Some("b"), "otter", 3 * 86_400),
    ]);
    let rows = state.rows(web(), NOW, &|_| false);
    let got: Vec<_> =
        rows.iter().map(|r| (r.title.as_str(), r.branch.as_str(), r.closed.as_str())).collect();
    assert_eq!(
        got,
        [("heron", "agent/heron", "closed 2h ago"), ("otter", "agent/otter", "closed 3d ago")]
    );
}

#[test]
fn a_row_shows_the_label_the_tab_had_and_its_summary_only_when_it_says_something() {
    let mut named = record(Some("a"), "heron", 60);
    named.label = "fix-login-redirect".into();
    named.summary = Some("Send people back after sign in".into());
    let mut blank = record(Some("b"), "otter", 120);
    blank.summary = Some("   ".into());
    let rows = loaded(vec![named, blank]).rows(web(), NOW, &|_| false);
    let got: Vec<_> = rows.iter().map(|r| (r.title.as_str(), r.summary.as_deref())).collect();
    assert_eq!(
        got,
        [("fix-login-redirect", Some("Send people back after sign in")), ("otter", None)]
    );
}

#[test]
fn only_the_asked_repo_shows_and_a_session_open_in_a_tab_again_hides_its_row() {
    let mut other = record(Some("b"), "otter", 60);
    other.repo = "/work/api".into();
    let state = loaded(vec![record(Some("a"), "heron", 60), other]);
    assert_eq!(state.rows(web(), NOW, &|id| id == "a"), []);
    let api = state.rows(Path::new("/work/api"), NOW, &|_| false);
    assert_eq!(api.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["otter"]);
}

#[test]
fn the_file_is_read_when_a_form_shows_and_after_a_tab_closed_one_read_at_a_time() {
    let mut state = State::default();
    assert!(!state.begin_load(false));
    assert!(state.begin_load(true));
    assert!(!state.begin_load(true), "a read is under way");
    state.finish_load(Vec::new());
    assert!(!state.begin_load(true), "nothing changed");
    state.mark_changed();
    assert!(state.begin_load(true));
    state.finish_load(Vec::new());
    state.mark_changed();
    assert!(!state.begin_load(false), "waits until a form shows");
    assert!(state.begin_load(true), "a form shows again");
}

#[test]
fn a_reopen_takes_the_newest_record_once_until_it_is_finished() {
    let mut state =
        loaded(vec![record(Some("a"), "heron", 5 * 86_400), record(Some("a"), "heron-2", 60)]);
    let taken = state.press("a", NOW).unwrap();
    assert_eq!(taken.title, "heron-2");
    assert!(state.press("a", NOW).is_none());
    assert!(state.rows(web(), NOW, &|_| false)[0].busy);
    state.finished("a");
    assert!(state.press("a", NOW).is_some());
    assert!(state.press("nobody", NOW).is_none());
}

#[test]
fn the_fake_list_has_three_rows() {
    assert_eq!(fake_rows().len(), 3);
}
