use super::*;
use crate::link_board::TitleFrom;

fn link(url: &str, title: &str, seen: i64, from: TitleFrom) -> Link {
    Link {
        url: url.to_owned(),
        title: title.to_owned(),
        note: None,
        first_seen_ms: seen,
        title_from: from,
        state: crate::link_board::LinkState::Unknown,
    }
}

#[test]
fn a_saved_board_loads_back_and_a_missing_or_broken_file_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("tab").join("links.json");
    assert_eq!(load(&file), None);
    let mut board = LinkBoard::default();
    board.absorb(&link("https://a.dev/x", "A", 1, TitleFrom::Agent));
    save(&file, &board).unwrap();
    assert_eq!(load(&file), Some(board));
    std::fs::write(&file, "{ not json").unwrap();
    assert_eq!(load(&file), None);
}

#[test]
fn a_merge_keeps_every_link_ever_saved_and_returns_the_board() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("links.json");
    merge(&file, &[link("https://a.dev/1", "One", 10, TitleFrom::Agent)]).unwrap();
    let board = merge(&file, &[link("https://a.dev/2", "Two", 20, TitleFrom::Agent)]).unwrap();
    assert_eq!(board.len(), 2);
    let board = merge(&file, &[]).unwrap();
    assert_eq!(board.len(), 2, "an empty report drops nothing");
    assert_eq!(load(&file).unwrap(), board);
}

#[test]
fn a_merge_into_a_saved_board_keeps_the_first_seen_time_and_the_agents_title() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("links.json");
    merge(&file, &[link("https://a.dev/1", "From the agent", 10, TitleFrom::Agent)]).unwrap();
    let board = merge(&file, &[link("https://a.dev/1", "Artifact", 99, TitleFrom::Guess)]).unwrap();
    assert_eq!(board.len(), 1);
    assert_eq!(board.links[0].title, "From the agent");
    assert_eq!(board.links[0].first_seen_ms, 10);
}

#[test]
fn two_threads_writing_at_once_lose_no_link() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("links.json");
    std::thread::scope(|scope| {
        for lane in 0..4 {
            let file = &file;
            scope.spawn(move || {
                for n in 0..15 {
                    let url = format!("https://a.dev/{lane}/{n}");
                    merge(file, &[link(&url, "t", n, TitleFrom::Agent)]).unwrap();
                }
            });
        }
    });
    assert_eq!(load(&file).unwrap().len(), 60);
}
