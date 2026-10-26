//! The state of one tab's "push and open a PR" card (PLAN.md, D7) and the wording of its outcomes.
//! The card appears once a worktree agent is Done with commits on top of its base; opening it reads
//! the commits, the user edits the title and text, and approving pushes and opens the pull request.
//! Everything here is plain data; the git and gh work runs in `flow_jobs`.

use tidewatch_core::delivery::{Delivered, DeliveryError, DeliveryPlan, Opens};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Form {
    pub(crate) plan: DeliveryPlan,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) opens: Opens,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Delivery {
    #[default]
    Hidden,
    Probing,
    Ready {
        commits: usize,
    },
    Building {
        commits: usize,
    },
    /// Auto reads the commits, pushes and opens the draft with no dialog. The agent was told first.
    Working {
        commits: usize,
    },
    Editing(Form),
    Delivering(Form),
    Failed {
        form: Form,
        error: DeliveryError,
    },
    Delivered(Delivered),
}

/// One step of the delivery of a fix a tab that runs on its own takes without a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Auto {
    Wait,
    Approve,
    Close,
}

impl Delivery {
    pub(crate) fn probe_started(&mut self) {
        if matches!(self, Self::Hidden | Self::Ready { .. }) {
            *self = Self::Probing;
        }
    }

    /// The probe counted the commits on top of the base. No commits, or a probe that failed
    /// (`None`), hides the card.
    pub(crate) fn probe_done(&mut self, commits: Option<usize>) {
        if !matches!(self, Self::Probing) {
            return;
        }
        *self = match commits {
            Some(commits) if commits > 0 => Self::Ready { commits },
            _ => Self::Hidden,
        };
    }

    /// Auto starts on the ready card: the card is out of the way while the push runs.
    pub(crate) fn start_auto(&mut self) {
        if let Self::Ready { commits } = *self {
            *self = Self::Working { commits };
        }
    }

    /// Auto's push finished. A failure keeps what was about to be sent, in the dialog that waits for
    /// the owner. A success leaves the pull request to the chat.
    pub(crate) fn auto_finished(&mut self, result: Result<(), (Option<Form>, DeliveryError)>) {
        let Self::Working { commits } = *self else { return };
        *self = match result {
            Ok(()) => Self::Hidden,
            Err((Some(form), error)) => Self::Failed { form, error },
            Err((None, _)) => Self::Ready { commits },
        };
    }

    /// The user opened the card. True when the plan should now be built.
    pub(crate) fn open(&mut self) -> bool {
        if let Self::Ready { commits } = *self {
            *self = Self::Building { commits };
            return true;
        }
        false
    }

    pub(crate) fn built(&mut self, result: Result<DeliveryPlan, DeliveryError>) {
        let Self::Building { commits } = *self else { return };
        *self = match result {
            Ok(plan) if plan.commits.is_empty() => Self::Hidden,
            Ok(plan) => Self::Editing(Form {
                title: plan.title.clone(),
                body: plan.body.clone(),
                plan,
                opens: Opens::Ready,
            }),
            Err(_) => Self::Ready { commits },
        };
    }

    /// Cancel or Esc: back to the compact card, keeping what was typed would only confuse a later
    /// reopen (the commits may have changed), so the next open reads them again.
    pub(crate) fn cancel(&mut self) {
        let commits = match self {
            Self::Editing(form) | Self::Failed { form, .. } => form.plan.commits.len(),
            Self::Building { commits } => *commits,
            _ => return,
        };
        *self = Self::Ready { commits };
    }

    /// Approve: the form to deliver, or `None` while the title is empty.
    pub(crate) fn approve(&mut self) -> Option<Form> {
        let Self::Editing(form) = self else { return None };
        if form.title.trim().is_empty() {
            return None;
        }
        let form = form.clone();
        *self = Self::Delivering(form.clone());
        Some(form)
    }

    pub(crate) fn delivered(&mut self, result: Result<Delivered, DeliveryError>) {
        let Self::Delivering(form) = self else { return };
        let form = form.clone();
        *self = match result {
            Ok(done) => Self::Delivered(done),
            Err(error) => Self::Failed { form, error },
        };
    }

    /// From a failure back to the form, to fix the text or retry.
    pub(crate) fn back(&mut self) {
        if let Self::Failed { form, .. } = self {
            *self = Self::Editing(form.clone());
        }
    }

    /// The agent works again: a card that only offered the last stretch of work goes away. A dialog
    /// the user is in stays.
    pub(crate) fn leave_done(&mut self) {
        if matches!(self, Self::Probing | Self::Ready { .. }) {
            *self = Self::Hidden;
        }
    }

    pub(crate) fn close_success(&mut self) {
        if matches!(self, Self::Delivered(_)) {
            *self = Self::Hidden;
        }
    }

    #[cfg(test)]
    pub(crate) fn form_mut(&mut self) -> Option<&mut Form> {
        match self {
            Self::Editing(form) => Some(form),
            _ => None,
        }
    }

    /// What a tab that runs on its own does next with the card of a fix it pushes, so it never waits
    /// for a click: approve the filled form, close the success dialog. A form with no title and every
    /// failure stay with the owner.
    pub(crate) fn auto_step(&self) -> Auto {
        match self {
            Self::Editing(form) if !form.title.trim().is_empty() => Auto::Approve,
            Self::Delivered(_) => Auto::Close,
            _ => Auto::Wait,
        }
    }

    /// The compact card is shown above the panes.
    pub(crate) fn card_commits(&self) -> Option<usize> {
        match self {
            Self::Ready { commits } | Self::Building { commits } => Some(*commits),
            _ => None,
        }
    }

    /// A dialog is open over the window.
    pub(crate) fn is_modal(&self) -> bool {
        matches!(
            self,
            Self::Editing(_) | Self::Delivering(_) | Self::Failed { .. } | Self::Delivered(_)
        )
    }
}

pub(crate) fn card_text(commits: usize, after_merge: bool) -> String {
    let noun = if commits == 1 { "commit" } else { "commits" };
    if after_merge {
        format!("{commits} {noun} after the merge")
    } else {
        format!("{commits} {noun} ready")
    }
}

/// A plain fix for what went wrong, or `None` when the error text says enough.
pub(crate) fn fix_hint(error: &DeliveryError) -> Option<&'static str> {
    match error {
        DeliveryError::GhMissing => Some("Install the GitHub CLI: brew install gh"),
        DeliveryError::GhNotAuthenticated => Some("Log in once: gh auth login"),
        DeliveryError::NoRemote => {
            Some("Add the GitHub repository as the origin remote: git remote add origin <url>")
        }
        DeliveryError::NoGithubRemote => Some(
            "Tidewatch opens pull requests on GitHub only. Point origin at a GitHub repository",
        ),
        DeliveryError::PushRejected(_) => Some(
            "The remote has commits this branch lacks. Fetch and rebase the branch in the worktree, then try again",
        ),
        DeliveryError::PushFailed(_) => Some(
            "Check the network and your git credentials: run git push in the worktree to see more",
        ),
        DeliveryError::NoBase => Some("Fetch the repository so origin/HEAD, main or master exists"),
        DeliveryError::NothingToDeliver
        | DeliveryError::EmptyTitle
        | DeliveryError::BadBranch(_)
        | DeliveryError::PrFailed(_)
        | DeliveryError::Git(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tidewatch_core::delivery::CommitLine;

    fn plan(commits: usize) -> DeliveryPlan {
        DeliveryPlan {
            dir: PathBuf::from("/w"),
            branch: "agent/brave-otter".into(),
            base: "main".into(),
            head: "agent/brave-otter".into(),
            commits: (0..commits)
                .map(|n| CommitLine {
                    sha: format!("abc{n}"),
                    subject: format!("c{n}"),
                    body: String::new(),
                })
                .collect(),
            rebase: false,
            title: "Add helper".into(),
            body: "## Changes".into(),
        }
    }

    fn ready(commits: usize) -> Delivery {
        let mut card = Delivery::Hidden;
        card.probe_started();
        card.probe_done(Some(commits));
        card
    }

    #[test]
    fn the_card_appears_only_when_the_probe_found_commits() {
        assert_eq!(ready(2), Delivery::Ready { commits: 2 });
        assert_eq!(ready(0), Delivery::Hidden);
        let mut failed = Delivery::Hidden;
        failed.probe_started();
        failed.probe_done(None);
        assert_eq!(failed, Delivery::Hidden);
        let mut stray = Delivery::Hidden;
        stray.probe_done(Some(3));
        assert_eq!(stray, Delivery::Hidden);
    }

    #[test]
    fn opening_builds_the_plan_and_prefills_the_form_from_it() {
        let mut card = ready(2);
        assert!(card.open());
        assert!(!card.open());
        assert_eq!(card.card_commits(), Some(2));
        card.built(Ok(plan(2)));
        let Delivery::Editing(form) = &card else { panic!("{card:?}") };
        assert_eq!((form.title.as_str(), form.body.as_str()), ("Add helper", "## Changes"));
        assert!(card.is_modal());
        assert_eq!(card.card_commits(), None);
    }

    #[test]
    fn a_plan_that_cannot_be_built_leaves_the_card_and_one_without_commits_hides_it() {
        let mut card = ready(2);
        card.open();
        card.built(Err(DeliveryError::NoBase));
        assert_eq!(card, Delivery::Ready { commits: 2 });
        card.open();
        card.built(Ok(plan(0)));
        assert_eq!(card, Delivery::Hidden);
    }

    #[test]
    fn cancel_returns_to_the_card_with_the_commit_count() {
        let mut card = ready(2);
        card.open();
        card.built(Ok(plan(2)));
        card.cancel();
        assert_eq!(card, Delivery::Ready { commits: 2 });
        card.cancel();
        assert_eq!(card, Delivery::Ready { commits: 2 });
    }

    #[test]
    fn an_empty_title_cannot_be_approved_and_an_edited_one_is_what_goes_out() {
        let mut card = ready(1);
        card.open();
        card.built(Ok(plan(1)));
        card.form_mut().unwrap().title = "  ".into();
        assert_eq!(card.approve(), None);
        assert!(matches!(card, Delivery::Editing(_)));
        card.form_mut().unwrap().title = "Ship it".into();
        card.form_mut().unwrap().body = "because".into();
        let form = card.approve().unwrap();
        assert_eq!((form.title.as_str(), form.body.as_str()), ("Ship it", "because"));
        assert!(matches!(card, Delivery::Delivering(_)));
    }

    #[test]
    fn success_shows_the_url_and_closing_hides_the_card() {
        let mut card = ready(1);
        card.open();
        card.built(Ok(plan(1)));
        card.approve();
        let done = Delivered {
            url: "https://github.com/o/r/pull/9".into(),
            head: "agent/brave-otter".into(),
            created: true,
        };
        card.delivered(Ok(done.clone()));
        assert_eq!(card, Delivery::Delivered(done));
        assert!(card.is_modal());
        card.close_success();
        assert_eq!(card, Delivery::Hidden);
    }

    #[test]
    fn the_card_of_a_fix_is_approved_and_closed_and_failures_are_left_to_the_owner() {
        let mut card = Delivery::Hidden;
        assert_eq!(card.auto_step(), Auto::Wait);
        card.probe_started();
        assert_eq!(card.auto_step(), Auto::Wait);
        card.probe_done(Some(2));
        assert_eq!(card.auto_step(), Auto::Wait, "a ready card is opened by a click or by Auto");
        card.open();
        assert_eq!(card.auto_step(), Auto::Wait, "building");
        card.built(Ok(plan(2)));
        assert_eq!(card.auto_step(), Auto::Approve);
        card.form_mut().unwrap().title = " ".into();
        assert_eq!(card.auto_step(), Auto::Wait, "no title, nothing to send");
        card.form_mut().unwrap().title = "Add helper".into();
        card.approve();
        assert_eq!(card.auto_step(), Auto::Wait, "delivering");
        card.delivered(Err(DeliveryError::GhNotAuthenticated));
        assert_eq!(card.auto_step(), Auto::Wait, "a failure is the owner's");
        card.back();
        card.approve();
        card.delivered(Ok(Delivered {
            url: "https://github.com/o/r/pull/9".into(),
            head: "agent/brave-otter".into(),
            created: true,
        }));
        assert_eq!(card.auto_step(), Auto::Close);
    }

    #[test]
    fn auto_pushes_with_no_dialog_and_a_failure_waits_for_the_owner_with_its_text() {
        let mut card = ready(2);
        card.start_auto();
        assert_eq!(card, Delivery::Working { commits: 2 });
        assert_eq!(card.card_commits(), None, "the bar is out of the way while it pushes");
        assert!(!card.is_modal(), "no dialog for a push Auto does");
        card.leave_done();
        assert_eq!(
            card,
            Delivery::Working { commits: 2 },
            "the agent working again does not end it"
        );
        let form = Form {
            title: "Add helper".into(),
            body: "text".into(),
            plan: plan(2),
            opens: Opens::Draft,
        };
        card.auto_finished(Err((Some(form.clone()), DeliveryError::GhNotAuthenticated)));
        assert_eq!(card, Delivery::Failed { form, error: DeliveryError::GhNotAuthenticated });
        assert!(card.is_modal());

        let mut done = ready(1);
        done.start_auto();
        done.auto_finished(Ok(()));
        assert_eq!(done, Delivery::Hidden);

        let mut unread = ready(3);
        unread.start_auto();
        unread.auto_finished(Err((None, DeliveryError::NoBase)));
        assert_eq!(unread, Delivery::Ready { commits: 3 }, "the bar comes back");

        let mut idle = Delivery::Hidden;
        idle.start_auto();
        idle.auto_finished(Ok(()));
        assert_eq!(idle, Delivery::Hidden, "only a ready card starts");
    }

    #[test]
    fn a_failure_keeps_the_text_and_back_returns_to_editing_it() {
        let mut card = ready(1);
        card.open();
        card.built(Ok(plan(1)));
        card.form_mut().unwrap().title = "Mine".into();
        card.approve();
        card.delivered(Err(DeliveryError::GhNotAuthenticated));
        assert!(matches!(&card, Delivery::Failed { error: DeliveryError::GhNotAuthenticated, .. }));
        card.back();
        let Delivery::Editing(form) = &card else { panic!("{card:?}") };
        assert_eq!(form.title, "Mine");
    }

    #[test]
    fn answers_for_a_card_that_is_not_waiting_for_them_change_nothing() {
        let mut card = Delivery::Hidden;
        card.built(Ok(plan(1)));
        card.delivered(Ok(Delivered { url: "u".into(), head: "b".into(), created: true }));
        card.back();
        card.close_success();
        assert_eq!(card.approve(), None);
        assert_eq!(card, Delivery::Hidden);
    }

    #[test]
    fn the_card_text_counts_commits() {
        assert_eq!(card_text(1, false), "1 commit ready");
        assert_eq!(card_text(3, false), "3 commits ready");
        assert_eq!(card_text(1, true), "1 commit after the merge");
        assert_eq!(card_text(2, true), "2 commits after the merge");
    }

    #[test]
    fn each_typed_failure_has_a_plain_fix() {
        assert_eq!(
            fix_hint(&DeliveryError::GhMissing),
            Some("Install the GitHub CLI: brew install gh")
        );
        assert_eq!(
            fix_hint(&DeliveryError::GhNotAuthenticated),
            Some("Log in once: gh auth login")
        );
        for error in [
            DeliveryError::NoRemote,
            DeliveryError::NoGithubRemote,
            DeliveryError::PushRejected("x".into()),
            DeliveryError::PushFailed("x".into()),
            DeliveryError::NoBase,
        ] {
            assert!(fix_hint(&error).is_some(), "{error:?}");
        }
        assert_eq!(fix_hint(&DeliveryError::PrFailed("x".into())), None);
    }
}
