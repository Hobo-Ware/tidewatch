//! The toast and its pill, and the dialogs drawn over the frame.

use std::time::Duration;

use egui::{Color32, Rect, Stroke, StrokeKind, pos2};

use super::{Closing, Tidewatch};
use crate::amphetamine_view;
use crate::brand;
use crate::close_dialog;
use crate::flow_glue;
use crate::layout;
use crate::notice;
use crate::screen_kit::body;
use crate::toast::Toast;
use crate::trust_view;

const TOAST_BOTTOM: f32 = 34.0;
const TOAST_PAD: egui::Vec2 = egui::vec2(14.0, 8.0);

fn draw_toast(ui: &egui::Ui, column: Rect, msg: &str, fade: f32) {
    let alpha = |color: Color32| color.gamma_multiply(fade);
    let wrap = (column.width() - 2.0 * (TOAST_PAD.x + 16.0)).max(120.0);
    let galley = ui.painter().layout(msg.to_owned(), body(13.0), brand::TEXT, wrap);
    let size = galley.size() + TOAST_PAD * 2.0;
    let center =
        pos2(column.center().x, column.bottom() - TOAST_BOTTOM - (size.y - 32.0).max(0.0) / 2.0);
    let rect = Rect::from_center_size(center, size);
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, alpha(brand::ITEM));
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, alpha(brand::ITEM_LINE)), StrokeKind::Inside);
    painter.galley_with_override_text_color(rect.min + TOAST_PAD, galley, alpha(brand::TEXT));
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

    pub(super) fn show_toast_pill(&mut self, ui: &egui::Ui) {
        let Some(toast) = &self.toast else { return };
        let now = ui.ctx().input(|i| i.time);
        if toast.expired(now) {
            self.toast = None;
            return;
        }
        let column = self.message_column(ui.max_rect());
        if toast.lines().is_empty() {
            draw_toast(ui, column, toast.text(), toast.fade(now));
        } else {
            amphetamine_view::toast(ui, column, toast.lines(), toast.fade(now));
        }
        ui.ctx().request_repaint_after(Duration::from_secs_f64(toast.next_frame_in(now)));
    }

    pub(super) fn show_dialogs(&mut self, ctx: &egui::Context) {
        let answer = match &self.closing {
            Some(Closing::Asking { worktree, safety, .. }) => {
                close_dialog::show(ctx, worktree, safety, self.close_enter)
            }
            _ => None,
        };
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
