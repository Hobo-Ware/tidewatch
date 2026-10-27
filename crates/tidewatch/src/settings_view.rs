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
use crate::screen_kit::TEXT_BODY;
use crate::screen_kit::selection_row;
use crate::settings::{Section, Settings};
use crate::settings_icons;
use crate::settings_ui::{
    FAINT, GROUND, HEADER_H, ORANGE, SIDEBAR, SIDEBAR_LINE, SOFT, TEAL_TEXT, TEXT, font_semibold,
};
use crate::shortcuts_view;
use crate::trust_view;
use crate::updates::Updates;
use crate::worktree_setup_view;

const SIDEBAR_WIDTH: f32 = 248.0;
const ITEM_HEIGHT: f32 = 44.0;
const ITEM_ICON: f32 = 20.0;
const CONTENT_MAX: f32 = 820.0;
const CONTENT_MIN_SIDE: f32 = 40.0;

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
    /// The New tabs card changed something: all of its choices. Applies at once too.
    NewTab(tidewatch_core::prefs::NewTabPrefs),
    Tabs(tidewatch_core::prefs::TabsPrefs),
    /// The Git card changed something: both of its choices. Applies at once too.
    Git(tidewatch_core::prefs::GitPrefs),
    /// The Done check switch moved: the new value. Applies at once too.
    DoneCheck(bool),
    /// The Chat switch for animated emoji moved: the new value. Applies at once too.
    AnimateEmoji(bool),
    KeepAwake(bool),
    /// Set up or Remove on the Touch ID row of the Keep awake card.
    TouchId(crate::touch_id_view::Click),
    /// The GitHub card changed something: all of its choices. Applies at once too.
    Github(tidewatch_core::prefs::GithubPrefs),
    /// A Guards switch or the private repo list changed. Applies at once too.
    Guards(crate::guards_view::Change),
    /// A font was picked for the terminal. Applies at once too.
    TerminalFont(tidewatch_core::prefs::FontChoice),
    TerminalCopyOnSelect(bool),
    LoginItem(login_item::Click),
    /// The Sync card: the switch, the repo, Sync now or Create private repo.
    Sync(crate::sync_view::Change),
    OpenHouseStyle,
    ResetHouseStyle,
    /// Edit on the Checklist card: open the list's file.
    EditChecklist,
    /// The Worktree setup page changed a repo's recipe: all of it. Applies at once too.
    WorktreeSetup {
        repo: std::path::PathBuf,
        recipe: tidewatch_core::setup_recipe::Recipe,
    },
}

#[derive(Default)]
pub(crate) struct Files {
    pub(crate) profiles: String,
    pub(crate) trust: String,
    pub(crate) general: String,
    pub(crate) house_style: String,
    pub(crate) worktree_setup: String,
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
        Section::General => scrolling(ui, main, section, |ui| {
            general_view::page(
                ui,
                host.general,
                &mut settings.window,
                &mut settings.font,
                updates,
                &mut settings.login,
                &mut settings.sync,
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
        Section::WorktreeSetup => scrolling(ui, main, section, |ui| {
            worktree_setup_view::page(ui, &mut settings.setup, &host.files.worktree_setup)
        }),
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
            let side = content_side(main.width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(side);
                ui.vertical(|ui| {
                    Frame::new()
                        .inner_margin(Margin { left: 0, right: 0, top: 0, bottom: 28 })
                        .show(ui, |ui| {
                            ui.set_width((main.width() - 2.0 * side).min(CONTENT_MAX));
                            ui.spacing_mut().item_spacing.y = 0.0;
                            add(ui)
                        })
                        .inner
                })
                .inner
            })
            .inner
        })
        .inner
}

fn content_side(width: f32) -> f32 {
    ((width - CONTENT_MAX) / 2.0).max(CONTENT_MIN_SIDE)
}

fn sidebar(ui: &mut Ui, rect: Rect, settings: &mut Settings, updates: &Updates) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, SIDEBAR);
    painter.vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, SIDEBAR_LINE));
    painter.hline(rect.x_range(), rect.top() + HEADER_H, Stroke::new(1.0, SIDEBAR_LINE));
    painter.text(
        rect.left_top() + vec2(24.0, HEADER_H / 2.0),
        Align2::LEFT_CENTER,
        "Settings",
        font_semibold(16.0),
        TEXT,
    );
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(rect.left() + 14.0, rect.top() + HEADER_H + 14.0),
                rect.max - vec2(14.0, 22.0),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    inner.spacing_mut().item_spacing.y = 2.0;
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
        FontId::proportional(13.0),
        FAINT,
    );
    painter.text(
        pos2(left, status.top() - 2.0),
        Align2::LEFT_BOTTOM,
        version,
        FontId::proportional(13.0),
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
    selection_row(painter, rect, selected, response.hovered());
    let (text, icon) = if selected { (TEXT, TEAL_TEXT) } else { (SOFT, SOFT) };
    let icon_rect = Rect::from_center_size(
        pos2(rect.left() + 12.0 + ITEM_ICON / 2.0, rect.center().y),
        vec2(ITEM_ICON, ITEM_ICON),
    );
    settings_icons::paint(painter, icon_rect, section, icon);
    painter.text(
        pos2(rect.left() + 44.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(TEXT_BODY),
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
