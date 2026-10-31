//! The services pill at the right end of the tab strip (one 8 px dot per service, then
//! `N services`, a 1 px outline and no fill), the 440 px list it opens (a card per service; the
//! selected one shows Logs, Restart and Stop while it runs and Logs and Start while it does not;
//! a menu with Edit and Remove opens in its own area and leaves the list open), and the dialogs:
//! Add or Edit service, and the one confirm before Remove.

use egui::containers::menu::menu_style;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, Id, Key, Layout, Margin, Modifiers, PopupCloseBehavior,
    PopupKind, Rect, RectAlign, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use stdusk_egui::widgets::{menu_item, style_menu};
use tidewatch_core::service_supervisor::Word;
use tidewatch_core::services::{Service, problems};

use crate::brand;
use crate::new_tab_fields::section_label;
use crate::notice::key_hint;
use crate::remote_pill::Popover;
use crate::screen_kit::{
    Size, TEXT_BODY, TEXT_TAG, body, button_width, danger_button, dialog_frame, footer_row, mono,
    primary_button_keyed, quiet_button, selection_row,
};
use crate::services_glue::{Command, Row};
use crate::settings_ui::{switch_line, text_input};

pub(crate) const PILL_H: f32 = Size::Compact.height();
const PILL_PAD: f32 = 12.0;
const DOT: f32 = 8.0;
const DOT_GAP: f32 = 5.0;
const DOT_TEXT_GAP: f32 = 10.0;
const MOST_DOTS: usize = 8;
const LIST_W: f32 = 440.0;
const LIST_MARGIN: i8 = 8;
const CARD_H: f32 = 76.0;
const CARD_PAD: f32 = 12.0;
const BUTTONS_H: f32 = Size::Compact.height() + 10.0;
const CARD_GAP: f32 = 2.0;
const DIALOG_W: f32 = 480.0;
const MENU_GAP: f32 = 4.0;
const MENU_W: f32 = 210.0;

/// Where the pill and the selected card's buttons were drawn, left in egui's memory for the
/// headless tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Placed {
    pub(crate) pill: Option<Rect>,
    pub(crate) buttons: Vec<(&'static str, Rect)>,
}

pub(crate) fn placed_id() -> Id {
    Id::new("services-pill-placed")
}

fn note_button(ctx: &egui::Context, name: &'static str, rect: Rect) {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Placed>(placed_id()).buttons.push((name, rect));
    });
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    Logs(String),
    Run(Command, String),
    Save { editing: Option<String>, service: Service },
    Remove(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Form {
    pub(crate) editing: Option<String>,
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) autostart: bool,
    pub(crate) focus: bool,
    pub(crate) tried: bool,
}

impl Form {
    pub(crate) fn add() -> Self {
        Self { focus: true, ..Self::default() }
    }

    fn edit(row: &Row) -> Self {
        Self {
            editing: Some(row.name.clone()),
            name: row.name.clone(),
            command: row.command.clone(),
            autostart: row.autostart,
            focus: true,
            tried: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Dialog {
    Form(Form),
    Remove(String),
}

#[derive(Debug, Default)]
pub(crate) struct Memory {
    selected: Option<String>,
    menu: Option<String>,
    menu_rect: Option<Rect>,
    dialog: Option<Dialog>,
    logs: Option<Logs>,
    popover: Popover,
}

/// The service whose terminal shows in place of the tab's: the tab it was opened from, `None`
/// until the first frame knows which one that is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Logs {
    pub(crate) name: String,
    pub(crate) tab: Option<u64>,
}

impl Memory {
    pub(crate) fn from_env() -> Self {
        let open = std::env::var("TIDEWATCH_DEV_SERVICES_POPOVER").is_ok_and(|v| v == "1");
        let dialog = match std::env::var("TIDEWATCH_DEV_SERVICES_DIALOG").ok().as_deref() {
            Some("add") => Some(Dialog::Form(Form::add())),
            Some("remove") => Some(Dialog::Remove("tunnel".to_owned())),
            _ => None,
        };
        let logs = std::env::var("TIDEWATCH_DEV_SERVICES_LOGS")
            .ok()
            .filter(|name| !name.is_empty())
            .map(|name| Logs { name, tab: None });
        Self {
            selected: std::env::var("TIDEWATCH_DEV_SERVICES_SELECT").ok().filter(|n| !n.is_empty()),
            menu: std::env::var("TIDEWATCH_DEV_SERVICES_MENU").ok().filter(|n| !n.is_empty()),
            menu_rect: None,
            dialog,
            logs,
            popover: if open { Popover::Open } else { Popover::OnClick },
        }
    }

    pub(crate) fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    pub(crate) fn open_add(&mut self) {
        self.dialog = Some(Dialog::Form(Form::add()));
    }

    /// The service whose logs show over `tab`; a tab that is not the one they were opened from
    /// closes them.
    pub(crate) fn logs_over(&mut self, tab: u64) -> Option<&str> {
        let logs = self.logs.as_mut()?;
        if *logs.tab.get_or_insert(tab) != tab {
            self.logs = None;
        }
        self.logs.as_ref().map(|logs| logs.name.as_str())
    }

    pub(crate) fn show_logs(&mut self, name: &str, tab: u64) {
        self.logs = Some(Logs { name: name.to_owned(), tab: Some(tab) });
    }

    pub(crate) fn close_logs(&mut self) {
        self.logs = None;
    }
}

pub(crate) fn label(count: usize) -> String {
    if count == 1 { "1 service".to_owned() } else { format!("{count} services") }
}

pub(crate) fn dot_color(word: Word) -> Color32 {
    match word {
        Word::Running => brand::GREEN,
        Word::Starting | Word::Restarting => brand::TEAL,
        Word::Crashed => brand::ORANGE,
        Word::Stopped => brand::FAINT,
    }
}

/// The pill's outline: teal when every service runs, orange when one gave up, else quiet.
pub(crate) fn outline(rows: &[Row]) -> Color32 {
    if rows.iter().any(|row| row.word == Word::Crashed) {
        brand::ORANGE
    } else if rows.iter().all(|row| row.word == Word::Running) {
        brand::TEAL
    } else {
        brand::FIELD_LINE
    }
}

pub(crate) fn is_running(word: Word) -> bool {
    matches!(word, Word::Running | Word::Starting | Word::Restarting)
}

fn line_ink(word: Word) -> Color32 {
    match word {
        Word::Crashed => brand::ORANGE,
        Word::Starting | Word::Restarting => brand::TEAL_TEXT,
        Word::Running | Word::Stopped => brand::MUTED,
    }
}

fn label_width(ui: &Ui, text: &str) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), body(Size::Compact.font()), brand::SOFT).size().x
}

pub(crate) fn pill_width(ui: &Ui, count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let dots = count.min(MOST_DOTS) as f32;
    PILL_PAD * 2.0
        + dots * DOT
        + (dots - 1.0) * DOT_GAP
        + DOT_TEXT_GAP
        + label_width(ui, &label(count))
}

/// The pill and, while it is open, the list. Nothing is drawn for no services.
pub(crate) fn pill(ui: &mut Ui, rows: &[Row], memory: &mut Memory) -> Option<Action> {
    if rows.is_empty() {
        return None;
    }
    let (rect, _) =
        ui.allocate_exact_size(vec2(pill_width(ui, rows.len()), PILL_H), Sense::hover());
    let response = ui.interact(rect, ui.id().with("services-pill"), Sense::click());
    ui.ctx().data_mut(|data| {
        data.insert_temp(placed_id(), Placed { pill: Some(rect), buttons: Vec::new() })
    });
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let line = if response.hovered() || open { outline(rows) } else { dim(outline(rows)) };
    let painter = ui.painter();
    painter.rect_stroke(rect, CornerRadius::same(255), Stroke::new(1.0, line), StrokeKind::Inside);
    let mut x = rect.left() + PILL_PAD;
    for row in rows.iter().take(MOST_DOTS) {
        painter.circle_filled(pos2(x + DOT / 2.0, rect.center().y), DOT / 2.0, dot_color(row.word));
        x += DOT + DOT_GAP;
    }
    painter.text(
        pos2(x - DOT_GAP + DOT_TEXT_GAP, rect.center().y),
        Align2::LEFT_CENTER,
        label(rows.len()),
        body(Size::Compact.font()),
        if response.hovered() || open { brand::TEXT } else { brand::SOFT },
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Services of this repo, kept running next to the agents");
    if memory.popover == Popover::Open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    list(&response, rows, memory)
}

fn dim(color: Color32) -> Color32 {
    if color == brand::FIELD_LINE { color } else { color.gamma_multiply(0.7) }
}

fn list(response: &egui::Response, rows: &[Row], memory: &mut Memory) -> Option<Action> {
    let mut action = None;
    let frame = egui::Frame::new()
        .fill(brand::CARD)
        .stroke(Stroke::new(1.0, brand::CARD_LINE))
        .corner_radius(12)
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(110),
        })
        .inner_margin(Margin::same(LIST_MARGIN));
    let menu_before = memory.menu.clone();
    let close_behavior = if menu_before.is_some() {
        PopupCloseBehavior::IgnoreClicks
    } else {
        PopupCloseBehavior::CloseOnClickOutside
    };
    let shown = egui::Popup::menu(response)
        .align(RectAlign::BOTTOM_END)
        .gap(6.0)
        .frame(frame)
        .close_behavior(close_behavior)
        .show(|ui| {
            ui.set_width(LIST_W - f32::from(LIST_MARGIN) * 2.0);
            ui.spacing_mut().item_spacing.y = CARD_GAP;
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            for row in rows {
                if let Some(done) = card(ui, row, memory) {
                    action = Some(done);
                }
            }
            ui.add_space(6.0);
            if quiet_button(ui, "+ Add service", true, Size::Compact).clicked() {
                memory.open_add();
                ui.close();
            }
        });
    let Some(shown) = shown else {
        memory.menu = None;
        memory.menu_rect = None;
        return action;
    };
    if menu_before.is_some() && memory.menu == menu_before {
        let click = response.ctx.input(|i| i.pointer.any_click().then(|| i.pointer.interact_pos()));
        if let Some(pos) = click.flatten()
            && !memory.menu_rect.is_some_and(|rect| rect.contains(pos))
        {
            memory.menu = None;
            if shown.response.clicked_elsewhere() {
                egui::Popup::close_all(&response.ctx);
            }
        }
    }
    if let Some(Action::Logs(_)) = &action {
        egui::Popup::close_all(&response.ctx);
    }
    action
}

fn card(ui: &mut Ui, row: &Row, memory: &mut Memory) -> Option<Action> {
    let selected = memory.selected.as_deref() == Some(row.name.as_str());
    let height = if selected { CARD_H + BUTTONS_H } else { CARD_H };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let body_rect = Rect::from_min_size(rect.min, vec2(rect.width(), CARD_H));
    selection_row(ui.painter(), rect, selected, response.hovered());
    paint_card_text(ui, body_rect, row);
    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        memory.selected = if selected { None } else { Some(row.name.clone()) };
    }
    let mut action = None;
    let more_w = button_width(ui, "...", Size::Compact);
    let more_rect = Rect::from_min_size(
        pos2(body_rect.right() - CARD_PAD + 4.0 - more_w, body_rect.top() + 8.0),
        vec2(more_w, Size::Compact.height()),
    );
    {
        let mut over = ui.new_child(UiBuilder::new().max_rect(more_rect));
        more_menu(&mut over, row, memory);
    }
    if selected {
        let strip = Rect::from_min_size(
            pos2(rect.left() + CARD_PAD, body_rect.bottom()),
            vec2(rect.width() - CARD_PAD * 2.0, BUTTONS_H - 10.0),
        );
        {
            let mut over = ui.new_child(
                UiBuilder::new().max_rect(strip).layout(Layout::left_to_right(Align::Center)),
            );
            let ui = &mut over;
            ui.spacing_mut().item_spacing.x = 8.0;
            {
                let logs = quiet_button(ui, "Logs", true, Size::Compact);
                note_button(ui.ctx(), "Logs", logs.rect);
                if logs.clicked() {
                    action = Some(Action::Logs(row.name.clone()));
                }
                if is_running(row.word) {
                    let restart = quiet_button(ui, "Restart", true, Size::Compact);
                    note_button(ui.ctx(), "Restart", restart.rect);
                    if restart.clicked() {
                        action = Some(Action::Run(Command::Restart, row.name.clone()));
                    }
                    let stop = quiet_button(ui, "Stop", true, Size::Compact);
                    note_button(ui.ctx(), "Stop", stop.rect);
                    if stop.clicked() {
                        action = Some(Action::Run(Command::Stop, row.name.clone()));
                    }
                } else {
                    let start = quiet_button(ui, "Start", true, Size::Compact);
                    note_button(ui.ctx(), "Start", start.rect);
                    if start.clicked() {
                        action = Some(Action::Run(Command::Start, row.name.clone()));
                    }
                }
            }
        }
    }
    action
}

fn more_menu(ui: &mut Ui, row: &Row, memory: &mut Memory) {
    let more = quiet_button(ui, "...", true, Size::Compact).on_hover_text("Edit or remove");
    note_button(ui.ctx(), "...", more.rect);
    let open = memory.menu.as_deref() == Some(row.name.as_str());
    if more.clicked() {
        memory.menu = if open { None } else { Some(row.name.clone()) };
    }
    if memory.menu.as_deref() != Some(row.name.as_str()) {
        return;
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        memory.menu = None;
        return;
    }
    let shown = egui::Popup::from_response(&more)
        .kind(PopupKind::Menu)
        .layout(Layout::top_down_justified(Align::Min))
        .style(menu_style)
        .align(RectAlign::BOTTOM_END)
        .gap(MENU_GAP)
        .close_behavior(PopupCloseBehavior::IgnoreClicks)
        .open(true)
        .show(|ui| {
            ui.set_max_width(MENU_W);
            style_menu(ui);
            let edit = menu_item(ui, "Edit", "");
            note_button(ui.ctx(), "Edit", edit.rect);
            if edit.clicked() {
                memory.dialog = Some(Dialog::Form(Form::edit(row)));
                memory.menu = None;
            }
            let remove = menu_item(ui, "Remove", "");
            note_button(ui.ctx(), "Remove", remove.rect);
            if remove.clicked() {
                memory.dialog = Some(Dialog::Remove(row.name.clone()));
                memory.menu = None;
            }
        });
    memory.menu_rect = shown.map(|shown| shown.response.rect);
}

fn paint_card_text(ui: &Ui, rect: Rect, row: &Row) {
    let painter = ui.painter();
    let left = rect.left() + CARD_PAD;
    let dot_x = left + DOT / 2.0;
    let name_y = rect.top() + 10.0;
    painter.circle_filled(pos2(dot_x, name_y + 9.0), DOT / 2.0, dot_color(row.word));
    let text_left = left + DOT + 10.0;
    let room = (rect.right() - CARD_PAD - text_left).max(1.0);
    painter.text(
        pos2(text_left, name_y),
        Align2::LEFT_TOP,
        &row.name,
        body(TEXT_BODY),
        brand::TEXT,
    );
    painter.text(
        pos2(text_left, name_y + 20.0),
        Align2::LEFT_TOP,
        &row.line,
        body(TEXT_TAG + 0.5),
        line_ink(row.word),
    );
    let galley = painter.layout_job(cut_to(&row.command, room));
    painter.galley(pos2(text_left, name_y + 40.0), galley, brand::FAINT);
}

fn cut_to(text: &str, width: f32) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat { font_id: mono(TEXT_TAG), color: brand::FAINT, ..TextFormat::default() },
    );
    job.wrap = TextWrapping {
        max_width: width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    job
}

/// A form's service, or each problem found.
pub(crate) fn form_service(form: &Form, existing: &[Row]) -> Result<Service, Vec<String>> {
    let service = Service::new(&form.name, &form.command, form.autostart);
    let others: Vec<Service> =
        existing.iter().map(|row| Service::new(&row.name, &row.command, row.autostart)).collect();
    let found = problems(&service, &others, form.editing.as_deref());
    if found.is_empty() { Ok(service) } else { Err(found) }
}

pub(crate) fn dialog(ctx: &egui::Context, memory: &mut Memory, existing: &[Row]) -> Option<Action> {
    let dialog = memory.dialog.take()?;
    let (next, action) = match dialog {
        Dialog::Form(form) => form_dialog(ctx, form, existing),
        Dialog::Remove(name) => remove_dialog(ctx, name),
    };
    memory.dialog = next;
    action
}

fn form_dialog(
    ctx: &egui::Context,
    mut form: Form,
    existing: &[Row],
) -> (Option<Dialog>, Option<Action>) {
    let title = if form.editing.is_some() { "Edit service" } else { "Add service" };
    let mut submit = false;
    let mut cancel = false;
    let verdict = form_service(&form, existing);
    let modal = dialog_frame(ctx, Id::new("service-form"), title, DIALOG_W, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        section_label(ui, "Name");
        let name = text_input(ui, &mut form.name, "db");
        if std::mem::take(&mut form.focus) {
            name.request_focus();
        }
        submit |= name.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        ui.add_space(4.0);
        section_label(ui, "Command");
        let command = text_input(ui, &mut form.command, "pscale connect shop main --port 3309");
        submit |= command.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        ui.add_space(4.0);
        ui.add_space(6.0);
        switch_line(ui, "Start with the repo", &mut form.autostart);
        if let (true, Err(found)) = (form.tried, &verdict) {
            ui.add_space(4.0);
            for problem in found {
                ui.label(egui::RichText::new(problem).size(TEXT_TAG).color(brand::RED));
            }
        }
        ui.add_space(6.0);
        footer_row(ui, "", |ui| {
            if primary_button_keyed(ui, "Save", "\u{21b5}", true, Size::Regular).clicked() {
                submit = true;
            }
            if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
                cancel = true;
            }
        });
        key_hint(ui, "Save adds the service, Start runs it  -  Esc cancels");
    });
    if cancel || modal.should_close() {
        return (None, None);
    }
    if !submit {
        return (Some(Dialog::Form(form)), None);
    }
    let Ok(service) = verdict else {
        form.tried = true;
        return (Some(Dialog::Form(form)), None);
    };
    (None, Some(Action::Save { editing: form.editing, service }))
}

fn remove_dialog(ctx: &egui::Context, name: String) -> (Option<Dialog>, Option<Action>) {
    let mut confirm = false;
    let mut cancel = false;
    let modal = dialog_frame(ctx, Id::new("service-remove"), "Remove service", DIALOG_W, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(
            egui::RichText::new(format!("Remove {name}?")).font(body(TEXT_BODY)).color(brand::TEXT),
        );
        ui.label(
            egui::RichText::new("Tidewatch stops it and forgets it. Your repo is not touched.")
                .size(TEXT_TAG)
                .color(brand::MUTED),
        );
        ui.add_space(6.0);
        footer_row(ui, "", |ui| {
            if danger_button(ui, "Remove", Size::Regular).clicked() {
                confirm = true;
            }
            if quiet_button(ui, "Cancel", true, Size::Regular).clicked() {
                cancel = true;
            }
        });
    });
    if confirm {
        return (None, Some(Action::Remove(name)));
    }
    if cancel || modal.should_close() {
        return (None, None);
    }
    (Some(Dialog::Remove(name)), None)
}

#[cfg(test)]
mod tests;
