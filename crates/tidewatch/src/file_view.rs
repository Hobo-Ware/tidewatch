//! The file view of the side layout: a file's diff in the terminal's place, as a card like History.
//! One 48 px header holds the back button, the file's path, its counts and the commit it is from, then
//! on the right the note buttons, the place among the commit's files with the arrows, and Unified or
//! Split. The notes strip sits above the diff body.

use std::sync::Arc;

use egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Id, Rect, Response, Sense, Stroke, StrokeKind,
    Ui, pos2, vec2,
};
use kelp_core::commit::FileChange;
use kelp_ui::diff::{DiffLayout, DiffStyle};
use tidewatch_core::status::AgentStatus;

use crate::brand;
use crate::commit_files::{split_dir, tail_fit};
use crate::diff_pane::{PaneAction, PaneOut, SHORT_ID, message, set_layout, show_body};
use crate::flow::PaneExtras;
use crate::history_pane;
use crate::layout::{CARD_MARGIN, CHROME_H};
use crate::screen_kit::{body, mono};
use crate::tab_view::TabView;
use crate::{file_nav, notes_strip, segmented};

pub(crate) const BUTTON_H: f32 = 26.0;
const BUTTON_RADIUS: u8 = 7;
pub(crate) const ARROW_W: f32 = 26.0;
const ARROW_RADIUS: u8 = 6;
const PAD_LEFT: f32 = 8.0;
const PAD_RIGHT: f32 = 10.0;
pub(crate) const GAP: f32 = 10.0;
const COUNT_GAP: f32 = 4.0;
const PILL_PAD: f32 = 10.0;
const MIN_PATH_W: f32 = 100.0;
const MIN_SHOWN_PATH_W: f32 = 40.0;
const FONT: f32 = 12.5;
const LAYOUTS: [&str; 2] = ["Unified", "Split"];

/// What the Terminal chip says about the agent behind the terminal: its profile and state.
pub(crate) struct TerminalChip {
    pub(crate) profile: Option<String>,
    pub(crate) status: Option<AgentStatus>,
}

fn state_word(status: &AgentStatus) -> Option<&'static str> {
    match status {
        AgentStatus::SettingUp => Some("starting"),
        AgentStatus::Working => Some("working"),
        AgentStatus::NeedsYou(_) => Some("needs you"),
        AgentStatus::Done => Some("ready"),
        AgentStatus::Failed(_) => Some("failed"),
        AgentStatus::Shell => None,
    }
}

impl TerminalChip {
    /// `Builder working`, or nothing for a plain shell.
    pub(crate) fn text(&self) -> Option<String> {
        let word = state_word(self.status.as_ref()?)?;
        Some(match &self.profile {
            Some(profile) => format!("{profile} {word}"),
            None => word.to_owned(),
        })
    }

    fn tip(&self) -> String {
        self.text().map_or_else(
            || "Back to the terminal".to_owned(),
            |text| format!("{text} - back to the terminal"),
        )
    }
}

/// `6 of 11`: where `current` stands among the commit's files, nothing when it is not one of them.
pub(crate) fn position_text(files: &[FileChange], current: Option<&str>) -> Option<String> {
    let at = files.iter().position(|file| Some(file.path.as_str()) == current)?;
    Some(format!("{} of {}", at + 1, files.len()))
}

/// What follows the counts: `in ` with the short id of the commit set apart, or the working tree.
pub(crate) fn origin(sha: Option<&str>) -> (&'static str, Option<&str>) {
    match sha {
        Some(sha) => ("in ", Some(sha)),
        None => ("in working tree", None),
    }
}

/// How many of the optional buttons, in priority order, fit in `room` with a gap before each.
fn buttons_that_fit(room: f32, widths: &[f32]) -> usize {
    let mut used = 0.0;
    widths
        .iter()
        .take_while(|width| {
            used += **width + GAP;
            used <= room
        })
        .count()
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    extras: Option<&PaneExtras>,
    chip: &TerminalChip,
    now: i64,
) -> PaneOut {
    let card = rect.shrink(CARD_MARGIN);
    history_pane::paint_card(ui, card);
    let mut out = PaneOut::default();
    ui.scope_builder(egui::UiBuilder::new().id_salt("file-view").max_rect(card), |ui| {
        ui.set_clip_rect(card.intersect(ui.clip_rect()));
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let header = Rect::from_min_size(card.min, vec2(card.width(), CHROME_H));
        ui.painter().hline(
            header.shrink2(vec2(1.0, 0.0)).x_range(),
            header.bottom(),
            Stroke::new(1.0, brand::CARD_LINE),
        );
        out.back = show_header(ui, header, view, extras, chip, &mut out.actions);
        let mut top = header.bottom();
        let strip_h = extras.map_or(0.0, |e| notes_strip::height(e.rows.len()));
        if let Some(extras) = extras.filter(|_| strip_h > 0.0) {
            let strip = Rect::from_min_size(
                pos2(card.left() + 1.0, top),
                vec2(card.width() - 2.0, strip_h),
            );
            let actions = notes_strip::show(ui, strip, &extras.rows, now);
            out.actions.extend(actions.into_iter().map(PaneAction::Strip));
            top += strip_h;
        }
        let area = Rect::from_min_max(pos2(card.left() + 1.0, top), card.max - vec2(1.0, 1.0));
        if view.loaded.is_none() {
            message(ui, area, view.error.as_deref(), "Loading history...");
        } else {
            out.notes_changed = show_body(ui, area, view, DiffStyle::embedded(brand::TEAL));
        }
        history_pane::round_bottom(ui, card);
    });
    out
}

fn text_width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), font.clone(), brand::TEXT).size().x
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Plain,
    On,
    Primary,
}

struct Pill {
    action: PaneAction,
    label: String,
    look: Look,
    ink: Option<Color32>,
    enabled: bool,
    tip: String,
}

fn pill_width(ui: &Ui, label: &str) -> f32 {
    text_width(ui, label, &body(FONT)) + 2.0 * PILL_PAD
}

/// One small bordered button with its right edge at `right`; returns the response and its left edge.
fn pill(ui: &mut Ui, id: Id, right: f32, center_y: f32, pill: &Pill) -> (Response, f32) {
    let width = pill_width(ui, &pill.label);
    let rect =
        Rect::from_min_size(pos2(right - width, center_y - BUTTON_H / 2.0), vec2(width, BUTTON_H));
    let sense = if pill.enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, id, sense);
    let hot = pill.enabled && response.hovered();
    let (fill, line, ink) = match pill.look {
        Look::Primary => (brand::TEAL, brand::TEAL, brand::KEY_EDGE),
        Look::On => (brand::SEGMENT_ON, brand::SEGMENT_LINE, brand::TEAL_TEXT),
        Look::Plain => (
            if hot { brand::BUTTON } else { brand::SEGMENT },
            brand::SEGMENT_LINE,
            if hot { brand::TEXT } else { brand::SOFT },
        ),
    };
    let ink = pill.ink.unwrap_or(ink);
    let ink = if pill.enabled { ink } else { brand::FAINT };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(BUTTON_RADIUS),
        fill,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    painter.text(rect.center(), Align2::CENTER_CENTER, &pill.label, body(FONT), ink);
    let response = if pill.enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    (response.on_hover_text(&pill.tip), rect.left())
}

pub(crate) fn arrow(ui: &mut Ui, id: Id, rect: Rect, up: bool, enabled: bool, tip: &str) -> bool {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, id, sense);
    let hot = enabled && response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(ARROW_RADIUS),
        if hot { brand::BUTTON } else { brand::SEGMENT },
        Stroke::new(1.0, brand::SEGMENT_LINE),
        StrokeKind::Inside,
    );
    let ink = match (enabled, hot) {
        (false, _) => brand::FAINT,
        (true, true) => brand::TEXT,
        (true, false) => brand::SOFT,
    };
    let mid = rect.center();
    let tip_y = if up { -1.5 } else { 1.5 };
    let wing_y = -tip_y;
    let stroke = Stroke::new(1.6, ink);
    painter.line_segment([mid + vec2(-3.5, wing_y), mid + vec2(0.0, tip_y)], stroke);
    painter.line_segment([mid + vec2(0.0, tip_y), mid + vec2(3.5, wing_y)], stroke);
    response.on_hover_text(tip).clicked() && enabled
}

fn back_button(ui: &mut Ui, left: f32, center_y: f32, chip: &TerminalChip) -> Response {
    let label = ui.painter().layout_no_wrap("Terminal".to_owned(), body(FONT), brand::SOFT);
    let width = 7.0 + 12.0 + 6.0 + label.size().x + PILL_PAD;
    let rect = Rect::from_min_size(pos2(left, center_y - BUTTON_H / 2.0), vec2(width, BUTTON_H));
    let response = ui.interact(rect, Id::new("terminal-chip"), Sense::click());
    let hot = response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(BUTTON_RADIUS),
        if hot { brand::BUTTON } else { brand::SEGMENT },
        Stroke::new(1.0, brand::SEGMENT_LINE),
        StrokeKind::Inside,
    );
    let ink = if hot { brand::TEXT } else { brand::SOFT };
    let caret = pos2(rect.left() + 7.0 + 6.0, center_y);
    let stroke = Stroke::new(1.6, ink);
    painter.line_segment([caret + vec2(2.0, -3.5), caret + vec2(-1.5, 0.0)], stroke);
    painter.line_segment([caret + vec2(-1.5, 0.0), caret + vec2(2.0, 3.5)], stroke);
    let text_at = pos2(rect.left() + 7.0 + 12.0 + 6.0, center_y - label.size().y / 2.0);
    painter.galley(text_at, label, ink);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(chip.tip())
}

/// Draws the header and returns whether the back button was clicked.
fn show_header(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    extras: Option<&PaneExtras>,
    chip: &TerminalChip,
    actions: &mut Vec<PaneAction>,
) -> bool {
    let center_y = rect.center().y;
    let back = back_button(ui, rect.left() + PAD_LEFT, center_y, chip);

    let active = usize::from(view.layout == DiffLayout::Split);
    let switch = segmented::show(
        ui,
        Id::new("file-view-layout"),
        rect.right() - PAD_RIGHT,
        center_y,
        &LAYOUTS,
        active,
    );
    if let Some(index) = switch.clicked {
        set_layout(view, if index == 0 { DiffLayout::Unified } else { DiffLayout::Split });
    }
    let mut right = switch.left - GAP;
    right = file_arrows(ui, right, center_y, view);
    if let Some(text) = position_text(&view.files, view.file.as_deref()) {
        let galley = ui.painter().layout_no_wrap(text, body(FONT), brand::MUTED);
        let at = pos2(right - 8.0 - galley.size().x, center_y - galley.size().y / 2.0);
        right = at.x - GAP;
        ui.painter().galley(at, galley, brand::MUTED);
    }
    let floor = back.rect.right() + GAP + MIN_PATH_W + GAP + counts_width(ui, view);
    if let Some(extras) = extras {
        right = note_buttons(ui, right, floor, center_y, extras, actions);
    }
    left_group(ui, back.rect.right() + GAP, right - GAP, center_y, view);
    back.clicked()
}

fn file_arrows(ui: &mut Ui, right: f32, center_y: f32, view: &mut TabView) -> f32 {
    let top = center_y - BUTTON_H / 2.0;
    let next = Rect::from_min_size(pos2(right - ARROW_W, top), vec2(ARROW_W, BUTTON_H));
    let prev = next.translate(vec2(-ARROW_W - 2.0, 0.0));
    let current = view.file.as_deref();
    let can_prev = file_nav::step_file(&view.files, current, -1).is_some();
    let can_next = file_nav::step_file(&view.files, current, 1).is_some();
    if arrow(ui, Id::new("file-view-prev"), prev, true, can_prev, "Previous file") {
        view.step_file(-1);
    }
    if arrow(ui, Id::new("file-view-next"), next, false, can_next, "Next file") {
        view.step_file(1);
    }
    prev.left()
}

/// The note buttons in priority order: Send all open when something is waiting, Deliver when idle,
/// the PR chip and Pull PR comments.
fn note_pills(extras: &PaneExtras) -> Vec<Pill> {
    let mut pills = Vec::with_capacity(4);
    if extras.pending > 0 {
        let (enabled, tip) = match extras.send_all {
            Ok(()) => (true, "Paste every unsent open note into the agent's terminal"),
            Err(reason) => (false, reason),
        };
        pills.push(Pill {
            action: PaneAction::SendAll,
            label: format!("Send all ({})", extras.pending),
            look: Look::Primary,
            ink: None,
            enabled,
            tip: tip.to_owned(),
        });
    }
    pills.push(Pill {
        action: PaneAction::ToggleAuto,
        label: "Deliver when idle".to_owned(),
        look: if extras.auto { Look::On } else { Look::Plain },
        ink: None,
        enabled: true,
        tip: "Send queued notes by themselves when the agent is done (never while it works or waits on you)".to_owned(),
    });
    if let Some(chip) = &extras.chip {
        pills.push(Pill {
            action: PaneAction::OpenUrl(chip.url.clone()),
            label: chip.text.clone(),
            look: Look::Plain,
            ink: Some(brand::GREEN),
            enabled: true,
            tip: format!("{} - click to open in the browser", chip.url),
        });
        pills.push(Pill {
            action: PaneAction::PullPr,
            label: if extras.pulling { "Pulling..." } else { "Pull PR comments" }.to_owned(),
            look: Look::Plain,
            ink: None,
            enabled: !extras.pulling,
            tip: "Import the pull request's review comments as notes".to_owned(),
        });
    }
    pills
}

/// Draws as many note buttons as fit between `floor` and `right`, right to left. Returns the left
/// edge of the free room.
pub(crate) fn note_buttons(
    ui: &mut Ui,
    right: f32,
    floor: f32,
    center_y: f32,
    extras: &PaneExtras,
    actions: &mut Vec<PaneAction>,
) -> f32 {
    let pills = note_pills(extras);
    let widths: Vec<f32> = pills.iter().map(|p| pill_width(ui, &p.label)).collect();
    let fit = buttons_that_fit(right - floor, &widths);
    let mut right = right;
    for (index, item) in pills.iter().take(fit).enumerate() {
        let (response, left) = pill(ui, Id::new(("file-view-note", index)), right, center_y, item);
        if response.clicked() && item.enabled {
            actions.push(item.action.clone());
        }
        right = left - GAP;
    }
    right
}

/// The room the path takes and whether `in <sha>` follows, for the `room` between the back button and
/// the right-hand buttons: the path is cut only once its counts no longer fit beside it, and the
/// origin is the first thing to go.
fn plan_left(path_w: f32, counts_w: f32, origin_w: f32, room: f32) -> (f32, bool) {
    let with_counts = path_w + GAP + counts_w;
    if with_counts + GAP + origin_w <= room {
        (path_w, true)
    } else if with_counts <= room {
        (path_w, false)
    } else {
        ((room - GAP - counts_w).max(MIN_SHOWN_PATH_W), false)
    }
}

fn counts_text(view: &TabView) -> (String, String) {
    let (added, removed) =
        view.open.as_ref().map_or((0, 0), |open| (open.diff.added, open.diff.removed));
    (format!("+{added}"), format!("-{removed}"))
}

fn counts_width(ui: &Ui, view: &TabView) -> f32 {
    let (plus, minus) = counts_text(view);
    text_width(ui, &plus, &body(12.0)) + COUNT_GAP + text_width(ui, &minus, &body(12.0))
}

/// The path with its folder dimmed, the file's counts and where it is from, from `left` up to
/// `limit`.
fn left_group(ui: &Ui, left: f32, limit: f32, center_y: f32, view: &TabView) {
    let painter = ui.painter();
    let put = |x: f32, galley: Arc<Galley>| -> f32 {
        let width = galley.size().x;
        painter.galley(pos2(x, center_y - galley.size().y / 2.0), galley, brand::TEXT);
        x + width
    };
    let lay =
        |text: &str, font: FontId, color| painter.layout_no_wrap(text.to_owned(), font, color);
    let path = view.file.as_deref().unwrap_or("");
    if path.is_empty() {
        return;
    }
    let (plus, minus) = counts_text(view);
    let sha = view.summary.as_ref().map(|(id, _)| id.to_hex_with_len(SHORT_ID).to_string());
    let (lead, sha) = origin(sha.as_deref());
    let lead = lay(lead, body(12.0), brand::MUTED);
    let sha = sha.map(|sha| lay(sha, mono(12.0), brand::SOFT));
    let origin_w = lead.size().x + sha.as_ref().map_or(0.0, |g| g.size().x);
    let font = mono(14.0);
    let path_w = text_width(ui, path, &font);
    let (path_room, with_origin) =
        plan_left(path_w, counts_width(ui, view), origin_w, limit - left);
    let shown = tail_fit(ui, path, &font, path_room);
    let (dir, name) = split_dir(&shown);
    let mut x = put(left, lay(dir, font.clone(), brand::MUTED));
    x = put(x, lay(name, font, brand::TEXT)) + GAP;
    x = put(x, lay(&plus, body(12.0), brand::GREEN)) + COUNT_GAP;
    x = put(x, lay(&minus, body(12.0), brand::RED)) + GAP;
    if with_origin {
        x = put(x, lead);
        if let Some(sha) = sha {
            put(x, sha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::commit::ChangeKind;
    use tidewatch_core::status::Reason;

    fn chip(profile: Option<&str>, status: Option<AgentStatus>) -> TerminalChip {
        TerminalChip { profile: profile.map(str::to_owned), status }
    }

    #[test]
    fn the_chip_names_the_profile_and_what_it_is_doing() {
        let table = [
            (AgentStatus::Working, "Builder working"),
            (AgentStatus::Done, "Builder ready"),
            (AgentStatus::NeedsYou(Reason::Permission), "Builder needs you"),
            (AgentStatus::SettingUp, "Builder starting"),
            (AgentStatus::Failed("x".into()), "Builder failed"),
        ];
        for (status, want) in table {
            assert_eq!(chip(Some("Builder"), Some(status)).text().as_deref(), Some(want));
        }
    }

    #[test]
    fn a_plain_shell_has_a_bare_chip() {
        assert_eq!(chip(None, Some(AgentStatus::Shell)).text(), None);
        assert_eq!(chip(None, None).text(), None);
        assert_eq!(chip(None, Some(AgentStatus::Working)).text().as_deref(), Some("working"));
    }

    fn files(paths: &[&str]) -> Vec<FileChange> {
        paths
            .iter()
            .map(|p| FileChange { path: (*p).to_owned(), kind: ChangeKind::Added })
            .collect()
    }

    #[test]
    fn the_position_counts_the_open_file_among_the_commits_files() {
        let list = files(&["a", "b", "c"]);
        assert_eq!(position_text(&list, Some("a")).as_deref(), Some("1 of 3"));
        assert_eq!(position_text(&list, Some("c")).as_deref(), Some("3 of 3"));
        assert_eq!(position_text(&list, Some("gone")), None);
        assert_eq!(position_text(&list, None), None);
        assert_eq!(position_text(&[], Some("a")), None);
    }

    #[test]
    fn the_origin_sets_the_short_id_apart_and_names_the_working_tree() {
        assert_eq!(origin(Some("298061d")), ("in ", Some("298061d")));
        assert_eq!(origin(None), ("in working tree", None));
    }

    #[test]
    fn note_buttons_drop_from_the_end_of_the_priority_list_when_room_runs_out() {
        let widths = [100.0, 80.0, 60.0, 90.0];
        assert_eq!(buttons_that_fit(1000.0, &widths), 4);
        assert_eq!(buttons_that_fit(100.0 + GAP + 80.0 + GAP, &widths), 2);
        assert_eq!(buttons_that_fit(100.0 + GAP + 80.0 + GAP - 1.0, &widths), 1);
        assert_eq!(buttons_that_fit(50.0, &widths), 0);
        assert_eq!(buttons_that_fit(1000.0, &[]), 0);
    }

    #[test]
    fn the_origin_goes_first_then_the_path_is_cut_beside_its_counts() {
        assert_eq!(plan_left(100.0, 40.0, 80.0, 1000.0), (100.0, true));
        assert_eq!(plan_left(100.0, 40.0, 80.0, 240.0), (100.0, true));
        assert_eq!(plan_left(100.0, 40.0, 80.0, 239.0), (100.0, false));
        assert_eq!(plan_left(100.0, 40.0, 80.0, 150.0), (100.0, false));
        assert_eq!(plan_left(100.0, 40.0, 80.0, 149.0), (99.0, false));
        assert_eq!(plan_left(100.0, 40.0, 80.0, 10.0), (MIN_SHOWN_PATH_W, false));
    }

    #[test]
    fn the_back_button_tip_names_the_agent_when_there_is_one() {
        assert_eq!(chip(None, None).tip(), "Back to the terminal");
        assert_eq!(
            chip(Some("Builder"), Some(AgentStatus::Working)).tip(),
            "Builder working - back to the terminal"
        );
    }
}
