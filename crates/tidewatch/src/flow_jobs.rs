//! The git, gh and notes-file work behind sending notes, delivering a branch and the pull request
//! link, run on `Jobs` threads. Each function starts one job and returns at once; its `Done` comes
//! back through `Jobs::drain`. Jobs that rewrite a tab's notes file take `pane_jobs::NOTES_TURN` and
//! read the file first, so an agent's reply written behind the app's copy is never lost.

use std::path::PathBuf;
use std::sync::PoisonError;

use tidewatch_core::delivery::{Ahead, pushed_as};
use tidewatch_core::note_paste;
use tidewatch_core::notes::Notes;
use tidewatch_core::pr::{self, MergeReport, NewNote, Posted, PullRequest};
use tidewatch_core::pr_lines::PrDiffLines;
use tidewatch_core::tools::{GhError, Tools};

use crate::jobs::Jobs;
use crate::pane_jobs::NOTES_TURN;
use crate::post_confirm::{Kind, Plan};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    One(u64),
    /// Every note of the draft review, with the user's message for the agent (blank for none).
    Review(String),
}

pub(crate) struct SentOut {
    /// The text to paste, `None` when there was nothing to send.
    pub(crate) text: Option<String>,
    pub(crate) count: usize,
    pub(crate) ids: Vec<u64>,
    pub(crate) notes: Box<Notes>,
}

pub(crate) enum Done {
    Sent { tab: u64, auto: bool, result: Result<SentOut, String> },
    Unsent { tab: u64, result: Result<Box<Notes>, String> },
    Probed { tab: u64, found: Option<Probe> },
    PrFound { tab: u64, result: Result<Option<PullRequest>, GhError> },
    PrLines { tab: u64, lines: Option<Box<PrDiffLines>> },
    Pulled { tab: u64, result: Result<(Box<Notes>, MergeReport), GhError> },
    Posted { tab: u64, result: Result<(Posted, Box<Notes>), GhError> },
}

/// What a probe of a tab's branch found: the commits ahead of the base and the base's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    pub(crate) commits: usize,
    pub(crate) base: String,
}

/// Marks the chosen threads sent in the notes file and answers with the paste text. `agent` is the
/// id the agent signs replies with, which decides what is still pending.
pub(crate) fn send_notes(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    auto: bool,
    file: PathBuf,
    which: Which,
    agent: String,
    now: i64,
) {
    jobs.spawn(ctx, move || {
        let _turn = NOTES_TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let mut notes = Notes::load(&file);
        let (ids, message) = match &which {
            Which::One(id) => (
                notes.threads.iter().filter(|t| t.id == *id && !t.resolved).map(|t| t.id).collect(),
                "",
            ),
            Which::Review(message) => (notes.pending(&agent), message.as_str()),
        };
        let threads: Vec<_> =
            ids.iter().filter_map(|id| notes.threads.iter().find(|t| t.id == *id)).collect();
        let text = (!threads.is_empty()).then(|| note_paste::format_review(&threads, message));
        if text.is_some() {
            notes.mark_sent(&ids, now);
            if let Err(e) = notes.save(&file) {
                return Done::Sent {
                    tab,
                    auto,
                    result: Err(format!("could not save {}: {e}", file.display())),
                };
            }
        }
        Done::Sent {
            tab,
            auto,
            result: Ok(SentOut { text, count: ids.len(), ids, notes: Box::new(notes) }),
        }
    });
}

pub(crate) fn unsend_notes(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    file: PathBuf,
    ids: Vec<u64>,
) {
    jobs.spawn(ctx, move || {
        let _turn = NOTES_TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let mut notes = Notes::load(&file);
        notes.unmark_sent(&ids);
        let result = notes
            .save(&file)
            .map(|()| Box::new(notes))
            .map_err(|e| format!("could not save {}: {e}", file.display()));
        Done::Unsent { tab, result }
    });
}

/// Counts the commits the branch has on top of its base, for the card. `None` when git could not say.
pub(crate) fn probe(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    branch: String,
    merged_head: Option<String>,
) {
    jobs.spawn(ctx, move || {
        let found = Ahead::build_after(&tools, &dir, &branch, merged_head.as_deref())
            .ok()
            .map(|ahead| Probe { commits: ahead.commits.len(), base: ahead.base });
        Done::Probed { tab, found }
    });
}

pub(crate) fn find_pr(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    branch: String,
) {
    jobs.spawn(ctx, move || {
        let head = pushed_as(&tools, &dir, &branch);
        Done::PrFound { tab, result: pr::find_open(&tools, &dir, &head) }
    });
}

pub(crate) fn load_pr_lines(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    head: String,
) {
    jobs.spawn(ctx, move || Done::PrLines {
        tab,
        lines: PrDiffLines::load(&tools, &dir, &head).ok().map(Box::new),
    });
}

/// Reads the notes file again, merges the pull request's review comments into it and saves.
pub(crate) fn pull_comments(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    file: PathBuf,
    pr: PullRequest,
) {
    jobs.spawn(ctx, move || {
        let result = pr::fetch_threads(&tools, &dir, &pr).map(|threads| {
            let _turn = NOTES_TURN.lock().unwrap_or_else(PoisonError::into_inner);
            let mut notes = Notes::load(&file);
            let report = pr::merge_into(&mut notes, threads);
            if let Err(e) = notes.save(&file) {
                eprintln!("tidewatch: could not save pulled comments to {}: {e}", file.display());
            }
            (Box::new(notes), report)
        });
        Done::Pulled { tab, result }
    });
}

/// Posts the confirmed note and links the local thread to the comment GitHub made, so the next pull
/// does not bring it back as a second thread.
pub(crate) fn post(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    file: PathBuf,
    pr: PullRequest,
    plan: Plan,
) {
    jobs.spawn(ctx, move || {
        let thread = plan.thread;
        let posted = match &plan.kind {
            Kind::New { path, line, start_line, side } => {
                let note = NewNote {
                    path: path.clone(),
                    line: *line,
                    start_line: *start_line,
                    side: *side,
                    body: plan.body.clone(),
                };
                pr::post_note(&tools, &dir, &pr, &note)
            }
            Kind::Reply { root } => pr::post_reply(&tools, &dir, &pr, *root, &plan.body),
        };
        let result = posted.map(|posted| {
            let _turn = NOTES_TURN.lock().unwrap_or_else(PoisonError::into_inner);
            let mut notes = Notes::load(&file);
            if matches!(plan.kind, Kind::New { .. }) {
                notes.link_pr_thread(posted.id, thread);
            }
            notes.mark_pr_comment_seen(posted.id);
            if let Err(e) = notes.save(&file) {
                eprintln!("tidewatch: could not save the post link to {}: {e}", file.display());
            }
            (posted, Box::new(notes))
        });
        Done::Posted { tab, result }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::review::{Anchor, Comment, Side};
    use std::time::{Duration, Instant};

    fn wait(jobs: &Jobs<Done>) -> Done {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(done) = jobs.drain().into_iter().next() {
                return done;
            }
            assert!(Instant::now() < deadline, "the job never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn anchor(line: u32) -> Anchor {
        Anchor {
            side: Side::New,
            line_hint: line,
            text: "x".into(),
            before: vec![],
            after: vec![],
            span: 0,
            block: vec![],
        }
    }

    fn comment(author: &str, body: &str) -> Comment {
        Comment { author: author.into(), time: 1, body: body.into() }
    }

    fn file_with_notes() -> (tempfile::TempDir, PathBuf, [u64; 3]) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.json");
        let mut notes = Notes::default();
        let a = notes.add("a.rs", None, anchor(1), comment("You", "first"));
        let b = notes.add("b.rs", None, anchor(2), comment("You", "second"));
        let done = notes.add("c.rs", None, anchor(3), comment("You", "gone"));
        notes.set_resolved(done, true);
        notes.save(&file).unwrap();
        (dir, file, [a, b, done])
    }

    fn sent(done: Done) -> SentOut {
        match done {
            Done::Sent { result, .. } => result.unwrap(),
            _ => panic!("not a send answer"),
        }
    }

    #[test]
    fn sending_the_review_pastes_the_open_notes_marks_them_sent_and_saves_the_file() {
        let (_dir, file, [a, b, done]) = file_with_notes();
        let jobs = Jobs::new();
        send_notes(
            &jobs,
            &egui::Context::default(),
            1,
            false,
            file.clone(),
            Which::Review(String::new()),
            "claude".into(),
            500,
        );
        let out = sent(wait(&jobs));
        let text = out.text.unwrap();
        assert_eq!(out.count, 2);
        assert!(text.contains(&format!("note id {a}")) && text.contains(&format!("note id {b}")));
        assert!(!text.contains(&format!("note id {done}")));
        let saved = Notes::load(&file);
        assert_eq!(saved.sent(a).map(|s| s.at), Some(500));
        assert_eq!(saved.pending("claude"), Vec::<u64>::new());
        assert_eq!(*out.notes, saved);
    }

    #[test]
    fn a_review_that_never_reached_the_agent_gives_its_notes_back_to_the_draft() {
        let (_dir, file, [a, b, _]) = file_with_notes();
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        let review = Which::Review(String::new());
        send_notes(&jobs, &ctx, 1, true, file.clone(), review, "claude".into(), 500);
        let out = sent(wait(&jobs));
        assert_eq!(out.ids, [a, b]);
        assert_eq!(Notes::load(&file).pending("claude"), Vec::<u64>::new());

        unsend_notes(&jobs, &ctx, 1, file.clone(), out.ids);
        let Done::Unsent { result, .. } = wait(&jobs) else { panic!("not an unsend answer") };
        let notes = result.unwrap();
        assert_eq!(notes.pending("claude"), [a, b]);
        assert_eq!(*notes, Notes::load(&file));
    }

    #[test]
    fn the_message_of_the_review_opens_the_paste_and_a_single_note_carries_none() {
        let (_dir, file, [a, ..]) = file_with_notes();
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        let review = Which::Review("Start with the first".into());
        send_notes(&jobs, &ctx, 1, false, file.clone(), review, "claude".into(), 1);
        let text = sent(wait(&jobs)).text.unwrap();
        assert!(text.contains("Message with the review: Start with the first"));
        send_notes(&jobs, &ctx, 1, false, file, Which::One(a), "claude".into(), 2);
        let one = sent(wait(&jobs)).text.unwrap();
        assert!(!one.contains("Message with the review"));
        assert!(one.starts_with("Tidewatch review notes (1)."));
    }

    #[test]
    fn a_second_send_has_nothing_new_and_one_note_can_be_sent_again_by_hand() {
        let (_dir, file, [a, ..]) = file_with_notes();
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        send_notes(
            &jobs,
            &ctx,
            1,
            false,
            file.clone(),
            Which::Review(String::new()),
            "claude".into(),
            1,
        );
        sent(wait(&jobs));
        send_notes(
            &jobs,
            &ctx,
            1,
            true,
            file.clone(),
            Which::Review(String::new()),
            "claude".into(),
            2,
        );
        let again = sent(wait(&jobs));
        assert_eq!((again.text, again.count), (None, 0));
        send_notes(&jobs, &ctx, 1, false, file.clone(), Which::One(a), "claude".into(), 3);
        let one = sent(wait(&jobs));
        assert_eq!(one.count, 1);
        assert_eq!(Notes::load(&file).sent(a).map(|s| s.at), Some(3));
    }

    #[test]
    fn a_reply_the_agent_wrote_behind_the_apps_copy_survives_a_send() {
        let (_dir, file, [a, ..]) = file_with_notes();
        let mut behind = Notes::load(&file);
        behind.reply(a, comment("claude", "done in abc"));
        behind.save(&file).unwrap();
        let jobs = Jobs::new();
        send_notes(
            &jobs,
            &egui::Context::default(),
            1,
            false,
            file.clone(),
            Which::Review(String::new()),
            "claude".into(),
            9,
        );
        sent(wait(&jobs));
        let saved = Notes::load(&file);
        let thread = saved.threads.iter().find(|t| t.id == a).unwrap();
        assert_eq!(thread.comments.len(), 2);
        assert_eq!(thread.comments[1].author, "claude");
    }

    #[cfg(unix)]
    fn fake_gh(dir: &std::path::Path, body: &str) -> Tools {
        use crate::test_exec::write_executable_from_a_child;
        write_executable_from_a_child(&dir.join("gh"), &format!("#!/bin/sh\n{body}\n"));
        let mut dirs = vec![dir.to_path_buf()];
        dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        Tools::with_path(std::env::join_paths(dirs).unwrap())
    }

    fn pull_request() -> PullRequest {
        PullRequest {
            number: 7,
            url: "https://github.com/o/r/pull/7".into(),
            head_sha: "abc".into(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn pulling_merges_the_prs_comments_into_the_reloaded_notes_and_a_second_pull_adds_nothing() {
        let (dir, file, [a, ..]) = file_with_notes();
        let comments = r#"[{"id":301,"diff_hunk":"@@ -0,0 +1,2 @@\n+one\n+two","path":"a.rs","commit_id":"abc","user":{"login":"rectifyer"},"body":"why?","created_at":"2026-10-13T09:00:00Z","html_url":"u","line":2,"original_line":2,"side":"RIGHT","subject_type":"line"}]"#;
        let tools = fake_gh(dir.path(), &format!("cat <<'JSON'\n{comments}\nJSON"));
        let mut behind = Notes::load(&file);
        behind.reply(a, comment("claude", "reply written behind the app"));
        behind.save(&file).unwrap();
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        pull_comments(
            &jobs,
            &ctx,
            1,
            tools.clone(),
            dir.path().to_path_buf(),
            file.clone(),
            pull_request(),
        );
        let Done::Pulled { result, .. } = wait(&jobs) else { panic!("not a pull answer") };
        let (notes, report) = result.unwrap();
        assert_eq!((report.new_threads, report.new_comments), (1, 1));
        assert_eq!(notes.threads.len(), 4);
        assert!(notes.threads.iter().any(|t| t.id == a && t.comments.len() == 2));
        pull_comments(&jobs, &ctx, 1, tools, dir.path().to_path_buf(), file, pull_request());
        let Done::Pulled { result, .. } = wait(&jobs) else { panic!("not a pull answer") };
        assert_eq!(result.unwrap().1, MergeReport::default());
    }

    #[cfg(unix)]
    #[test]
    fn posting_a_note_links_the_local_thread_so_the_next_pull_does_not_duplicate_it() {
        let (dir, file, [a, ..]) = file_with_notes();
        let tools = fake_gh(
            dir.path(),
            r#"echo '{"id":5001,"html_url":"https://github.com/o/r/pull/7#discussion_r5001"}'"#,
        );
        let plan = Plan {
            thread: a,
            kind: Kind::New { path: "a.rs".into(), line: 1, start_line: None, side: Side::New },
            body: "first".into(),
        };
        let jobs = Jobs::new();
        post(
            &jobs,
            &egui::Context::default(),
            1,
            tools,
            dir.path().to_path_buf(),
            file.clone(),
            pull_request(),
            plan,
        );
        let Done::Posted { result, .. } = wait(&jobs) else { panic!("not a post answer") };
        let (posted, notes) = result.unwrap();
        assert_eq!(posted.id, 5001);
        assert_eq!(notes.pr_root_of(a), Some(5001));
        assert!(notes.pr_comment_seen(5001));
        assert_eq!(Notes::load(&file).pr_root_of(a), Some(5001));
    }

    #[cfg(unix)]
    #[test]
    fn a_422_from_github_comes_back_as_gh_says_it_and_changes_no_notes() {
        let (dir, file, [a, ..]) = file_with_notes();
        let tools = fake_gh(
            dir.path(),
            r#"echo '{"message":"Unprocessable Entity: line must be part of the diff"}'; exit 1"#,
        );
        let before = Notes::load(&file);
        let plan = Plan {
            thread: a,
            kind: Kind::New { path: "a.rs".into(), line: 99, start_line: None, side: Side::New },
            body: "first".into(),
        };
        let jobs = Jobs::new();
        post(
            &jobs,
            &egui::Context::default(),
            1,
            tools,
            dir.path().to_path_buf(),
            file.clone(),
            pull_request(),
            plan,
        );
        let Done::Posted { result, .. } = wait(&jobs) else { panic!("not a post answer") };
        let error = result.err().unwrap().to_string();
        assert!(error.contains("line must be part of the diff"), "{error}");
        assert_eq!(Notes::load(&file), before);
    }

    #[cfg(unix)]
    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[cfg(unix)]
    fn commit(dir: &std::path::Path, name: &str, subject: &str) {
        std::fs::write(dir.join(name), subject).unwrap();
        git(dir, &["add", name]);
        git(dir, &["commit", "-q", "-m", subject]);
    }

    #[cfg(unix)]
    struct Pushing {
        _root: tempfile::TempDir,
        work: PathBuf,
        remote: PathBuf,
        gh: PathBuf,
        tools: Tools,
    }

    #[cfg(unix)]
    fn pushing() -> Pushing {
        let root = tempfile::tempdir().unwrap();
        let remote = root.path().join("remote.git");
        let work = root.path().join("work");
        let gh = root.path().join("gh-bin");
        for dir in [&remote, &work, &gh] {
            std::fs::create_dir_all(dir).unwrap();
        }
        git(&remote, &["init", "-q", "--bare", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["config", "user.name", "t"]);
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["remote", "add", "origin", remote.to_str().unwrap()]);
        commit(&work, "readme.md", "chore: init");
        git(&work, &["push", "-q", "-u", "origin", "main"]);
        git(&work, &["remote", "set-head", "origin", "main"]);
        git(&work, &["switch", "-q", "-c", "agent/jolly-dolphin"]);
        let log = root.path().join("gh.log");
        let tools = fake_gh(
            &gh,
            &format!(
                "[ \"$1\" = '--version' ] && exit 0\n\
                 echo \"$@\" >> '{log}'\n\
                 prev=''\n\
                 for a in \"$@\"; do [ \"$prev\" = '--body-file' ] && cat \"$a\" > '{log}.body'; prev=\"$a\"; done\n\
                 echo https://github.com/o/r/pull/42",
                log = log.display()
            ),
        );
        Pushing { _root: root, work, remote, gh: log, tools }
    }

    #[cfg(unix)]
    fn probed(rig: &Pushing, merged_head: Option<String>) -> Option<Probe> {
        let jobs = Jobs::new();
        probe(
            &jobs,
            &egui::Context::default(),
            1,
            rig.tools.clone(),
            rig.work.clone(),
            "agent/jolly-dolphin".into(),
            merged_head,
        );
        let Done::Probed { found, .. } = wait(&jobs) else { panic!("not a probe answer") };
        found
    }

    #[cfg(unix)]
    #[test]
    fn the_probe_counts_the_commits_and_names_the_base_without_pushing_or_calling_gh() {
        let rig = pushing();
        commit(&rig.work, "tide.rs", "feat: add the tide table");
        commit(&rig.work, "more.rs", "fix: more");
        assert_eq!(probed(&rig, None), Some(Probe { commits: 2, base: "main".into() }));
        assert_eq!(git(&rig.remote, &["branch", "--list", "agent/jolly-dolphin"]), "");
        assert!(!rig.gh.exists(), "gh was never started");
    }

    #[cfg(unix)]
    #[test]
    fn the_probe_after_a_merge_counts_only_what_came_after_it() {
        let rig = pushing();
        commit(&rig.work, "tide.rs", "feat: add the tide table");
        let merged = git(&rig.work, &["rev-parse", "HEAD"]).trim().to_owned();
        commit(&rig.work, "later.rs", "fix: after the merge");
        assert_eq!(probed(&rig, Some(merged)), Some(Probe { commits: 1, base: "main".into() }));
        assert_eq!(git(&rig.remote, &["branch", "--list", "agent/jolly-dolphin"]), "");
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_of_a_repo_git_cannot_read_says_nothing() {
        let rig = pushing();
        std::fs::remove_dir_all(rig.work.join(".git")).unwrap();
        assert_eq!(probed(&rig, None), None);
    }
}
