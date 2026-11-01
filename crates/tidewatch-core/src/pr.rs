//! The pull request side of review comments (PLAN.md, D4), through `gh`.
//!
//! [`find_open`] finds the open pull request of a branch, [`fetch_threads`] pulls its review
//! comments in as [`PrThread`]s and [`merge_into`] folds them into a tab's [`Notes`] without
//! duplicating anything on the next pull. [`post_note`] and [`post_reply`] write to GitHub, as the
//! user: the app calls them only after the user confirmed.
//!
//! `find_open`, `fetch_threads`, `post_note` and `post_reply` run `gh` and block: call them off the UI
//! thread. The parsing and the merge are pure. Response shapes follow GitHub's REST schema for pull
//! request review comments; `gh` was checked at 2.98.0.
//!
//! GitHub's REST API does not say whether a review thread is resolved, so every imported thread is
//! open.

use std::path::Path;

use kelp_core::review::{Anchor, Comment, Side};
use serde_json::Value;

use crate::branch_name::is_usable_ref;
use crate::notes::Notes;
use crate::rfc3339;
use crate::tools::{GhError, Tools, run_gh};

const CONTEXT_LINES: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    /// The head commit, which a new review comment must name.
    pub head_sha: String,
}

/// Where a review thread sits in the pull request's diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Line(u32),
    /// The code moved on; the line is where it was when the comment was written, if known.
    Outdated(Option<u32>),
    /// A comment on the whole file.
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrComment {
    pub id: u64,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    pub body: String,
}

/// One review thread: the first comment and its replies, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrThread {
    /// GitHub's id of the first comment; replies are posted against it.
    pub root_id: u64,
    pub path: String,
    pub placement: Placement,
    pub side: Side,
    pub commit: String,
    pub url: String,
    pub anchor: Anchor,
    pub comments: Vec<PrComment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Posted {
    pub id: u64,
    pub url: String,
}

/// A note to post as a new review comment. `line` is a line of `path` in the pull request's diff
/// at its head commit, on `side`; GitHub refuses any other line. `start_line` makes it a multi-line
/// comment: GitHub's `line` is then the LAST line of the range and `start_line` the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewNote {
    pub path: String,
    pub line: u32,
    pub start_line: Option<u32>,
    pub side: Side,
    pub body: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MergeReport {
    pub new_threads: usize,
    pub new_comments: usize,
}

/// The open pull request whose head is `branch`, or `None` when there is none (a merged or closed
/// one does not count). Blocks on the network.
pub fn find_open(tools: &Tools, dir: &Path, branch: &str) -> Result<Option<PullRequest>, GhError> {
    if !is_usable_ref(branch) {
        return Err(GhError::Failed(format!("{branch:?} is not a usable branch name")));
    }
    let args = ["pr", "view", branch, "--json", "number,url,state,headRefOid"];
    match run_gh(tools, dir, &args) {
        Ok(json) => Ok(parse_pull_request(&json)),
        Err(GhError::Failed(message)) if message.contains("no pull requests found") => Ok(None),
        Err(error) => Err(error),
    }
}

/// Reads the output of `gh pr view --json number,url,state,headRefOid`. `None` unless the pull
/// request is open.
#[must_use]
pub fn parse_pull_request(json: &str) -> Option<PullRequest> {
    let value: Value = serde_json::from_str(json).ok()?;
    (value["state"].as_str()? == "OPEN").then_some(())?;
    Some(PullRequest {
        number: value["number"].as_u64()?,
        url: value["url"].as_str()?.to_owned(),
        head_sha: value["headRefOid"].as_str()?.to_owned(),
    })
}

/// Pulls every review comment of `pr` and groups them into threads. Blocks on the network.
pub fn fetch_threads(
    tools: &Tools,
    dir: &Path,
    pr: &PullRequest,
) -> Result<Vec<PrThread>, GhError> {
    let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", pr.number);
    let json = run_gh(tools, dir, &["api", &endpoint, "--paginate"])?;
    parse_threads(&json).map_err(GhError::Failed)
}

/// Posts `note` as a review comment on the head commit of `pr`. Blocks on the network.
pub fn post_note(
    tools: &Tools,
    dir: &Path,
    pr: &PullRequest,
    note: &NewNote,
) -> Result<Posted, GhError> {
    let endpoint = comments_endpoint(pr);
    let body = format!("body={}", note.body);
    let path = format!("path={}", note.path);
    let line = format!("line={}", note.line);
    let commit = format!("commit_id={}", pr.head_sha);
    let side = format!("side={}", side_name(note.side));
    let mut args =
        vec!["api", &endpoint, "-f", &body, "-f", &path, "-F", &line, "-f", &commit, "-f", &side];
    let start_line = note.start_line.map(|first| format!("start_line={first}"));
    let start_side = format!("start_side={}", side_name(note.side));
    if let Some(start_line) = &start_line {
        args.extend(["-F", start_line, "-f", &start_side]);
    }
    parse_posted(&run_gh(tools, dir, &args)?)
}

/// Posts `body` as a reply in the thread whose first comment has GitHub id `root_id`. Blocks on
/// the network.
pub fn post_reply(
    tools: &Tools,
    dir: &Path,
    pr: &PullRequest,
    root_id: u64,
    body: &str,
) -> Result<Posted, GhError> {
    let endpoint = comments_endpoint(pr);
    let body = format!("body={body}");
    let reply_to = format!("in_reply_to={root_id}");
    parse_posted(&run_gh(tools, dir, &["api", &endpoint, "-f", &body, "-F", &reply_to])?)
}

fn comments_endpoint(pr: &PullRequest) -> String {
    format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", pr.number)
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::New => "RIGHT",
        Side::Old => "LEFT",
    }
}

fn parse_posted(json: &str) -> Result<Posted, GhError> {
    let value: Value = serde_json::from_str(json)
        .map_err(|e| GhError::Failed(format!("GitHub sent an unreadable answer: {e}")))?;
    let (Some(id), Some(url)) = (value["id"].as_u64(), value["html_url"].as_str()) else {
        return Err(GhError::Failed("GitHub's answer has no comment id".into()));
    };
    Ok(Posted { id, url: url.to_owned() })
}

struct Raw {
    id: u64,
    reply_to: Option<u64>,
    author: String,
    time: i64,
    body: String,
    path: String,
    line: Option<u32>,
    original_line: Option<u32>,
    start_line: Option<u32>,
    original_start_line: Option<u32>,
    side: Side,
    commit: String,
    original_commit: String,
    hunk: String,
    file_level: bool,
    url: String,
}

fn raw_comment(value: &Value) -> Option<Raw> {
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
    let number = |key: &str| value[key].as_u64().and_then(|n| u32::try_from(n).ok());
    Some(Raw {
        id: value["id"].as_u64()?,
        reply_to: value["in_reply_to_id"].as_u64(),
        author: value["user"]["login"].as_str().unwrap_or("ghost").to_owned(),
        time: value["created_at"].as_str().and_then(rfc3339::unix_millis).map_or(0, |ms| ms / 1000),
        body: text("body"),
        path: value["path"].as_str()?.to_owned(),
        line: number("line"),
        original_line: number("original_line"),
        start_line: number("start_line"),
        original_start_line: number("original_start_line"),
        side: if value["side"].as_str() == Some("LEFT") { Side::Old } else { Side::New },
        commit: text("commit_id"),
        original_commit: text("original_commit_id"),
        hunk: text("diff_hunk"),
        file_level: value["subject_type"].as_str() == Some("file"),
        url: text("html_url"),
    })
}

/// Reads the output of `gh api .../pulls/<n>/comments --paginate`: one JSON array per page, back to
/// back (a single array of arrays, from `--slurp`, reads the same). A comment with no id or path is
/// skipped. Replies are grouped under the first comment of their thread.
pub fn parse_threads(json: &str) -> Result<Vec<PrThread>, String> {
    let mut raws: Vec<Raw> = Vec::new();
    for page in serde_json::Deserializer::from_str(json).into_iter::<Value>() {
        let page = page.map_err(|e| format!("GitHub sent unreadable comments: {e}"))?;
        collect_comments(&page, &mut raws);
    }
    raws.sort_by_key(|raw| (raw.time, raw.id));
    Ok(group(raws))
}

fn collect_comments(value: &Value, into: &mut Vec<Raw>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| collect_comments(item, into)),
        Value::Object(_) => into.extend(raw_comment(value)),
        _ => {}
    }
}

fn group(raws: Vec<Raw>) -> Vec<PrThread> {
    let ids: Vec<u64> = raws.iter().map(|raw| raw.id).collect();
    let parent_of: std::collections::HashMap<u64, u64> = raws
        .iter()
        .filter_map(|raw| raw.reply_to.filter(|p| ids.contains(p)).map(|p| (raw.id, p)))
        .collect();
    let root_of = |mut id: u64| {
        let mut hops = 0;
        while let Some(parent) = parent_of.get(&id) {
            id = *parent;
            hops += 1;
            if hops > ids.len() {
                break;
            }
        }
        id
    };
    let mut threads: Vec<PrThread> = Vec::new();
    for raw in raws {
        let root = root_of(raw.id);
        let comment = PrComment {
            id: raw.id,
            author: raw.author.clone(),
            time: raw.time,
            body: raw.body.clone(),
        };
        match threads.iter_mut().find(|t| t.root_id == root) {
            Some(thread) => thread.comments.push(comment),
            None => threads.push(thread_from_root(&raw, comment)),
        }
    }
    threads
}

fn thread_from_root(raw: &Raw, first: PrComment) -> PrThread {
    let placement = match (raw.line, raw.file_level) {
        (_, true) => Placement::File,
        (Some(line), false) => Placement::Line(line),
        (None, false) => Placement::Outdated(raw.original_line),
    };
    let (last, begin) = match raw.line {
        Some(line) => (Some(line), raw.start_line),
        None => (raw.original_line, raw.original_start_line),
    };
    let last = last.unwrap_or(1);
    let span = begin.map_or(0, |begin| last.saturating_sub(begin));
    let commit = match placement {
        Placement::Outdated(_) if !raw.original_commit.is_empty() => raw.original_commit.clone(),
        _ => raw.commit.clone(),
    };
    PrThread {
        root_id: raw.id,
        path: raw.path.clone(),
        placement,
        side: raw.side,
        commit,
        url: raw.url.clone(),
        anchor: anchor_from_hunk(&raw.hunk, raw.side, last, span),
        comments: vec![first],
    }
}

/// The last line of a review comment's diff hunk is the last line it covers, so it and the lines
/// above it make an anchor that `Anchor::locate` can find again after the code moved. `last_line` is
/// GitHub's `line` and `span` the lines before it that a multi-line comment covers (`line` minus
/// `start_line`).
fn anchor_from_hunk(hunk: &str, side: Side, last_line: u32, span: u32) -> Anchor {
    let kept: Vec<&str> = hunk
        .lines()
        .skip_while(|line| line.starts_with("@@"))
        .filter_map(|line| {
            let (marker, text) = (line.chars().next()?, line.get(1..)?);
            let on_this_side =
                matches!((marker, side), (' ', _) | ('+', Side::New) | ('-', Side::Old));
            on_this_side.then_some(text)
        })
        .collect();
    let owned = |lines: &[&str]| lines.iter().map(|line| (*line).to_owned()).collect();
    if kept.is_empty() {
        return Anchor {
            side,
            line_hint: last_line,
            text: String::new(),
            before: Vec::new(),
            after: Vec::new(),
            span: 0,
            block: Vec::new(),
        };
    }
    let span = (span as usize).min(kept.len() - 1);
    let (above, covered) = kept.split_at(kept.len() - 1 - span);
    let start = above.len().saturating_sub(CONTEXT_LINES);
    Anchor {
        side,
        line_hint: last_line.saturating_sub(span as u32),
        text: covered[0].to_owned(),
        before: owned(&above[start..]),
        after: Vec::new(),
        span: span as u32,
        block: owned(&covered[1..]),
    }
}

/// Folds `threads` into `notes`. A thread already imported gets only its new comments; one the user
/// deleted stays deleted. Imported threads start open. Nothing is sent to GitHub.
pub fn merge_into(notes: &mut Notes, threads: Vec<PrThread>) -> MergeReport {
    let mut report = MergeReport::default();
    for thread in threads {
        let comment = |c: &PrComment| Comment {
            author: c.author.clone(),
            time: c.time,
            body: c.body.clone(),
        };
        let local = match notes.pr_thread(thread.root_id) {
            Some(id) if notes.threads.iter().any(|t| t.id == id) => id,
            Some(_) => {
                thread.comments.iter().for_each(|c| notes.mark_pr_comment_seen(c.id));
                continue;
            }
            None => {
                let Some(first) = thread.comments.first() else { continue };
                let id = notes.add(
                    &thread.path,
                    Some(thread.commit.clone()).filter(|c| !c.is_empty()),
                    thread.anchor.clone(),
                    comment(first),
                );
                notes.link_pr_thread(thread.root_id, id);
                notes.mark_pr_comment_seen(first.id);
                report.new_threads += 1;
                report.new_comments += 1;
                id
            }
        };
        for c in &thread.comments {
            if notes.pr_comment_seen(c.id) {
                continue;
            }
            notes.reply(local, comment(c));
            notes.mark_pr_comment_seen(c.id);
            report.new_comments += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMENTS: &str = include_str!("../tests/fixtures/pr_comments.json");

    fn threads() -> Vec<PrThread> {
        parse_threads(COMMENTS).unwrap()
    }

    #[test]
    fn two_pages_become_four_threads_with_replies_grouped_oldest_first() {
        let threads = threads();
        let roots: Vec<u64> = threads.iter().map(|t| t.root_id).collect();
        assert_eq!(roots, [101, 201, 202, 203]);
        let first = &threads[0];
        let ids: Vec<u64> = first.comments.iter().map(|c| c.id).collect();
        assert_eq!(ids, [101, 102, 103]);
        let authors: Vec<&str> = first.comments.iter().map(|c| c.author.as_str()).collect();
        assert_eq!(authors, ["rectifyer", "vlad", "anodpixels"]);
        assert_eq!(first.comments[0].time, 1_791_799_200);
        assert_eq!(first.comments[0].body, "Use the shared `expiry()` helper here.");
    }

    #[test]
    fn a_line_comment_anchors_on_the_last_hunk_line_with_context() {
        let first = &threads()[0];
        assert_eq!(first.path, "src/auth.rs");
        assert_eq!(first.placement, Placement::Line(12));
        assert_eq!(first.side, Side::New);
        assert_eq!(first.commit, "9f8e7d6c5b4a39281706f5e4d3c2b1a098765432");
        assert_eq!(first.anchor.line_hint, 12);
        assert_eq!(first.anchor.text, "    let d = 4;");
        assert_eq!(first.anchor.before, ["    let b = 3;", "    let c = expiry();"]);
        assert_eq!(first.anchor.after, Vec::<String>::new());
    }

    fn comment_json(line: &str, start: &str, original_line: &str, original_start: &str) -> String {
        format!(
            r#"[{{"id":5,"path":"a.rs","commit_id":"c","user":{{"login":"u"}},"body":"b",
            "created_at":"2026-10-12T10:00:00Z","side":"RIGHT","subject_type":"line",
            "diff_hunk":"@@ -1,2 +1,6 @@\n a\n+b\n+c\n+d\n+e",
            "line":{line},"start_line":{start},"original_line":{original_line},
            "original_start_line":{original_start}}}]"#
        )
    }

    #[test]
    fn a_multi_line_comment_becomes_a_block_note_from_start_line_to_line() {
        let cases = [
            ("5", "3", "5", "3", 3, 2, "c", &["d", "e"][..], Placement::Line(5)),
            ("5", "null", "5", "null", 5, 0, "e", &[][..], Placement::Line(5)),
            ("5", "5", "5", "5", 5, 0, "e", &[][..], Placement::Line(5)),
            ("5", "1", "5", "1", 1, 4, "a", &["b", "c", "d", "e"][..], Placement::Line(5)),
            ("5", "-9", "5", "-9", 5, 0, "e", &[][..], Placement::Line(5)),
            ("null", "null", "5", "4", 4, 1, "d", &["e"][..], Placement::Outdated(Some(5))),
        ];
        for (line, start, original, original_start, hint, span, text, block, placement) in cases {
            let json = comment_json(line, start, original, original_start);
            let thread = &parse_threads(&json).unwrap()[0];
            let want_block: Vec<String> = block.iter().map(|l| (*l).to_owned()).collect();
            let got = (
                thread.anchor.line_hint,
                thread.anchor.span,
                thread.anchor.text.as_str(),
                &thread.anchor.block,
                thread.placement,
            );
            assert_eq!(got, (hint, span, text, &want_block, placement), "{line} {start}");
        }
    }

    #[test]
    fn a_range_longer_than_its_hunk_covers_only_what_the_hunk_shows() {
        let json = comment_json("5", "1", "5", "1").replace("\\n+c\\n+d\\n+e", "\\n+e");
        let anchor = &parse_threads(&json).unwrap()[0].anchor;
        assert_eq!((anchor.line_hint, anchor.span, anchor.text.as_str()), (3, 2, "a"));
        assert_eq!(anchor.block, ["b", "e"]);
    }

    #[test]
    fn a_left_side_comment_uses_the_old_file_lines() {
        let left = &threads()[1];
        assert_eq!(left.side, Side::Old);
        assert_eq!(left.anchor.text, "    old_call();");
        assert_eq!(left.anchor.before, ["fn main() {"]);
    }

    #[test]
    fn an_outdated_comment_keeps_its_original_line_and_commit() {
        let outdated = &threads()[2];
        assert_eq!(outdated.placement, Placement::Outdated(Some(7)));
        assert_eq!(outdated.commit, "2222222222222222222222222222222222222222");
        assert_eq!(outdated.anchor.line_hint, 7);
        assert_eq!(outdated.comments[0].author, "ghost");
    }

    #[test]
    fn a_file_comment_has_no_line() {
        let file = &threads()[3];
        assert_eq!(file.placement, Placement::File);
        assert_eq!(file.anchor.text, "");
        assert_eq!(file.anchor.line_hint, 1);
    }

    #[test]
    fn slurped_output_reads_the_same() {
        let pages: Vec<Value> = serde_json::Deserializer::from_str(COMMENTS)
            .into_iter::<Value>()
            .map(Result::unwrap)
            .collect();
        let slurped = serde_json::to_string(&pages).unwrap();
        assert_eq!(parse_threads(&slurped).unwrap(), threads());
    }

    #[test]
    fn junk_and_empty_output() {
        assert_eq!(parse_threads("").unwrap(), []);
        assert_eq!(parse_threads("[]").unwrap(), []);
        assert_eq!(parse_threads(r#"[{"message":"x"},{"id":1},{"path":"a"},3,null]"#).unwrap(), []);
        assert!(parse_threads("{ not json").is_err());
    }

    #[test]
    fn a_reply_whose_parent_is_missing_starts_its_own_thread() {
        let json = r#"[{"id":9,"in_reply_to_id":8,"path":"a.rs","body":"b","line":3,"commit_id":"c","created_at":"2026-10-12T10:00:00Z"}]"#;
        let threads = parse_threads(json).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].root_id, 9);
    }

    #[test]
    fn pull_request_json_only_counts_when_open() {
        let open = r#"{"headRefOid":"abc","number":42,"state":"OPEN","url":"https://github.com/acme/web/pull/42"}"#;
        assert_eq!(
            parse_pull_request(open),
            Some(PullRequest {
                number: 42,
                url: "https://github.com/acme/web/pull/42".into(),
                head_sha: "abc".into()
            })
        );
        assert_eq!(parse_pull_request(&open.replace("OPEN", "MERGED")), None);
        assert_eq!(parse_pull_request(&open.replace("OPEN", "CLOSED")), None);
        assert_eq!(parse_pull_request("nope"), None);
        assert_eq!(parse_pull_request(r#"{"state":"OPEN"}"#), None);
    }

    #[test]
    fn merging_twice_adds_nothing_and_new_replies_append() {
        let mut notes = Notes::default();
        let report = merge_into(&mut notes, threads());
        assert_eq!(report, MergeReport { new_threads: 4, new_comments: 6 });
        assert_eq!(notes.threads.len(), 4);
        assert_eq!(notes.threads[0].comments.len(), 3);
        assert_eq!(notes.threads[0].path, "src/auth.rs");
        assert!(notes.threads.iter().all(|t| !t.resolved));
        assert_eq!(merge_into(&mut notes, threads()), MergeReport::default());

        let mut more = threads();
        more[0].comments.push(PrComment {
            id: 104,
            author: "rectifyer".into(),
            time: 1_791_900_000,
            body: "One more thing".into(),
        });
        assert_eq!(merge_into(&mut notes, more), MergeReport { new_threads: 0, new_comments: 1 });
        assert_eq!(notes.threads[0].comments.len(), 4);
    }

    #[test]
    fn a_deleted_imported_thread_is_not_brought_back() {
        let mut notes = Notes::default();
        merge_into(&mut notes, threads());
        let id = notes.threads[0].id;
        notes.delete(id);
        assert_eq!(merge_into(&mut notes, threads()), MergeReport::default());
        assert_eq!(notes.threads.len(), 3);
    }

    #[test]
    fn a_local_reply_does_not_confuse_the_next_import() {
        let mut notes = Notes::default();
        merge_into(&mut notes, threads());
        let id = notes.threads[0].id;
        notes.reply(id, Comment { author: "You".into(), time: 1, body: "local".into() });
        assert_eq!(merge_into(&mut notes, threads()), MergeReport::default());
        assert_eq!(notes.threads[0].comments.len(), 4);
    }

    #[test]
    fn imported_links_survive_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.json");
        let mut notes = Notes::default();
        merge_into(&mut notes, threads());
        notes.save(&file).unwrap();
        let mut loaded = Notes::load(&file);
        assert_eq!(merge_into(&mut loaded, threads()), MergeReport::default());
        assert_eq!(loaded, notes);
    }
}
