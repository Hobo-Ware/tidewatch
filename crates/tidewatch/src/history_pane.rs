//! The History pane: a header with the branch, how far it is ahead of the default branch and the
//! scope switch, then Kelp's commit graph for the tab. Side by side it is a raised card.

use std::collections::HashMap;

use std::f32::consts::FRAC_PI_2;

use egui::text::LayoutJob;
use egui::{CornerRadius, Mesh, Rect, RichText, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use kelp_core::history::History;
use kelp_ui::avatars::AvatarStore;
use kelp_ui::graph::Marks;
use kelp_ui::graph::{
    GraphAction, GraphColumns, GraphInput, GraphStyle, MenuFor, Preview, Selection, Wip,
};
use stdusk_egui::widgets::{menu_item, style_menu};
use tidewatch_core::history::{BaseCompare, Scope};

use crate::brand;
use crate::commit_files::{self, Spot};
use crate::progress_view::{self, Side, TabState};
use crate::screen_kit::{body, mono};
use crate::tab_view::TabView;
use crate::waiting_notes::Waiting;
use crate::{layout, segmented};

const CARD_PAD: f32 = 18.0;
pub(crate) const CARD_RADIUS: u8 = 12;
const BODY_PAD_X: f32 = 8.0;
const BODY_PAD_Y: f32 = 6.0;
const PILL_GAP: f32 = 10.0;
const SWITCH_GAP: f32 = 8.0;
const CORNER_STEPS: u32 = 8;

pub(crate) const SCOPES: [&str; 2] = ["This branch", "Whole repo"];

/// A tab's ledger for the card: the panel's input and the tab's memory of its two choices.
pub(crate) struct Progress<'a> {
    pub(crate) input: progress_view::Input<'a>,
    pub(crate) state: &'a mut TabState,
}

#[derive(Default)]
pub(crate) struct Out {
    pub(crate) graph: Option<GraphAction>,
    pub(crate) progress: Option<progress_view::Action>,
}

/// The pane is a card in both layouts. `side` is the layout where a click on a commit opens its
/// files under the row and a file's diff takes the terminal's place. `agent` is the id the tab's
/// agent signs replies with, `""` without one. With `progress` the header offers Progress |
/// History and the card shows the side the tab chose.
pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    avatars: &mut AvatarStore,
    side: bool,
    (agent, progress): (&str, Option<Progress<'_>>),
) -> Out {
    paint_card(ui, rect);
    let header = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + layout::CHROME_H));
    let (input, mut state) = match progress {
        Some(Progress { input, state }) => (Some(input), Some(state)),
        None => (None, None),
    };
    let shown = state.as_deref().map(|state| progress_view::side_shown(state.chosen, true));
    if let Some(chose) = show_card_header(ui, header, view, shown)
        && let Some(state) = state.as_deref_mut()
    {
        state.chosen = Some(chose);
    }
    let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max)
        .shrink2(vec2(BODY_PAD_X, BODY_PAD_Y));
    if let (Some(Side::Progress), Some(input), Some(state)) = (shown, input.as_ref(), state) {
        let panel = Rect::from_min_max(pos2(rect.left(), header.bottom() + 1.0), rect.max);
        return Out { graph: None, progress: progress_view::show(ui, panel, input, state) };
    }
    let waiting = Waiting::of(&view.notes, agent);
    let mut action = None;
    ui.scope_builder(UiBuilder::new().id_salt("history-graph").max_rect(body), |ui| {
        ui.set_clip_rect(body.intersect(ui.clip_rect()));
        action = show_graph(ui, view, avatars, side, &waiting);
        if side && let Some(spot) = card_spot(view) {
            commit_files::show(ui, view, spot, &waiting);
        }
    });
    Out { graph: action, progress: None }
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

/// Gives the card's bottom corners back to the ground where the square diff body overlapped them.
pub(crate) fn round_bottom(ui: &Ui, card: Rect) {
    let radius = f32::from(CARD_RADIUS);
    let painter = ui.painter();
    for left_side in [true, false] {
        let sign = if left_side { 1.0 } else { -1.0 };
        let corner = pos2(if left_side { card.left() } else { card.right() }, card.bottom());
        let center = pos2(corner.x + sign * radius, corner.y - radius);
        let mut mesh = Mesh::default();
        mesh.colored_vertex(corner, brand::GROUND);
        for step in 0..=CORNER_STEPS {
            let angle = FRAC_PI_2 * step as f32 / CORNER_STEPS as f32;
            let at = center + vec2(-sign * radius * angle.cos(), radius * angle.sin());
            mesh.colored_vertex(at, brand::GROUND);
        }
        for step in 0..CORNER_STEPS {
            mesh.add_triangle(0, step + 1, step + 2);
        }
        painter.add(mesh);
    }
    painter.rect_stroke(
        card,
        CornerRadius::same(CARD_RADIUS),
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

/// Draws the header. Returns the side the user picked on the Progress | History control, which
/// shows (`shown`) only on a tab with a ledger.
fn show_card_header(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    shown: Option<Side>,
) -> Option<Side> {
    let painter = ui.painter().clone();
    painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, brand::CARD_LINE));
    let center_y = rect.center().y;
    let mut right = rect.right() - 10.0;
    let mut chose = None;
    if let Some(side) = shown {
        let id = ui.id().with("history-side");
        let control = segmented::show(ui, id, right, center_y, &progress_view::SIDES, side.index());
        chose = control.clicked.map(Side::at);
        right = control.left - SWITCH_GAP;
    }
    if shown == Some(Side::Progress) {
        return chose;
    }
    let showing_whole = view.scope == Scope::Whole;
    let id = ui.id().with("history-scope");
    let active = usize::from(showing_whole);
    let switch = segmented::show(ui, id, right, center_y, &SCOPES, active);
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
    chose
}

fn side_style() -> GraphStyle {
    GraphStyle::embedded(brand::TEAL)
}

/// The row the open commit's card hangs under and the gap it asks the graph for, `None` when no
/// commit is open.
fn card_gap(view: &TabView, style: &GraphStyle, available: f32) -> Option<(usize, f32)> {
    if !view.expanded() {
        return None;
    }
    let Some(Selection::Commit(row)) = view.selection() else { return None };
    let files = if view.is_loading_source() { 0 } else { view.files.len() };
    Some((row, commit_files::gap_height(files, available - style.row_h)))
}

/// Where the graph put the gap of the open commit in the frame it just drew.
fn card_spot(view: &TabView) -> Option<Spot> {
    let style = side_style();
    let loaded = view.loaded.as_ref()?;
    let pinned_wip = !view.changes().is_empty() && loaded.head_row.is_some_and(|row| row > 0);
    Some(Spot {
        gap: view.graph.expanded_rect?,
        row_h: style.row_h,
        inset: commit_files::message_inset(&style, loaded.history.layout.lane_count(), pinned_wip),
        viewport: view.graph.viewport_rect,
    })
}

fn show_graph(
    ui: &mut Ui,
    view: &mut TabView,
    avatars: &mut AvatarStore,
    side: bool,
    waiting: &Waiting,
) -> Option<GraphAction> {
    let selected = view.selection();
    let style = side_style();
    let expanded = if side { card_gap(view, &style, ui.available_height()) } else { None };
    if view.loaded.is_some() && (expanded.is_none() || !view.is_loading_source()) {
        view.graph.scroll_to = view.take_room().or(view.graph.scroll_to);
    }
    let Some(loaded) = &view.loaded else {
        message(ui, view.error.as_deref(), "Loading history...");
        return None;
    };
    let history = &loaded.history;
    let summaries = &loaded.summaries;
    let changes = view.wip.as_deref().unwrap_or_default();
    let wip = (!changes.is_empty()).then(|| Wip {
        head_row: loaded.head_row.unwrap_or(0),
        changes,
        notes: waiting.uncommitted(),
    });
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
        notes: waiting.commits(),
        style,
        expanded,
    };
    view.graph.ui(ui, input, avatars, |ui, target| context_menu(ui, &target, history))
}

fn message(ui: &mut Ui, error: Option<&str>, idle: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| match error {
        Some(error) => ui.label(RichText::new(error).size(12.5).color(brand::RED)),
        None => ui.label(RichText::new(idle).size(12.5).color(brand::FAINT)),
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

    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Shape};
    use tidewatch_core::progress::Ledger;

    use crate::progress_state::TabLedger;

    const LEDGER: &str = "# Goal\nShip it\n\n## Status\nstate: working\n\n## Checklist\n### 1. Build\n- [~] Write it\n\n## Needs you\n- Pick one\n";

    struct Card {
        ctx: egui::Context,
        view: TabView,
        avatars: AvatarStore,
        ledger: TabLedger,
        state: TabState,
        with_ledger: bool,
        texts: Vec<(String, egui::Rect)>,
        out: Out,
    }

    fn collect(shape: &Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            Shape::Text(text) => out.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
            _ => {}
        }
    }

    impl Card {
        fn new(with_ledger: bool) -> Self {
            let ctx = crate::fonts::test_context();
            let mut card = Self {
                avatars: AvatarStore::offline(ctx.clone()),
                ctx,
                view: TabView::new(1, "/work/web".into(), false, None),
                ledger: TabLedger { ledger: Ledger::parse(LEDGER), updated_ms: None },
                state: TabState::default(),
                with_ledger,
                texts: Vec::new(),
                out: Out::default(),
            };
            card.frame(Vec::new());
            card.frame(Vec::new());
            card
        }

        fn frame(&mut self, events: Vec<Event>) {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 600.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let (view, avatars, ledger, state) =
                (&mut self.view, &mut self.avatars, &self.ledger, &mut self.state);
            let with_ledger = self.with_ledger;
            let mut out = Out::default();
            let output = self.ctx.run_ui(raw, |ui| {
                let progress = with_ledger.then_some(Progress {
                    input: progress_view::Input { ledger, now_ms: 0, time: 0.0, animate: false },
                    state: &mut *state,
                });
                let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 600.0));
                out = show(ui, rect, view, avatars, true, ("", progress));
            });
            self.out = out;
            self.texts.clear();
            for clipped in &output.shapes {
                collect(&clipped.shape, &mut self.texts);
            }
        }

        fn at(&self, text: &str) -> Option<Pos2> {
            self.texts.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| rect.center())
        }

        fn click(&mut self, at: Pos2) {
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(Vec::new());
        }
    }

    #[test]
    fn a_tab_with_a_ledger_opens_on_progress_and_the_switch_flips_to_history_and_back() {
        let mut card = Card::new(true);
        assert!(card.at("Progress").is_some() && card.at("History").is_some());
        assert!(card.at("This branch").is_none(), "Progress hides the scope switch");
        assert!(card.at("Answer").is_some());
        let history = card.at("History").unwrap();
        card.click(history);
        assert_eq!(card.state.chosen, Some(Side::History));
        assert!(card.at("This branch").is_some(), "History brings its own header back");
        assert!(card.at("Answer").is_none());
        let progress = card.at("Progress").unwrap();
        card.click(progress);
        assert_eq!(card.state.chosen, Some(Side::Progress));
        assert!(card.at("Answer").is_some());
    }

    #[test]
    fn a_tab_without_a_ledger_has_history_and_no_switch() {
        let card = Card::new(false);
        assert!(card.at("Progress").is_none());
        assert!(card.at("This branch").is_some());
    }

    #[test]
    fn answer_in_the_card_reaches_the_caller() {
        let mut card = Card::new(true);
        let answer = card.at("Answer").unwrap();
        let button = |pressed| Event::PointerButton {
            pos: answer,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        card.frame(vec![Event::PointerMoved(answer)]);
        card.frame(vec![button(true)]);
        card.frame(vec![button(false)]);
        assert_eq!(card.out.progress, Some(progress_view::Action::Answer));
    }
}
