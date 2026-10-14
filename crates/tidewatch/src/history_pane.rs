//! The History pane: a header with the branch, how far it is ahead of the default branch and the
//! This worktree / Whole repo switch, then Kelp's commit graph for the tab.

use std::collections::HashMap;

use egui::{Align, Layout, Rect, RichText, Ui, UiBuilder, pos2};
use kelp_core::history::History;
use kelp_ui::avatars::AvatarStore;
use kelp_ui::graph::{GraphAction, GraphColumns, GraphInput, MenuFor, Preview, Selection, Wip};
use kelp_ui::{graph::Marks, theme};
use stdusk_egui::widgets::{chip, menu_item, style_menu};
use tidewatch_core::history::{BaseCompare, Scope};

use crate::brand;
use crate::tab_view::TabView;

pub(crate) const HEADER_H: f32 = 34.0;
const SIDE_PAD: f32 = 14.0;
const MAIN_TITLE: &str = "main checkout";

pub(crate) fn scope_labels(is_main: bool) -> [&'static str; 2] {
    if is_main { ["This branch", "Whole repo"] } else { ["This worktree", "Whole repo"] }
}

pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    avatars: &mut AvatarStore,
) -> Option<GraphAction> {
    ui.painter().rect_filled(rect, 0.0, theme::bg());
    let header = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + HEADER_H));
    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    show_header(ui, header, view);
    let mut action = None;
    ui.scope_builder(UiBuilder::new().id_salt("history-graph").max_rect(body), |ui| {
        ui.set_clip_rect(body.intersect(ui.clip_rect()));
        action = show_graph(ui, view, avatars);
    });
    action
}

fn show_header(ui: &mut Ui, rect: Rect, view: &mut TabView) {
    ui.painter().hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1.0, theme::border()));
    let title = if view.is_main { MAIN_TITLE } else { view.name.as_str() };
    let compare = view.loaded.as_ref().and_then(|l| l.base.as_ref()).map(BaseCompare::describe);
    let left = Rect::from_min_max(rect.min + egui::vec2(SIDE_PAD, 0.0), rect.max);
    ui.scope_builder(
        UiBuilder::new()
            .id_salt("history-title")
            .max_rect(left)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.label(
                RichText::new("HISTORY")
                    .size(11.0)
                    .family(theme::semibold())
                    .color(theme::text_faint()),
            );
            ui.label(RichText::new(title).size(12.5).color(brand::TEAL_TEXT));
            if let Some(compare) = compare {
                ui.label(RichText::new(compare).size(12.0).color(theme::text_muted()));
            }
        },
    );
    let right = Rect::from_min_max(rect.min, rect.max - egui::vec2(SIDE_PAD - 4.0, 0.0));
    ui.scope_builder(
        UiBuilder::new()
            .id_salt("history-scope")
            .max_rect(right)
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            let [current, whole] = scope_labels(view.is_main);
            let showing_whole = view.scope == Scope::Whole;
            if chip(ui, whole, showing_whole).clicked() {
                view.set_scope(Scope::Whole);
            }
            if chip(ui, current, !showing_whole).clicked() {
                view.set_scope(Scope::Current);
            }
        },
    );
}

fn show_graph(ui: &mut Ui, view: &mut TabView, avatars: &mut AvatarStore) -> Option<GraphAction> {
    let selected = view.selection();
    let Some(loaded) = &view.loaded else {
        message(ui, view.error.as_deref(), "Loading history...");
        return None;
    };
    let history = &loaded.history;
    let summaries = &loaded.summaries;
    let changes = view.wip.as_deref().unwrap_or_default();
    let wip =
        (!changes.is_empty()).then(|| Wip { head_row: loaded.head_row.unwrap_or(0), changes });
    let mut summary = |row: usize| summaries.get(history.id(row));
    let checks = HashMap::new();
    let input = GraphInput {
        history,
        summary: &mut summary,
        remote_owners: &[],
        preview: Preview::default(),
        selected,
        head_row: loaded.head_row,
        wip,
        lit: None,
        descriptions: false,
        other_wips: &[],
        columns: GraphColumns { date: true, ..GraphColumns::default() },
        compare: Marks::default(),
        picking: false,
        filter_active: false,
        checks: &checks,
    };
    view.graph.ui(ui, input, avatars, |ui, target| context_menu(ui, &target, history))
}

fn message(ui: &mut Ui, error: Option<&str>, idle: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| match error {
        Some(error) => ui.label(RichText::new(error).size(12.5).color(theme::deleted())),
        None => ui.label(RichText::new(idle).size(12.5).color(theme::text_faint())),
    });
}

fn context_menu(ui: &mut Ui, target: &MenuFor<'_>, history: &History) {
    style_menu(ui);
    match target {
        MenuFor::Commit(Selection::Commit(row), _) => {
            copy_item(ui, "Copy commit hash", history.id(*row).to_string());
        }
        MenuFor::Ref(label) => copy_item(ui, "Copy branch name", label.name.clone()),
        MenuFor::Commit(Selection::Wip, _) | MenuFor::Drop(_) | MenuFor::Worktree(_) => {}
    }
}

fn copy_item(ui: &mut Ui, label: &str, text: String) {
    if menu_item(ui, label, "").clicked() {
        ui.ctx().copy_text(text);
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_switch_names_the_main_checkout_a_branch_and_agents_a_worktree() {
        assert_eq!(scope_labels(true), ["This branch", "Whole repo"]);
        assert_eq!(scope_labels(false), ["This worktree", "Whole repo"]);
    }
}
