//! The state of one tab's "push and open a PR" card (PLAN.md, D7) and the wording of its outcomes.
//! The card appears once a worktree agent is Done with commits on top of its base; opening it reads
//! the commits, the user edits the title and text, and approving pushes and opens the pull request.
//! Everything here is plain data; the git and gh work runs in `flow_jobs`.

use tidewatch_core::delivery::{Delivered, DeliveryError, DeliveryPlan};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Form {
    pub(crate) plan: DeliveryPlan,
    pub(crate) title: String,
    pub(crate) body: String,
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
    Editing(Form),
    Delivering(Form),
    Failed {
        form: Form,
        error: DeliveryError,
    },
    Delivered(Delivered),
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
            Ok(plan) => {
                Self::Editing(Form { title: plan.title.clone(), body: plan.body.clone(), plan })
            }
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

pub(crate) fn card_text(commits: usize) -> String {
    let noun = if commits == 1 { "commit" } else { "commits" };
    format!("{commits} {noun} ready")
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
            commits: (0..commits)
                .map(|n| CommitLine {
                    sha: format!("abc{n}"),
                    subject: format!("c{n}"),
                    body: String::new(),
                })
                .collect(),
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
        let done = Delivered { url: "https://github.com/o/r/pull/9".into(), created: true };
        card.delivered(Ok(done.clone()));
        assert_eq!(card, Delivery::Delivered(done));
        assert!(card.is_modal());
        card.close_success();
        assert_eq!(card, Delivery::Hidden);
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
        card.delivered(Ok(Delivered { url: "u".into(), created: true }));
        card.back();
        card.close_success();
        assert_eq!(card.approve(), None);
        assert_eq!(card, Delivery::Hidden);
    }

    #[test]
    fn the_card_text_counts_commits() {
        assert_eq!(card_text(1), "1 commit ready");
        assert_eq!(card_text(3), "3 commits ready");
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
