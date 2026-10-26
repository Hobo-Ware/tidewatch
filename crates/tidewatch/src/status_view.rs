//! How an agent status looks: its tone, wording and color, the CLI logo it gets in the tab, and the
//! status dot at the right end of a tab. Working is the only tone that animates, so a tab that waits for
//! you or is done costs no frames.

use crate::brand;
use egui::{Color32, Id, LayerId, Order, Pos2, Rect, pos2};
use tidewatch_core::progress::State;
use tidewatch_core::status::{AgentStatus, Reason};

const DOT: f32 = 8.0;
const PULSE_HZ: f64 = 0.8;
const PULSE_LOW: f32 = 0.45;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tone {
    Setup,
    Working,
    Needs,
    Waiting,
    Blocked,
    Done,
    Failed,
    Stopped,
    Shell,
}

/// A quiet terminal asked for nothing, so it reads as `Done` (idle), never as `Needs`.
pub(crate) fn tone(status: &AgentStatus) -> Tone {
    match status {
        AgentStatus::SettingUp => Tone::Setup,
        AgentStatus::Working => Tone::Working,
        AgentStatus::NeedsYou(Reason::Quiet) | AgentStatus::Done => Tone::Done,
        AgentStatus::NeedsYou(_) => Tone::Needs,
        AgentStatus::Failed(_) => Tone::Failed,
        AgentStatus::Stopped => Tone::Stopped,
        AgentStatus::Shell => Tone::Shell,
    }
}

/// The tone a tab shows everywhere: what the agent reported when it is working or idle, and the
/// facts Tidewatch owns when they say more: setting up, a permission prompt, failed, stopped. A
/// report older than the turn that now runs no longer holds, so the caller passes `Working` then.
pub(crate) fn shown(status: &AgentStatus, reported: Option<State>) -> Tone {
    let owned = tone(status);
    match (owned, reported) {
        (Tone::Working | Tone::Done, Some(state)) => match state {
            State::Working => Tone::Working,
            State::NeedsYou => Tone::Needs,
            State::Waiting => Tone::Waiting,
            State::Blocked => Tone::Blocked,
            State::Done => Tone::Done,
        },
        _ => owned,
    }
}

/// The report state a shown tone stands for; `None` for the facts Tidewatch owns.
pub(crate) fn state_of(tone: Tone) -> Option<State> {
    match tone {
        Tone::Working => Some(State::Working),
        Tone::Needs => Some(State::NeedsYou),
        Tone::Waiting => Some(State::Waiting),
        Tone::Blocked => Some(State::Blocked),
        Tone::Done => Some(State::Done),
        Tone::Setup | Tone::Failed | Tone::Stopped | Tone::Shell => None,
    }
}

pub(crate) fn label(tone: Tone) -> &'static str {
    match tone {
        Tone::Setup => "Setting up",
        Tone::Working => "Working",
        Tone::Needs => "Needs you",
        Tone::Waiting => "Waiting",
        Tone::Blocked => "Blocked",
        Tone::Done => "Done",
        Tone::Failed => "Failed",
        Tone::Stopped => "Stopped",
        Tone::Shell => "Shell",
    }
}

pub(crate) fn reason_label(reason: Reason) -> &'static str {
    match reason {
        Reason::Permission => "wants permission",
        Reason::Question => "asked a question",
        Reason::Input => "waiting for your message",
        Reason::Attention => "rang the bell",
        Reason::Quiet => "went quiet",
    }
}

/// The label plus the reason or error, for tooltips: `Needs you - wants permission`.
pub(crate) fn describe(status: &AgentStatus) -> String {
    let base = label(tone(status));
    match status {
        AgentStatus::NeedsYou(Reason::Quiet) => format!("Idle - {}", reason_label(Reason::Quiet)),
        AgentStatus::NeedsYou(reason) => format!("{base} - {}", reason_label(*reason)),
        AgentStatus::Failed(why) => format!("{base} - {why}"),
        _ => base.to_owned(),
    }
}

/// `None` for a plain shell: it has no status to color.
pub(crate) fn color(tone: Tone) -> Option<Color32> {
    match tone {
        Tone::Setup => Some(brand::WORKING.gamma_multiply(0.6)),
        Tone::Working => Some(brand::WORKING),
        Tone::Needs => Some(brand::NEEDS),
        Tone::Waiting => Some(brand::WAITING),
        Tone::Blocked => Some(brand::BLOCKED),
        Tone::Done => Some(brand::DONE),
        Tone::Failed => Some(brand::FAILED),
        Tone::Stopped => Some(brand::MUTED),
        Tone::Shell => None,
    }
}

pub(crate) fn is_animated(tone: Tone) -> bool {
    tone == Tone::Working
}

/// How bright the working dot is at `time`, from `PULSE_LOW` to 1.0.
pub(crate) fn pulse(time: f64) -> f32 {
    let wave = 0.5 - 0.5 * (time * PULSE_HZ * std::f64::consts::TAU).cos();
    PULSE_LOW + (1.0 - PULSE_LOW) * wave as f32
}

pub(crate) fn dot_center(tab: Rect, trailing: f32) -> Pos2 {
    pos2(tab.right() - trailing, tab.center().y)
}

/// Paints the status dot at the right end of a tab, on the layer `draw_tab` uses for its own edge
/// strokes. A shell has no status and gets none.
pub(crate) fn paint_dot(ui: &egui::Ui, center: Pos2, tone: Tone, time: f64) {
    let Some(color) = color(tone) else { return };
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Middle, Id::new("tab_deco")));
    let color = if tone == Tone::Working { color.gamma_multiply(pulse(time)) } else { color };
    painter.circle_filled(center, DOT / 2.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_has_one_tone() {
        let cases = [
            (AgentStatus::SettingUp, Tone::Setup),
            (AgentStatus::Working, Tone::Working),
            (AgentStatus::NeedsYou(Reason::Permission), Tone::Needs),
            (AgentStatus::NeedsYou(Reason::Quiet), Tone::Done),
            (AgentStatus::Done, Tone::Done),
            (AgentStatus::Failed("exited with code 2".into()), Tone::Failed),
            (AgentStatus::Stopped, Tone::Stopped),
            (AgentStatus::Shell, Tone::Shell),
        ];
        for (status, want) in cases {
            assert_eq!(tone(&status), want, "{status:?}");
        }
    }

    #[test]
    fn what_the_agent_reported_decides_the_tone_unless_tidewatch_owns_a_fact() {
        use State::{Blocked, Done as Finished, NeedsYou as Asks, Waiting, Working as Busy};
        let reasons = [Reason::Permission, Reason::Question, Reason::Input, Reason::Attention];
        let cases = [
            (AgentStatus::Working, Some(Busy), Tone::Working),
            (AgentStatus::Working, Some(Asks), Tone::Needs),
            (AgentStatus::Working, Some(Waiting), Tone::Waiting),
            (AgentStatus::Working, Some(Blocked), Tone::Blocked),
            (AgentStatus::Working, Some(Finished), Tone::Done),
            (AgentStatus::Done, Some(Busy), Tone::Working),
            (AgentStatus::Done, Some(Waiting), Tone::Waiting),
            (AgentStatus::Done, Some(Blocked), Tone::Blocked),
            (AgentStatus::Done, Some(Finished), Tone::Done),
            (AgentStatus::NeedsYou(Reason::Quiet), Some(Waiting), Tone::Waiting),
            (AgentStatus::Done, None, Tone::Done),
            (AgentStatus::Working, None, Tone::Working),
            (AgentStatus::SettingUp, Some(Finished), Tone::Setup),
            (AgentStatus::Failed("x".into()), Some(Busy), Tone::Failed),
            (AgentStatus::Stopped, Some(Waiting), Tone::Stopped),
            (AgentStatus::Shell, Some(Busy), Tone::Shell),
        ];
        for (status, reported, want) in cases {
            assert_eq!(shown(&status, reported), want, "{status:?} {reported:?}");
        }
        for reason in reasons {
            assert_eq!(
                shown(&AgentStatus::NeedsYou(reason), Some(Finished)),
                Tone::Needs,
                "a prompt that asks for you wins over a report: {reason:?}"
            );
        }
    }

    #[test]
    fn the_five_reported_states_have_their_own_word_and_colour_and_map_back() {
        let tones = [Tone::Working, Tone::Needs, Tone::Waiting, Tone::Blocked, Tone::Done];
        let words: Vec<&str> = tones.iter().map(|tone| label(*tone)).collect();
        assert_eq!(words, ["Working", "Needs you", "Waiting", "Blocked", "Done"]);
        for (i, a) in tones.iter().enumerate() {
            assert!(state_of(*a).is_some());
            for b in &tones[i + 1..] {
                assert_ne!(color(*a), color(*b), "{a:?} and {b:?}");
            }
        }
        assert_eq!(state_of(Tone::Failed), None);
        assert_eq!(state_of(Tone::Setup), None);
    }

    #[test]
    fn tones_read_in_plain_words() {
        let labels: Vec<&str> = [
            Tone::Setup,
            Tone::Working,
            Tone::Needs,
            Tone::Waiting,
            Tone::Blocked,
            Tone::Done,
            Tone::Failed,
            Tone::Stopped,
            Tone::Shell,
        ]
        .into_iter()
        .map(label)
        .collect();
        assert_eq!(
            labels,
            [
                "Setting up",
                "Working",
                "Needs you",
                "Waiting",
                "Blocked",
                "Done",
                "Failed",
                "Stopped",
                "Shell"
            ]
        );
    }

    #[test]
    fn the_description_adds_the_reason_or_the_error() {
        assert_eq!(describe(&AgentStatus::Working), "Working");
        assert_eq!(
            describe(&AgentStatus::NeedsYou(Reason::Permission)),
            "Needs you - wants permission"
        );
        assert_eq!(
            describe(&AgentStatus::Failed("exited with code 2".into())),
            "Failed - exited with code 2"
        );
        assert_eq!(describe(&AgentStatus::Stopped), "Stopped");
        assert_eq!(describe(&AgentStatus::NeedsYou(Reason::Quiet)), "Idle - went quiet");
    }

    #[test]
    fn colors_follow_the_traffic_light_and_a_shell_has_none() {
        assert_eq!(color(Tone::Needs), Some(brand::NEEDS));
        assert_eq!(color(Tone::Done), Some(brand::DONE));
        assert_eq!(color(Tone::Failed), Some(brand::FAILED));
        assert_eq!(color(Tone::Working), Some(brand::WORKING));
        assert_eq!(color(Tone::Stopped), Some(brand::MUTED));
        assert_eq!(color(Tone::Shell), None);
        let lit = [
            Tone::Setup,
            Tone::Working,
            Tone::Needs,
            Tone::Waiting,
            Tone::Blocked,
            Tone::Done,
            Tone::Failed,
            Tone::Stopped,
        ];
        for (i, a) in lit.iter().enumerate() {
            for b in &lit[i + 1..] {
                assert_ne!(color(*a), color(*b), "{a:?} and {b:?} must look different");
            }
        }
    }

    #[test]
    fn only_working_animates() {
        let animated: Vec<Tone> =
            [Tone::Setup, Tone::Working, Tone::Needs, Tone::Done, Tone::Failed, Tone::Shell]
                .into_iter()
                .filter(|tone| is_animated(*tone))
                .collect();
        assert_eq!(animated, [Tone::Working]);
    }

    #[test]
    fn the_pulse_stays_between_dim_and_full_and_moves() {
        let samples: Vec<f32> = (0..200).map(|step| pulse(f64::from(step) * 0.017)).collect();
        assert!(samples.iter().all(|v| (PULSE_LOW..=1.0).contains(v)));
        assert_eq!(pulse(0.0), PULSE_LOW);
        assert!(samples.iter().any(|v| *v > 0.95));
    }
}
