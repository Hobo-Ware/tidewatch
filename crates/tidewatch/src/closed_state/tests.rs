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
    }
}

fn place(repo: &Path) -> String {
    repo.file_name().unwrap().to_string_lossy().into_owned()
}

fn loaded(records: Vec<Closed>) -> State {
    let mut state = State::new(false);
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
    let card = state.card(NOW, &|_| false, &place).unwrap();
    let got: Vec<_> =
        card.rows.iter().map(|r| (r.title.as_str(), r.repo.as_str(), r.closed.as_str())).collect();
    assert_eq!(got, [("heron", "web", "closed 2h ago"), ("otter", "web", "closed 3d ago")]);
    assert!(!card.open);
}

#[test]
fn a_session_open_in_a_tab_again_hides_its_row_and_no_rows_means_no_card() {
    let state = loaded(vec![record(Some("a"), "heron", 60)]);
    assert!(state.card(NOW, &|id| id == "a", &place).is_none());
    assert!(loaded(Vec::new()).card(NOW, &|_| false, &place).is_none());
}

#[test]
fn the_file_is_read_when_the_overview_opens_and_after_a_tab_closed_one_read_at_a_time() {
    let mut state = State::new(false);
    assert!(!state.begin_load(false));
    assert!(state.begin_load(true));
    assert!(!state.begin_load(true), "a read is under way");
    state.finish_load(Vec::new());
    assert!(!state.begin_load(true), "nothing changed");
    state.mark_changed();
    assert!(state.begin_load(true));
    state.finish_load(Vec::new());
    state.mark_changed();
    assert!(!state.begin_load(false), "waits until the Overview shows");
    assert!(state.begin_load(true), "the Overview shows again");
}

#[test]
fn a_reopen_takes_the_newest_record_once_until_it_is_finished() {
    let mut state =
        loaded(vec![record(Some("a"), "heron", 5 * 86_400), record(Some("a"), "heron-2", 60)]);
    let taken = state.press("a", NOW).unwrap();
    assert_eq!(taken.title, "heron-2");
    assert!(state.press("a", NOW).is_none());
    assert!(state.card(NOW, &|_| false, &place).unwrap().rows[0].busy);
    state.finished("a");
    assert!(state.press("a", NOW).is_some());
    assert!(state.press("nobody", NOW).is_none());
}

#[test]
fn the_fold_toggles() {
    let mut state = State::new(false);
    state.toggle();
    assert!(state.is_open());
}

#[test]
fn the_fake_card_has_three_rows() {
    assert_eq!(fake_card(true).rows.len(), 3);
}
