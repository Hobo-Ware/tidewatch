//! What a saved agent tab does at launch (PLAN.md, D11): run it, type it, or open a shell.

use crate::agents::Level;
use crate::prefs::ResumeAgents;
use crate::restore::Reason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The window quit, or crashed once: the tab comes back as the setting says.
    Clean,
    /// The window crashed again soon after a launch (`run_marker::CRASH_LOOP_SECS`): agents wait for
    /// the user, so a resume that brings the window down cannot repeat on its own.
    CrashLoop,
    AgentCrashed,
}

impl Ended {
    #[must_use]
    pub fn of(crash_loop: bool, agent_crashed: bool) -> Self {
        match (crash_loop, agent_crashed) {
            (true, _) => Self::CrashLoop,
            (false, true) => Self::AgentCrashed,
            (false, false) => Self::Clean,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    Chosen,
    CrashLoop,
    AgentCrashed,
    Yolo,
}

impl Why {
    /// What the tab says about its waiting resume command. `typed` is whether the command sits at the
    /// prompt (Enter runs it) or was too long to type (the Resume button runs it).
    #[must_use]
    pub fn note(self, typed: bool) -> &'static str {
        match (self, typed) {
            (Self::Chosen, true) => {
                "Resume command typed for you, as Settings say. Press Enter to continue the conversation."
            }
            (Self::Chosen, false) => {
                "Settings say to type the resume command, but it is too long to type. Press Resume to continue the conversation."
            }
            (Self::CrashLoop, true) => {
                "Tidewatch stopped again soon after it started, so the agent was not restarted. Press Enter to resume it."
            }
            (Self::CrashLoop, false) => {
                "Tidewatch stopped again soon after it started, so the agent was not restarted. Press Resume to resume it."
            }
            (Self::AgentCrashed, true) => {
                "The agent stopped unexpectedly last time, so it was not restarted. Press Enter to resume it."
            }
            (Self::AgentCrashed, false) => {
                "The agent stopped unexpectedly last time, so it was not restarted. Press Resume to resume it."
            }
            (Self::Yolo, true) => {
                "YOLO tabs wait for you. Press Enter to resume with the same permissions."
            }
            (Self::Yolo, false) => {
                "YOLO tabs wait for you. Press Resume to continue with the same permissions."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    Run,
    TypeOnly(Why),
}

/// How to bring an agent tab back, or the reason it comes back as a plain shell. A YOLO tab is
/// never run on its own, in any mode and however it ended: its command is typed and waits for Enter.
/// A window that crashed once resumes its agents as the setting says; one that crashed again right
/// after a launch, and an agent that crashed on its own, wait for the user.
pub fn decide(mode: ResumeAgents, ended: Ended, level: Level) -> Result<Resume, Reason> {
    match (mode, ended) {
        (ResumeAgents::Off, _) => Err(Reason::ResumeOff),
        _ if level == Level::Yolo => Ok(Resume::TypeOnly(Why::Yolo)),
        (_, Ended::CrashLoop) => Ok(Resume::TypeOnly(Why::CrashLoop)),
        (_, Ended::AgentCrashed) => Ok(Resume::TypeOnly(Why::AgentCrashed)),
        (ResumeAgents::Prefill, Ended::Clean) => Ok(Resume::TypeOnly(Why::Chosen)),
        (ResumeAgents::Auto, Ended::Clean) => Ok(Resume::Run),
    }
}

/// Whether an agent's exit status says it did not finish on its own terms. A pty reports a
/// program killed by a signal as status 1, so a kill and a failing exit look alike, and both count.
/// 0 is a quit, 130 and 143 are the exits of an interrupted or terminated program.
#[must_use]
pub fn exit_is_abnormal(code: i32) -> bool {
    !matches!(code, 0 | 130 | 143)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [ResumeAgents; 3] = [ResumeAgents::Auto, ResumeAgents::Prefill, ResumeAgents::Off];
    const ENDS: [Ended; 3] = [Ended::Clean, Ended::CrashLoop, Ended::AgentCrashed];
    const LEVELS: [Level; 5] = [Level::Plan, Level::Ask, Level::Edits, Level::Auto, Level::Yolo];

    #[test]
    fn every_mode_and_end_decides_as_the_table_says() {
        use ResumeAgents::{Auto, Off, Prefill};
        let cases = [
            (Auto, Ended::Clean, Ok(Resume::Run)),
            (Auto, Ended::CrashLoop, Ok(Resume::TypeOnly(Why::CrashLoop))),
            (Auto, Ended::AgentCrashed, Ok(Resume::TypeOnly(Why::AgentCrashed))),
            (Prefill, Ended::Clean, Ok(Resume::TypeOnly(Why::Chosen))),
            (Prefill, Ended::CrashLoop, Ok(Resume::TypeOnly(Why::CrashLoop))),
            (Prefill, Ended::AgentCrashed, Ok(Resume::TypeOnly(Why::AgentCrashed))),
            (Off, Ended::Clean, Err(Reason::ResumeOff)),
            (Off, Ended::CrashLoop, Err(Reason::ResumeOff)),
            (Off, Ended::AgentCrashed, Err(Reason::ResumeOff)),
        ];
        for (mode, ended, want) in cases {
            assert_eq!(decide(mode, ended, Level::Edits), want, "{mode:?} {ended:?}");
        }
    }

    #[test]
    fn yolo_is_typed_and_waits_for_enter_unless_resuming_is_off() {
        use ResumeAgents::{Auto, Off, Prefill};
        let typed = Ok(Resume::TypeOnly(Why::Yolo));
        let off = Err(Reason::ResumeOff);
        let cases = [
            (Auto, Ended::Clean, typed.clone()),
            (Auto, Ended::CrashLoop, typed.clone()),
            (Auto, Ended::AgentCrashed, typed.clone()),
            (Prefill, Ended::Clean, typed.clone()),
            (Prefill, Ended::CrashLoop, typed.clone()),
            (Prefill, Ended::AgentCrashed, typed),
            (Off, Ended::Clean, off.clone()),
            (Off, Ended::CrashLoop, off.clone()),
            (Off, Ended::AgentCrashed, off),
        ];
        for (mode, ended, want) in cases {
            assert_eq!(decide(mode, ended, Level::Yolo), want, "{mode:?} {ended:?}");
        }
    }

    #[test]
    fn every_level_but_yolo_decides_alike() {
        for mode in MODES {
            for ended in ENDS {
                let edits = decide(mode, ended, Level::Edits);
                for level in LEVELS.into_iter().filter(|level| *level != Level::Yolo) {
                    assert_eq!(decide(mode, ended, level), edits, "{mode:?} {ended:?} {level:?}");
                }
            }
        }
    }

    #[test]
    fn a_crash_loop_and_an_agent_crash_are_never_run_at_any_level() {
        for mode in [ResumeAgents::Auto, ResumeAgents::Prefill] {
            for level in LEVELS {
                for ended in [Ended::CrashLoop, Ended::AgentCrashed] {
                    assert_ne!(decide(mode, ended, level), Ok(Resume::Run), "{mode:?} {level:?}");
                }
            }
        }
    }

    #[test]
    fn a_crash_loop_hides_what_happened_to_the_tab() {
        let cases = [
            (false, false, Ended::Clean),
            (false, true, Ended::AgentCrashed),
            (true, false, Ended::CrashLoop),
            (true, true, Ended::CrashLoop),
        ];
        for (loop_, agent, want) in cases {
            assert_eq!(Ended::of(loop_, agent), want, "loop {loop_} agent {agent}");
        }
    }

    #[test]
    fn each_reason_has_its_own_note_that_names_what_to_press() {
        let whys = [Why::Chosen, Why::CrashLoop, Why::AgentCrashed, Why::Yolo];
        for (typed, key) in [(true, "Press Enter"), (false, "Press Resume")] {
            let notes = whys.map(|why| why.note(typed));
            for (i, note) in notes.iter().enumerate() {
                assert!(!notes[..i].contains(note), "{note}");
                assert!(note.contains(key), "{note}");
                assert!(!note.contains(['\u{2013}', '\u{2014}']), "{note}");
            }
        }
    }

    #[test]
    fn exit_statuses_split_into_quits_and_abnormal_ends() {
        let cases = [
            (0, false),
            (130, false),
            (143, false),
            (1, true),
            (2, true),
            (127, true),
            (137, true),
            (139, true),
            (-1, true),
        ];
        for (code, want) in cases {
            assert_eq!(exit_is_abnormal(code), want, "status {code}");
        }
    }
}
