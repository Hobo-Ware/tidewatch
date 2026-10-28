use super::*;

const DAY: u64 = 24 * 60 * 60;
const NOW: u64 = 100 * DAY;

fn closed(session: Option<&str>, age_secs: u64) -> Closed {
    Closed {
        at: NOW - age_secs,
        repo: "/work/web".into(),
        path: "/root/web/heron".into(),
        branch: "agent/heron".into(),
        sha: Some("abc123".into()),
        head: Some("abc123".into()),
        profile: "builder".into(),
        level: Level::Edits,
        agent: "claude".into(),
        session_id: session.map(str::to_owned),
        title: "heron".into(),
        branch_kept: false,
    }
}

fn ids(rows: &[&Closed]) -> Vec<(Option<String>, u64)> {
    rows.iter().map(|row| (row.session_id.clone(), NOW - row.at)).collect()
}

#[test]
fn the_listing_keeps_fourteen_days_newest_first() {
    let all = [
        closed(Some("old"), 14 * DAY + 1),
        closed(Some("edge"), 14 * DAY),
        closed(Some("recent"), 60),
        closed(Some("week"), 7 * DAY),
    ];
    let rows = listing(&all, NOW, &|_| false);
    assert_eq!(
        ids(&rows),
        [
            (Some("recent".into()), 60),
            (Some("week".into()), 7 * DAY),
            (Some("edge".into()), 14 * DAY),
        ]
    );
}

#[test]
fn a_tab_without_a_session_id_is_not_listed() {
    let all = [closed(None, 10), closed(Some(""), 20), closed(Some("kept"), 30)];
    let rows = listing(&all, NOW, &|_| false);
    assert_eq!(ids(&rows), [(Some("kept".into()), 30)]);
}

#[test]
fn a_session_open_in_a_tab_again_is_hidden() {
    let all = [closed(Some("a"), 10), closed(Some("b"), 20)];
    let rows = listing(&all, NOW, &|id| id == "a");
    assert_eq!(ids(&rows), [(Some("b".into()), 20)]);
}

#[test]
fn a_session_closed_twice_is_one_row_and_an_open_one_hides_both_lines() {
    let all = [closed(Some("a"), 5 * DAY), closed(Some("a"), 60), closed(Some("b"), 30)];
    let rows = listing(&all, NOW, &|_| false);
    assert_eq!(ids(&rows), [(Some("b".into()), 30), (Some("a".into()), 60)]);
    let hidden = listing(&all, NOW, &|id| id == "a");
    assert_eq!(ids(&hidden), [(Some("b".into()), 30)]);
}

#[test]
fn lines_are_appended_and_only_the_newest_hundred_stay() {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    for n in 0..KEEP + 5 {
        append(&paths, &closed(Some(&format!("s{n}")), 0)).unwrap();
    }
    let all = load(&paths);
    assert_eq!(all.len(), KEEP);
    assert_eq!(all[0].session_id.as_deref(), Some("s5"));
    assert_eq!(all[KEEP - 1].session_id.as_deref(), Some(format!("s{}", KEEP + 4).as_str()));
    let text = std::fs::read_to_string(paths.closed_tabs_log()).unwrap();
    assert_eq!(text.lines().count(), KEEP);
    assert!(text.ends_with('\n'));
}

#[test]
fn a_cut_or_broken_line_is_skipped_on_load() {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    append(&paths, &closed(Some("a"), 0)).unwrap();
    let mut text = std::fs::read_to_string(paths.closed_tabs_log()).unwrap();
    text.push_str("{\"at\":1,\"repo\":\n");
    std::fs::write(paths.closed_tabs_log(), text).unwrap();
    append(&paths, &closed(Some("b"), 0)).unwrap();
    let all = load(&paths);
    assert_eq!(all.iter().map(|c| c.session_id.clone().unwrap()).collect::<Vec<_>>(), ["a", "b"]);
}

#[test]
fn a_removal_entry_and_a_kept_worktree_both_become_a_line() {
    let tab = Tab {
        title: "Fix login".into(),
        agent: "claude".into(),
        profile: "builder".into(),
        level: Level::Auto,
        session_id: Some("s1".into()),
    };
    let entry = Entry {
        at: 77,
        why: "closed tab".into(),
        path: "/root/web/heron".into(),
        repo: "/work/web".into(),
        branch: "agent/heron".into(),
        sha: Some("abc".into()),
        head: Some("def".into()),
        mode: "discard".into(),
        branch_kept: false,
        lost: Vec::new(),
    };
    let removed = Closed::removed(&tab, &entry);
    assert_eq!(
        (removed.at, removed.sha.as_deref(), removed.head.as_deref()),
        (77, Some("abc"), Some("def"))
    );
    assert_eq!(
        (removed.title.as_str(), removed.level, removed.branch_kept),
        ("Fix login", Level::Auto, false)
    );
    let kept = Closed::kept(&tab, &removed.worktree(), Some("abc".into()), None);
    assert!(kept.branch_kept);
    assert_eq!(kept.worktree(), removed.worktree());
}

#[test]
fn the_saved_window_is_one_agent_tab_in_the_old_folder_with_its_session() {
    let window = closed(Some("s1"), 0).saved_window();
    assert_eq!(window.dir, std::path::Path::new("/work/web"));
    let [SavedTab::Agent(tab)] = window.tabs.as_slice() else { panic!("{window:?}") };
    assert_eq!(tab.dir, std::path::Path::new("/root/web/heron"));
    assert_eq!(tab.branch.as_deref(), Some("agent/heron"));
    assert_eq!(
        (tab.agent.as_str(), tab.profile.as_str(), tab.level),
        ("claude", "builder", Level::Edits)
    );
    assert_eq!(tab.session_id.as_deref(), Some("s1"));
}
