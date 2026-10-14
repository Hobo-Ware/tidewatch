//! The status line above an agent tab: its state, who runs it, its branch and the view choice.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use stdusk_egui::colors;

use crate::brand;
use crate::pane::{FirstPrompt, Pane};
use crate::reader::Shown;
use crate::screen_kit::{body, mono};
use crate::status_view::{self, Tone};

pub(crate) const HEIGHT: f32 = 38.0;
const SIDE_PAD: f32 = 20.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) label: &'static str,
    pub(crate) tone: Tone,
    pub(crate) ready: bool,
    pub(crate) who: String,
    pub(crate) branch: Option<String>,
    pub(crate) base: Option<String>,
    pub(crate) reader_on: bool,
    pub(crate) reader_available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Reader,
    Terminal,
}

/// The line of an agent tab that is not a workflow's (its stepper says more). `None` for the rest.
pub(crate) fn line_of<T>(
    pane: &Pane<T>,
    shown: Shown,
    reader_available: bool,
    cli_name: &str,
    base: Option<&str>,
) -> Option<Line> {
    let agent = pane.agent.as_ref().filter(|agent| agent.workflow.is_none())?;
    let tone = status_view::tone(agent.run.status());
    let ready = agent.first_prompt == FirstPrompt::Awaiting
        && matches!(tone, Tone::Working | Tone::Done | Tone::Shell);
    let level = agent.launched.as_ref().map(|launched| launched.level.label());
    let who = [Some(agent.profile_name.as_str()), Some(cli_name), level]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let branch = pane.worktree.as_ref().map(|worktree| worktree.branch.clone());
    Some(Line {
        label: if ready { "Ready" } else { status_view::label(tone) },
        tone,
        ready,
        who,
        base: branch.as_ref().and(base.map(str::to_owned)),
        branch,
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

pub(crate) fn show(ui: &mut Ui, rect: Rect, line: &Line) -> Option<Choice> {
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, colors::bg());
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, colors::border()));
    let y = rect.center().y;
    let mut x = rect.left() + SIDE_PAD;
    painter.circle_filled(pos2(x + 4.0, y), 4.0, dot_color(line));
    x += 18.0;
    let text = |x: &mut f32, text: &str, font: egui::FontId, color: Color32| {
        let galley = painter.layout_no_wrap(text.to_owned(), font, color);
        let width = galley.size().x;
        painter.galley(pos2(*x, y - galley.size().y / 2.0), galley, color);
        *x += width;
    };
    text(&mut x, line.label, body(13.0), colors::fg());
    for part in [Some(line.who.as_str()), line.branch.as_deref()].into_iter().flatten() {
        text(&mut x, "   \u{b7}   ", body(13.0), colors::dim());
        let font = if line.branch.as_deref() == Some(part) { mono(12.5) } else { body(13.0) };
        text(&mut x, part, font, colors::dim());
    }
    if let (Some(_), Some(base)) = (&line.branch, &line.base) {
        text(&mut x, &format!(" from {base}"), body(13.0), colors::dim());
    }
    toggles(ui, rect, line)
}

fn toggles(ui: &mut Ui, rect: Rect, line: &Line) -> Option<Choice> {
    let mut choice = None;
    let mut right = rect.right() - SIDE_PAD;
    for (label, which, active) in [
        ("Terminal", Choice::Terminal, !line.reader_on),
        ("Reader", Choice::Reader, line.reader_on),
    ] {
        if which == Choice::Reader && !line.reader_available {
            continue;
        }
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            body(13.0),
            if active { colors::fg() } else { colors::dim() },
        );
        let size = galley.size() + vec2(20.0, 10.0);
        let pill = Rect::from_min_size(pos2(right - size.x, rect.center().y - size.y / 2.0), size);
        let response = ui.interact(pill, ui.id().with(("status-line", label)), Sense::click());
        let painter = ui.painter();
        if active {
            painter.rect(
                pill,
                CornerRadius::same(6),
                colors::elevated(),
                Stroke::new(1.0, colors::border()),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(pill, 6.0, colors::hover());
        }
        let ink = if active { colors::fg() } else { colors::dim() };
        painter.galley(pill.center() - galley.size() / 2.0, galley, ink);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            choice = Some(which);
        }
        right = pill.left() - 4.0;
    }
    choice
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
    fn a_tab_that_waits_for_its_first_message_is_ready_and_names_who_where_and_from() {
        let mut pane = pane(FirstPrompt::Awaiting, true);
        let run = &mut pane.agent.as_mut().unwrap().run;
        run.started(None, 0.0);
        run.refresh(1.0);
        let line = line_of(&pane, Shown::Reader, true, "Claude Code", Some("main")).unwrap();
        assert_eq!(line.who, "Builder, Claude Code, Auto edit");
        assert_eq!(line.branch.as_deref(), Some("agent/quick-beacon"));
        assert_eq!(line.base.as_deref(), Some("main"));
        assert!(line.reader_on);
        assert_eq!((line.label, line.ready), ("Ready", true));
    }

    #[test]
    fn a_tab_still_setting_up_is_not_ready_and_a_plain_folder_has_no_branch() {
        let pane = pane(FirstPrompt::Awaiting, false);
        let line =
            line_of(&pane, Shown::Terminal(None), false, "Claude Code", Some("main")).unwrap();
        assert_eq!((line.label, line.ready), ("Setting up", false));
        assert_eq!((line.branch, line.base), (None, None));
        assert!(!line.reader_on && !line.reader_available);
    }

    #[test]
    fn once_the_first_message_is_in_the_status_is_the_agents_own() {
        let pane = pane(FirstPrompt::NotNeeded, true);
        let line = line_of(&pane, Shown::Reader, true, "Claude Code", None).unwrap();
        assert!(!line.ready);
        assert_eq!(line.base, None);
    }

    #[test]
    fn a_plain_shell_has_no_line() {
        let pane: Pane<u32> = Pane::shell(0);
        assert_eq!(line_of(&pane, Shown::Terminal(None), false, "", None), None);
    }
}
