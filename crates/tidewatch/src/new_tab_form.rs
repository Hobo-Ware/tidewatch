//! Draws the new-tab form in the left column of a draft tab and reads its keys.

use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, Galley, Id, Key, Layout, Modifiers, Rect, ScrollArea, Sense, Stroke,
    Ui, UiBuilder, pos2, vec2,
};
use tidewatch_core::agents::Level;

use crate::brand;
use crate::catalog::Catalog;
use crate::new_tab::{Kind, NameCheck, NewTab, RemoteOffer, Request, level_hint, level_refusal};
use crate::new_tab_cards;
use crate::new_tab_fields::{self, NameEdit, section_label, text_box};
use crate::new_tab_lane;
use crate::new_tab_layout::{self as layout, Above, LABEL_GAP, LABEL_H, SECTION_GAP};
use crate::new_tab_remote;
use crate::new_tab_workflow::{self, StageCard};
use crate::repo_base::RepoFacts;
use crate::screen_kit::{
    Size, body, confirm_key, heading, keyed_button_width, mono, primary_button_keyed,
};
use crate::segmented;

const TYPING: &str = "new-tab-typing";
const PLACED: &str = "new-tab-placed";
const CAPTION_GAP: f32 = 8.0;
const DESCRIPTION_GAP: f32 = 6.0;
const NOTE_GAP: f32 = 4.0;
const HINT_GAP: f32 = 6.0;
const WARNING_GAP: f32 = 2.0;
const FOOTER_HINT_GAP: f32 = 14.0;

#[derive(Debug, PartialEq)]
pub(crate) enum Out {
    Start(Request),
    Close,
}

/// Where the form put its main parts in the last frame, for a headless test to read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Placed {
    pub(crate) lane: Option<Rect>,
    pub(crate) permission: Option<Rect>,
    pub(crate) remote: Option<Rect>,
    pub(crate) remote_switch: Option<Rect>,
    pub(crate) name: Option<Rect>,
    pub(crate) base: Option<Rect>,
    pub(crate) message: Option<Rect>,
    pub(crate) selected_row: Option<Rect>,
    pub(crate) hints: Option<Rect>,
    pub(crate) start: Option<Rect>,
}

fn typing_id() -> Id {
    Id::new(TYPING)
}

pub(crate) fn placed_id() -> Id {
    Id::new(PLACED)
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

/// The keys of the form. Cmd+Enter starts from anywhere in it; the bare keys (A, W, S, 1-9, Up, Down,
/// Left, Right, Esc) are only read while no text field has the keyboard. Esc is also left alone for the
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
    let popup_open = egui::Popup::is_any_open(ctx);
    if !popup_open {
        if pressed(ctx, Key::ArrowUp) {
            state.select(state.selected().saturating_sub(1), catalog);
        }
        if pressed(ctx, Key::ArrowDown) {
            state.select(state.selected() + 1, catalog);
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
    (!typed_before && !popup_open && pressed(ctx, Key::Escape)).then_some(Out::Close)
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

type Hint = &'static [(&'static str, bool)];

/// The key hints of the footer, longest first: the first that fits beside Start is shown.
fn footer_hints(kind: Kind) -> [Hint; 3] {
    match kind {
        Kind::Workflow => [
            &[
                ("1-9 Up Down", true),
                (" workflow \u{b7} ", false),
                ("Esc", true),
                (" closes the tab", false),
            ],
            &[("1-9", true), (" workflow \u{b7} ", false), ("Esc", true), (" closes", false)],
            &[("Esc", true), (" closes", false)],
        ],
        Kind::Agent | Kind::Shell => [
            &[
                ("1-9 Up Down", true),
                (" agent \u{b7} ", false),
                ("Left Right", true),
                (" level \u{b7} ", false),
                ("Esc", true),
                (" closes the tab", false),
            ],
            &[
                ("1-9", true),
                (" agent \u{b7} ", false),
                ("Left Right", true),
                (" level \u{b7} ", false),
                ("Esc", true),
                (" closes", false),
            ],
            &[("Esc", true), (" closes", false)],
        ],
    }
}

fn start_label(state: &NewTab, catalog: &Catalog) -> String {
    match state.kind() {
        Kind::Workflow => "Start run".to_owned(),
        Kind::Agent | Kind::Shell => state
            .selected_entry(catalog)
            .map_or_else(|| "Start".to_owned(), |(profile, _)| format!("Start {}", profile.name)),
    }
}

fn wrapped(ui: &Ui, text: &str, size: f32, color: Color32, width: f32) -> Arc<Galley> {
    ui.painter().layout(text.to_owned(), body(size), color, width.max(1.0))
}

fn message_hint(state: &NewTab) -> &'static str {
    match (state.makes_a_worktree(), state.kind()) {
        (true, Kind::Workflow) => "The worktree name follows your task until you edit it.",
        (true, _) => {
            "The worktree name follows your message until you edit it. Leave the message empty to start the agent idle."
        }
        (false, _) => "Leave the message empty to start the agent idle.",
    }
}

fn refusal_note(state: &NewTab, catalog: &Catalog) -> Option<String> {
    let (_, agent) = state.selected_entry(catalog)?;
    Level::ALL.iter().find_map(|level| level_refusal(state.group(), agent, *level))
}

/// What the pane shows this frame, gathered once so that measuring and drawing agree.
struct Content<'a> {
    kind: Kind,
    stages: Vec<StageCard>,
    stage_name: String,
    warnings: Vec<String>,
    refusal: Option<String>,
    problem: bool,
    has_permission: bool,
    remote: Option<RemoteOffer>,
    state: &'a NewTab,
}

impl<'a> Content<'a> {
    fn gather(state: &'a NewTab, catalog: &Catalog, facts: Option<&RepoFacts>) -> Self {
        let stages = match (state.selected_workflow(catalog), catalog.profiles()) {
            (Some(workflow), Some(profiles)) => new_tab_workflow::stage_cards(workflow, profiles),
            _ => Vec::new(),
        };
        let stage_name = state.selected_workflow(catalog).map_or(String::new(), |w| w.name.clone());
        Self {
            kind: state.kind(),
            stages,
            stage_name,
            warnings: state.warnings(catalog),
            refusal: refusal_note(state, catalog),
            problem: !state.name_check(facts).is_fine(),
            has_permission: state.kind() != Kind::Workflow
                && state.selected_entry(catalog).is_some(),
            remote: (state.kind() != Kind::Workflow)
                .then(|| state.selected_entry(catalog).map(|(_, agent)| RemoteOffer::of(agent)))
                .flatten(),
            state,
        }
    }

    fn above(&self, ui: &Ui, width: f32) -> Above {
        let mut above = Above::default();
        if self.has_permission {
            let control =
                segmented::inline_or_chips_height(ui, &Level::ALL.map(Level::label), width);
            let description = wrapped(ui, level_hint(self.state.level()), 13.0, brand::SOFT, width);
            let note = self.refusal.as_deref().map_or(0.0, |note| {
                NOTE_GAP + wrapped(ui, note, 12.0, brand::MUTED, width).size().y
            });
            above.permission =
                LABEL_H + CAPTION_GAP + control + DESCRIPTION_GAP + description.size().y + note;
        }
        if self.remote.is_some() {
            above.remote = new_tab_remote::ROW_H;
        }
        if self.kind == Kind::Workflow && !self.stages.is_empty() {
            let columns = layout::stage_columns(width);
            above.stages =
                LABEL_H + CAPTION_GAP + new_tab_cards::stages_height(columns, self.stages.len());
        }
        above.warnings = self
            .warnings
            .iter()
            .map(|warning| wrapped(ui, warning, 12.0, brand::ORANGE, width).size().y + WARNING_GAP)
            .sum();
        if self.state.makes_a_worktree() {
            above.fields = layout::fields_height(width, self.problem);
        }
        above
    }

    fn hint_height(&self, ui: &Ui, width: f32) -> f32 {
        HINT_GAP + wrapped(ui, message_hint(self.state), 12.0, brand::FAINT, width).size().y
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
    let columns = layout::columns(layout::form_rect(area));
    let mut header_ui = ui.new_child(UiBuilder::new().max_rect(columns.header));
    out = out.or(header(&mut header_ui, state));
    let selected_row = lane(ui, columns.lane, state, catalog);
    let mut placed = Placed { lane: Some(columns.lane), selected_row, ..Placed::default() };
    out = out.or(pane(ui, columns.pane, state, catalog, facts, &mut placed));
    let typing = ctx.text_edit_focused();
    ctx.data_mut(|data| {
        data.insert_temp(typing_id(), typing);
        data.insert_temp(placed_id(), placed);
    });
    out
}

fn lane(ui: &mut Ui, rect: Rect, state: &mut NewTab, catalog: &Catalog) -> Option<Rect> {
    let entries = new_tab_lane::entries(state, catalog);
    let noun = if state.kind() == Kind::Workflow { "workflow" } else { "profile" };
    let empty = new_tab_cards::empty_text(state.filter(), catalog.is_loaded(), noun);
    let scroll = std::mem::take(&mut state.scroll_to_selected);
    let out = new_tab_lane::show(ui, rect, state, &entries, &empty, scroll);
    if let Some(text) = out.filter {
        state.set_filter(&text, catalog);
    }
    if let Some(index) = out.picked {
        state.select(index, catalog);
    }
    out.selected
}

fn header(ui: &mut Ui, state: &mut NewTab) -> Option<Out> {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), layout::HEADER_H), Sense::hover());
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

fn pane(
    ui: &mut Ui,
    rect: Rect,
    state: &mut NewTab,
    catalog: &Catalog,
    facts: Option<&RepoFacts>,
    placed: &mut Placed,
) -> Option<Out> {
    let content = Content::gather(state, catalog, facts);
    let plan =
        layout::pane(rect, |width| (content.above(ui, width), content.hint_height(ui, width)));
    let stages = content.stages.clone();
    let stage_name = content.stage_name.clone();
    let warnings = content.warnings.clone();
    let refusal = content.refusal.clone();
    let has_permission = content.has_permission;
    let remote = content.remote.clone();
    let kind = content.kind;
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(plan.body));
    ScrollArea::vertical().id_salt("new-tab-pane").auto_shrink([false, false]).show(
        &mut body_ui,
        |ui| {
            ui.set_width(plan.content_w);
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            let width = plan.content_w;
            if kind == Kind::Workflow && !stages.is_empty() {
                section_label(ui, &format!("Stages of {stage_name}"));
                ui.add_space(CAPTION_GAP);
                new_tab_cards::stages(ui, &stages, layout::stage_columns(width));
                ui.add_space(SECTION_GAP);
            }
            if has_permission {
                placed.permission = permission(ui, state, catalog, refusal.as_deref());
                ui.add_space(SECTION_GAP);
            }
            if let Some(offer) = &remote {
                let row = new_tab_remote::show(ui, offer, state.remote());
                placed.remote = Some(row.rect);
                placed.remote_switch = Some(row.switch);
                if let Some(on) = row.toggled {
                    state.set_remote(on);
                }
                ui.add_space(SECTION_GAP);
            }
            if !warnings.is_empty() {
                for warning in &warnings {
                    let galley = wrapped(ui, warning, 12.0, brand::ORANGE, width);
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(width, galley.size().y + WARNING_GAP),
                        Sense::hover(),
                    );
                    ui.painter().galley(rect.min, galley, brand::ORANGE);
                }
                ui.add_space(SECTION_GAP);
            }
            if let Some((name, base)) = fields(ui, state, facts) {
                placed.name = Some(name);
                placed.base = Some(base);
                ui.add_space(SECTION_GAP);
            }
            placed.message = Some(message(ui, state, plan.message_h));
        },
    );
    if plan.scrolls {
        ui.painter().hline(
            rect.x_range(),
            plan.footer.top() - 6.0,
            Stroke::new(1.0, brand::ITEM_LINE),
        );
    }
    footer(ui, plan.footer, state, catalog, facts, placed)
}

fn message(ui: &mut Ui, state: &mut NewTab, height: f32) -> Rect {
    let (title, hint) = match state.kind() {
        Kind::Workflow => ("Task", "What should this run deliver?"),
        Kind::Agent | Kind::Shell => {
            ("First message", "Optional. Leave it empty to start the agent idle.")
        }
    };
    section_label(ui, title);
    ui.add_space(LABEL_GAP);
    let (rect, text) = text_box(ui, state, height, hint);
    if let Some(text) = text {
        state.set_text(&text);
    }
    ui.add_space(HINT_GAP);
    let width = ui.available_width();
    let galley = wrapped(ui, message_hint(state), 12.0, brand::FAINT, width);
    let (hint_rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    ui.painter().galley(hint_rect.min, galley, brand::FAINT);
    rect
}

fn permission(
    ui: &mut Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    note: Option<&str>,
) -> Option<Rect> {
    let (_, agent) = state.selected_entry(catalog)?;
    let width = ui.available_width();
    section_label(ui, "Permission");
    ui.add_space(CAPTION_GAP);
    let labels = Level::ALL.map(Level::label);
    let tones = Level::ALL.map(|level| (level == Level::Yolo).then_some(brand::RED));
    let active = Level::ALL.iter().position(|level| *level == state.level()).unwrap_or(0);
    let refusals = Level::ALL.map(|level| level_refusal(state.group(), agent, level));
    let top = ui.cursor().top();
    let picked = segmented::inline_or_chips(ui, Id::new("new_tab_level"), &labels, &tones, active);
    let control = Rect::from_min_max(
        pos2(ui.min_rect().left(), top),
        pos2(ui.min_rect().left() + width, ui.cursor().top()),
    );
    ui.add_space(DESCRIPTION_GAP);
    let text = level_hint(state.level());
    let galley = wrapped(ui, text, 13.0, brand::SOFT, width);
    let (rect, response) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    ui.painter().galley(rect.min, galley, brand::SOFT);
    let flags = agent.level_args(state.level()).map(|args| args.join(" ")).unwrap_or_default();
    let tip = if flags.is_empty() { text.to_owned() } else { format!("{text}\nAdds {flags}") };
    response.on_hover_text(tip);
    if let Some(note) = note {
        ui.add_space(NOTE_GAP);
        let galley = wrapped(ui, note, 12.0, brand::MUTED, width);
        let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
        ui.painter().galley(rect.min, galley, brand::MUTED);
    }
    if let Some(index) = picked
        && refusals[index].is_none()
    {
        state.set_level(Level::ALL[index], catalog);
    }
    Some(control)
}

fn fields(ui: &mut Ui, state: &mut NewTab, facts: Option<&RepoFacts>) -> Option<(Rect, Rect)> {
    if !state.makes_a_worktree() {
        return None;
    }
    let problem = !state.name_check(facts).is_fine();
    let width = ui.available_width();
    let area =
        Rect::from_min_size(ui.cursor().min, vec2(width, layout::fields_height(width, problem)));
    let (name_rect, base_rect) = layout::field_rects(area, problem);
    let mut left = ui.new_child(UiBuilder::new().max_rect(name_rect));
    let mut right = ui.new_child(UiBuilder::new().max_rect(base_rect));
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
    ui.allocate_rect(area, Sense::hover());
    Some((name_rect, base_rect))
}

fn footer(
    ui: &mut Ui,
    rect: Rect,
    state: &mut NewTab,
    catalog: &Catalog,
    facts: Option<&RepoFacts>,
    placed: &mut Placed,
) -> Option<Out> {
    let blocked = state.start_request(catalog, facts).err();
    let label = start_label(state, catalog);
    let key = confirm_key();
    let button_w = keyed_button_width(ui, &label, key, Size::Regular);
    let room = (rect.width() - button_w - FOOTER_HINT_GAP).max(0.0);
    placed.hints = Some(Rect::from_min_size(rect.min, vec2(room, rect.height())));
    let painter = ui.painter();
    if let Some(error) = &state.error {
        let galley = crate::reader_view::ellipsized(painter, error, body(12.5), brand::RED, room);
        let at = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
        painter.galley(at, galley, brand::RED);
    } else {
        let hints = footer_hints(state.kind());
        let mut job = hints
            .iter()
            .map(|parts| hint_job(parts))
            .find(|job| painter.layout_job(job.clone()).size().x <= room)
            .unwrap_or_else(|| hint_job(hints[2]));
        job.wrap = TextWrapping::truncate_at_width(room.max(1.0));
        let galley = painter.layout_job(job);
        let at = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
        painter.galley(at, galley, brand::FAINT);
    }
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)));
    let button = primary_button_keyed(&mut child, &label, key, blocked.is_none(), Size::Regular);
    placed.start = Some(button.rect);
    let button = match &blocked {
        Some(reason) => button.on_hover_text(reason),
        None => button,
    };
    if button.clicked() { start(state, catalog, facts) } else { None }
}

#[cfg(test)]
mod tests;
