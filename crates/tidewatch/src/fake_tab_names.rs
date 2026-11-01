//! `TIDEWATCH_FAKE_TAB_NAMES=1`: gives the agent tabs, in turn, the sources a tab label is picked
//! from (a goal, a name the agent chose, the owner's rename, a session title), for captures. Nothing
//! is read or written; the screenshot harness saves no session.

use crate::pane::AgentTab;

struct Fake {
    goal: Option<&'static str>,
    chosen: Option<&'static str>,
    own: Option<&'static str>,
    session: Option<&'static str>,
}

const NONE: Fake = Fake { goal: None, chosen: None, own: None, session: None };

const FAKES: [Fake; 6] = [
    Fake { goal: Some("Soundtrack backfill blind spots"), ..NONE },
    Fake { chosen: Some("fix-login-redirect"), goal: Some("Fix the login redirect"), ..NONE },
    Fake { own: Some("Release notes"), chosen: Some("draft-release-notes"), ..NONE },
    Fake { goal: Some("Review the auth middleware for token leaks"), ..NONE },
    Fake { chosen: Some("retry-nightly-sync"), ..NONE },
    Fake { session: Some("Plan the Q4 migration"), ..NONE },
];

pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("TIDEWATCH_FAKE_TAB_NAMES")
            .is_ok_and(|value| !value.is_empty() && value != "0")
    })
}

pub(crate) fn apply<'a>(agents: impl Iterator<Item = &'a mut AgentTab>) {
    for (agent, fake) in agents.zip(FAKES.iter().cycle()) {
        agent.run.set_goal(fake.goal);
        agent.chosen_name = fake.chosen.map(str::to_owned);
        agent.reader.fake_own_title(fake.own);
        if let Some(session) = fake.session {
            agent.reader.fake_session_title(session);
        }
    }
}
