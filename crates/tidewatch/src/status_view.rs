//! How an agent status looks: its tone, wording and color, the CLI logo it gets in the tab, and the
//! thin bar along a tab's top edge. Working is the only tone that animates, so a tab that waits for
//! you or is done costs no frames.

use crate::brand;
use egui::{Color32, Id, LayerId, Order, Rect, Stroke, pos2, vec2};
use stdusk_core::procwatch::Cli;
use tidewatch_core::status::{AgentStatus, Reason};

const BAR_HEIGHT: f32 = 2.0;
const NEEDS_BAR_HEIGHT: f32 = 3.0;
const SWEEP_WIDTH: f32 = 0.35;
const SWEEP_PERIODS_PER_SECOND: f64 = 0.8;
const FAIL_TICK_HEIGHT: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tone {
    Setup,
    Working,
    Needs,
    Done,
    Failed,
    Shell,
}

pub(crate) fn tone(status: &AgentStatus) -> Tone {
    match status {
        AgentStatus::SettingUp => Tone::Setup,
        AgentStatus::Working => Tone::Working,
        AgentStatus::NeedsYou(_) => Tone::Needs,
        AgentStatus::Done => Tone::Done,
        AgentStatus::Failed(_) => Tone::Failed,
        AgentStatus::Shell => Tone::Shell,
    }
}

pub(crate) fn label(tone: Tone) -> &'static str {
    match tone {
        Tone::Setup => "Setting up",
        Tone::Working => "Working",
        Tone::Needs => "Needs you",
        Tone::Done => "Done",
        Tone::Failed => "Failed",
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
        Tone::Done => Some(brand::DONE),
        Tone::Failed => Some(brand::FAILED),
        Tone::Shell => None,
    }
}

pub(crate) fn is_animated(tone: Tone) -> bool {
    tone == Tone::Working
}

/// The moving segment of the working bar as fractions of the tab width, `start <= end`, both inside
/// `0.0..=1.0`. It enters from the left edge and leaves at the right.
pub(crate) fn sweep(time: f64) -> (f32, f32) {
    let phase = (time * SWEEP_PERIODS_PER_SECOND).rem_euclid(1.0) as f32;
    let head = phase * (1.0 + SWEEP_WIDTH);
    ((head - SWEEP_WIDTH).clamp(0.0, 1.0), head.clamp(0.0, 1.0))
}

/// The CLI mark stdusk already draws in a tab's trailing slot. A custom agent has none.
pub(crate) fn cli_of(agent_id: &str) -> Option<Cli> {
    match agent_id {
        "claude" => Some(Cli::Claude),
        "codex" => Some(Cli::Codex),
        "gemini" => Some(Cli::Gemini),
        "copilot" => Some(Cli::Copilot),
        _ => None,
    }
}

/// Paints the status bar over a tab's top edge, on the layer `draw_tab` uses for its own edge
/// strokes. `fraction` is a real percentage from the agent, drawn instead of the moving segment.
pub(crate) fn paint_bar(ui: &egui::Ui, tab: Rect, tone: Tone, time: f64, fraction: Option<f32>) {
    let Some(color) = color(tone) else { return };
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Middle, Id::new("tab_deco")));
    let height = if tone == Tone::Needs { NEEDS_BAR_HEIGHT } else { BAR_HEIGHT };
    let bar = |from: f32, to: f32| {
        Rect::from_min_max(
            pos2(tab.left() + tab.width() * from, tab.top()),
            pos2(tab.left() + tab.width() * to, tab.top() + height),
        )
    };
    match (tone, fraction) {
        (Tone::Working, Some(done)) => {
            painter.rect_filled(bar(0.0, 1.0), 0.0, color.gamma_multiply(0.25));
            painter.rect_filled(bar(0.0, done.clamp(0.0, 1.0)), 0.0, color);
        }
        (Tone::Working, None) => {
            painter.rect_filled(bar(0.0, 1.0), 0.0, color.gamma_multiply(0.25));
            let (start, end) = sweep(time);
            painter.rect_filled(bar(start, end), 0.0, color);
        }
        _ => {
            painter.rect_filled(bar(0.0, 1.0), 0.0, color);
        }
    }
    if tone == Tone::Failed {
        let tick = Rect::from_center_size(
            pos2(tab.left() + 2.0, tab.center().y),
            vec2(2.0, FAIL_TICK_HEIGHT),
        );
        painter.line_segment([tick.center_top(), tick.center_bottom()], Stroke::new(2.0, color));
    }
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
            (AgentStatus::NeedsYou(Reason::Quiet), Tone::Needs),
            (AgentStatus::Done, Tone::Done),
            (AgentStatus::Failed("exited with code 2".into()), Tone::Failed),
            (AgentStatus::Shell, Tone::Shell),
        ];
        for (status, want) in cases {
            assert_eq!(tone(&status), want, "{status:?}");
        }
    }

    #[test]
    fn tones_read_in_plain_words() {
        let labels: Vec<&str> =
            [Tone::Setup, Tone::Working, Tone::Needs, Tone::Done, Tone::Failed, Tone::Shell]
                .into_iter()
                .map(label)
                .collect();
        assert_eq!(labels, ["Setting up", "Working", "Needs you", "Done", "Failed", "Shell"]);
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
    }

    #[test]
    fn colors_follow_the_traffic_light_and_a_shell_has_none() {
        assert_eq!(color(Tone::Needs), Some(brand::NEEDS));
        assert_eq!(color(Tone::Done), Some(brand::DONE));
        assert_eq!(color(Tone::Failed), Some(brand::FAILED));
        assert_eq!(color(Tone::Working), Some(brand::WORKING));
        assert_eq!(color(Tone::Shell), None);
        let lit = [Tone::Setup, Tone::Working, Tone::Needs, Tone::Done, Tone::Failed];
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
    fn the_sweep_stays_inside_the_tab_and_moves() {
        let mut starts = Vec::new();
        for step in 0..200 {
            let (start, end) = sweep(f64::from(step) * 0.017);
            assert!((0.0..=1.0).contains(&start) && (0.0..=1.0).contains(&end));
            assert!(start <= end);
            assert!(end - start <= SWEEP_WIDTH + 1e-6);
            starts.push(start);
        }
        assert_ne!(starts.first(), starts.last());
        assert_eq!(sweep(0.0), (0.0, 0.0));
    }

    #[test]
    fn the_four_built_in_agents_have_a_logo_and_custom_ones_do_not() {
        assert_eq!(cli_of("claude"), Some(Cli::Claude));
        assert_eq!(cli_of("codex"), Some(Cli::Codex));
        assert_eq!(cli_of("gemini"), Some(Cli::Gemini));
        assert_eq!(cli_of("copilot"), Some(Cli::Copilot));
        assert_eq!(cli_of("my-agent"), None);
    }
}
