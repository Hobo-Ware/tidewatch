//! The strip under an agent tab's status line while its pull request is known (PLAN.md, D4
//! follow-up): number, title, pills, the tab's GitHub mode with its popover, and Open on GitHub.

use egui::{
    Align2, Color32, CornerRadius, Id, Key, Modifiers, Rect, Response, Sense, Stroke, StrokeKind,
    Ui, pos2, vec2,
};
use tidewatch_core::pr_status::{Decision, State, Summary, SummaryChecks, Tone};
use tidewatch_core::prefs::{AutoJob, GithubPrefs, PrMode};

use crate::brand;
use crate::pr_card_model::Icon;
use crate::pr_cards_view::paint_icon;
use crate::reader_view::{INSET, ellipsized};
use crate::screen_kit::{
    Size, TEXT_BODY, TEXT_SECONDARY, TEXT_TAG, TagFill, body, mono, paint_tag, tag_width,
};
use crate::segmented;

pub(crate) const HEIGHT: f32 = 44.0;
const CARD_H: f32 = 36.0;
const TITLE_MAX_W: f32 = 300.0;
const GAP: f32 = 8.0;
const CHEVRON_ROOM: f32 = 12.0;
const POPOVER_W: f32 = 420.0;

pub(crate) struct Strip<'a> {
    pub(crate) summary: &'a Summary,
    pub(crate) policy: GithubPrefs,
    pub(crate) dev_open_popover: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Open(String),
    Policy(GithubPrefs),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pill {
    pub(crate) text: String,
    pub(crate) tone: Tone,
    pub(crate) outlined: bool,
}

fn state_pill(state: State) -> Option<Pill> {
    let (text, tone) = match state {
        State::Open => return None,
        State::Merged => ("Merged", Tone::Good),
        State::Closed => ("Closed", Tone::Bad),
    };
    Some(Pill { text: text.to_owned(), tone, outlined: true })
}

/// The pills of a pull request, most important first.
pub(crate) fn pills(summary: &Summary) -> Vec<Pill> {
    let mut pills: Vec<Pill> = state_pill(summary.state).into_iter().collect();
    let mut add =
        |text: &str, tone| pills.push(Pill { text: text.to_owned(), tone, outlined: false });
    if summary.state == State::Open {
        match summary.checks {
            SummaryChecks::Running => add("Checks running", Tone::Warn),
            SummaryChecks::Failed => add("Checks failed", Tone::Bad),
            SummaryChecks::Passed => add("Checks passed", Tone::Good),
            SummaryChecks::None => {}
        }
        match summary.decision {
            Decision::ChangesRequested => add("Changes requested", Tone::Warn),
            Decision::Approved => add("Approved", Tone::Good),
            Decision::None | Decision::ReviewRequired => {}
        }
        if summary.conflicting {
            add("Conflicts", Tone::Warn);
        }
    }
    if summary.comments > 0 {
        let noun = if summary.comments == 1 { "comment" } else { "comments" };
        add(&format!("{} {noun}", summary.comments), Tone::Quiet);
    }
    pills
}

pub(crate) fn mode_label(mode: PrMode) -> &'static str {
    match mode {
        PrMode::Ask => "Ask me",
        PrMode::Auto => "Auto",
        PrMode::Off => "Off",
    }
}

pub(crate) fn tone_ink(tone: Tone) -> Color32 {
    match tone {
        Tone::Bad => brand::RED,
        Tone::Warn => brand::AMBER,
        Tone::Good => brand::GREEN,
        Tone::Quiet => brand::SOFT,
    }
}

pub(crate) struct JobRow {
    pub(crate) which: AutoJob,
    pub(crate) label: &'static str,
    pub(crate) caption: Option<&'static str>,
}

pub(crate) const JOBS: [JobRow; 4] = [
    JobRow {
        which: AutoJob::FailedChecks,
        label: "Fix failed checks, with the failing log lines",
        caption: None,
    },
    JobRow {
        which: AutoJob::Reviews,
        label: "Answer review comments and requested changes",
        caption: None,
    },
    JobRow {
        which: AutoJob::Conflicts,
        label: "Fix merge conflicts with the base branch",
        caption: None,
    },
    JobRow {
        which: AutoJob::Push,
        label: "Push and open the pull request",
        caption: Some(
            "When the agent is done and has commits. Opens a draft with the agent's summary as the body. The agent is told first.",
        ),
    },
];

const MODES: [(PrMode, &str); 3] =
    [(PrMode::Ask, "Ask me"), (PrMode::Auto, "Auto"), (PrMode::Off, "Off")];

fn mode_help(mode: PrMode) -> &'static str {
    match mode {
        PrMode::Ask => "Ask me shows a card with one button.",
        PrMode::Auto => {
            "Auto does the ticked jobs by itself, only when the agent is idle, at most 3 times per PR. It tells the agent each time."
        }
        PrMode::Off => "Off stops looking at this pull request.",
    }
}

pub(crate) fn show(ui: &mut Ui, band: Rect, strip: &Strip<'_>) -> Option<Action> {
    let card = Rect::from_center_size(band.center(), vec2(band.width() - 2.0 * INSET, CARD_H));
    let painter = ui.painter().clone();
    painter.rect(
        card,
        CornerRadius::same(10),
        brand::CARD,
        Stroke::new(1.0, brand::CARD_LINE),
        StrokeKind::Inside,
    );
    let summary = strip.summary;
    let y = card.center().y;
    let icon = match summary.state {
        State::Open => (Icon::Opened, brand::TEAL),
        State::Merged => (Icon::Merged, brand::TEAL),
        State::Closed => (Icon::Closed, brand::MUTED),
    };
    paint_icon(&painter, pos2(card.left() + 22.0, y), icon.0, icon.1);
    let number =
        painter.layout_no_wrap(format!("#{}", summary.number), mono(TEXT_SECONDARY), brand::TEXT);
    let number_w = number.size().x;
    painter.galley(pos2(card.left() + 38.0, y - number.size().y / 2.0), number, brand::TEXT);
    let mut action = None;

    let mut right = card.right() - 14.0;
    let link =
        painter.layout_no_wrap("Open on GitHub".to_owned(), body(TEXT_SECONDARY), brand::TEAL_TEXT);
    right -= link.size().x;
    let link_rect = Rect::from_min_size(pos2(right, y - 11.0), vec2(link.size().x, 22.0));
    let link_response = ui.interact(link_rect, ui.id().with("pr-strip-open"), Sense::click());
    let link_ink = if link_response.hovered() { brand::TEXT } else { brand::TEAL_TEXT };
    painter.galley(pos2(right, y - link.size().y / 2.0), link, link_ink);
    if link_response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        action = Some(Action::Open(summary.url.clone()));
    }
    right -= GAP + 4.0;

    let chip = mode_chip(ui, &painter, strip.policy.mode, right, y);
    right = chip.rect.left() - GAP;
    if strip.dev_open_popover {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&chip));
    }
    if let Some(policy) = popover(&chip, summary.number, strip.policy) {
        action = Some(Action::Policy(policy));
    }

    let title_left = card.left() + 38.0 + number_w + 10.0;
    for (drawn, pill) in pills(summary).into_iter().enumerate() {
        let ink = tone_ink(pill.tone);
        let width = tag_width(&painter, &pill.text);
        if right - width < title_left + 90.0 && drawn > 0 {
            break;
        }
        right -= width;
        let fill = if pill.outlined {
            TagFill::Line(ink.gamma_multiply(0.6))
        } else {
            TagFill::Tint(ink.gamma_multiply(0.12))
        };
        paint_tag(&painter, (right, y), &pill.text, ink, fill);
        right -= GAP;
    }
    let room = (right - title_left - 4.0).min(TITLE_MAX_W);
    if room > 30.0 {
        let title = ellipsized(&painter, &summary.title, body(TEXT_BODY), brand::SOFT, room);
        painter.galley(pos2(title_left, y - title.size().y / 2.0), title, brand::SOFT);
    }
    action
}

fn mode_chip(ui: &mut Ui, painter: &egui::Painter, mode: PrMode, right: f32, y: f32) -> Response {
    let label = mode_label(mode);
    let galley = painter.layout_no_wrap(label.to_owned(), body(Size::Compact.font()), brand::SOFT);
    let width = galley.size().x + 2.0 * Size::Compact.pad() + CHEVRON_ROOM;
    let rect = Rect::from_min_size(
        pos2(right - width, y - Size::Compact.height() / 2.0),
        vec2(width, Size::Compact.height()),
    );
    let response = ui.interact(rect, ui.id().with("pr-strip-mode"), Sense::click());
    let line = if response.hovered() { brand::TEAL.gamma_multiply(0.5) } else { brand::FIELD_LINE };
    painter.rect(
        rect,
        CornerRadius::same(Size::Compact.radius()),
        Color32::TRANSPARENT,
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let ink = if response.hovered() { brand::TEXT } else { brand::SOFT };
    painter.galley(pos2(rect.left() + Size::Compact.pad(), y - galley.size().y / 2.0), galley, ink);
    let chevron = pos2(rect.right() - Size::Compact.pad() - 3.0, y);
    let stroke = Stroke::new(1.3, ink);
    painter.line_segment([chevron + vec2(-3.0, -1.5), chevron + vec2(0.0, 1.5)], stroke);
    painter.line_segment([chevron + vec2(0.0, 1.5), chevron + vec2(3.0, -1.5)], stroke);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("What Tidewatch does when GitHub changes")
}

fn popover(chip: &Response, number: u64, policy: GithubPrefs) -> Option<GithubPrefs> {
    let mut changed = None;
    egui::Popup::menu(chip).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(
        |ui| {
            ui.set_width(POPOVER_W);
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("PR #{number}")).size(TEXT_BODY).color(brand::TEXT),
                );
                ui.label(
                    egui::RichText::new("what Tidewatch does when GitHub changes")
                        .size(TEXT_SECONDARY)
                        .color(brand::FAINT),
                );
            });
            let labels: Vec<&str> = MODES.iter().map(|(_, label)| *label).collect();
            let active = MODES.iter().position(|(mode, _)| *mode == policy.mode).unwrap_or(0);
            if let Some(index) = segmented::inline(ui, Id::new("pr-mode"), &labels, active) {
                changed = Some(GithubPrefs { mode: MODES[index].0, ..policy });
            }
            ui.label(
                egui::RichText::new(mode_help(policy.mode)).size(TEXT_SECONDARY).color(brand::SOFT),
            );
            ui.label(
                egui::RichText::new("AUTO MAY")
                    .size(TEXT_TAG)
                    .extra_letter_spacing(0.8)
                    .color(brand::FAINT),
            );
            for job in JOBS {
                let on = policy.allows(job.which);
                if tick(ui, job.label, job.caption, on) {
                    changed = Some(policy.with(job.which, !on));
                }
            }
            ui.label(
                egui::RichText::new("Default for new tabs: Settings, General, GitHub")
                    .size(TEXT_SECONDARY)
                    .color(brand::FAINT),
            );
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
                ui.close();
            }
        },
    );
    changed
}

fn tick(ui: &mut Ui, label: &str, caption: Option<&str>, on: bool) -> bool {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let clicked = tick_row(ui, label, on);
        if let Some(text) = caption {
            egui::Frame::new().inner_margin(egui::Margin { left: 26, ..egui::Margin::ZERO }).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new(text).size(TEXT_SECONDARY).color(brand::MUTED));
                },
            );
        }
        clicked
    })
    .inner
}

fn tick_row(ui: &mut Ui, label: &str, on: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let boxed = Rect::from_min_size(pos2(rect.left(), rect.center().y - 8.0), vec2(16.0, 16.0));
    let painter = ui.painter();
    if on {
        painter.rect_filled(boxed, CornerRadius::same(4), brand::TEAL);
        let stroke = Stroke::new(1.8, brand::KEY_EDGE);
        painter.line_segment(
            [boxed.center() + vec2(-4.0, 0.5), boxed.center() + vec2(-1.2, 3.3)],
            stroke,
        );
        painter.line_segment(
            [boxed.center() + vec2(-1.2, 3.3), boxed.center() + vec2(4.2, -3.0)],
            stroke,
        );
    } else {
        painter.rect_stroke(
            boxed,
            CornerRadius::same(4),
            Stroke::new(1.0, brand::FIELD_LINE),
            StrokeKind::Inside,
        );
    }
    painter.text(
        pos2(boxed.right() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        body(TEXT_BODY),
        brand::TEXT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

#[cfg(test)]
mod tests;
