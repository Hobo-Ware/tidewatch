//! The History pane: a header with the branch, how far it is ahead of the default branch and the
//! scope switch, then Kelp's commit graph for the tab. Side by side it is a raised card.

use std::collections::HashMap;

use egui::text::LayoutJob;
use egui::{
    Align, CornerRadius, Layout, Rect, RichText, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use kelp_core::history::History;
use kelp_ui::avatars::AvatarStore;
use kelp_ui::graph::{
    GraphAction, GraphColumns, GraphInput, MenuFor, Preview, ROW_H, Selection, Wip,
};
use kelp_ui::{graph::Marks, theme};
use stdusk_egui::widgets::{chip, menu_item, style_menu};
use tidewatch_core::history::{BaseCompare, Scope};

use crate::brand;
use crate::commit_files::{self, Geometry};
use crate::screen_kit::{body, mono};
use crate::tab_view::TabView;
use crate::{layout, segmented};

const HEADER_H: f32 = 34.0;
const SIDE_PAD: f32 = 14.0;
const MAIN_TITLE: &str = "main checkout";
const CARD_PAD: f32 = 18.0;
const CARD_RADIUS: u8 = 12;
const BODY_PAD_X: f32 = 8.0;
const BODY_PAD_Y: f32 = 6.0;
const PILL_GAP: f32 = 10.0;

pub(crate) fn scope_labels(is_main: bool) -> [&'static str; 2] {
    if is_main { ["This branch", "Whole repo"] } else { ["This worktree", "Whole repo"] }
}

pub(crate) const SCOPES: [&str; 2] = ["This branch", "Whole repo"];

/// `side` is the layout where the pane is a card, a click on a commit opens its files under the
/// row and a file's diff takes the terminal's place.
pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    avatars: &mut AvatarStore,
    side: bool,
) -> Option<GraphAction> {
    let body = if side {
        paint_card(ui, rect);
        let header =
            Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + layout::CHROME_H));
        show_card_header(ui, header, view);
        Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max)
            .shrink2(vec2(BODY_PAD_X, BODY_PAD_Y))
    } else {
        ui.painter().rect_filled(rect, 0.0, theme::bg());
        let header = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + HEADER_H));
        show_header(ui, header, view);
        Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max)
    };
    let mut action = None;
    let mut geometry = None;
    ui.scope_builder(UiBuilder::new().id_salt("history-graph").max_rect(body), |ui| {
        ui.set_clip_rect(body.intersect(ui.clip_rect()));
        action = show_graph(ui, view, avatars);
        if side && view.expanded() {
            geometry = row_geometry(ui, view);
        }
    });
    if side && let Some(geometry) = geometry {
        commit_files::show(ui, view, geometry, body);
    }
    action
}

/// The raised surface of the side layout.
pub(crate) fn paint_card(ui: &Ui, rect: Rect) {
    ui.painter().rect(
        rect,
        CornerRadius::same(CARD_RADIUS),
        brand::CARD,
        Stroke::new(1.0, brand::CARD_LINE),
        StrokeKind::Inside,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PillKind {
    Ahead,
    Behind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pill {
    text: String,
    kind: PillKind,
}

/// The pills of the card header: how far ahead of the base, and how far behind.
fn pills(compare: Option<&BaseCompare>) -> Vec<Pill> {
    let Some(compare) = compare else { return Vec::new() };
    let mut pills = Vec::new();
    if compare.ahead > 0 {
        let text = format!("{} ahead of {}", compare.ahead, compare.base);
        pills.push(Pill { text, kind: PillKind::Ahead });
    }
    if compare.behind > 0 {
        let text = if compare.ahead > 0 {
            format!("{} behind", compare.behind)
        } else {
            format!("{} behind {}", compare.behind, compare.base)
        };
        pills.push(Pill { text, kind: PillKind::Behind });
    }
    pills
}

fn branch_title(branch: Option<&str>) -> &str {
    branch.unwrap_or("detached HEAD")
}

fn show_card_header(ui: &mut Ui, rect: Rect, view: &mut TabView) {
    let painter = ui.painter().clone();
    painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, brand::CARD_LINE));
    let center_y = rect.center().y;
    let showing_whole = view.scope == Scope::Whole;
    let id = ui.id().with("history-scope");
    let active = usize::from(showing_whole);
    let switch = segmented::show(ui, id, rect.right() - 10.0, center_y, &SCOPES, active);
    if let Some(index) = switch.clicked {
        view.set_scope(if index == 1 { Scope::Whole } else { Scope::Current });
    }
    let loaded = view.loaded.as_ref();
    let pills = pills(loaded.and_then(|l| l.base.as_ref()));
    let limit = switch.left - PILL_GAP;
    let mut x = rect.left() + CARD_PAD;
    let title = branch_title(loaded.and_then(|l| l.branch.as_deref()));
    let mut job = LayoutJob::simple_singleline(title.to_owned(), mono(14.0), brand::TEXT);
    job.wrap.max_width = (limit - x).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    let width = galley.size().x;
    painter.galley(pos2(x, center_y - galley.size().y / 2.0), galley, brand::TEXT);
    x += width + PILL_GAP;
    for pill in pills {
        let (ink, fill) = match pill.kind {
            PillKind::Ahead => (brand::GREEN, brand::GREEN.gamma_multiply(0.12)),
            PillKind::Behind => (brand::ORANGE, brand::ORANGE.gamma_multiply(0.12)),
        };
        let galley = painter.layout_no_wrap(pill.text, body(12.0), ink);
        let size = galley.size() + vec2(14.0, 4.0);
        if x + size.x > limit {
            break;
        }
        let pill_rect = Rect::from_min_size(pos2(x, center_y - size.y / 2.0), size);
        painter.rect_filled(pill_rect, CornerRadius::same(255), fill);
        painter.galley(pill_rect.center() - galley.size() / 2.0, galley, ink);
        x = pill_rect.right() + 8.0;
    }
}

/// Where the selected row is on screen. The graph keeps its scroll offset in egui's memory and
/// draws from the row that offset starts in, so the row's place follows from both.
fn row_geometry(ui: &Ui, view: &TabView) -> Option<Geometry> {
    let display = match view.selection()? {
        Selection::Wip => 0,
        Selection::Commit(row) => row + usize::from(!view.changes().is_empty()),
    };
    let scroll_id = ui.make_persistent_id(egui::IdSalt::new("scroll_area"));
    let offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)?.offset.y;
    let rows = ui.ctx().read_response(view.graph.focus_id?)?.rect;
    let first = (offset / ROW_H).floor();
    let viewport_top = rows.top() + offset - first * ROW_H;
    Some(Geometry {
        row_top: rows.top() + (display as f32 - first) * ROW_H,
        row_h: ROW_H,
        viewport: Rect::from_min_max(
            pos2(rows.left(), viewport_top),
            pos2(rows.right(), ui.max_rect().bottom()),
        ),
    })
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

    fn compare(ahead: usize, behind: usize) -> BaseCompare {
        BaseCompare { base: "main".into(), ahead, behind }
    }

    #[test]
    fn the_card_header_pills_say_how_far_ahead_and_behind_the_base() {
        let pill = |text: &str, kind| Pill { text: text.to_owned(), kind };
        let cases = [
            (None, vec![]),
            (Some(compare(0, 0)), vec![]),
            (Some(compare(2, 0)), vec![pill("2 ahead of main", PillKind::Ahead)]),
            (Some(compare(0, 3)), vec![pill("3 behind main", PillKind::Behind)]),
            (
                Some(compare(2, 1)),
                vec![pill("2 ahead of main", PillKind::Ahead), pill("1 behind", PillKind::Behind)],
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(pills(input.as_ref()), expected, "{input:?}");
        }
    }

    #[test]
    fn a_detached_head_has_no_branch_name_to_show() {
        assert_eq!(branch_title(Some("agent/sunny-reef")), "agent/sunny-reef");
        assert_eq!(branch_title(None), "detached HEAD");
    }
}
