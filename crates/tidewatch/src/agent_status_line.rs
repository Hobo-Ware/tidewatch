//! The status line above an agent tab: its state, who runs it, its branch and the view choice.

use egui::{Color32, Rect, Ui, pos2};
use stdusk_egui::colors;
use tidewatch_core::status::{AgentStatus, Reason};

use crate::brand;
use crate::layout::CHROME_H;
use crate::pane::{AgentTab, FirstPrompt, Pane};
use crate::reader::Shown;
use crate::remote_pill::{self, Pill, Popover};
use crate::screen_kit::{Size, body};
use crate::segmented;
use crate::status_view;
use tidewatch_core::panel_view::Tone;

pub(crate) const HEIGHT: f32 = CHROME_H;
const LEFT_PAD: f32 = 10.0;
const RIGHT_PAD: f32 = 6.0;
const PILL_GAP: f32 = 10.0;
const CHIP_PAD: f32 = 10.0;
const CHEVRON_W: f32 = 12.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) label: &'static str,
    pub(crate) tone: Tone,
    pub(crate) ready: bool,
    pub(crate) who: String,
    pub(crate) reader_on: bool,
    pub(crate) reader_available: bool,
    /// The agent runs with remote control on.
    pub(crate) remote: Option<Pill>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Reader,
    Terminal,
}

/// What the line asks for this frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Out {
    pub(crate) choice: Option<Choice>,
    /// The profile chip was pressed: open Change this tab.
    pub(crate) change: bool,
    pub(crate) remote: Option<remote_pill::Action>,
}

/// A tab that waits for its first message with its agent up, or still printing its banner.
pub(crate) fn is_ready(first_prompt: FirstPrompt, tone: Tone) -> bool {
    first_prompt == FirstPrompt::Awaiting
        && matches!(tone, Tone::Working | Tone::Done | Tone::Shell)
}

/// The line of an agent tab. `None` for the rest.
/// `remote_app` is where the CLI's remote session is driven from.
pub(crate) fn line_of<T>(
    pane: &Pane<T>,
    shown: Shown,
    reader_available: bool,
    (cli_name, remote_app): (&str, Option<&'static str>),
) -> Option<Line> {
    let agent = pane.agent.as_ref()?;
    let ready = is_ready(agent.first_prompt, Tone::of(agent.run.status()));
    let tone = agent.run.shown_tone();
    let level = agent.launched.as_ref().map(|launched| launched.level.label());
    let who = [Some(agent.profile_name.as_str()), Some(cli_name), level]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
    Some(Line {
        label: label_of(agent, ready, tone),
        tone,
        ready,
        who,
        reader_on: shown == Shown::Reader,
        reader_available,
        remote: agent.remote.as_ref().map(|session| Pill {
            name: session.name.clone(),
            app: remote_app.unwrap_or("the app"),
            link: session.link().map(str::to_owned),
            saved_on: agent.launched.as_ref().is_some_and(|launched| launched.remote),
        }),
    })
}

const UNREPORTED: &str = "Quiet since the restart, auto-sends wait";

fn label_of(agent: &AgentTab, ready: bool, tone: Tone) -> &'static str {
    if ready {
        "Ready"
    } else if agent.run.unreported() && *agent.run.status() == AgentStatus::NeedsYou(Reason::Quiet)
    {
        UNREPORTED
    } else {
        status_view::label(tone)
    }
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

pub(crate) fn show(ui: &mut Ui, rect: Rect, line: &Line, popover: Popover) -> Out {
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
    let mut text_right = switch.left;
    let mut remote = None;
    if let Some(pill) = &line.remote {
        let shown = remote_pill::show(ui, switch.left - PILL_GAP, y, pill, popover);
        text_right = shown.left;
        remote = shown.action;
    }
    let painter = ui
        .painter()
        .with_clip_rect(Rect::from_min_max(rect.min, pos2(text_right - 8.0, rect.bottom())));
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
    let change = who_chip(ui, &painter, pos2(x, y), &line.who, text_right - 8.0);
    Out { choice: switch.clicked.map(|index| choices[index]), change, remote }
}

/// Who runs the tab as a chip with a chevron: pressing it opens Change this tab. Returns whether
/// it was pressed.
fn who_chip(ui: &Ui, painter: &egui::Painter, at: egui::Pos2, text: &str, right: f32) -> bool {
    if text.is_empty() {
        return false;
    }
    let galley = painter.layout_no_wrap(text.to_owned(), body(Size::Compact.font()), brand::SOFT);
    let width = (galley.size().x + 2.0 * CHIP_PAD + CHEVRON_W).min((right - at.x).max(0.0));
    let rect = Rect::from_min_size(
        pos2(at.x, at.y - Size::Compact.height() / 2.0),
        egui::vec2(width, Size::Compact.height()),
    );
    let response = ui
        .interact(rect, ui.id().with("who-chip"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Change this tab");
    let line = if response.hovered() { brand::FAINT } else { brand::FIELD_LINE };
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(Size::Compact.radius()),
        egui::Stroke::new(1.0, line),
        egui::StrokeKind::Inside,
    );
    let ink = if response.hovered() { brand::TEXT } else { brand::SOFT };
    let text_at = pos2(rect.left() + CHIP_PAD, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_at, galley, ink);
    let chevron = pos2(rect.right() - CHIP_PAD - CHEVRON_W / 2.0 + 2.0, rect.center().y);
    let half = 3.5;
    let stroke = egui::Stroke::new(1.5, ink);
    painter
        .line_segment([chevron + egui::vec2(-half, -1.5), chevron + egui::vec2(0.0, 2.0)], stroke);
    painter
        .line_segment([chevron + egui::vec2(0.0, 2.0), chevron + egui::vec2(half, -1.5)], stroke);
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tidewatch_core::agents::Level;
    use tidewatch_core::worktree::Worktree;

    use crate::agent_run::AgentRun;
    use crate::pane::{AgentTab, Launched};
    use crate::remote_session::RemoteSession;

    fn pane(first: FirstPrompt, worktree: bool) -> Pane<u32> {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        agent.first_prompt = first;
        agent.launched = Some(Launched {
            profile_id: "builder".into(),
            level: Level::Edits,
            session_id: None,
            remote: false,
            dir: None,
        });
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
        let line = line_of(&pane, Shown::Reader, true, ("Claude Code", None)).unwrap();
        assert_eq!(line.who, "Builder \u{b7} Claude Code \u{b7} Auto edit");
        assert!(line.reader_on);
        assert_eq!((line.label, line.ready), ("Ready", true));
    }

    #[test]
    fn a_tab_still_setting_up_is_not_ready() {
        let pane = pane(FirstPrompt::Awaiting, false);
        let line = line_of(&pane, Shown::Terminal(None), false, ("Claude Code", None)).unwrap();
        assert_eq!((line.label, line.ready), ("Setting up", false));
        assert!(!line.reader_on && !line.reader_available);
    }

    #[test]
    fn a_handed_over_agent_nothing_has_spoken_for_says_why_its_auto_sends_wait() {
        let mut pane = pane(FirstPrompt::NotNeeded, true);
        let run = &mut pane.agent.as_mut().unwrap().run;
        run.started(None, 0.0);
        run.await_report();
        run.refresh(600.0);
        let line = line_of(&pane, Shown::Reader, true, ("Claude Code", None)).unwrap();
        assert_eq!(line.label, "Quiet since the restart, auto-sends wait");
        let run = &mut pane.agent.as_mut().unwrap().run;
        run.hold(Some(AgentStatus::Done), None);
        run.refresh(601.0);
        let line = line_of(&pane, Shown::Reader, true, ("Claude Code", None)).unwrap();
        assert_eq!(line.label, "Done");
    }

    #[test]
    fn once_the_first_message_is_in_the_status_is_the_agents_own() {
        let pane = pane(FirstPrompt::NotNeeded, true);
        let line = line_of(&pane, Shown::Reader, true, ("Claude Code", None)).unwrap();
        assert!(!line.ready);
    }

    #[test]
    fn a_plain_shell_has_no_line() {
        let pane: Pane<u32> = Pane::shell(0);
        assert_eq!(line_of(&pane, Shown::Terminal(None), false, ("", None)), None);
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
            let line = line_of(&pane, shown, available, ("Claude Code", None)).unwrap();
            assert_eq!(segments(&line), (choices, active), "{shown:?} {available}");
        }
    }

    fn remote_pane(saved_on: bool) -> Pane<u32> {
        let mut pane = pane(FirstPrompt::NotNeeded, true);
        let agent = pane.agent.as_mut().unwrap();
        agent.remote = Some(RemoteSession::new("web \u{b7} otter".into()));
        agent.launched.as_mut().unwrap().remote = saved_on;
        pane
    }

    #[test]
    fn a_tab_running_with_remote_control_gets_a_pill_with_the_session_the_app_and_the_saved_setting()
     {
        let app = Some("the Claude app");
        let line = line_of(&remote_pane(true), Shown::Reader, true, ("Claude Code", app)).unwrap();
        assert_eq!(
            line.remote,
            Some(Pill {
                name: "web \u{b7} otter".into(),
                app: "the Claude app",
                link: None,
                saved_on: true
            })
        );
        let turned_off = line_of(&remote_pane(false), Shown::Reader, true, ("Claude Code", app));
        assert!(!turned_off.unwrap().remote.unwrap().saved_on);
    }

    #[test]
    fn a_link_seen_in_the_terminal_reaches_the_pill() {
        let mut pane = remote_pane(true);
        let link = "https://claude.ai/code/session_01AbCd";
        pane.agent.as_mut().unwrap().remote.as_mut().unwrap().scanned(
            1,
            0.0,
            &[format!("Remote Control: {link}")],
            80,
            "claude",
        );
        let line = line_of(&pane, Shown::Reader, true, ("Claude Code", Some("the Claude app")));
        assert_eq!(line.unwrap().remote.unwrap().link.as_deref(), Some(link));
    }

    #[test]
    fn a_tab_without_it_has_no_pill() {
        let pane = pane(FirstPrompt::NotNeeded, true);
        assert_eq!(
            line_of(&pane, Shown::Reader, true, ("Claude Code", None)).unwrap().remote,
            None
        );
    }
}
