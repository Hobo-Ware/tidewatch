//! The new-tab form's fields: the worktree name, the branch it starts from, and the message box.

use egui::{
    Align2, Id, Margin, Popup, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui,
    UiBuilder, pos2, vec2,
};

use crate::brand;
use crate::composer::{self, Draft, Mode, Spec};
use crate::new_tab::{NameCheck, NewTab};
use crate::new_tab_layout::{FIELD_H, LABEL_GAP, LABEL_H, PROBLEM_H};
use crate::reader_view::{ellipsized, field_frame_on};
use crate::repo_base::RepoFacts;
use crate::screen_kit::TEXT_TAG;
use crate::screen_kit::{Size, body, field_label, field_label_width, mono, quiet_button};

const PREFIX: &str = "agent/";
const NOTE_GAP: f32 = 12.0;
const TEXT_ID: &str = "new-tab-text";
const BRANCHES_MAX_H: f32 = 260.0;
/// The name and branch fields are egui frames, which paint their 1 px outline outside their slot; the
/// box reaches that far on the sides and ends that much short at the bottom to line up with them.
const OUTLINE: f32 = 1.0;

fn text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), body(size), brand::MUTED).size().x
}

pub(crate) fn text_id() -> Id {
    Id::new(TEXT_ID)
}

/// What the name field did this frame.
pub(crate) enum NameEdit {
    Untouched,
    Typed(String),
    UseSuggestion,
}

fn problem_line(ui: &mut Ui, name: &str, check: &NameCheck) -> bool {
    let mut accept = false;
    ui.horizontal(|ui| {
        ui.set_min_height(PROBLEM_H - 4.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        match check {
            NameCheck::Fine => {}
            NameCheck::Invalid(problem) => {
                ui.label(RichText::new(problem.message()).size(12.0).color(brand::RED));
            }
            NameCheck::Taken { suggestion } => {
                let text = format!("{PREFIX}{name} is taken.");
                ui.label(RichText::new(text).size(12.0).color(brand::RED));
                accept =
                    quiet_button(ui, &format!("Use {suggestion}"), true, Size::Compact).clicked();
            }
        }
    });
    accept
}

pub(crate) fn name_field(ui: &mut Ui, state: &NewTab, facts: Option<&RepoFacts>) -> NameEdit {
    let top = ui.cursor().top();
    field_label(ui, "Worktree name");
    let follows = state.name_follows_text().then_some("follows the message");
    if let Some(note) = follows {
        let label_w = field_label_width(ui.painter(), "Worktree name");
        let room = ui.max_rect().width() - label_w;
        if text_width(ui, note, 11.5) + NOTE_GAP <= room {
            let at = pos2(ui.max_rect().right(), top + LABEL_H / 2.0);
            ui.painter().text(at, Align2::RIGHT_CENTER, note, body(TEXT_TAG), brand::FAINT);
        }
    }
    ui.add_space(LABEL_GAP);
    let mut text = state.name(facts);
    let shown = text.clone();
    let response = field_frame_on(ui, brand::SEGMENT, true, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_height(FIELD_H - 12.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(PREFIX).font(mono(13.0)).color(brand::FAINT));
            ui.add(
                TextEdit::singleline(&mut text)
                    .id(Id::new("new-tab-name"))
                    .frame(egui::Frame::NONE)
                    .margin(Margin::ZERO)
                    .font(mono(13.0))
                    .text_color(brand::TEXT)
                    .desired_width(ui.available_width().max(40.0)),
            )
        })
        .inner
    });
    let check = state.name_check(facts);
    let accept = if check.is_fine() {
        false
    } else {
        ui.add_space(2.0);
        problem_line(ui, &shown, &check)
    };
    match (accept, response.changed()) {
        (true, _) => NameEdit::UseSuggestion,
        (false, true) => NameEdit::Typed(text),
        (false, false) => NameEdit::Untouched,
    }
}

fn paint_branch_icon(ui: &Ui, center: Pos2) {
    let painter = ui.painter();
    let stroke = Stroke::new(1.4, brand::MUTED);
    let (top, bottom, side) =
        (center + vec2(-3.5, -4.5), center + vec2(-3.5, 4.5), center + vec2(3.5, -2.5));
    for point in [top, bottom, side] {
        painter.circle_stroke(point, 1.9, stroke);
    }
    painter.line_segment([top + vec2(0.0, 1.9), bottom - vec2(0.0, 1.9)], stroke);
    painter.line_segment([side + vec2(0.0, 1.9), side + vec2(0.0, 3.5)], stroke);
    painter.line_segment([side + vec2(0.0, 3.5), bottom + vec2(1.9, -0.5)], stroke);
}

fn paint_chevron(ui: &Ui, center: Pos2) {
    let stroke = Stroke::new(1.5, brand::MUTED);
    let points = [center + vec2(-3.0, -1.5), center + vec2(0.0, 1.5), center + vec2(3.0, -1.5)];
    ui.painter().line_segment([points[0], points[1]], stroke);
    ui.painter().line_segment([points[1], points[2]], stroke);
}

/// The dropdown of local branches. Returns the one picked.
pub(crate) fn base_field(ui: &mut Ui, state: &NewTab, facts: Option<&RepoFacts>) -> Option<String> {
    field_label(ui, "Start from");
    ui.add_space(LABEL_GAP);
    let current = state.base(facts).unwrap_or_else(|| "HEAD".to_owned());
    let response = field_frame_on(ui, brand::SEGMENT, true, |ui| {
        let size = vec2(ui.available_width(), FIELD_H - 12.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        paint_branch_icon(ui, pos2(rect.left() + 7.0, rect.center().y));
        let room = rect.width() - 36.0;
        let label = ellipsized(ui.painter(), &current, mono(13.0), brand::TEXT, room);
        let at = pos2(rect.left() + 22.0, rect.center().y - label.size().y / 2.0);
        ui.painter().galley(at, label, brand::TEXT);
        paint_chevron(ui, pos2(rect.right() - 8.0, rect.center().y));
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    let options = NewTab::base_options(facts);
    let mut picked = None;
    Popup::menu(&response).width(response.rect.width()).show(|ui| {
        ui.set_min_width(response.rect.width());
        ScrollArea::vertical().max_height(BRANCHES_MAX_H).show(ui, |ui| {
            for branch in &options {
                let row = egui::Button::selectable(*branch == current, mono_text(branch))
                    .min_size(vec2(ui.available_width(), 0.0));
                if ui.add(row).clicked() {
                    picked = Some(branch.clone());
                    ui.close();
                }
            }
        });
    });
    picked
}

fn mono_text(text: &str) -> RichText {
    RichText::new(text).font(mono(13.0))
}

/// The box a first message or a task is typed into, `height` tall: where it went and the text when
/// it changed. It is the Reader's composer as a field: pictures and files become chips, `:` opens the
/// emoji list and a slash the commands, and Enter is a new line because Start is the form's.
pub(crate) fn text_box(
    ui: &mut Ui,
    state: &mut NewTab,
    height: f32,
    hint: &str,
) -> (Rect, Option<String>) {
    let mut text = state.text().to_owned();
    let width = ui.available_width();
    let origin = ui.cursor().min;
    let wanted = composer::field_height(
        ui,
        &text,
        state.pictures.len(),
        (width + 2.0 * OUTLINE, height - OUTLINE),
    );
    let drawn =
        Rect::from_min_size(origin - vec2(OUTLINE, 0.0), vec2(width + 2.0 * OUTLINE, wanted));
    let mut child = ui.new_child(UiBuilder::new().max_rect(drawn));
    let spec = Spec {
        id: text_id(),
        hint,
        enabled: true,
        commands: &state.commands,
        ghost: None,
        mode: Mode::Field { height: height - OUTLINE },
        stop: false,
        held: 0,
    };
    let draft = Draft {
        text: &mut text,
        pictures: &mut state.pictures,
        focus: &mut state.focus_message,
        recall: None,
    };
    composer::show(&mut child, draft, &spec);
    let rect = Rect::from_min_size(origin, vec2(width, wanted));
    ui.allocate_rect(rect, Sense::hover());
    (rect, (text != state.text()).then_some(text))
}
