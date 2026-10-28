//! The History pane: a header with the branch, how far it is ahead of the default branch and the
//! Progress | Changes | History switch, then the side the tab shows: the Changes list, or the
//! commit search over Kelp's commit graph. Side by side it is a raised card.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use std::f32::consts::FRAC_PI_2;

use egui::text::LayoutJob;
use egui::{
    CornerRadius, Mesh, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use kelp_core::history::History;
use kelp_ui::avatars::AvatarStore;
use kelp_ui::graph::Marks;
use kelp_ui::graph::{
    GraphAction, GraphColumns, GraphInput, GraphStyle, Highlight, MenuFor, Preview, Selection, Wip,
};
use stdusk_egui::widgets::{menu_item, style_menu};
use tidewatch_core::avatar::Avatar;
use tidewatch_core::folder_repos::Scan;
use tidewatch_core::history::BaseCompare;

use crate::agent_avatar;
use crate::brand;
use crate::commit_files::{self, Spot};
use crate::folder_view;
use crate::last_turn::LastTurn;
use crate::progress_view::{self, Side, TabState};
use crate::screen_kit::TEXT_SECONDARY;
use crate::screen_kit::mono;
use crate::screen_kit::{TagFill, paint_tag, tag_width};
use crate::search_bar;
use crate::selectable;
use crate::tab_view::TabView;
use crate::waiting_notes::Waiting;
use crate::{changes_view, layout, segmented};

pub(crate) const CARD_PAD: f32 = 18.0;
pub(crate) const CARD_RADIUS: u8 = 12;
const BODY_PAD_X: f32 = 8.0;
const BODY_PAD_Y: f32 = 6.0;
const PILL_GAP: f32 = 10.0;
const SWITCH_GAP: f32 = 8.0;
const DOT_R: f32 = 3.0;
const FACE: f32 = 24.0;
const FACE_GAP: f32 = 8.0;
const CORNER_STEPS: u32 = 8;
const MATCH_FILL: f32 = 0.25;

/// A tab's ledger for the card: the panel's input and the tab's memory of its two choices.
pub(crate) struct Face<'a> {
    pub(crate) tab: u64,
    pub(crate) avatar: &'a Arc<Avatar>,
}

pub(crate) struct Panel<'a> {
    pub(crate) agent: &'a str,
    pub(crate) progress: Option<progress_view::Input<'a>>,
    pub(crate) state: &'a mut TabState,
    pub(crate) face: Option<Face<'a>>,
    pub(crate) folder: Option<folder_view::Head<'a>>,
}

#[derive(Default)]
pub(crate) struct Out {
    pub(crate) graph: Option<GraphAction>,
    pub(crate) progress: Option<progress_view::Action>,
    pub(crate) repo: Option<PathBuf>,
}

/// The pane is a card in both layouts. `side` is the layout where a click on a commit opens its
/// files under the row and a file's diff takes the terminal's place. `agent` is the id the tab's
/// agent signs replies with, `""` without one. The header offers Progress (with a ledger) |
/// Changes | History and the card shows the side the tab chose.
pub(crate) fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TabView,
    avatars: &mut AvatarStore,
    side: bool,
    panel: Panel<'_>,
) -> Out {
    let Panel { agent, progress, state, face, folder } = panel;
    paint_card(ui, rect);
    let header = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + layout::CHROME_H));
    let first = progress.as_ref().map_or(Side::Progress, |input| input.first);
    let shown = progress_view::side_shown(state.chosen, progress.is_some(), first);
    state.shown = Some(shown);
    let attention =
        progress.as_ref().and_then(|input| input.last_turn.as_ref()).and_then(LastTurn::attention);
    let dot = shown != Side::Progress && state.unseen(attention);
    let loaded = view.loaded.as_ref();
    let info = Header {
        title: branch_title(loaded.and_then(|l| l.branch.as_deref())),
        badge: view.changes().len(),
        base: loaded.and_then(|l| l.base.as_ref()),
        picker: folder.as_ref(),
        sides: progress_view::sides(progress.is_some()),
    };
    let chosen = show_card_header(ui, header, &info, (shown, dot), face.as_ref());
    if let Some(chose) = chosen.side {
        state.chosen = Some(chose);
    }
    let repo = chosen.repo;
    let below = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    match (shown, progress.as_ref()) {
        (Side::Progress, Some(input)) => {
            let panel = Rect::from_min_max(pos2(rect.left(), header.bottom() + 1.0), rect.max);
            let progress = progress_view::show(ui, panel, input, state);
            return Out { graph: None, progress, repo };
        }
        (Side::Changes, _) => {
            changes_view::show(ui, below, view, side);
            return Out { repo, ..Out::default() };
        }
        _ => {}
    }
    let toolbar = Rect::from_min_size(below.min, vec2(below.width(), search_bar::HEIGHT));
    search_bar::show(ui, toolbar, view);
    let body = Rect::from_min_max(pos2(rect.left(), toolbar.bottom()), rect.max)
        .shrink2(vec2(BODY_PAD_X, BODY_PAD_Y));
    let waiting = Waiting::of(&view.notes, agent);
    let mut action = None;
    ui.scope_builder(UiBuilder::new().id_salt("history-graph").max_rect(body), |ui| {
        ui.set_clip_rect(body.intersect(ui.clip_rect()));
        action = show_graph(ui, view, avatars, side, &waiting);
        if side && let Some(spot) = card_spot(view) {
            commit_files::show(ui, view, spot, &waiting);
        }
    });
    Out { graph: action, progress: None, repo }
}

/// The panel of a folder tab that holds no git repo: the same card and header, with the folder's
/// name for a title and the folder view for Changes.
pub(crate) fn show_folder(
    ui: &mut Ui,
    rect: Rect,
    scan: &Scan,
    now_ms: i64,
    panel: Panel<'_>,
) -> Out {
    let Panel { progress, state, face, .. } = panel;
    paint_card(ui, rect);
    let header = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + layout::CHROME_H));
    let first = progress.as_ref().map_or(Side::Progress, |input| input.first);
    let shown = progress_view::folder_side_shown(state.chosen, progress.is_some(), first);
    state.shown = Some(shown);
    let attention =
        progress.as_ref().and_then(|input| input.last_turn.as_ref()).and_then(LastTurn::attention);
    let dot = shown != Side::Progress && state.unseen(attention);
    let title = folder_view::folder_name(&scan.folder);
    let info = Header {
        title: &title,
        badge: scan.recent.len(),
        base: None,
        picker: None,
        sides: progress_view::folder_sides(progress.is_some()),
    };
    if let Some(chose) = show_card_header(ui, header, &info, (shown, dot), face.as_ref()).side {
        state.chosen = Some(chose);
    }
    if let (Side::Progress, Some(input)) = (shown, progress.as_ref()) {
        let panel = Rect::from_min_max(pos2(rect.left(), header.bottom() + 1.0), rect.max);
        let progress = progress_view::show(ui, panel, input, state);
        return Out { progress, ..Out::default() };
    }
    let below = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
    folder_view::show(ui, below, scan, now_ms);
    Out::default()
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

struct Header<'a> {
    title: &'a str,
    badge: usize,
    base: Option<&'a BaseCompare>,
    picker: Option<&'a folder_view::Head<'a>>,
    sides: &'static [Side],
}

struct Chosen {
    side: Option<Side>,
    repo: Option<PathBuf>,
}

fn show_card_header(
    ui: &mut Ui,
    rect: Rect,
    info: &Header<'_>,
    (shown, dot): (Side, bool),
    face: Option<&Face<'_>>,
) -> Chosen {
    let painter = ui.painter().clone();
    painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, brand::CARD_LINE));
    let center_y = rect.center().y;
    let sides = info.sides;
    let labels: Vec<&str> = sides.iter().map(|side| side.label()).collect();
    let badges: Vec<Option<usize>> =
        sides.iter().map(|side| (*side == Side::Changes).then_some(info.badge)).collect();
    let id = ui.id().with("history-side");
    let active = sides.iter().position(|side| *side == shown).unwrap_or_default();
    let control =
        segmented::show_badged(ui, id, rect.right() - 10.0, center_y, (&labels, &badges), active);
    let side = control.clicked.and_then(|index| sides.get(index).copied());
    if dot && let Some(segment) = sides.iter().position(|side| *side == Side::Progress) {
        let at = control.segments[segment].right_top() + vec2(-6.0, 6.0);
        painter.circle_filled(at, DOT_R, brand::NEEDS);
    }
    let right = control.left - SWITCH_GAP;
    let mut x = rect.left() + CARD_PAD;
    if let Some(face) = face {
        let place = Rect::from_center_size(pos2(x + FACE / 2.0, center_y), vec2(FACE, FACE));
        if agent_avatar::paint(ui.ctx(), &painter, place, face.avatar) {
            agent_avatar::hover(ui, place, face.tab, face.avatar);
            x += FACE + FACE_GAP;
        }
        let click = ui.interact(place, ui.id().with(("history-face", face.tab)), Sense::click());
        agent_avatar::menu(&click, face.tab);
    }
    let mut repo = None;
    if let Some(head) = info.picker {
        let (next, chosen) = folder_view::picker(ui, (x, right), center_y, head);
        x = next;
        repo = chosen;
    }
    let mut job = LayoutJob::simple_singleline(info.title.to_owned(), mono(14.0), brand::TEXT);
    job.wrap.max_width = (right - x).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    let width = galley.size().x;
    selectable::galley(ui, pos2(x, center_y - galley.size().y / 2.0), galley, brand::TEXT);
    if shown == Side::Progress {
        return Chosen { side, repo };
    }
    x += width + PILL_GAP;
    for pill in pills(info.base) {
        let (ink, fill) = match pill.kind {
            PillKind::Ahead => (brand::GREEN, brand::GREEN.gamma_multiply(0.12)),
            PillKind::Behind => (brand::ORANGE, brand::ORANGE.gamma_multiply(0.12)),
        };
        if x + tag_width(&painter, &pill.text) > right {
            break;
        }
        let pill_rect = paint_tag(&painter, (x, center_y), &pill.text, ink, TagFill::Tint(fill));
        x = pill_rect.right() + 8.0;
    }
    Chosen { side, repo }
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
    let highlight = view.search.highlight().map(|needle| Highlight {
        needle: needle.to_owned(),
        fill: brand::TEAL.gamma_multiply(MATCH_FILL),
    });
    if view.graph.highlight != highlight {
        view.graph.highlight = highlight;
    }
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
        lit: view.search.lit(history.len()),
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
        Some(error) => ui.label(RichText::new(error).size(TEXT_SECONDARY).color(brand::RED)),
        None => ui.label(RichText::new(idle).size(TEXT_SECONDARY).color(brand::FAINT)),
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
        stale: bool,
        face: Option<Arc<Avatar>>,
        last_turn: Option<LastTurn>,
        shapes: Vec<egui::epaint::ClippedShape>,
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
                stale: false,
                face: None,
                last_turn: None,
                shapes: Vec::new(),
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
            let (with_ledger, stale) = (self.with_ledger, self.stale);
            let face = self.face.clone();
            let last_turn = self.last_turn.clone();
            let mut out = Out::default();
            let output = self.ctx.run_ui(raw, |ui| {
                let progress = with_ledger.then_some(progress_view::Input {
                    ledger: Some(ledger),
                    stale,
                    jobs: &[],
                    now_ms: 0,
                    time: 0.0,
                    animate: false,
                    first: Side::Progress,
                    meeseeks: None,
                    last_turn: last_turn.clone(),
                    shown: None,
                    links: None,
                });
                let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(420.0, 600.0));
                let face = face.as_ref().map(|avatar| Face { tab: 1, avatar });
                let panel = Panel { agent: "", progress, state: &mut *state, face, folder: None };
                out = show(ui, rect, view, avatars, true, panel);
            });
            self.out = out;
            self.shapes.clone_from(&output.shapes);
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
        assert!(card.at("Write it").is_some());
        let history = card.at("History").unwrap();
        card.click(history);
        assert_eq!(card.state.chosen, Some(Side::History));
        assert!(card.at("This branch").is_some(), "History brings its own header back");
        assert!(card.at("Write it").is_none());
        let progress = card.at("Progress").unwrap();
        card.click(progress);
        assert_eq!(card.state.chosen, Some(Side::Progress));
        assert!(card.at("Write it").is_some());
    }

    #[test]
    fn a_stale_ledger_says_the_progress_is_old_and_the_panel_never_asks() {
        let mut card = Card::new(true);
        card.ledger.updated_ms = Some(-480_000);
        card.frame(Vec::new());
        assert!(card.at("Pick one").is_none(), "the question is the Reader's card now");
        card.stale = true;
        card.frame(Vec::new());
        assert!(card.at("Pick one").is_none());
        let line = card.texts.iter().map(|(drawn, _)| drawn.as_str());
        assert!(
            line.into_iter()
                .any(|text| text.ends_with("Working \u{b7} progress updated 8 min ago"))
        );
    }

    #[test]
    fn a_tab_without_a_ledger_has_history_and_no_switch() {
        let card = Card::new(false);
        assert!(card.at("Progress").is_none());
        assert!(card.at("This branch").is_some());
    }

    fn asking() -> LastTurn {
        let block = tidewatch_core::end_block::EndBlock {
            done: vec!["Did it".into()],
            needs_you: vec!["Which base?".into()],
            next: Vec::new(),
        };
        let today = tidewatch_core::civil_date::Date::parse("2026-10-27").unwrap();
        LastTurn::of(Some(&block), None, Ok(()), today).unwrap()
    }

    fn dot_pixels(card: &Card) -> usize {
        let on = card.at("Progress").unwrap();
        let mut found = 0;
        for clipped in &card.shapes {
            count_dots(&clipped.shape, on, &mut found);
        }
        found
    }

    fn count_dots(shape: &Shape, near: Pos2, found: &mut usize) {
        match shape {
            Shape::Circle(circle)
                if circle.fill == brand::NEEDS && (circle.center - near).length() < 40.0 =>
            {
                *found += 1;
            }
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| count_dots(shape, near, found)),
            _ => {}
        }
    }

    #[test]
    fn a_new_needs_you_item_puts_a_dot_on_progress_while_another_side_shows_and_showing_it_clears_it()
     {
        let mut card = Card::new(true);
        card.last_turn = Some(asking());
        card.state.chosen = Some(Side::History);
        card.frame(Vec::new());
        assert_eq!(dot_pixels(&card), 1);
        let progress = card.at("Progress").unwrap();
        card.click(progress);
        assert_eq!(card.state.shown, Some(Side::Progress));
        assert!(card.at("Which base?").is_some());
        let history = card.at("History").unwrap();
        card.click(history);
        assert_eq!(card.state.shown, Some(Side::History));
        assert_eq!(dot_pixels(&card), 0, "what Progress showed is not new again");
        card.last_turn = Some(LastTurn { needs: vec!["Another?".into()], ..asking() });
        card.frame(Vec::new());
        assert_eq!(dot_pixels(&card), 1, "a different question is new");
    }

    #[test]
    fn no_dot_without_a_needs_you_item_or_a_check() {
        let mut card = Card::new(true);
        card.state.chosen = Some(Side::History);
        card.last_turn = Some(LastTurn { needs: Vec::new(), ..asking() });
        card.frame(Vec::new());
        assert_eq!(dot_pixels(&card), 0);
        card.last_turn = None;
        card.frame(Vec::new());
        assert_eq!(dot_pixels(&card), 0);
    }

    fn picture() -> Arc<Avatar> {
        let (svg, reason) = crate::avatar_glue::FAKE[0];
        Arc::new(tidewatch_core::avatar::clean(svg, reason).unwrap())
    }

    #[test]
    fn an_avatar_takes_the_left_of_the_header_on_both_sides_and_pushes_the_title_over() {
        for with_ledger in [false, true] {
            let plain = Card::new(with_ledger);
            let mut drawn = Card::new(with_ledger);
            drawn.face = Some(picture());
            drawn.frame(Vec::new());
            drawn.frame(Vec::new());
            let left = |card: &Card| {
                card.texts.iter().find(|(text, _)| text == "detached HEAD").unwrap().1.left()
            };
            assert!(
                (left(&drawn) - left(&plain) - (FACE + FACE_GAP)).abs() < 0.5,
                "{with_ledger}: {} vs {}",
                left(&drawn),
                left(&plain)
            );
        }
    }

    #[test]
    fn a_progress_header_names_the_branch_where_the_mockup_has_the_tab_name() {
        let card = Card::new(true);
        assert!(card.at("detached HEAD").is_some());
        assert!(card.at("Progress").is_some());
    }
}
