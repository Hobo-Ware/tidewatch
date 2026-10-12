//! The double-click checkout rule (PLAN.md Phase 5, concept): a chip in the graph switches
//! branches only in the main tab's checkout and only when its working tree is clean. A worktree
//! tab never checks out, because an agent may own it.

use std::path::Path;

use kelp_core::ops::Op;
use kelp_core::refs::{RefKind, RefLabel};
use tidewatch_core::history;

pub(crate) const AGENT_OWNED: &str = "Checkout is disabled while an agent owns this worktree";
pub(crate) const NOT_CLEAN: &str = "Commit or discard your changes before switching branches";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    Ignore,
    Refuse(&'static str),
    Switch(Op),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Switched(String),
    Dirty,
    Failed(String),
}

fn op_for(label: &RefLabel) -> Option<Op> {
    match label.kind {
        RefKind::Local if !label.is_head => Some(Op::Switch(label.name.clone())),
        RefKind::Remote => Some(Op::SwitchTrack(label.name.clone())),
        RefKind::Local | RefKind::Tag => None,
    }
}

pub(crate) fn plan(is_main_tab: bool, label: &RefLabel) -> Plan {
    if !is_main_tab {
        return Plan::Refuse(AGENT_OWNED);
    }
    op_for(label).map_or(Plan::Ignore, Plan::Switch)
}

/// Blocks on git: run it on a background thread.
pub(crate) fn run(dir: &Path, op: &Op) -> Outcome {
    match history::is_clean(dir) {
        Ok(true) => match op.run(dir) {
            Ok(_) => Outcome::Switched(op.label()),
            Err(e) => Outcome::Failed(e.to_string()),
        },
        Ok(false) => Outcome::Dirty,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

pub(crate) fn message(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Switched(what) => what.clone(),
        Outcome::Dirty => NOT_CLEAN.to_owned(),
        Outcome::Failed(error) => error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::ObjectId;
    use std::process::Command;

    fn label(name: &str, kind: RefKind, is_head: bool) -> RefLabel {
        RefLabel {
            name: name.into(),
            kind,
            target: ObjectId::from_hex(&[b'0'; 40]).unwrap(),
            row: Some(0),
            is_head,
            has_remote: false,
            hidden: false,
            pull: None,
        }
    }

    #[test]
    fn a_worktree_tab_never_checks_out() {
        for kind in [RefKind::Local, RefKind::Remote, RefKind::Tag] {
            assert_eq!(plan(false, &label("feat", kind, false)), Plan::Refuse(AGENT_OWNED));
        }
        assert_eq!(plan(false, &label("main", RefKind::Local, true)), Plan::Refuse(AGENT_OWNED));
    }

    #[test]
    fn the_main_tab_switches_to_other_local_and_remote_branches() {
        assert_eq!(
            plan(true, &label("feat", RefKind::Local, false)),
            Plan::Switch(Op::Switch("feat".into()))
        );
        assert_eq!(
            plan(true, &label("origin/feat", RefKind::Remote, false)),
            Plan::Switch(Op::SwitchTrack("origin/feat".into()))
        );
    }

    #[test]
    fn the_main_tab_ignores_the_current_branch_and_tags() {
        assert_eq!(plan(true, &label("main", RefKind::Local, true)), Plan::Ignore);
        assert_eq!(plan(true, &label("v1", RefKind::Tag, false)), Plan::Ignore);
    }

    #[test]
    fn outcomes_read_as_toast_text() {
        assert_eq!(message(&Outcome::Dirty), NOT_CLEAN);
        assert_eq!(message(&Outcome::Switched("Switching to feat".into())), "Switching to feat");
        assert_eq!(message(&Outcome::Failed("boom".into())), "boom");
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.email=t@example.com",
                "-c",
                "user.name=T",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn repo_with_branch() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().canonicalize().unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        git(&repo, &["branch", "feat"]);
        (dir, repo)
    }

    #[test]
    fn a_clean_checkout_switches_the_branch() {
        let (_dir, repo) = repo_with_branch();
        let outcome = run(&repo, &Op::Switch("feat".into()));
        assert!(matches!(outcome, Outcome::Switched(_)), "{outcome:?}");
        assert_eq!(git(&repo, &["branch", "--show-current"]), "feat");
    }

    #[test]
    fn a_dirty_checkout_is_refused_and_leaves_the_branch_alone() {
        let (_dir, repo) = repo_with_branch();
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
        assert_eq!(run(&repo, &Op::Switch("feat".into())), Outcome::Dirty);
        assert_eq!(git(&repo, &["branch", "--show-current"]), "main");
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();
        assert_eq!(run(&repo, &Op::Switch("feat".into())), Outcome::Dirty);
    }

    #[test]
    fn a_missing_branch_fails_with_git_message() {
        let (_dir, repo) = repo_with_branch();
        let outcome = run(&repo, &Op::Switch("nope".into()));
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    }
}
