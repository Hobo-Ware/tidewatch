//! The toast and its pill, and the dialogs drawn over the frame.

use std::time::Duration;

use egui::{Color32, Rect, Stroke, StrokeKind, pos2, vec2};

use super::Tidewatch;
use crate::agent_avatar::{self, Face};
use crate::amphetamine_view;
use crate::brand;
use crate::close_dialog;
use crate::flow_glue;
use crate::layout;
use crate::notice;
use crate::screen_kit::{FACE, body};
use crate::toast::Toast;
use crate::trust_view;

const TOAST_BOTTOM: f32 = 34.0;
const TOAST_PAD: egui::Vec2 = egui::vec2(14.0, 8.0);
const TOAST_FACE_GAP: f32 = 8.0;

pub(super) fn draw_toast(ui: &egui::Ui, column: Rect, msg: &str, fade: f32, face: Option<&Face>) {
    let alpha = |color: Color32| color.gamma_multiply(fade);
    let lead = face.map_or(0.0, |_| FACE + TOAST_FACE_GAP);
    let wrap = (column.width() - 2.0 * (TOAST_PAD.x + 16.0) - lead).max(120.0);
    let galley = ui.painter().layout(msg.to_owned(), body(13.0), brand::TEXT, wrap);
    let size = vec2(galley.size().x + lead, galley.size().y.max(FACE)) + TOAST_PAD * 2.0;
    let center =
        pos2(column.center().x, column.bottom() - TOAST_BOTTOM - (size.y - 32.0).max(0.0) / 2.0);
    let rect = Rect::from_center_size(center, size);
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, alpha(brand::ITEM));
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, alpha(brand::ITEM_LINE)), StrokeKind::Inside);
    if let Some(face) = face {
        let plate = Rect::from_center_size(
            pos2(rect.left() + TOAST_PAD.x + FACE / 2.0, rect.center().y),
            vec2(FACE, FACE),
        );
        let mut faded = painter.clone();
        faded.multiply_opacity(fade);
        agent_avatar::paint_face(ui.ctx(), &faded, plate, face);
    }
    let at = pos2(rect.left() + TOAST_PAD.x + lead, rect.center().y - galley.size().y / 2.0);
    painter.galley_with_override_text_color(at, galley, alpha(brand::TEXT));
}

impl Tidewatch {
    /// The part of `area` a message centers on: the left column while History shares the window.
    fn message_column(&self, area: Rect) -> Rect {
        let workspace = !self.overview && self.settings.is_none() && self.welcome.is_none();
        if workspace && self.side_layout() && self.panes_showing() {
            layout::split_side(area, self.side_fraction).diff
        } else {
            area
        }
    }

    pub(super) fn show_toast(&mut self, text: impl Into<String>, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.toast = Some(Toast::new(text, now));
        ctx.request_repaint();
    }

    pub(super) fn show_toast_about(
        &mut self,
        tab: u64,
        text: impl Into<String>,
        ctx: &egui::Context,
    ) {
        let now = ctx.input(|i| i.time);
        self.toast = Some(Toast::about(self.tab_face(tab), text, now));
        ctx.request_repaint();
    }

    pub(super) fn show_toast_pill(&mut self, ui: &egui::Ui) {
        let Some(toast) = &self.toast else { return };
        let now = ui.ctx().input(|i| i.time);
        if toast.expired(now) {
            self.toast = None;
            return;
        }
        let column = self.message_column(ui.max_rect());
        if toast.lines().is_empty() {
            draw_toast(ui, column, toast.text(), toast.fade(now), toast.face());
        } else {
            amphetamine_view::toast(ui, column, toast.lines(), toast.fade(now));
        }
        ui.ctx().request_repaint_after(Duration::from_secs_f64(toast.next_frame_in(now)));
    }

    pub(super) fn show_dialogs(&mut self, ctx: &egui::Context) {
        self.show_workflow_card(ctx, self.message_column(ctx.content_rect()));
        let answer = self
            .closing
            .as_ref()
            .and_then(|closing| close_dialog::show_closing(ctx, closing, self.close_enter));
        if let Some(answer) = answer {
            self.answer_close(answer, ctx);
        }
        self.updates.show_fix_dialog(ctx);
        self.show_palette(ctx);
        self.show_change_tab(ctx);
        self.show_open_link(ctx);
        self.show_service_dialog(ctx);
        self.show_paste_confirm(ctx);
        self.show_takeover_dialog(ctx);
        self.leftovers.show(ctx, self.paths.as_ref(), self.trust.grant().as_ref());
        flow_glue::dialogs(ctx, &mut self.flow);
        if self.trust.dialog_open()
            && let Some(event) = trust_view::dialog(ctx, &self.trust.page())
        {
            self.trust.send(event, ctx);
        }
        if let Some(notice) = self.amphetamine.notice()
            && amphetamine_view::notice(ctx, notice, self.message_column(ctx.content_rect()))
        {
            self.amphetamine.dismiss_notice();
        }
        if let Some(text) = &self.notice
            && notice::show(ctx, text, self.message_column(ctx.content_rect()))
        {
            self.notice = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted(face: Option<&Face>) -> Vec<(String, Rect)> {
        let ctx = crate::fonts::test_context();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0));
        let raw = egui::RawInput { screen_rect: Some(screen), ..egui::RawInput::default() };
        let output = ctx.run_ui(raw, |ui| draw_toast(ui, screen, "stuck on tests", 1.0, face));
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            crate::test_support::painted_text_rects(&clipped.shape, &mut texts);
        }
        texts
    }

    #[test]
    fn a_toast_about_a_tab_draws_its_face_left_of_the_text_and_a_plain_one_does_not() {
        let at = |texts: &[(String, Rect)], wanted: &str| {
            texts.iter().find(|(text, _)| text == wanted).map(|(_, rect)| *rect)
        };
        let plain = painted(None);
        assert_eq!(at(&plain, "Z"), None);
        let worn = painted(Some(&Face::of("Zed", "claude", None)));
        let letter = at(&worn, "Z").expect("the face letter is drawn");
        let text = at(&worn, "stuck on tests").unwrap();
        assert!(letter.right() < text.left(), "{letter:?} left of {text:?}");
        assert!(text.left() > at(&plain, "stuck on tests").unwrap().left());
    }
}
