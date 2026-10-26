//! The git, gh and notes-file work behind sending notes, delivering a branch and the pull request
//! link, run on `Jobs` threads. Each function starts one job and returns at once; its `Done` comes
//! back through `Jobs::drain`. Jobs that rewrite a tab's notes file take `pane_jobs::NOTES_TURN` and
//! read the file first, so an agent's reply written behind the app's copy is never lost.

use std::path::PathBuf;
use std::sync::PoisonError;

use tidewatch_core::delivery::{
    Delivered, DeliveryError, DeliveryPlan, Opens, agent_body, deliver, pushed_as,
};
use tidewatch_core::note_paste;
use tidewatch_core::notes::Notes;
use tidewatch_core::pr::{self, MergeReport, NewNote, Posted, PullRequest};
use tidewatch_core::pr_lines::PrDiffLines;
use tidewatch_core::tools::{GhError, Tools};

use crate::delivery_card::Form;
use crate::flow::LedgerNote;
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
    Probed { tab: u64, commits: Option<usize> },
    Built { tab: u64, result: Result<Box<DeliveryPlan>, DeliveryError> },
    Delivered { tab: u64, result: Result<Delivered, DeliveryError> },
    AutoDelivered { tab: u64, result: Result<Delivered, Box<AutoFailed>> },
    PrFound { tab: u64, result: Result<Option<PullRequest>, GhError> },
    PrLines { tab: u64, lines: Option<Box<PrDiffLines>> },
    Pulled { tab: u64, result: Result<(Box<Notes>, MergeReport), GhError> },
    Posted { tab: u64, result: Result<(Posted, Box<Notes>), GhError> },
}

/// Auto could not push and open the draft: what it was about to send, when it got that far.
#[derive(Debug)]
pub(crate) struct AutoFailed {
    pub(crate) form: Option<Form>,
    pub(crate) error: DeliveryError,
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
    summary: Option<String>,
    merged_head: Option<String>,
) {
    jobs.spawn(ctx, move || {
        let commits = DeliveryPlan::build_after(
            &tools,
            &dir,
            &branch,
            summary.as_deref(),
            merged_head.as_deref(),
        )
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
    merged_head: Option<String>,
) {
    jobs.spawn(ctx, move || Done::Built {
        tab,
        result: DeliveryPlan::build_after(
            &tools,
            &dir,
            &branch,
            summary.as_deref(),
            merged_head.as_deref(),
        )
        .map(Box::new),
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
        result: deliver(&tools, &form.plan, &form.title, &form.body, form.opens),
    });
}

/// What Auto needs to push the branch and open the draft: where, the agent's own summary when it
/// wrote one, and what its ledger says to fall back on for the body.
pub(crate) struct AutoPush {
    pub(crate) dir: PathBuf,
    pub(crate) branch: String,
    pub(crate) summary: Option<String>,
    pub(crate) ledger: Option<LedgerNote>,
    pub(crate) merged_head: Option<String>,
}

pub(crate) fn auto_deliver(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    tab: u64,
    tools: Tools,
    push: AutoPush,
) {
    jobs.spawn(ctx, move || Done::AutoDelivered { tab, result: auto_run(&tools, &push) });
}

fn auto_run(tools: &Tools, push: &AutoPush) -> Result<Delivered, Box<AutoFailed>> {
    let failed = |form, error| Box::new(AutoFailed { form, error });
    let plan = DeliveryPlan::build_after(
        tools,
        &push.dir,
        &push.branch,
        push.summary.as_deref(),
        push.merged_head.as_deref(),
    )
    .map_err(|error| failed(None, error))?;
    let wrote_it = push.summary.is_some() && !plan.body.trim().is_empty();
    let body = match (&push.ledger, wrote_it) {
        (_, true) => plan.body.clone(),
        (Some(note), false) => agent_body(&note.goal, &note.verified, &plan.commits),
        (None, false) => agent_body("", &[], &plan.commits),
    };
    let form = Form { title: plan.title.clone(), body, plan, opens: Opens::Draft };
    deliver(tools, &form.plan, &form.title, &form.body, form.opens)
        .map_err(|error| failed(Some(form), error))
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
    use tidewatch_core::delivery::Opens;

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
    fn push_of(rig: &Pushing, summary: Option<&str>, merged_head: Option<String>) -> AutoPush {
        AutoPush {
            dir: rig.work.clone(),
            branch: "agent/jolly-dolphin".into(),
            summary: summary.map(str::to_owned),
            ledger: Some(LedgerNote {
                goal: "Add the harbor tide table".into(),
                verified: vec!["tests pass".into()],
            }),
            merged_head,
        }
    }

    #[cfg(unix)]
    fn auto_done(rig: &Pushing, push: AutoPush) -> Result<Delivered, Box<AutoFailed>> {
        let jobs = Jobs::new();
        auto_deliver(&jobs, &egui::Context::default(), 1, rig.tools.clone(), push);
        let Done::AutoDelivered { result, .. } = wait(&jobs) else { panic!("not an Auto answer") };
        result
    }

    #[cfg(unix)]
    #[test]
    fn auto_opens_a_draft_with_a_body_from_the_ledger_after_pushing_the_branch() {
        let rig = pushing();
        commit(&rig.work, "tide.rs", "feat: add the tide table");
        let delivered = auto_done(&rig, push_of(&rig, None, None)).unwrap();
        assert_eq!(delivered.url, "https://github.com/o/r/pull/42");
        let calls = std::fs::read_to_string(&rig.gh).unwrap();
        assert!(calls.starts_with("pr create "), "{calls}");
        assert!(calls.contains("--draft"), "{calls}");
        assert!(calls.contains("--head agent/jolly-dolphin"), "{calls}");
        let body = std::fs::read_to_string(format!("{}.body", rig.gh.display())).unwrap();
        assert_eq!(
            body,
            "Add the harbor tide table\n\n## Verified\n\n- tests pass\n\n## Changes\n\n- feat: add the tide table"
        );
        assert!(git(&rig.remote, &["branch", "--list", "agent/jolly-dolphin"]).contains("jolly"));
    }

    #[cfg(unix)]
    #[test]
    fn auto_uses_the_agents_own_summary_for_the_body_when_it_wrote_one() {
        let rig = pushing();
        commit(&rig.work, "tide.rs", "feat: add the tide table");
        auto_done(&rig, push_of(&rig, Some("Tide table\n\nThe harbor now lists tides."), None))
            .unwrap();
        let body = std::fs::read_to_string(format!("{}.body", rig.gh.display())).unwrap();
        assert_eq!(body, "The harbor now lists tides.");
    }

    #[cfg(unix)]
    #[test]
    fn auto_never_opens_a_pull_request_with_an_empty_body() {
        let rig = pushing();
        commit(&rig.work, "tide.rs", "feat: add the tide table");
        let bare = AutoPush { ledger: None, ..push_of(&rig, None, None) };
        auto_done(&rig, bare).unwrap();
        let body = std::fs::read_to_string(format!("{}.body", rig.gh.display())).unwrap();
        assert_eq!(body, "## Changes\n\n- feat: add the tide table");
    }

    #[cfg(unix)]
    #[test]
    fn auto_sends_what_came_after_a_merge_to_a_new_branch_never_the_merged_one() {
        let rig = pushing();
        commit(&rig.work, "tide.rs", "feat: add the tide table");
        git(&rig.work, &["push", "-q", "-u", "origin", "agent/jolly-dolphin"]);
        let merged = git(&rig.work, &["rev-parse", "HEAD"]).trim().to_owned();
        commit(&rig.work, "later.rs", "fix: after the merge");
        auto_done(&rig, push_of(&rig, None, Some(merged.clone()))).unwrap();
        let calls = std::fs::read_to_string(&rig.gh).unwrap();
        assert!(calls.contains("--head agent/jolly-dolphin-2"), "{calls}");
        assert!(calls.contains("--draft"), "{calls}");
        assert_eq!(git(&rig.remote, &["rev-parse", "agent/jolly-dolphin"]).trim(), merged);
        assert!(git(&rig.remote, &["branch", "--list", "agent/jolly-dolphin-2"]).contains("-2"));
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_push_keeps_the_text_for_the_dialog_that_waits_for_the_owner() {
        let rig = pushing();
        commit(&rig.work, "tide.rs", "feat: add the tide table");
        git(&rig.work, &["remote", "set-url", "origin", "/nowhere/at/all.git"]);
        let failed = auto_done(&rig, push_of(&rig, None, None)).unwrap_err();
        let form = failed.form.expect("the text it was about to send");
        assert_eq!(form.opens, Opens::Draft);
        assert_ne!(form.body.len(), 0);
        assert!(matches!(failed.error, DeliveryError::PushFailed(_)), "{:?}", failed.error);
    }
}
