//! The toast and its pill, and the dialogs drawn over the frame.

use std::time::Duration;

use stdusk_egui::ui::draw_toast;

use super::{Closing, Tidewatch};
use crate::close_dialog;
use crate::flow_glue;
use crate::new_tab_dialog;
use crate::notice;
use crate::toast::Toast;
use crate::trust_view;

impl Tidewatch {
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
        draw_toast(ui, toast.text(), toast.fade(now));
        ui.ctx().request_repaint_after(Duration::from_secs_f64(toast.next_frame_in(now)));
    }

    pub(super) fn show_dialogs(&mut self, ctx: &egui::Context, active_id: u64) {
        let answer = match &self.closing {
            Some(Closing::Asking { worktree, inspection, .. }) => {
                close_dialog::show(ctx, worktree, inspection)
            }
            _ => None,
        };
        if let Some(answer) = answer {
            self.answer_close(answer, ctx);
        }
        self.show_palette(ctx);
        self.leftovers.show(ctx, self.paths.as_ref(), self.trust.grant().as_ref());
        flow_glue::dialogs(ctx, &mut self.flow, active_id);
        if let Some(state) = &mut self.new_tab
            && let Some(outcome) =
                new_tab_dialog::show(ctx, state, &self.catalog, self.paths.as_ref())
        {
            self.answer_new_tab(outcome, ctx);
        }
        if self.trust.dialog_open()
            && let Some(event) = trust_view::dialog(ctx, &self.trust.page())
        {
            self.trust.send(event, ctx);
        }
        if let Some(text) = &self.notice
            && notice::show(ctx, text)
        {
            self.notice = None;
        }
    }
}
