//! Dev-only switches for the screenshot and runnable checks of sending notes, delivery and the pull
//! request link, read once at startup. The harness cannot click, so each switch does once what a
//! click would, as soon as the thing it needs is there.
//!
//! - `TIDEWATCH_DEV_GH_DIR=<dir>`: look `git` and `gh` up in `<dir>` first, so a fake `gh` script is
//!   the one that runs and the real GitHub API is never reached.
//! - `TIDEWATCH_DEV_AUTO=1`: hold the newest agent tab's review until the agent is idle, once its
//!   notes are loaded; it goes out the moment the agent is Done.
//! - `TIDEWATCH_DEV_SEND=one|all`: send the first note, or the whole review, to the agent;
//!   `TIDEWATCH_DEV_SEND_MESSAGE=<text>` is the message the review goes with.
//! - `TIDEWATCH_DEV_DELIVER=ask|asked|missing`: press the delivery card's Ask button once, or show
//!   the card as `Asked <Profile> to open the pull request` or as `No pull request yet`, without
//!   waiting for an agent.
//! - `TIDEWATCH_DEV_PR=pull|ask|post`: pull the PR's comments; open the post confirm for the first
//!   note; or open it and confirm.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SendMode {
    One,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeliverMode {
    Ask,
    Asked,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrMode {
    Pull,
    Ask,
    Post,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DevFlow {
    pub(crate) gh_dir: Option<PathBuf>,
    pub(crate) message: String,
    auto: bool,
    send: Option<SendMode>,
    deliver: Option<DeliverMode>,
    pr: Option<PrMode>,
    taken: Vec<Step>,
}

/// What the app has ready for the newest agent tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fact {
    Notes,
    Card,
    Pr,
    Lines,
    Confirm,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ready(pub(crate) Vec<Fact>);

impl Ready {
    fn has(&self, fact: Fact) -> bool {
        self.0.contains(&fact)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    EnableAuto,
    Send(SendMode),
    AskCard,
    ShowAsked,
    ShowMissing,
    Pull,
    AskPost,
    ApprovePost,
}

impl DevFlow {
    pub(crate) fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        let message = var("TIDEWATCH_DEV_SEND_MESSAGE").unwrap_or_default();
        Self {
            message,
            ..Self::parse(
                var("TIDEWATCH_DEV_GH_DIR").as_deref(),
                var("TIDEWATCH_DEV_AUTO").as_deref(),
                var("TIDEWATCH_DEV_SEND").as_deref(),
                var("TIDEWATCH_DEV_DELIVER").as_deref(),
                var("TIDEWATCH_DEV_PR").as_deref(),
            )
        }
    }

    fn parse(
        gh_dir: Option<&str>,
        auto: Option<&str>,
        send: Option<&str>,
        deliver: Option<&str>,
        pr: Option<&str>,
    ) -> Self {
        Self {
            gh_dir: gh_dir.filter(|d| !d.is_empty()).map(PathBuf::from),
            auto: auto.is_some_and(|v| !v.is_empty() && v != "0"),
            send: match send {
                Some("one") => Some(SendMode::One),
                Some("all") => Some(SendMode::All),
                _ => None,
            },
            deliver: match deliver {
                Some("ask") => Some(DeliverMode::Ask),
                Some("asked") => Some(DeliverMode::Asked),
                Some("missing") => Some(DeliverMode::Missing),
                _ => None,
            },
            pr: match pr {
                Some("pull") => Some(PrMode::Pull),
                Some("ask") => Some(PrMode::Ask),
                Some("post") => Some(PrMode::Post),
                _ => None,
            },
            ..Self::default()
        }
    }

    /// `base` with the fake-`gh` directory in front when one is set.
    pub(crate) fn tool_path(&self, base: OsString) -> OsString {
        let Some(dir) = &self.gh_dir else { return base };
        let mut dirs = vec![dir.clone()];
        dirs.extend(std::env::split_paths(&base));
        std::env::join_paths(dirs).unwrap_or(base)
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.send.is_none() && self.deliver.is_none() && self.pr.is_none() && !self.auto
    }

    /// The steps to take now, each at most once and in the order a user would.
    pub(crate) fn steps(&mut self, ready: &Ready) -> Vec<Step> {
        let mut steps = Vec::new();
        if self.auto && ready.has(Fact::Notes) {
            self.auto = false;
            steps.push(Step::EnableAuto);
        }
        if let Some(mode) = self.send
            && ready.has(Fact::Notes)
        {
            self.send = None;
            steps.push(Step::Send(mode));
        }
        if let Some(mode) = self.deliver
            && ready.has(Fact::Card)
        {
            let step = match mode {
                DeliverMode::Ask => Step::AskCard,
                DeliverMode::Asked => Step::ShowAsked,
                DeliverMode::Missing => Step::ShowMissing,
            };
            if self.once(step) {
                steps.push(step);
            }
        }
        let pr = self.pr;
        match pr {
            Some(PrMode::Pull) if ready.has(Fact::Pr) => {
                self.pr = None;
                steps.push(Step::Pull);
            }
            Some(PrMode::Ask | PrMode::Post)
                if ready.has(Fact::Pr)
                    && ready.has(Fact::Lines)
                    && ready.has(Fact::Notes)
                    && self.once(Step::AskPost) =>
            {
                steps.push(Step::AskPost);
            }
            _ => {}
        }
        if self.pr == Some(PrMode::Post) && ready.has(Fact::Confirm) && self.once(Step::ApprovePost)
        {
            steps.push(Step::ApprovePost);
        }
        steps
    }

    /// True the first time `step` is asked for.
    fn once(&mut self, step: Step) -> bool {
        if self.taken.contains(&step) {
            return false;
        }
        self.taken.push(step);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(send: Option<&str>, deliver: Option<&str>, pr: Option<&str>) -> DevFlow {
        DevFlow::parse(None, None, send, deliver, pr)
    }

    #[test]
    fn no_switch_means_nothing_to_do_and_junk_values_are_ignored() {
        assert!(DevFlow::parse(None, None, None, None, None).is_idle());
        assert!(DevFlow::parse(Some(""), Some("0"), Some("some"), Some("x"), Some("y")).is_idle());
    }

    fn r(facts: &[Fact]) -> Ready {
        Ready(facts.to_vec())
    }

    #[test]
    fn a_send_waits_for_the_notes_and_goes_out_once() {
        let mut flow = dev(Some("all"), None, None);
        assert_eq!(flow.steps(&Ready::default()), []);
        assert_eq!(flow.steps(&r(&[Fact::Notes])), [Step::Send(SendMode::All)]);
        assert_eq!(flow.steps(&r(&[Fact::Notes])), []);
    }

    #[test]
    fn each_delivery_switch_takes_its_one_step_once_the_card_is_ready() {
        let table =
            [("ask", Step::AskCard), ("asked", Step::ShowAsked), ("missing", Step::ShowMissing)];
        for (value, step) in table {
            let mut flow = dev(None, Some(value), None);
            assert_eq!(flow.steps(&Ready::default()), [], "{value}: no card yet");
            assert_eq!(flow.steps(&r(&[Fact::Card])), [step], "{value}");
            assert_eq!(flow.steps(&r(&[Fact::Card])), [], "{value}: once");
        }
        assert!(dev(None, Some("open"), None).is_idle(), "the old dialog switch is gone");
    }

    #[test]
    fn a_post_waits_for_the_pr_its_lines_and_a_note_then_confirms_after_the_dialog_opens() {
        let mut flow = dev(None, None, Some("post"));
        let partial = [Fact::Pr, Fact::Notes];
        assert_eq!(flow.steps(&r(&partial)), []);
        let full = [Fact::Pr, Fact::Notes, Fact::Lines];
        assert_eq!(flow.steps(&r(&full)), [Step::AskPost]);
        assert_eq!(flow.steps(&r(&full)), []);
        let dialog = [Fact::Pr, Fact::Notes, Fact::Lines, Fact::Confirm];
        assert_eq!(flow.steps(&r(&dialog)), [Step::ApprovePost]);
        assert_eq!(flow.steps(&r(&dialog)), []);
        let mut ask = dev(None, None, Some("ask"));
        ask.steps(&r(&full));
        assert_eq!(ask.steps(&r(&dialog)), []);
    }

    #[test]
    fn a_pull_runs_once_the_pr_is_known() {
        let mut flow = dev(None, None, Some("pull"));
        assert_eq!(flow.steps(&Ready::default()), []);
        assert_eq!(flow.steps(&r(&[Fact::Pr])), [Step::Pull]);
        assert_eq!(flow.steps(&r(&[Fact::Pr])), []);
    }

    #[test]
    fn the_fake_gh_directory_goes_first_on_the_path() {
        let flow = DevFlow::parse(Some("/fake"), None, None, None, None);
        assert_eq!(
            flow.tool_path(OsString::from("/usr/bin:/bin")),
            OsString::from("/fake:/usr/bin:/bin")
        );
        let none = DevFlow::default();
        assert_eq!(none.tool_path(OsString::from("/usr/bin")), OsString::from("/usr/bin"));
    }

    #[test]
    fn auto_holds_the_review_once_the_notes_are_loaded_and_only_once() {
        let mut flow = DevFlow::parse(None, Some("1"), None, None, None);
        assert_eq!(flow.steps(&Ready::default()), []);
        assert_eq!(flow.steps(&r(&[Fact::Notes])), [Step::EnableAuto]);
        assert_eq!(flow.steps(&r(&[Fact::Notes])), []);
    }
}
