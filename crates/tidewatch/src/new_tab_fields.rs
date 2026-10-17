//! The new-tab form's fields: the worktree name, the branch it starts from, and the message box.

use egui::{
    Align2, Id, Margin, Popup, Pos2, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, pos2, vec2,
};

use crate::brand;
use crate::new_tab::{Kind, NameCheck, NewTab};
use crate::reader_view::{ellipsized, field_frame_on};
use crate::repo_base::RepoFacts;
use crate::screen_kit::{body, mono, quiet_button};

pub(crate) const LABEL_H: f32 = 14.0;
pub(crate) const LABEL_GAP: f32 = 6.0;
pub(crate) const FIELD_H: f32 = 32.0;
pub(crate) const PROBLEM_H: f32 = 24.0;
const PREFIX: &str = "agent/";
const TEXT_ID: &str = "new-tab-text";
const BRANCHES_MAX_H: f32 = 260.0;

pub(crate) fn text_id() -> Id {
    Id::new(TEXT_ID)
}

pub(crate) fn section_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(11.5).color(brand::MUTED));
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
                accept = quiet_button(ui, &format!("Use {suggestion}"), true).clicked();
            }
        }
    });
    accept
}

pub(crate) fn name_field(ui: &mut Ui, state: &NewTab, facts: Option<&RepoFacts>) -> NameEdit {
    let top = ui.cursor().top();
    section_label(ui, "Worktree name");
    let follows = state.name_follows_text().then(|| match state.kind() {
        Kind::Workflow => "follows the task",
        Kind::Agent | Kind::Shell => "follows the message",
    });
    if let Some(note) = follows {
        let at = pos2(ui.max_rect().right(), top + LABEL_H / 2.0);
        ui.painter().text(at, Align2::RIGHT_CENTER, note, body(11.5), brand::FAINT);
    }
    ui.add_space(LABEL_GAP);
    let mut text = state.name(facts);
    let shown = text.clone();
    let response = field_frame_on(ui, brand::SEGMENT, true, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_height(FIELD_H - 10.0);
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
    section_label(ui, "Start from");
    ui.add_space(LABEL_GAP);
    let current = state.base(facts).unwrap_or_else(|| "HEAD".to_owned());
    let response = field_frame_on(ui, brand::SEGMENT, true, |ui| {
        let size = vec2(ui.available_width(), FIELD_H - 10.0);
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

/// The box a first message or a task is typed into, `height` tall. Returns the text when it
/// changed.
pub(crate) fn text_box(ui: &mut Ui, state: &mut NewTab, height: f32, hint: &str) -> Option<String> {
    let mut text = state.text().to_owned();
    let inner = (height - 12.0).max(40.0);
    let width = ui.available_width();
    let response = field_frame_on(ui, brand::SEGMENT, true, |ui| {
        ui.set_min_height(inner);
        ScrollArea::vertical()
            .id_salt("new-tab-text-scroll")
            .max_height(inner)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    TextEdit::multiline(&mut text)
                        .id(text_id())
                        .frame(egui::Frame::NONE)
                        .margin(Margin::symmetric(4, 4))
                        .desired_width(width - 28.0)
                        .desired_rows(((inner - 8.0) / 18.0).floor().max(2.0) as usize)
                        .font(body(14.0))
                        .text_color(brand::TEXT)
                        .hint_text(RichText::new(hint).color(brand::FAINT)),
                )
            })
            .inner
    });
    if std::mem::take(&mut state.focus_message) {
        response.request_focus();
    }
    (text != state.text()).then_some(text)
}
