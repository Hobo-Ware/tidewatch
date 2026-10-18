//! The remote session of a running agent tab: its name and the link its CLI printed, found by
//! looking at the terminal now and then (pure; the caller reads the terminal).

use tidewatch_core::agents::Wiring;
use tidewatch_core::remote::find_link;

const SCAN_EVERY_S: f64 = 2.0;

/// What a frame does about finding the link.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Step {
    /// Nothing to look for, or no new output since the last look.
    Rest,
    /// Look at the terminal now.
    Scan,
    /// New output came in but the last look was too recent: look again at this time.
    Later(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RemoteSession {
    pub(crate) name: String,
    link: Option<String>,
    seen_output: u64,
    next_scan: f64,
}

impl RemoteSession {
    pub(crate) fn new(name: String) -> Self {
        Self { name, link: None, seen_output: 0, next_scan: 0.0 }
    }

    pub(crate) fn link(&self) -> Option<&str> {
        self.link.as_deref()
    }

    /// Whether to look at a terminal whose output counter reads `output`. Output has to be new,
    /// and looks are at least two seconds apart. Once a link is found nothing is looked for.
    pub(crate) fn step(&self, output: u64, now: f64) -> Step {
        if self.link.is_some() || output == self.seen_output {
            Step::Rest
        } else if now >= self.next_scan {
            Step::Scan
        } else {
            Step::Later(self.next_scan)
        }
    }

    /// Records a look at the terminal at `output`: the link it showed, if any.
    pub(crate) fn scanned(
        &mut self,
        output: u64,
        now: f64,
        rows: &[String],
        cols: usize,
        agent: &str,
    ) {
        self.seen_output = output;
        self.next_scan = now + SCAN_EVERY_S;
        if let Some(found) = find_link(wiring_of(agent), rows, cols) {
            self.link = Some(found);
        }
    }
}

/// The CLIs that have a remote control flag, by agent id. Any other agent gets no link.
fn wiring_of(agent: &str) -> Wiring {
    match agent {
        "claude" => Wiring::Claude,
        "copilot" => Wiring::Copilot,
        _ => Wiring::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "https://claude.ai/code/session_01AbCd";

    fn rows(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn a_session_looks_only_at_new_output_and_not_twice_within_two_seconds() {
        let mut session = RemoteSession::new("web \u{b7} otter".into());
        assert_eq!(session.step(0, 0.0), Step::Rest, "no output yet");
        assert_eq!(session.step(5, 0.1), Step::Scan);
        session.scanned(5, 0.1, &rows("starting"), 80, "claude");
        assert_eq!(session.step(5, 9.0), Step::Rest, "the same output is not read again");
        assert_eq!(session.step(6, 1.0), Step::Later(2.1));
        assert_eq!(session.step(6, 2.1), Step::Scan);
    }

    #[test]
    fn a_found_link_stops_the_looking_and_stays() {
        let mut session = RemoteSession::new("web".into());
        assert_eq!(session.link(), None);
        session.scanned(3, 1.0, &rows(&format!("Remote Control active\n{LINK}")), 80, "claude");
        assert_eq!(session.link(), Some(LINK));
        assert_eq!(session.step(4, 50.0), Step::Rest);
        session.scanned(4, 50.0, &rows("cleared"), 80, "claude");
        assert_eq!(session.link(), Some(LINK));
    }

    #[test]
    fn only_the_clis_with_a_flag_find_a_link() {
        let cases = [("claude", true), ("copilot", false), ("codex", false), ("aider", false)];
        for (agent, found) in cases {
            let mut session = RemoteSession::new("web".into());
            session.scanned(1, 0.0, &rows(LINK), 80, agent);
            assert_eq!(session.link().is_some(), found, "{agent}");
        }
        let mut copilot = RemoteSession::new("web".into());
        copilot.scanned(1, 0.0, &rows("https://github.com/acme/web/tasks/1"), 80, "copilot");
        assert_eq!(copilot.link(), Some("https://github.com/acme/web/tasks/1"));
    }
}
