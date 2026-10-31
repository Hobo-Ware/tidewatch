//! The confirm dialog before a note goes to GitHub as the user (PLAN.md, D4): it names the pull
//! request and the repository, shows the exact text, and posts only on Post (or Enter). Esc and Cancel
//! post nothing.

use crate::brand;
use crate::reader_view::item_frame;
use crate::screen_kit::TEXT_SECONDARY;
use crate::screen_kit::{Size, body, dialog_frame, footer_row, mono, primary_button, quiet_button};
use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{Id, Key, Margin, Modifiers, RichText};
use kelp_core::review::{Side, Thread};
use tidewatch_core::notes::Notes;
use tidewatch_core::pr::PullRequest;

const DIALOG_WIDTH: f32 = 480.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A new review comment on a line of the pull request's diff. `line` is the last line of a block
    /// and `start_line` its first, as in GitHub's API.
    New { path: String, line: u32, start_line: Option<u32>, side: Side },
    /// A reply in the thread whose first comment has this GitHub id.
    Reply { root: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) thread: u64,
    pub(crate) kind: Kind,
    pub(crate) body: String,
}

impl Plan {
    /// What posting `thread` sends: a thread that is already on GitHub gets its newest comment as a
    /// reply; any other note goes as a new comment with its first comment's text.
    pub(crate) fn of(thread: &Thread, notes: &Notes) -> Option<Self> {
        if let Some(root) = notes.pr_root_of(thread.id) {
            let body = thread.comments.last()?.body.trim().to_owned();
            return Some(Self { thread: thread.id, kind: Kind::Reply { root }, body });
        }
        let body = thread.comments.first()?.body.trim().to_owned();
        let kind = Kind::New {
            path: thread.path.clone(),
            line: thread.anchor.last_line_hint(),
            start_line: (thread.anchor.span > 0).then_some(thread.anchor.line_hint),
            side: thread.anchor.side,
        };
        Some(Self { thread: thread.id, kind, body })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Asking,
    Posting,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PostConfirm {
    pub(crate) tab: u64,
    pub(crate) pr: PullRequest,
    pub(crate) repo: String,
    pub(crate) plan: Plan,
    pub(crate) phase: Phase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Post,
    Cancel,
}

pub(crate) fn heading(confirm: &PostConfirm) -> String {
    format!("Post to PR #{} in {}?", confirm.pr.number, confirm.repo)
}

pub(crate) fn detail(plan: &Plan) -> String {
    match (&plan.kind, location(plan)) {
        (Kind::New { .. }, Some(place)) => format!("A new review comment on {place}, as you."),
        _ => "A reply in this thread, as you.".to_owned(),
    }
}

/// The `path:line` (`path:a-b` for a block) a new comment lands on; a reply has none.
pub(crate) fn location(plan: &Plan) -> Option<String> {
    match &plan.kind {
        Kind::New { path, line, start_line: None, .. } => Some(format!("{path}:{line}")),
        Kind::New { path, line, start_line: Some(first), .. } => {
            Some(format!("{path}:{first}-{line}"))
        }
        Kind::Reply { .. } => None,
    }
}

/// The detail line with the place in mono.
fn detail_job(plan: &Plan) -> LayoutJob {
    let text = detail(plan);
    let mut job = LayoutJob::default();
    let prose = |ink| TextFormat::simple(body(TEXT_SECONDARY), ink);
    match location(plan)
        .and_then(|place| text.split_once(&place).map(|(a, b)| (a.to_owned(), place, b.to_owned())))
    {
        Some((before, place, after)) => {
            job.append(&before, 0.0, prose(brand::MUTED));
            job.append(&place, 0.0, TextFormat::simple(mono(12.0), brand::SOFT));
            job.append(&after, 0.0, prose(brand::MUTED));
        }
        None => job.append(&text, 0.0, prose(brand::MUTED)),
    }
    job
}

pub(crate) fn show(ctx: &egui::Context, confirm: &PostConfirm) -> Option<Answer> {
    let mut answer = None;
    let asking = confirm.phase == Phase::Asking;
    let modal = dialog_frame(ctx, Id::new("post_to_pr"), &heading(confirm), DIALOG_WIDTH, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(detail_job(&confirm.plan));
        item_frame().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&confirm.plan.body).size(13.0).color(brand::TEXT));
        });
        match &confirm.phase {
            Phase::Asking => {}
            Phase::Posting => {
                ui.label(RichText::new("Posting...").color(brand::MUTED));
            }
            Phase::Failed(message) => {
                ui.label(RichText::new(message).color(brand::RED));
            }
        }
        ui.add_space(4.0);
        let hint = match confirm.phase {
            Phase::Asking => "Enter posts  -  Esc cancels",
            Phase::Posting => "Esc closes  -  the post carries on",
            Phase::Failed(_) => "",
        };
        footer_row(ui, hint, |ui| {
            if asking && primary_button(ui, "Post to PR", true, Size::Regular).clicked() {
                answer = Some(Answer::Post);
            }
            let cancel = if matches!(confirm.phase, Phase::Failed(_)) { "Close" } else { "Cancel" };
            if quiet_button(ui, cancel, true, Size::Regular).clicked() {
                answer = Some(Answer::Cancel);
            }
        });
    });
    if answer.is_none() && asking && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        answer = Some(Answer::Post);
    }
    if answer.is_none() && modal.should_close() {
        answer = Some(Answer::Cancel);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, RawInput, Rect, pos2, vec2};
    use kelp_core::review::{Anchor, Comment};

    fn note(id: u64, comments: &[&str]) -> Thread {
        Thread {
            id,
            path: "src/a.rs".into(),
            commit: None,
            anchor: Anchor {
                side: Side::New,
                line_hint: 12,
                text: "x".into(),
                before: vec![],
                after: vec![],
                span: 0,
                block: vec![],
            },
            resolved: false,
            comments: comments
                .iter()
                .map(|body| Comment { author: "You".into(), time: 0, body: (*body).into() })
                .collect(),
        }
    }

    fn confirm(phase: Phase) -> PostConfirm {
        PostConfirm {
            tab: 4,
            pr: PullRequest {
                number: 12,
                url: "https://github.com/Hobo-Ware/demo/pull/12".into(),
                head_sha: "abc".into(),
            },
            repo: "Hobo-Ware/demo".into(),
            plan: Plan { thread: 1, kind: Kind::Reply { root: 9 }, body: "Use expiry()".into() },
            phase,
        }
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        ctx
    }

    fn frame(ctx: &egui::Context, data: &PostConfirm, events: Vec<Event>) -> Option<Answer> {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut answer = None;
        let _ = ctx.run_ui(raw, |ui| {
            answer = show(ui.ctx(), data);
        });
        answer
    }

    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn a_new_note_posts_its_first_comment_and_a_linked_thread_replies_with_its_newest() {
        let mut notes = Notes::default();
        let thread = note(1, &["first", "second"]);
        let plan = Plan::of(&thread, &notes).unwrap();
        assert_eq!(plan.body, "first");
        assert_eq!(
            plan.kind,
            Kind::New { path: "src/a.rs".into(), line: 12, start_line: None, side: Side::New }
        );
        notes.link_pr_thread(900, 1);
        let plan = Plan::of(&thread, &notes).unwrap();
        assert_eq!((plan.body.as_str(), plan.kind), ("second", Kind::Reply { root: 900 }));
        assert_eq!(Plan::of(&note(2, &[]), &notes), None);
    }

    #[test]
    fn the_heading_names_the_pull_request_and_the_repository() {
        assert_eq!(heading(&confirm(Phase::Asking)), "Post to PR #12 in Hobo-Ware/demo?");
        assert!(detail(&confirm(Phase::Asking).plan).contains("reply"));
        let new = Plan {
            kind: Kind::New { path: "a.rs".into(), line: 3, start_line: None, side: Side::New },
            ..confirm(Phase::Asking).plan
        };
        assert_eq!(detail(&new), "A new review comment on a.rs:3, as you.");
    }

    #[test]
    fn a_block_note_posts_from_its_first_line_to_its_last() {
        let mut thread = note(1, &["rename these"]);
        thread.anchor.span = 3;
        let plan = Plan::of(&thread, &Notes::default()).unwrap();
        let want =
            Kind::New { path: "src/a.rs".into(), line: 15, start_line: Some(12), side: Side::New };
        assert_eq!(plan.kind, want);
        assert_eq!(location(&plan).as_deref(), Some("src/a.rs:12-15"));
        assert_eq!(detail(&plan), "A new review comment on src/a.rs:12-15, as you.");
    }

    #[test]
    fn only_a_new_comment_has_a_place_to_show_in_mono() {
        let new = Plan {
            kind: Kind::New {
                path: "src/session.rs".into(),
                line: 1,
                start_line: None,
                side: Side::New,
            },
            ..confirm(Phase::Asking).plan
        };
        assert_eq!(location(&new).as_deref(), Some("src/session.rs:1"));
        assert_eq!(location(&confirm(Phase::Asking).plan), None);
    }

    #[test]
    fn enter_posts_and_escape_cancels_and_a_quiet_frame_does_nothing() {
        let data = confirm(Phase::Asking);
        let ctx = context();
        assert_eq!(frame(&ctx, &data, vec![]), None);
        assert_eq!(frame(&ctx, &data, vec![key(Key::Enter)]), Some(Answer::Post));
        let ctx = context();
        frame(&ctx, &data, vec![]);
        assert_eq!(frame(&ctx, &data, vec![key(Key::Escape)]), Some(Answer::Cancel));
    }

    #[test]
    fn enter_does_not_post_again_while_posting_or_after_a_failure() {
        for phase in [Phase::Posting, Phase::Failed("HTTP 422".into())] {
            let data = confirm(phase);
            let ctx = context();
            frame(&ctx, &data, vec![]);
            assert_eq!(frame(&ctx, &data, vec![key(Key::Enter)]), None);
        }
    }

    #[test]
    fn escape_answers_cancel_in_every_phase_including_posting() {
        for phase in [Phase::Asking, Phase::Posting, Phase::Failed("HTTP 422".into())] {
            let data = confirm(phase.clone());
            let ctx = context();
            frame(&ctx, &data, vec![]);
            assert_eq!(
                frame(&ctx, &data, vec![key(Key::Escape)]),
                Some(Answer::Cancel),
                "{phase:?}"
            );
        }
    }
}
