//! The settings view: a sidebar of pages and the page that shows. It replaces the terminal area
//! while open, like a settings tab. Every page is drawn from `settings_ui`; each file a page
//! edits is named in a quiet footer or a tooltip, never at the top.

use egui::{
    Align, Align2, CornerRadius, FontId, Frame, Key, Layout, Margin, Modifiers, Rect, ScrollArea,
    Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};

use crate::general_view::{self, General};
use crate::house_style_view;
use crate::login_item;
use crate::macos::LoginItem;
use crate::profiles_view::{self, Probes};
use crate::settings::{Section, Settings};
use crate::settings_icons;
use crate::settings_ui::{
    FAINT, GROUND, ORANGE, SIDEBAR, SIDEBAR_LINE, SOFT, TEAL_NAV, TEAL_TEXT, TEXT, font_semibold,
};
use crate::shortcuts_view;
use crate::trust_view;
use crate::updates::Updates;
use crate::workflow_editor_view;

const SIDEBAR_WIDTH: f32 = 232.0;
const ITEM_HEIGHT: f32 = 38.0;
const CONTENT_MAX: f32 = 980.0;

pub(crate) enum Action {
    Save,
    Close,
    Trust(tidewatch_core::consent::Event),
    /// The restore switch moved: the new value. It applies at once, so there is nothing to save.
    Restore(bool),
    ResumeAgents(tidewatch_core::prefs::ResumeAgents),
    /// The Window section changed something: all of its settings. Applies at once too.
    Window(tidewatch_core::prefs::WindowPrefs),
    /// An Updates switch moved: the whole table. Applies at once too.
    Updates(tidewatch_core::prefs::UpdatePrefs),
    LoginItem(login_item::Click),
    OpenHouseStyle,
    ResetHouseStyle,
}

#[derive(Default)]
pub(crate) struct Files {
    pub(crate) profiles: String,
    pub(crate) workflows: String,
    pub(crate) trust: String,
    pub(crate) general: String,
    pub(crate) house_style: String,
}

pub(crate) struct Host<'a> {
    pub(crate) files: &'a Files,
    pub(crate) trust: &'a trust_view::Page<'a>,
    pub(crate) general: &'a General,
    pub(crate) probes: &'a Probes,
    pub(crate) read_login: fn() -> LoginItem,
    /// Esc closes the settings: false while a dialog over them owns the key.
    pub(crate) esc_closes: bool,
}

pub(crate) fn show(
    ui: &mut Ui,
    area: Rect,
    settings: &mut Settings,
    host: &Host,
    updates: &mut Updates,
) -> Option<Action> {
    let typing_id = egui::Id::new("settings_typing");
    let typing = ui.ctx().data(|data| data.get_temp::<bool>(typing_id)).unwrap_or(false);
    ui.painter().rect_filled(area, CornerRadius::ZERO, GROUND);
    let sidebar_rect = Rect::from_min_size(area.min, vec2(SIDEBAR_WIDTH, area.height()));
    sidebar(ui, sidebar_rect, settings, updates);
    let main = Rect::from_min_max(pos2(sidebar_rect.right(), area.top()), area.max);
    let action = page(ui, main, settings, host, updates);
    let focused = ui.ctx().text_edit_focused();
    ui.ctx().data_mut(|data| data.insert_temp(typing_id, focused));
    let escape = host.esc_closes
        && !typing
        && ui.ctx().input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    action.or_else(|| escape.then_some(Action::Close))
}

fn page(
    ui: &mut Ui,
    main: Rect,
    settings: &mut Settings,
    host: &Host,
    updates: &mut Updates,
) -> Option<Action> {
    let section = settings.section;
    match section {
        Section::Profiles => {
            profiles_view::page(ui, main, &mut settings.profiles, host.probes, &host.files.profiles)
        }
        Section::Workflows => {
            workflow_editor_view::page(ui, main, &mut settings.workflows, &host.files.workflows)
        }
        Section::General => scrolling(ui, main, section, |ui| {
            general_view::page(
                ui,
                host.general,
                &mut settings.window,
                updates,
                &mut settings.login,
                host.read_login,
                &host.files.general,
            )
        }),
        Section::Trust => scrolling(ui, main, section, |ui| {
            trust_view::settings_page(ui, host.trust, &host.files.trust).map(Action::Trust)
        }),
        Section::Shortcuts => scrolling(ui, main, section, |ui| {
            shortcuts_view::page(ui);
            None
        }),
        Section::HouseStyle => {
            scrolling(ui, main, section, |ui| house_style_view::page(ui, &host.files.house_style))
        }
    }
}

fn scrolling(
    ui: &mut Ui,
    main: Rect,
    section: Section,
    add: impl FnOnce(&mut Ui) -> Option<Action>,
) -> Option<Action> {
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(main).layout(Layout::top_down(Align::Min)));
    ScrollArea::vertical()
        .id_salt(format!("settings_page_{section:?}"))
        .auto_shrink(false)
        .show(&mut child, |ui| {
            Frame::new()
                .inner_margin(Margin { left: 40, right: 40, top: 28, bottom: 28 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().min(CONTENT_MAX));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    add(ui)
                })
                .inner
        })
        .inner
}

fn sidebar(ui: &mut Ui, rect: Rect, settings: &mut Settings, updates: &Updates) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, SIDEBAR);
    painter.vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, SIDEBAR_LINE));
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(14.0, 22.0)))
            .layout(Layout::top_down(Align::Min)),
    );
    inner.spacing_mut().item_spacing.y = 2.0;
    let (title, _) = inner.allocate_exact_size(vec2(inner.available_width(), 36.0), Sense::hover());
    let galley = inner.painter().layout_no_wrap("Settings".to_owned(), font_semibold(20.0), TEXT);
    inner.painter().galley(title.min + vec2(10.0, 0.0), galley, TEXT);
    for (section, label, unsaved) in settings.sections() {
        if item(&mut inner, section, label, settings.section == section, unsaved).clicked() {
            settings.section = section;
        }
    }
    let version = format!("Tidewatch {}", env!("CARGO_PKG_VERSION"));
    let left = rect.left() + 14.0 + 10.0;
    let painter = ui.painter();
    let status = painter.text(
        pos2(left, rect.bottom() - 22.0),
        Align2::LEFT_BOTTOM,
        updates.short_status(),
        FontId::proportional(12.0),
        FAINT,
    );
    painter.text(
        pos2(left, status.top() - 2.0),
        Align2::LEFT_BOTTOM,
        version,
        FontId::proportional(12.0),
        FAINT,
    );
}

fn item(
    ui: &mut Ui,
    section: Section,
    label: &str,
    selected: bool,
    unsaved: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ITEM_HEIGHT), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(8), TEAL_NAV);
    } else if response.hovered() {
        painter.rect_filled(
            rect,
            CornerRadius::same(8),
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 8),
        );
    }
    let (text, icon) = if selected { (TEXT, TEAL_TEXT) } else { (SOFT, SOFT) };
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0));
    settings_icons::paint(painter, icon_rect, section, icon);
    painter.text(
        pos2(rect.left() + 44.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        text,
    );
    if unsaved {
        painter.circle_filled(pos2(rect.right() - 14.0, rect.center().y), 3.5, ORANGE);
    }
    kelp_ui::widgets::focus_ring(ui, &response, 8.0);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests;
