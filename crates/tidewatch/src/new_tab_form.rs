//! Draws the new-tab form in the left column of a draft tab and reads its keys.

use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    Align, Align2, Color32, Id, Key, Layout, Modifiers, Rect, RichText, Sense, TextEdit, Ui,
    UiBuilder, pos2, vec2,
};
use tidewatch_core::agents::Level;

use crate::brand;
use crate::catalog::Catalog;
use crate::commands::Platform;
use crate::new_tab::{Kind, NameCheck, NewTab, Request, level_hint, level_refusal};
use crate::new_tab_cards::{self, Cards};
use crate::new_tab_fields::{
    self, FIELD_H, LABEL_GAP, LABEL_H, NameEdit, PROBLEM_H, section_label, text_box,
};
use crate::new_tab_workflow;
use crate::reader_view::field_frame_on;
use crate::repo_base::RepoFacts;
use crate::screen_kit::{body, heading, mono, primary_button};
use crate::segmented;

const SIDE_PAD: f32 = 26.0;
const TOP_PAD: f32 = 8.0;
const COLUMN_MAX_W: f32 = 880.0;
const GAP: f32 = 18.0;
const HEADER_H: f32 = 46.0;
const SECTION_ROW_H: f32 = 28.0;
const FILTER_W: f32 = 200.0;
const CONTROL_ROW_H: f32 = segmented::HEIGHT;
const WARNING_H: f32 = 17.0;
const NOTE_H: f32 = 18.0;
const HINT_H: f32 = 22.0;
const FOOTER_H: f32 = 32.0;
const FOOTER_PAD: f32 = 14.0;
const MIN_GRID_H: f32 = 56.0;
const MESSAGE_H: (f32, f32) = (120.0, 210.0);
const TASK_H: (f32, f32) = (90.0, 160.0);
const COLUMN_GAP: f32 = 12.0;
const TYPING: &str = "new-tab-typing";

#[derive(Debug, PartialEq)]
pub(crate) enum Out {
    Start(Request),
    Close,
}

fn typing_id() -> Id {
    Id::new(TYPING)
}

fn pressed(ctx: &egui::Context, key: Key) -> bool {
    ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key))
}

const NUMBERS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

fn start(state: &mut NewTab, catalog: &Catalog, facts: Option<&RepoFacts>) -> Option<Out> {
    match state.start_request(catalog, facts) {
        Ok(request) => Some(Out::Start(request)),
        Err(reason) => {
            state.error = Some(reason);
            None
        }
    }
}

/// The keys of the form. Cmd+Enter starts from anywhere in it; the bare keys (A, W, S, 1-9, Left,
/// Right, Esc) are only read while no text field has the keyboard. Esc is also left alone for the
/// frame in which it took the keyboard from a text field.
fn keys(
    ctx: &egui::Context,
    state: &mut NewTab,
    catalog: &Catalog,
    facts: Option<&RepoFacts>,
) -> Option<Out> {
    if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::Enter)) {
        return start(state, catalog, facts);
    }
    let typing = ctx.text_edit_focused();
    let typed_before = ctx.data(|data| data.get_temp::<bool>(typing_id())).unwrap_or(false);
    if typing {
        return None;
    }
    for (key, kind) in [(Key::A, Kind::Agent), (Key::W, Kind::Workflow), (Key::S, Kind::Shell)] {
        if pressed(ctx, key) {
            return state.choose(kind).map(Out::Start);
        }
    }
    for (index, key) in NUMBERS.into_iter().enumerate() {
        if pressed(ctx, key) {
            state.select(index, catalog);
        }
    }
    if state.kind() == Kind::Agent {
        if pressed(ctx, Key::ArrowLeft) {
            state.step_level(false, catalog);
        }
        if pressed(ctx, Key::ArrowRight) {
            state.step_level(true, catalog);
        }
    }
    let popup_open = egui::Popup::is_any_open(ctx);
    (!typed_before && !popup_open && pressed(ctx, Key::Escape)).then_some(Out::Close)
}

fn column_rect(area: Rect) -> Rect {
    let width = (area.width() - 2.0 * SIDE_PAD).clamp(0.0, COLUMN_MAX_W);
    Rect::from_min_size(
        pos2(area.left() + SIDE_PAD, area.top() + TOP_PAD),
        vec2(width, (area.height() - TOP_PAD).max(0.0)),
    )
}

fn hint_job(parts: &[(&str, bool)]) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (text, key) in parts {
        let (font, color) =
            if *key { (mono(12.0), brand::SOFT) } else { (body(12.5), brand::FAINT) };
        job.append(text, 0.0, TextFormat::simple(font, color));
    }
    job
}

fn footer_hint(state: &NewTab) -> LayoutJob {
    match state.kind() {
        Kind::Workflow => hint_job(&[
            ("1-9", true),
            (" workflow \u{b7} ", false),
            ("Esc", true),
            (" closes the tab", false),
        ]),
        Kind::Agent | Kind::Shell => hint_job(&[
            ("1-9", true),
            (" agent \u{b7} ", false),
            ("Left Right", true),
            (" level \u{b7} ", false),
            ("Esc", true),
            (" closes the tab", false),
        ]),
    }
}

fn start_label(state: &NewTab, catalog: &Catalog) -> String {
    let keys = match Platform::CURRENT {
        Platform::Mac => "\u{2318}\u{21b5}",
        Platform::Linux => "Ctrl+Enter",
    };
    let what = match state.kind() {
        Kind::Workflow => "Start run".to_owned(),
        Kind::Agent | Kind::Shell => state
            .selected_entry(catalog)
            .map_or_else(|| "Start".to_owned(), |(profile, _)| format!("Start {}", profile.name)),
    };
    format!("{what}  {keys}")
}

/// Everything the layout needs to know before it draws, to share the height out.
struct Plan {
    cards_natural: f32,
    stages_h: f32,
    note: bool,
    warnings: usize,
    problem: bool,
}

impl Plan {
    fn after_cards(&self, kind: Kind, text_min: f32) -> f32 {
        let permission = match kind {
            Kind::Workflow => 0.0,
            Kind::Agent | Kind::Shell => {
                GAP + LABEL_H + 8.0 + CONTROL_ROW_H + if self.note { NOTE_H } else { 0.0 }
            }
        };
        let stages = if kind == Kind::Workflow { GAP + LABEL_H + 8.0 + self.stages_h } else { 0.0 };
        let warnings = if self.warnings > 0 { 8.0 + self.warnings as f32 * WARNING_H } else { 0.0 };
        let fields =
            GAP + LABEL_H + LABEL_GAP + FIELD_H + if self.problem { PROBLEM_H + 2.0 } else { 0.0 };
        let text = GAP
            + LABEL_H
            + LABEL_GAP
            + text_min
            + if kind == Kind::Workflow { 0.0 } else { HINT_H };
        permission + stages + warnings + fields + text + GAP + FOOTER_H + FOOTER_PAD
    }

    /// The height the cards get: all they need, else what is left, never less than a row and a bit.
    fn grid_h(&self, kind: Kind, column_h: f32) -> f32 {
        let text_min = if kind == Kind::Workflow { TASK_H.0 } else { MESSAGE_H.0 };
        let above = HEADER_H + GAP + SECTION_ROW_H + 8.0;
        let room = column_h - above - self.after_cards(kind, text_min);
        self.cards_natural.min(room.max(MIN_GRID_H))
    }
}

pub(crate) fn show(
    ui: &mut Ui,
    area: Rect,
    state: &mut NewTab,
    catalog: &Catalog,
    facts: Option<&RepoFacts>,
    keys_free: bool,
) -> Option<Out> {
    state.settle(catalog);
    let ctx = ui.ctx().clone();
    let mut out = if keys_free { keys(&ctx, state, catalog, facts) } else { None };
    let column = column_rect(area);
    let footer = Rect::from_min_max(
        pos2(column.left(), column.bottom() - FOOTER_PAD - FOOTER_H),
        pos2(column.right(), column.bottom() - FOOTER_PAD),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(column));
    child.spacing_mut().item_spacing = vec2(8.0, 0.0);
    out = out.or(header(&mut child, state));
    child.add_space(GAP);
    let kind = state.kind();
    let ids = state.visible(catalog);
    let check = state.name_check(facts);
    let stages = match (state.selected_workflow(catalog), catalog.profiles()) {
        (Some(workflow), Some(profiles)) => new_tab_workflow::stage_cards(workflow, profiles),
        _ => Vec::new(),
    };
    let warnings = state.warnings(catalog);
    let refusal = refusal_note(state, catalog);
    let plan = Plan {
        cards_natural: match kind {
            Kind::Workflow => new_tab_cards::workflows_height(ids.len()),
            Kind::Agent | Kind::Shell => new_tab_cards::agents_height(ids.len()),
        },
        stages_h: new_tab_cards::stages_height(stages.len()),
        note: refusal.is_some(),
        warnings: warnings.len(),
        problem: !check.is_fine(),
    };
    let grid_h = plan.grid_h(kind, column.height());
    let scroll = std::mem::take(&mut state.scroll_to_selected);
    let cards = Cards {
        ids: &ids,
        selected: state.selected(),
        scroll_to_selected: scroll,
        max_height: grid_h,
    };
    let picked = match kind {
        Kind::Workflow => {
            section_row(&mut child, "Workflow", None);
            if ids.is_empty() {
                empty(&mut child, state, catalog, "workflow");
                None
            } else {
                new_tab_cards::workflows(&mut child, catalog, &cards)
            }
        }
        Kind::Agent | Kind::Shell => {
            let filter = section_row(&mut child, "Agent", Some(state.filter()));
            if let Some(text) = filter {
                state.set_filter(&text, catalog);
            }
            if ids.is_empty() {
                empty(&mut child, state, catalog, "profile");
                None
            } else {
                new_tab_cards::agents(&mut child, catalog, &cards)
            }
        }
    };
    if let Some(index) = picked {
        state.select(index, catalog);
    }
    if kind == Kind::Workflow {
        child.add_space(GAP);
        let name = state.selected_workflow(catalog).map_or(String::new(), |w| w.name.clone());
        section_label(&mut child, &format!("Stages of {name}"));
        child.add_space(8.0);
        new_tab_cards::stages(&mut child, &stages);
    } else {
        permission(&mut child, state, catalog, refusal.as_deref());
    }
    if !warnings.is_empty() {
        child.add_space(8.0);
    }
    for warning in &warnings {
        child.label(RichText::new(warning).size(12.0).color(brand::ORANGE));
    }
    child.add_space(GAP);
    fields(&mut child, state, facts);
    child.add_space(GAP);
    let (title, hint, bounds) = match kind {
        Kind::Workflow => ("Task", "What should this run deliver?", TASK_H),
        Kind::Agent | Kind::Shell => {
            ("First message", "Optional. Leave it empty to start the agent idle.", MESSAGE_H)
        }
    };
    section_label(&mut child, title);
    child.add_space(LABEL_GAP);
    let trailing = if kind == Kind::Workflow { 0.0 } else { HINT_H };
    let room = footer.top() - child.cursor().top() - GAP - trailing;
    if let Some(text) = text_box(&mut child, state, room.clamp(bounds.0, bounds.1), hint) {
        state.set_text(&text);
    }
    if kind != Kind::Workflow {
        child.add_space(6.0);
        child.label(RichText::new(message_hint(state)).size(12.0).color(brand::FAINT));
    }
    out = out.or(footer_row(ui, footer, state, catalog, facts));
    let typing = ctx.text_edit_focused();
    ctx.data_mut(|data| data.insert_temp(typing_id(), typing));
    out
}

fn message_hint(state: &NewTab) -> &'static str {
    if state.makes_a_worktree() {
        "The worktree name follows your message until you edit it. Leave the message empty to start the agent idle."
    } else {
        "Leave the message empty to start the agent idle."
    }
}

fn empty(ui: &mut Ui, state: &NewTab, catalog: &Catalog, noun: &str) {
    let text = new_tab_cards::empty_text(state.filter(), catalog.is_loaded(), noun);
    new_tab_cards::empty(ui, &text);
}

fn header(ui: &mut Ui, state: &mut NewTab) -> Option<Out> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let painter = ui.painter();
    painter.text(rect.left_top(), Align2::LEFT_TOP, "New tab", heading(20.0), brand::TEXT);
    let mut job = LayoutJob::default();
    let faint = |color: Color32| TextFormat::simple(body(13.0), color);
    job.append("in ", 0.0, faint(brand::MUTED));
    job.append(state.label(), 0.0, faint(brand::TEAL_TEXT));
    let tail = if state.makes_a_worktree() {
        ", a fresh worktree"
    } else {
        ", no worktree, runs in the folder itself"
    };
    job.append(tail, 0.0, faint(brand::MUTED));
    painter.galley(rect.left_top() + vec2(0.0, 27.0), painter.layout_job(job), brand::MUTED);
    let choices: Vec<(&str, &str, Kind)> = [
        ("Agent", "A", Kind::Agent),
        ("Workflow", "W", Kind::Workflow),
        ("Shell", "S", Kind::Shell),
    ]
    .into_iter()
    .filter(|(_, _, kind)| state.offers(*kind))
    .collect();
    let labels: Vec<(&str, &str)> = choices.iter().map(|(label, key, _)| (*label, *key)).collect();
    let active = choices.iter().position(|(_, _, kind)| *kind == state.kind()).unwrap_or(0);
    let shown = segmented::show_keyed(
        ui,
        Id::new("new-tab-kind"),
        rect.right(),
        rect.center().y,
        &labels,
        active,
    );
    let kind = shown.clicked.map(|index| choices[index].2)?;
    state.choose(kind).map(Out::Start)
}

/// The label of a section, and for the agent list the filter on its right. Returns the filter's new
/// text when it changed.
fn section_row(ui: &mut Ui, title: &str, filter: Option<&str>) -> Option<String> {
    let (row, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), SECTION_ROW_H), Sense::hover());
    ui.painter().text(
        row.left_center(),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        body(11.5),
        brand::MUTED,
    );
    ui.add_space(8.0);
    let current = filter?;
    let mut text = current.to_owned();
    let box_ = Rect::from_min_size(
        pos2(row.right() - FILTER_W, row.top() - 2.0),
        vec2(FILTER_W, SECTION_ROW_H + 2.0),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(box_));
    field_frame_on(&mut child, brand::SEGMENT, false, |ui| {
        ui.add(
            TextEdit::singleline(&mut text)
                .id(Id::new("new-tab-filter"))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(body(12.5))
                .text_color(brand::TEXT)
                .hint_text(RichText::new("Filter").color(brand::FAINT))
                .desired_width(ui.available_width()),
        )
    });
    (text != current).then_some(text)
}

fn refusal_note(state: &NewTab, catalog: &Catalog) -> Option<String> {
    let (_, agent) = state.selected_entry(catalog)?;
    Level::ALL.iter().find_map(|level| level_refusal(state.group(), agent, *level))
}

fn permission(ui: &mut Ui, state: &mut NewTab, catalog: &Catalog, note: Option<&str>) {
    let Some((_, agent)) = state.selected_entry(catalog) else { return };
    ui.add_space(GAP);
    section_label(ui, "Permission");
    ui.add_space(8.0);
    let labels = Level::ALL.map(Level::label);
    let tones = Level::ALL.map(|level| (level == Level::Yolo).then_some(brand::RED));
    let active = Level::ALL.iter().position(|level| *level == state.level()).unwrap_or(0);
    let refusals = Level::ALL.map(|level| level_refusal(state.group(), agent, level));
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.set_min_height(CONTROL_ROW_H);
        ui.spacing_mut().item_spacing.x = 14.0;
        picked = segmented::inline_toned(ui, Id::new("new_tab_level"), &labels, &tones, active);
        let text = RichText::new(level_hint(state.level())).size(13.0).color(brand::SOFT);
        let hint = ui.add(egui::Label::new(text).truncate());
        let flags = agent.level_args(state.level()).map(|args| args.join(" ")).unwrap_or_default();
        let tip = if flags.is_empty() {
            level_hint(state.level()).to_owned()
        } else {
            format!("{}\nAdds {flags}", level_hint(state.level()))
        };
        hint.on_hover_text(tip);
    });
    if let Some(note) = note {
        ui.add_space(4.0);
        ui.label(RichText::new(note).size(12.0).color(brand::MUTED));
    }
    if let Some(index) = picked
        && refusals[index].is_none()
    {
        state.set_level(Level::ALL[index], catalog);
    }
}

/// The worktree name and the branch it starts from, side by side.
fn fields(ui: &mut Ui, state: &mut NewTab, facts: Option<&RepoFacts>) {
    if !state.makes_a_worktree() {
        return;
    }
    let width = ui.available_width();
    let each = (width - COLUMN_GAP) / 2.0;
    let top = ui.cursor().top();
    let origin = ui.cursor().left();
    let left_rect = Rect::from_min_size(pos2(origin, top), vec2(each, 120.0));
    let right_rect = Rect::from_min_size(pos2(origin + each + COLUMN_GAP, top), vec2(each, 120.0));
    let mut left = ui.new_child(UiBuilder::new().max_rect(left_rect));
    let mut right = ui.new_child(UiBuilder::new().max_rect(right_rect));
    left.spacing_mut().item_spacing = vec2(8.0, 0.0);
    right.spacing_mut().item_spacing = vec2(8.0, 0.0);
    let edit = new_tab_fields::name_field(&mut left, state, facts);
    let base = new_tab_fields::base_field(&mut right, state, facts);
    match edit {
        NameEdit::Untouched => {}
        NameEdit::Typed(name) => state.type_name(&name),
        NameEdit::UseSuggestion => {
            if let NameCheck::Taken { suggestion } = state.name_check(facts) {
                state.type_name(&suggestion);
            }
        }
    }
    if let Some(branch) = base {
        state.pick_base(&branch);
    }
    let height = left.min_rect().height().max(right.min_rect().height());
    ui.allocate_space(vec2(width, height));
}

fn footer_row(
    ui: &mut Ui,
    rect: Rect,
    state: &mut NewTab,
    catalog: &Catalog,
    facts: Option<&RepoFacts>,
) -> Option<Out> {
    let galley = ui.painter().layout_job(footer_hint(state));
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        brand::FAINT,
    );
    if let Some(error) = state.error.clone() {
        ui.painter().text(
            pos2(rect.left(), rect.top() - 6.0),
            Align2::LEFT_BOTTOM,
            error,
            body(12.5),
            brand::RED,
        );
    }
    let blocked = state.start_request(catalog, facts).err();
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)));
    let label = start_label(state, catalog);
    let button = primary_button(&mut child, &label, blocked.is_none());
    let button = match &blocked {
        Some(reason) => button.on_hover_text(reason),
        None => button,
    };
    if button.clicked() { start(state, catalog, facts) } else { None }
}

#[cfg(test)]
mod tests;
