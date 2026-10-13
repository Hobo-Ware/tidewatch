//! The git, gh and notes-file work behind sending notes, delivering a branch and the pull request
//! link, run on `Jobs` threads. Each function starts one job and returns at once; its `Done` comes
//! back through `Jobs::drain`. Jobs that rewrite a tab's notes file take `pane_jobs::NOTES_TURN` and
//! read the file first, so an agent's reply written behind the app's copy is never lost.

use std::path::PathBuf;
use std::sync::PoisonError;

use tidewatch_core::delivery::{Delivered, DeliveryError, DeliveryPlan, deliver};
use tidewatch_core::note_paste;
use tidewatch_core::notes::Notes;
use tidewatch_core::pr::{self, MergeReport, NewNote, Posted, PullRequest};
use tidewatch_core::pr_lines::PrDiffLines;
use tidewatch_core::tools::{GhError, Tools};

use crate::delivery_card::Form;
use crate::jobs::Jobs;
use crate::pane_jobs::NOTES_TURN;
use crate::post_confirm::{Kind, Plan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    One(u64),
    AllPending,
}

pub(crate) struct SentOut {
    /// The text to paste, `None` when there was nothing to send.
    pub(crate) text: Option<String>,
    pub(crate) count: usize,
    pub(crate) notes: Box<Notes>,
}

pub(crate) enum Done {
    Sent { tab: u64, auto: bool, result: Result<SentOut, String> },
    Probed { tab: u64, commits: Option<usize> },
    Built { tab: u64, result: Result<Box<DeliveryPlan>, DeliveryError> },
    Delivered { tab: u64, result: Result<Delivered, DeliveryError> },
    PrFound { tab: u64, result: Result<Option<PullRequest>, GhError> },
    PrLines { tab: u64, lines: Option<Box<PrDiffLines>> },
    Pulled { tab: u64, result: Result<(Box<Notes>, MergeReport), GhError> },
    Posted { tab: u64, result: Result<(Posted, Box<Notes>), GhError> },
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
        let ids = match which {
            Which::One(id) => {
                notes.threads.iter().filter(|t| t.id == id && !t.resolved).map(|t| t.id).collect()
            }
            Which::AllPending => notes.pending(&agent),
        };
        let threads: Vec<_> =
            ids.iter().filter_map(|id| notes.threads.iter().find(|t| t.id == *id)).collect();
        let text = (!threads.is_empty()).then(|| note_paste::format(&threads));
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
            result: Ok(SentOut { text, count: ids.len(), notes: Box::new(notes) }),
        }
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
    summary: Option<String>,
) {
    jobs.spawn(ctx, move || {
        let commits = DeliveryPlan::build(&tools, &dir, &branch, summary.as_deref())
            .ok()
            .map(|plan| plan.commits.len());
        Done::Probed { tab, commits }
    });
}

pub(crate) fn build_plan(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    dir: PathBuf,
    branch: String,
    summary: Option<String>,
) {
    jobs.spawn(ctx, move || Done::Built {
        tab,
        result: DeliveryPlan::build(&tools, &dir, &branch, summary.as_deref()).map(Box::new),
    });
}

pub(crate) fn deliver_plan(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    form: Form,
) {
    jobs.spawn(ctx, move || Done::Delivered {
        tab,
        result: deliver(&tools, &form.plan, &form.title, &form.body),
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
    jobs.spawn(ctx, move || Done::PrFound { tab, result: pr::find_open(&tools, &dir, &branch) });
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
            Kind::New { path, line, side } => {
                let note = NewNote {
                    path: path.clone(),
                    line: *line,
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
        Anchor { side: Side::New, line_hint: line, text: "x".into(), before: vec![], after: vec![] }
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
    fn sending_all_pending_pastes_the_open_notes_marks_them_sent_and_saves_the_file() {
        let (_dir, file, [a, b, done]) = file_with_notes();
        let jobs = Jobs::new();
        send_notes(
            &jobs,
            &egui::Context::default(),
            1,
            false,
            file.clone(),
            Which::AllPending,
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
    fn a_second_send_has_nothing_new_and_one_note_can_be_sent_again_by_hand() {
        let (_dir, file, [a, ..]) = file_with_notes();
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        send_notes(&jobs, &ctx, 1, false, file.clone(), Which::AllPending, "claude".into(), 1);
        sent(wait(&jobs));
        send_notes(&jobs, &ctx, 1, true, file.clone(), Which::AllPending, "claude".into(), 2);
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
            Which::AllPending,
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
        use std::os::unix::fs::PermissionsExt as _;
        let gh = dir.join("gh");
        std::fs::write(&gh, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
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
            kind: Kind::New { path: "a.rs".into(), line: 1, side: Side::New },
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
            kind: Kind::New { path: "a.rs".into(), line: 99, side: Side::New },
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
}
