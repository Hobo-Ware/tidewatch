//! The pull request calls (D4) against a fake `gh` that records its arguments and answers with
//! hand-written GitHub-shaped JSON. The real `gh` is never started.

mod common;

use common::{FakeGh, reply};
use kelp_core::review::Side;
use tidewatch_core::pr::{
    NewNote, Placement, PullRequest, fetch_threads, find_open, post_note, post_reply,
};
use tidewatch_core::tools::{GhError, Tools};

const COMMENTS: &str = include_str!("fixtures/pr_comments.json");

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn pull() -> PullRequest {
    PullRequest {
        number: 42,
        url: "https://github.com/acme/web/pull/42".into(),
        head_sha: "9f8e7d6c5b4a39281706f5e4d3c2b1a098765432".into(),
    }
}

#[test]
fn find_open_asks_gh_pr_view_for_the_four_fields() {
    let json = r#"{"headRefOid":"abc123","number":42,"state":"OPEN","url":"https://github.com/acme/web/pull/42"}"#;
    let gh = FakeGh::new(&[reply("pr view", 0, Box::leak(json.to_owned().into_boxed_str()), "")]);
    let tools = Tools::with_path(gh.path());
    let cwd = dir();
    let found = find_open(&tools, cwd.path(), "agent/brave-otter").unwrap();
    assert_eq!(
        found,
        Some(PullRequest {
            number: 42,
            url: "https://github.com/acme/web/pull/42".into(),
            head_sha: "abc123".into()
        })
    );
    assert_eq!(
        gh.calls(),
        [["pr", "view", "agent/brave-otter", "--json", "number,url,state,headRefOid"]]
    );
}

#[test]
fn no_pull_request_for_the_branch_is_none_not_an_error() {
    let gh = FakeGh::new(&[reply(
        "pr view",
        1,
        "",
        "no pull requests found for branch \"agent/brave-otter\"\n",
    )]);
    let tools = Tools::with_path(gh.path());
    assert_eq!(find_open(&tools, dir().path(), "agent/brave-otter"), Ok(None));
}

#[test]
fn a_merged_pull_request_is_not_open() {
    let json = r#"{"headRefOid":"abc","number":9,"state":"MERGED","url":"https://github.com/acme/web/pull/9"}"#;
    let gh = FakeGh::new(&[reply("pr view", 0, Box::leak(json.to_owned().into_boxed_str()), "")]);
    let tools = Tools::with_path(gh.path());
    assert_eq!(find_open(&tools, dir().path(), "agent/x"), Ok(None));
}

#[test]
fn find_open_sorts_gh_failures() {
    let cases = [
        (
            "none of the git remotes configured for this repository point to a known GitHub host.",
            GhError::NoGithubRemote,
        ),
        ("To get started with GitHub CLI, please run:  gh auth login", GhError::NotAuthenticated),
    ];
    for (stderr, want) in cases {
        let message: &'static str = Box::leak(stderr.to_owned().into_boxed_str());
        let gh = FakeGh::new(&[reply("pr view", 1, "", message)]);
        let tools = Tools::with_path(gh.path());
        assert_eq!(find_open(&tools, dir().path(), "b"), Err(want), "{stderr}");
    }
}

#[test]
fn a_branch_that_looks_like_an_option_never_reaches_gh() {
    let gh = FakeGh::new(&[]);
    let tools = Tools::with_path(gh.path());
    assert!(find_open(&tools, dir().path(), "--web").is_err());
    assert_eq!(gh.calls(), Vec::<Vec<String>>::new());
}

#[test]
fn fetch_threads_pages_through_gh_api_and_maps_the_comments() {
    let gh = FakeGh::new(&[reply("api repos", 0, COMMENTS, "")]);
    let tools = Tools::with_path(gh.path());
    let threads = fetch_threads(&tools, dir().path(), &pull()).unwrap();
    assert_eq!(gh.calls(), [["api", "repos/{owner}/{repo}/pulls/42/comments", "--paginate"]]);
    assert_eq!(threads.len(), 4);
    assert_eq!(threads[0].comments.len(), 3);
    assert_eq!(threads[0].placement, Placement::Line(12));
}

#[test]
fn fetch_threads_reports_unreadable_output() {
    let gh = FakeGh::new(&[reply("api repos", 0, "{ nope", "")]);
    let tools = Tools::with_path(gh.path());
    assert!(matches!(
        fetch_threads(&tools, dir().path(), &pull()),
        Err(GhError::Failed(message)) if message.contains("unreadable")
    ));
}

#[test]
fn post_note_sends_body_path_line_commit_and_side() {
    let answer = r#"{"id":777,"html_url":"https://github.com/acme/web/pull/42#discussion_r777"}"#;
    let gh =
        FakeGh::new(&[reply("api repos", 0, Box::leak(answer.to_owned().into_boxed_str()), "")]);
    let tools = Tools::with_path(gh.path());
    let note = NewNote {
        path: "src/auth.rs".into(),
        line: 12,
        start_line: None,
        side: Side::New,
        body: "Use the helper.\n\n- and add a test".into(),
    };
    let posted = post_note(&tools, dir().path(), &pull(), &note).unwrap();
    assert_eq!(posted.id, 777);
    assert_eq!(posted.url, "https://github.com/acme/web/pull/42#discussion_r777");
    assert_eq!(
        gh.calls(),
        [[
            "api",
            "repos/{owner}/{repo}/pulls/42/comments",
            "-f",
            "body=Use the helper.\n\n- and add a test",
            "-f",
            "path=src/auth.rs",
            "-F",
            "line=12",
            "-f",
            "commit_id=9f8e7d6c5b4a39281706f5e4d3c2b1a098765432",
            "-f",
            "side=RIGHT",
        ]]
    );
}

#[test]
fn a_note_on_the_old_side_is_posted_as_left() {
    let answer = r#"{"id":1,"html_url":"u"}"#;
    let gh = FakeGh::new(&[reply("api repos", 0, answer, "")]);
    let tools = Tools::with_path(gh.path());
    let note = NewNote {
        path: "a.rs".into(),
        line: 3,
        start_line: None,
        side: Side::Old,
        body: "b".into(),
    };
    post_note(&tools, dir().path(), &pull(), &note).unwrap();
    assert_eq!(gh.calls()[0].last().map(String::as_str), Some("side=LEFT"));
}

#[test]
fn a_block_note_is_posted_with_its_first_line_as_start_line_and_its_last_as_line() {
    let answer = r#"{"id":1,"html_url":"u"}"#;
    for (side, name) in [(Side::New, "RIGHT"), (Side::Old, "LEFT")] {
        let gh = FakeGh::new(&[reply("api repos", 0, answer, "")]);
        let tools = Tools::with_path(gh.path());
        let note =
            NewNote { path: "a.rs".into(), line: 15, start_line: Some(12), side, body: "b".into() };
        post_note(&tools, dir().path(), &pull(), &note).unwrap();
        let call = &gh.calls()[0];
        let at = |flag: &str| call.iter().position(|a| a == flag).unwrap();
        assert!(call.contains(&"line=15".to_owned()), "{call:?}");
        assert!(call.contains(&"start_line=12".to_owned()), "{call:?}");
        assert!(call.contains(&format!("side={name}")), "{call:?}");
        assert!(call.contains(&format!("start_side={name}")), "{call:?}");
        assert_eq!(call[at("start_line=12") - 1], "-F");
        assert_eq!(call[at(&format!("start_side={name}")) - 1], "-f");
    }
}

#[test]
fn a_one_line_note_sends_no_start_line_or_start_side() {
    let gh = FakeGh::new(&[reply("api repos", 0, r#"{"id":1,"html_url":"u"}"#, "")]);
    let tools = Tools::with_path(gh.path());
    let note = NewNote {
        path: "a.rs".into(),
        line: 3,
        start_line: None,
        side: Side::New,
        body: "b".into(),
    };
    post_note(&tools, dir().path(), &pull(), &note).unwrap();
    assert!(gh.calls()[0].iter().all(|arg| !arg.starts_with("start_")));
}

#[test]
fn a_line_outside_the_diff_comes_back_as_ghs_message() {
    let body = r#"{"message":"Unprocessable Entity","errors":["pull_request_review_thread.line must be part of the diff"]}"#;
    let gh = FakeGh::new(&[reply("api repos", 1, body, "gh: Unprocessable Entity (HTTP 422)\n")]);
    let tools = Tools::with_path(gh.path());
    let note = NewNote {
        path: "a.rs".into(),
        line: 999,
        start_line: None,
        side: Side::New,
        body: "b".into(),
    };
    assert_eq!(
        post_note(&tools, dir().path(), &pull(), &note),
        Err(GhError::Failed("gh: Unprocessable Entity (HTTP 422)".into()))
    );
}

#[test]
fn post_reply_names_the_thread_it_answers() {
    let answer = r#"{"id":778,"html_url":"https://github.com/acme/web/pull/42#discussion_r778"}"#;
    let gh =
        FakeGh::new(&[reply("api repos", 0, Box::leak(answer.to_owned().into_boxed_str()), "")]);
    let tools = Tools::with_path(gh.path());
    let posted = post_reply(&tools, dir().path(), &pull(), 101, "Fixed.").unwrap();
    assert_eq!(posted.id, 778);
    assert_eq!(
        gh.calls(),
        [[
            "api",
            "repos/{owner}/{repo}/pulls/42/comments",
            "-f",
            "body=Fixed.",
            "-F",
            "in_reply_to=101"
        ]]
    );
}

#[test]
fn an_answer_without_a_comment_id_is_an_error() {
    let gh = FakeGh::new(&[reply("api repos", 0, r#"{"ok":true}"#, "")]);
    let tools = Tools::with_path(gh.path());
    assert!(post_reply(&tools, dir().path(), &pull(), 1, "x").is_err());
}
