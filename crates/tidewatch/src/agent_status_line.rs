//! The status line above an agent tab: its state, who runs it, its branch and the view choice.

use egui::{Color32, Rect, Ui, pos2};
use stdusk_egui::colors;

use crate::brand;
use crate::layout::CHROME_H;
use crate::pane::{FirstPrompt, Pane};
use crate::reader::Shown;
use crate::screen_kit::body;
use crate::segmented;
use crate::status_view::{self, Tone};

pub(crate) const HEIGHT: f32 = CHROME_H;
const LEFT_PAD: f32 = 10.0;
const RIGHT_PAD: f32 = 6.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) label: &'static str,
    pub(crate) tone: Tone,
    pub(crate) ready: bool,
    pub(crate) who: String,
    pub(crate) reader_on: bool,
    pub(crate) reader_available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Reader,
    Terminal,
}

/// A tab that waits for its first message with its agent up, or still printing its banner.
pub(crate) fn is_ready(first_prompt: FirstPrompt, tone: Tone) -> bool {
    first_prompt == FirstPrompt::Awaiting
        && matches!(tone, Tone::Working | Tone::Done | Tone::Shell)
}

/// The line of an agent tab that is not a workflow's (its stepper says more). `None` for the rest.
pub(crate) fn line_of<T>(
    pane: &Pane<T>,
    shown: Shown,
    reader_available: bool,
    cli_name: &str,
) -> Option<Line> {
    let agent = pane.agent.as_ref().filter(|agent| agent.workflow.is_none())?;
    let tone = status_view::tone(agent.run.status());
    let ready = is_ready(agent.first_prompt, tone);
    let level = agent.launched.as_ref().map(|launched| launched.level.label());
    let who = [Some(agent.profile_name.as_str()), Some(cli_name), level]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
    Some(Line {
        label: if ready { "Ready" } else { status_view::label(tone) },
        tone,
        ready,
        who,
        reader_on: shown == Shown::Reader,
        reader_available,
    })
}

fn dot_color(line: &Line) -> Color32 {
    if line.ready {
        return brand::TEAL;
    }
    status_view::color(line.tone).unwrap_or_else(colors::dim)
}

/// The choices the segmented control offers, left to right, and which one is on.
fn segments(line: &Line) -> (Vec<Choice>, usize) {
    if !line.reader_available {
        return (vec![Choice::Terminal], 0);
    }
    (vec![Choice::Reader, Choice::Terminal], usize::from(!line.reader_on))
}

pub(crate) fn show(ui: &mut Ui, rect: Rect, line: &Line) -> Option<Choice> {
    let y = rect.center().y;
    let (choices, active) = segments(line);
    let labels: Vec<&str> = choices
        .iter()
        .map(|choice| match choice {
            Choice::Reader => "Reader",
            Choice::Terminal => "Terminal",
        })
        .collect();
    let id = ui.id().with("status-line");
    let switch = segmented::show(ui, id, rect.right() - RIGHT_PAD, y, &labels, active);
    let painter = ui
        .painter()
        .with_clip_rect(Rect::from_min_max(rect.min, pos2(switch.left - 8.0, rect.bottom())));
    let tone = dot_color(line);
    let mut x = rect.left() + LEFT_PAD + 4.0;
    painter.circle_filled(pos2(x + 4.0, y), 8.0, tone.gamma_multiply(0.15));
    painter.circle_filled(pos2(x + 4.0, y), 4.0, tone);
    x += 18.0;
    let text = |x: &mut f32, text: &str, size: f32, color: Color32| {
        let galley = painter.layout_no_wrap(text.to_owned(), body(size), color);
        let width = galley.size().x;
        painter.galley(pos2(*x, y - galley.size().y / 2.0), galley, color);
        *x += width + 10.0;
    };
    text(&mut x, line.label, 14.0, brand::TEXT);
    text(&mut x, &line.who, 13.0, brand::MUTED);
    switch.clicked.map(|index| choices[index])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tidewatch_core::agents::Level;
    use tidewatch_core::worktree::Worktree;

    use crate::agent_run::AgentRun;
    use crate::pane::{AgentTab, Launched};

    fn pane(first: FirstPrompt, worktree: bool) -> Pane<u32> {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        agent.first_prompt = first;
        agent.launched =
            Some(Launched { profile_id: "builder".into(), level: Level::Edits, session_id: None });
        let worktree = worktree.then(|| Worktree {
            repo: PathBuf::from("/work/web"),
            path: PathBuf::from("/home/.tidewatch/web-1/quick-beacon"),
            branch: "agent/quick-beacon".into(),
        });
        Pane { body: crate::pane::Body::Creating, worktree, agent: Some(agent) }
    }

    #[test]
    fn a_tab_that_waits_for_its_first_message_is_ready_and_names_who() {
        let mut pane = pane(FirstPrompt::Awaiting, true);
        let run = &mut pane.agent.as_mut().unwrap().run;
        run.started(None, 0.0);
        run.refresh(1.0);
        let line = line_of(&pane, Shown::Reader, true, "Claude Code").unwrap();
        assert_eq!(line.who, "Builder \u{b7} Claude Code \u{b7} Auto edit");
        assert!(line.reader_on);
        assert_eq!((line.label, line.ready), ("Ready", true));
    }

    #[test]
    fn a_tab_still_setting_up_is_not_ready() {
        let pane = pane(FirstPrompt::Awaiting, false);
        let line = line_of(&pane, Shown::Terminal(None), false, "Claude Code").unwrap();
        assert_eq!((line.label, line.ready), ("Setting up", false));
        assert!(!line.reader_on && !line.reader_available);
    }

    #[test]
    fn once_the_first_message_is_in_the_status_is_the_agents_own() {
        let pane = pane(FirstPrompt::NotNeeded, true);
        let line = line_of(&pane, Shown::Reader, true, "Claude Code").unwrap();
        assert!(!line.ready);
    }

    #[test]
    fn a_plain_shell_has_no_line() {
        let pane: Pane<u32> = Pane::shell(0);
        assert_eq!(line_of(&pane, Shown::Terminal(None), false, ""), None);
    }

    #[test]
    fn the_control_offers_the_reader_only_when_there_is_one_and_marks_the_view_in_use() {
        let pane = pane(FirstPrompt::NotNeeded, true);
        let cases = [
            (Shown::Reader, true, vec![Choice::Reader, Choice::Terminal], 0),
            (Shown::Terminal(None), true, vec![Choice::Reader, Choice::Terminal], 1),
            (Shown::Terminal(None), false, vec![Choice::Terminal], 0),
        ];
        for (shown, available, choices, active) in cases {
            let line = line_of(&pane, shown, available, "Claude Code").unwrap();
            assert_eq!(segments(&line), (choices, active), "{shown:?} {available}");
        }
    }
}
