//! What each PR card says and which buttons it has, as plain data (PLAN.md, D4 follow-up): the
//! title, the line under it, the blocks of its body and its buttons. `pr_cards_view` only draws a
//! [`Model`], so the words and the rules about when a button shows are unit tests.

use tidewatch_core::merged_branch;
use tidewatch_core::pr_cards::{Cards, send_cap};
use tidewatch_core::pr_prompt::{self, short_step};
use tidewatch_core::pr_status::{Body, Job, ReviewComment, Via};
use tidewatch_core::prefs::{GithubPrefs, PrMode};

use crate::merge_cleanup::{self, Cleanup};
use crate::pr_watch::{Handling, Pause, Pending};

/// Most bars drawn for running checks.
const MOST_BARS: usize = 12;

/// The glyph in the card's 28 px box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    Opened,
    Running,
    Failed,
    Passed,
    Review,
    Comment(char),
    Conflict,
    Approved,
    Merged,
    Closed,
    Bang,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ink {
    Teal,
    Green,
    Red,
    Orange,
    Amber,
    Quiet,
}

/// What a click on a card asks the app for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CardAction {
    /// The card whose message to send.
    Ask(u64),
    /// A GitHub page, opened in the browser.
    Open(String),
    Rerun(u64),
    OpenNotes,
    CloseTab,
    KeepIt(u64),
    /// Remove the merged tab's worktree and delete its branch.
    Cleanup,
    PushFix(u64),
    NotNow(u64),
    SendNow(u64),
    Cancel(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Button {
    pub(crate) label: String,
    pub(crate) primary: bool,
    pub(crate) action: CardAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JobLine {
    pub(crate) passed: bool,
    pub(crate) name: String,
    /// `failed at clippy`.
    pub(crate) at: Option<String>,
    pub(crate) took: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Mono(String),
    Text(String),
    Jobs(Vec<JobLine>),
    Log(Vec<String>),
    /// `runner.rs:212` over the comment.
    Comments(Vec<(String, String)>),
    Bars {
        done: usize,
        total: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Model {
    pub(crate) icon: Icon,
    pub(crate) ink: Ink,
    pub(crate) title: String,
    pub(crate) sub: String,
    /// In place of the age on the right of the header.
    pub(crate) live: bool,
    pub(crate) body: Vec<Block>,
    pub(crate) buttons: Vec<Button>,
    /// A quiet line at the right end of the buttons.
    pub(crate) note: Option<String>,
}

/// What Tidewatch pasted, drawn as the user's bubble.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bubble {
    pub(crate) who: &'static str,
    pub(crate) tag: String,
    pub(crate) headline: String,
    /// `+ 12 log lines`.
    pub(crate) more: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    Card(Model),
    Bubble(Bubble),
}

/// Everything a card's wording depends on besides the card itself.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Context<'a> {
    pub(crate) cards: &'a Cards,
    pub(crate) profile: &'a str,
    pub(crate) mode: PrMode,
    pub(crate) handling: Handling,
    pub(crate) policy: GithubPrefs,
    /// The messages waiting for the agent to be idle.
    pub(crate) pending: &'a [Pending],
    pub(crate) url: &'a str,
    /// The checks are green right now, for the line under Approved.
    pub(crate) checks_green: bool,
    pub(crate) cleanup: &'a Cleanup,
}

fn merged_body(cleanup: &Cleanup) -> Vec<Block> {
    match cleanup {
        Cleanup::Unchecked | Cleanup::Checking => {
            vec![Block::Text("PR merged. Checking whether the worktree can go.".to_owned())]
        }
        Cleanup::Checked(report) if report.is_safe() => {
            let mut body = vec![Block::Text("PR merged. Remove worktree and branch?".to_owned())];
            if !report.ignored.is_empty() {
                body.push(Block::Text(
                    "Ignored files that exist only here and would be lost:".to_owned(),
                ));
                body.push(Block::Log(merge_cleanup::ignored_lines(report)));
            }
            body
        }
        Cleanup::Checked(report) => vec![
            Block::Text("PR merged, but this worktree holds work that is not in it.".to_owned()),
            Block::Log(merge_cleanup::blocker_lines(report)),
        ],
        Cleanup::Failed(why) => vec![Block::Text(format!(
            "PR merged. The worktree could not be checked ({why}). Closing the tab asks first."
        ))],
    }
}

fn merged_buttons(cleanup: &Cleanup, card: u64) -> Vec<Button> {
    let keep =
        Button { label: "Keep it".to_owned(), primary: false, action: CardAction::KeepIt(card) };
    let close = Button {
        label: "Close tab and clean up".to_owned(),
        primary: false,
        action: CardAction::CloseTab,
    };
    match cleanup {
        Cleanup::Unchecked | Cleanup::Checking => vec![keep],
        Cleanup::Checked(report) if report.is_safe() => vec![
            Button {
                label: "Remove worktree and branch".to_owned(),
                primary: true,
                action: CardAction::Cleanup,
            },
            keep,
        ],
        Cleanup::Checked(_) | Cleanup::Failed(_) => vec![close, keep],
    }
}

pub(crate) fn look(index: usize, context: Context<'_>) -> Option<Look> {
    let card = context.cards.cards().get(index)?;
    let current = context.cards.is_current(index);
    let hands_off = context.hands_off(&card.body);
    let asks = current && !hands_off;
    let model = |icon, ink, title: String, sub: String| Model {
        icon,
        ink,
        title,
        sub,
        live: false,
        body: Vec::new(),
        buttons: Vec::new(),
        note: None,
    };
    let ask = |verb: &str| Button {
        label: format!("Ask {} to {verb}", context.profile),
        primary: true,
        action: CardAction::Ask(card.id),
    };
    let quiet = |label: &str, action| Button { label: label.to_owned(), primary: false, action };
    let open = |label: &str, url: &str| quiet(label, CardAction::Open(url.to_owned()));
    Some(match &card.body {
        Body::Opened { number, base, branch, commits, .. } => {
            let pushed = commits.map_or_else(String::new, |n| {
                format!(" \u{b7} {n} {} pushed", if n == 1 { "commit" } else { "commits" })
            });
            let mut m = model(
                Icon::Opened,
                Ink::Teal,
                format!("Pull request #{number} opened"),
                format!("by Tidewatch{pushed}"),
            );
            m.body = vec![Block::Mono(format!("{branch} \u{2192} {base}"))];
            Look::Card(m)
        }
        Body::ChecksRunning { done, total, .. } => {
            let mut m = model(
                Icon::Running,
                Ink::Amber,
                "Checks running".to_owned(),
                format!("{done} of {total} done"),
            );
            m.live = true;
            m.body = vec![Block::Bars { done: *done, total: (*total).min(MOST_BARS) }];
            Look::Card(m)
        }
        Body::ChecksFailed { jobs, .. } => {
            let failed = jobs.iter().filter(|job| !job.passed).count();
            let mut m = model(
                Icon::Failed,
                Ink::Red,
                "Checks failed".to_owned(),
                if failed < jobs.len() {
                    format!("{failed} of {} \u{b7} {} passed", jobs.len(), jobs.len() - failed)
                } else {
                    format!("{failed} of {}", jobs.len())
                },
            );
            m.body =
                vec![Block::Jobs(jobs.iter().filter(|job| !job.passed).map(job_line).collect())];
            if let Some(job) = jobs.iter().find(|job| !job.passed && !job.log.is_empty()) {
                m.body.push(Block::Log(job.log.clone()));
            }
            if current {
                if asks {
                    m.buttons.push(ask("fix"));
                }
                if let Some(url) = jobs.iter().find(|job| !job.passed).map(|job| &job.url) {
                    m.buttons.push(open("Full log", url));
                }
                if let Some(run) = jobs.iter().find(|job| !job.passed).and_then(|job| job.run_id) {
                    m.buttons.push(quiet("Re-run", CardAction::Rerun(run)));
                }
                m.note = Some(note_of(&context, &card.body));
            }
            Look::Card(m)
        }
        Body::ChecksPassed { total, secs } => {
            let took = secs.map_or_else(String::new, |secs| format!(" \u{b7} {}", duration(secs)));
            let mut m = model(
                Icon::Passed,
                Ink::Green,
                "All checks passed".to_owned(),
                format!("{total} of {total}{took}"),
            );
            m.body = vec![Block::Text(format!(
                "Ready for review. Nothing for {} to do.",
                context.profile
            ))];
            Look::Card(m)
        }
        Body::ChangesRequested { author, body, comments } => {
            let count = comments.len();
            let mut m = model(
                Icon::Review,
                Ink::Orange,
                "Changes requested".to_owned(),
                format!(
                    "by @{author} \u{b7} {count} {}",
                    if count == 1 { "comment" } else { "comments" }
                ),
            );
            if !body.trim().is_empty() {
                m.body.push(Block::Text(body.trim().to_owned()));
            }
            if !comments.is_empty() {
                m.body.push(Block::Comments(comments.iter().map(place_and_text).collect()));
            }
            if asks {
                m.buttons.push(ask("address"));
                m.buttons.push(quiet("Open as notes", CardAction::OpenNotes));
            }
            if current {
                m.note = Some(note_of(&context, &card.body));
            }
            Look::Card(m)
        }
        Body::Commented { author, text, url } => {
            let initial = author.chars().next().map_or('?', |c| c.to_ascii_uppercase());
            let mut m = model(
                Icon::Comment(initial),
                Ink::Quiet,
                format!("@{author} commented"),
                String::new(),
            );
            m.body = vec![Block::Text(text.trim().to_owned())];
            if asks {
                m.buttons.push(ask("answer"));
                let page = if url.is_empty() { context.url } else { url };
                m.buttons.push(open("Reply myself", page));
            } else if current && hands_off {
                m.note = Some(note_of(&context, &card.body));
            }
            Look::Card(m)
        }
        Body::Conflict { base } => {
            let mut m = model(
                Icon::Conflict,
                Ink::Orange,
                format!("Conflicts with {base}"),
                format!("{base} has moved on"),
            );
            if asks {
                m.buttons.push(ask("rebase"));
            }
            if current {
                m.note = Some(note_of(&context, &card.body));
            }
            Look::Card(m)
        }
        Body::Approved { author } => {
            let green = if context.checks_green { " \u{b7} checks green" } else { "" };
            let mut m = model(
                Icon::Approved,
                Ink::Green,
                "Approved".to_owned(),
                format!("by @{author}{green}"),
            );
            if current {
                m.buttons.push(open("Merge on GitHub", context.url));
                m.note = Some("Merging stays yours".to_owned());
            }
            Look::Card(m)
        }
        Body::Merged { number, base } => {
            let mut m =
                model(Icon::Merged, Ink::Teal, format!("Merged into {base}"), format!("#{number}"));
            m.body = merged_body(context.cleanup);
            m.note = merged_branch::note(context.cards.after_merge);
            if current {
                m.buttons = merged_buttons(context.cleanup, card.id);
            }
            Look::Card(m)
        }
        Body::Closed { number } => {
            let mut m = model(
                Icon::Closed,
                Ink::Quiet,
                "Closed without merging".to_owned(),
                format!("#{number}"),
            );
            m.buttons.push(open("Reopen on GitHub", context.url));
            Look::Card(m)
        }
        Body::PushFix { commits, .. } => {
            let noun = if *commits == 1 { "commit" } else { "commits" };
            let mut m = model(
                Icon::Opened,
                Ink::Teal,
                format!("{} committed {commits} new {noun}", context.profile),
                "Not on the pull request yet".to_owned(),
            );
            if current && context.handling == Handling::Owner {
                m.buttons.push(Button {
                    label: "Push fix".to_owned(),
                    primary: true,
                    action: CardAction::PushFix(card.id),
                });
                m.buttons.push(quiet("Not now", CardAction::NotNow(card.id)));
            } else if current {
                m.note = Some("Tidewatch pushes this on its own".to_owned());
            }
            Look::Card(m)
        }
        Body::Sent { via, tag, headline, extra_lines, .. } => Look::Bubble(Bubble {
            who: match via {
                Via::You => "You, via Tidewatch",
                Via::Auto => "Tidewatch, auto-fix",
            },
            tag: tag.clone(),
            headline: headline.clone(),
            more: (*extra_lines > 0).then(|| {
                let noun = if tag == "CI failure" { "log lines" } else { "lines" };
                format!("+ {extra_lines} {noun}")
            }),
        }),
    })
}

impl Context<'_> {
    /// The agent answers this card on its own, so it offers the owner nothing to ask: the tab runs on
    /// its own, Auto-fix is on, and the card's kind is one the tab's tick boxes hand over.
    fn hands_off(&self, body: &Body) -> bool {
        self.handling != Handling::Owner && pr_prompt::wanted(&self.policy, body)
    }
}

/// The quiet line at the right of a card's buttons: what Auto-fix does with it.
fn note_of(context: &Context<'_>, body: &Body) -> String {
    if !context.hands_off(body) {
        return auto_note(context.mode).to_owned();
    }
    if context.handling == Handling::Paused {
        return format!("Auto-fix paused after {} sends", send_cap(true));
    }
    let queued = pr_prompt::ask_of(body)
        .is_some_and(|(ask, _)| context.pending.iter().any(|p| p.message.ask.tag == ask.tag));
    let verb = if queued { "Queued for" } else { "Sent to" };
    format!("{verb} {}", context.profile)
}

fn auto_note(mode: PrMode) -> &'static str {
    match mode {
        PrMode::Auto => "Auto-fix is on for this tab",
        PrMode::Ask | PrMode::Off => "Auto-fix is off for this tab",
    }
}

fn job_line(job: &Job) -> JobLine {
    JobLine {
        passed: job.passed,
        name: job.name.clone(),
        at: (!job.passed)
            .then(|| job.step.as_deref().map(short_step).filter(|step| !step.is_empty()))
            .flatten(),
        took: job.secs.map(duration),
    }
}

fn place_and_text(comment: &ReviewComment) -> (String, String) {
    let file = comment.path.rsplit('/').next().unwrap_or(&comment.path);
    let place = match comment.line {
        Some(line) => format!("{file}:{line}"),
        None => file.to_owned(),
    };
    (place, comment.text.clone())
}

/// `48s`, `3m 02s`, `1h 03m`.
pub(crate) fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// `just now`, `4m ago`, `2h ago`, `3d ago`: the local clock is not known here, so times are ages.
pub(crate) fn age(now_ms: i64, at_ms: i64) -> String {
    let secs = (now_ms - at_ms) / 1000;
    match secs {
        i64::MIN..45 => "just now".to_owned(),
        45..3600 => format!("{}m ago", (secs + 30) / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

/// A waiting row: `Will ask Builder to fix CI when it finishes this turn`.
pub(crate) fn waiting_text(profile: &str, pending: &Pending) -> String {
    format!("Will ask {profile} to {} when it finishes this turn", pending.message.ask.phrase)
}

pub(crate) fn waiting_buttons(pending: &Pending) -> [Button; 2] {
    [
        Button {
            label: "Send now".to_owned(),
            primary: false,
            action: CardAction::SendNow(pending.id),
        },
        Button {
            label: "Cancel".to_owned(),
            primary: false,
            action: CardAction::Cancel(pending.id),
        },
    ]
}

/// `PR status paused: gh is not signed in`, with the command that fixes it.
pub(crate) fn paused_text(pause: &Pause) -> (String, Option<&'static str>) {
    (format!("PR status paused: {}", pause.reason()), pause.fix())
}

/// Whether a click may open `url`: only a web page.
pub(crate) fn is_web_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace))
}

#[cfg(test)]
mod tests;
