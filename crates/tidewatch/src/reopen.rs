//! A closed tab coming back from its schedule entry: the one-tab saved window the restore path plans
//! (pure).

use std::path::Path;

use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::profiles::Profiles;
use tidewatch_core::session::{SavedAgentTab, SavedTab, SavedWindow};
use tidewatch_core::worktree::BRANCH_PREFIX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Why {
    NoSession,
    NoProfile,
}

impl Why {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::NoSession => "That tab has no session to come back to",
            Self::NoProfile => "That tab's profile is gone, so it cannot be reopened",
        }
    }
}

/// What the schedule index remembers of a tab.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Target<'a> {
    pub(crate) repo: &'a Path,
    pub(crate) tab: &'a str,
    pub(crate) session_id: Option<&'a str>,
    pub(crate) profile_id: Option<&'a str>,
}

pub(crate) fn saved_window(
    paths: &TidewatchPaths,
    profiles: &Profiles,
    target: Target<'_>,
) -> Result<SavedWindow, Why> {
    let session = target.session_id.filter(|id| !id.is_empty()).ok_or(Why::NoSession)?;
    let id = target.profile_id.filter(|id| !id.is_empty()).ok_or(Why::NoProfile)?;
    let profile = profiles.profile.get(id).ok_or(Why::NoProfile)?;
    let tab = SavedTab::Agent(SavedAgentTab {
        dir: paths.worktree_dir(target.repo, target.tab),
        branch: Some(format!("{BRANCH_PREFIX}{}", target.tab)),
        agent: profile.agent.clone(),
        profile: id.to_owned(),
        level: profile.level,
        session_id: Some(session.to_owned()),
        stages: None,
        crashed: false,
        remote: false,
        meeseeks: None,
    });
    Ok(SavedWindow {
        dir: target.repo.to_path_buf(),
        other: false,
        active: 0,
        focus_mode: false,
        tabs: vec![tab],
        parked: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILES: &str =
        "[profile.builder]\nname = \"Builder\"\nagent = \"claude\"\nlevel = \"edits\"\n";

    fn target<'a>(session: Option<&'a str>, profile: Option<&'a str>) -> Target<'a> {
        Target {
            repo: Path::new("/work/web"),
            tab: "heron",
            session_id: session,
            profile_id: profile,
        }
    }

    fn setup() -> (TidewatchPaths, Profiles) {
        (TidewatchPaths::new(Path::new("/home/u")), Profiles::parse(PROFILES).unwrap())
    }

    #[test]
    fn a_tab_comes_back_in_its_own_worktree_with_its_session_and_profile() {
        let (paths, profiles) = setup();
        let window =
            saved_window(&paths, &profiles, target(Some("sess-1"), Some("builder"))).unwrap();
        assert_eq!(window.dir, Path::new("/work/web"));
        let [SavedTab::Agent(tab)] = window.tabs.as_slice() else { panic!("{window:?}") };
        assert_eq!(tab.dir, paths.worktree_dir(Path::new("/work/web"), "heron"));
        assert_eq!(tab.branch.as_deref(), Some("agent/heron"));
        assert_eq!((tab.agent.as_str(), tab.profile.as_str()), ("claude", "builder"));
        assert_eq!(tab.session_id.as_deref(), Some("sess-1"));
        assert!(tab.stages.is_none() && !tab.crashed);
    }

    #[test]
    fn without_a_session_or_a_known_profile_there_is_nothing_to_reopen() {
        let (paths, profiles) = setup();
        let cases = [
            (None, Some("builder"), Why::NoSession),
            (Some(""), Some("builder"), Why::NoSession),
            (Some("s"), None, Why::NoProfile),
            (Some("s"), Some("gone"), Why::NoProfile),
            (Some("s"), Some(""), Why::NoProfile),
        ];
        for (session, profile, want) in cases {
            let got = saved_window(&paths, &profiles, target(session, profile));
            assert_eq!(got.unwrap_err(), want, "{session:?} {profile:?}");
        }
        assert_ne!(Why::NoSession.text(), Why::NoProfile.text());
    }
}
