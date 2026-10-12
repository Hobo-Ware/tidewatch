//! The `eframe::App`: tab strip on top, the active tab's terminal below it, then the two
//! placeholder panels. Frame work collects `tabs::Action`s and applies them after drawing.

use egui::{CornerRadius, Frame, Key, Margin, Modifiers, RichText};
use stdusk_core::terminal::{PtyTerm, SpawnOpts};
use stdusk_egui::fonts::build_fonts;
use stdusk_egui::ui::{
    CursorStyle, GridStyle, TAB_H, apply_theme, collect_input, draw_tab, icons, render_grid,
};
use stdusk_egui::widgets::icon_button;
use stdusk_egui::{colors, repaint_handle};

use crate::dev_screenshot::DevScreenshot;
use crate::layout;
use crate::panels;
use crate::repo::Repo;
use crate::tabs::{Action, Tabs};

const DEFAULT_THEME: &str = "one-half-dark";
const FONT_SIZE: f32 = 13.0;
const INITIAL_COLS: usize = 80;
const INITIAL_ROWS: usize = 24;
const SCROLLBACK_LINES: usize = 25_000;
const WORD_SEPARATORS: &str = " ()[]{}'\"";
const MIN_CONTRAST: f32 = 4.0;
const STRIP_TOP_MARGIN: i8 = 6;
const STRIP_SIDE_MARGIN: i8 = 8;
const STRIP_GAP: f32 = 4.0;
const STRIP_CORNER: u8 = 10;

pub(crate) struct Tidewatch {
    repo: Repo,
    tabs: Tabs<PtyTerm>,
    screenshot: Option<DevScreenshot>,
}

impl Tidewatch {
    pub(crate) fn new(
        cc: &eframe::CreationContext<'_>,
        repo: Repo,
        screenshot: Option<DevScreenshot>,
    ) -> Self {
        colors::init(colors::by_name(DEFAULT_THEME));
        cc.egui_ctx.set_fonts(build_fonts(None, None));
        apply_theme(&cc.egui_ctx);
        if screenshot.is_some() {
            cc.egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
        }
        let main = spawn_shell(&cc.egui_ctx, &repo);
        Self { repo, tabs: Tabs::new(main), screenshot }
    }
}

fn spawn_shell(ctx: &egui::Context, repo: &Repo) -> PtyTerm {
    let opts = SpawnOpts {
        detect_progress: true,
        shell_integration: true,
        autosuggestions: false,
        scrollback_lines: SCROLLBACK_LINES,
        word_separators: WORD_SEPARATORS.into(),
        bold_bright: true,
        cwd: Some(repo.root.to_string_lossy().into_owned()),
        profile: None,
        agent_tracking: None,
    };
    PtyTerm::spawn(INITIAL_COLS, INITIAL_ROWS, repaint_handle(ctx), &opts)
}

fn shortcut_actions(ctx: &egui::Context) -> Vec<Action> {
    ctx.input_mut(|i| {
        let mut actions = Vec::new();
        if i.consume_key(Modifiers::COMMAND, Key::T) {
            actions.push(Action::New);
        }
        if i.consume_key(Modifiers::COMMAND, Key::W) {
            actions.push(Action::CloseActive);
        }
        for key in [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ] {
            if let Some(idx) = layout::digit_index(key)
                && i.consume_key(Modifiers::COMMAND, key)
            {
                actions.push(Action::Select(idx));
            }
        }
        actions
    })
}

impl eframe::App for Tidewatch {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut actions = shortcut_actions(&ctx);

        egui::Panel::top("tabbar")
            .exact_size(TAB_H + f32::from(STRIP_TOP_MARGIN))
            .frame(
                Frame::new()
                    .fill(colors::titlebar())
                    .corner_radius(CornerRadius {
                        nw: STRIP_CORNER,
                        ne: STRIP_CORNER,
                        sw: 0,
                        se: 0,
                    })
                    .inner_margin(Margin {
                        left: STRIP_SIDE_MARGIN,
                        right: STRIP_SIDE_MARGIN,
                        top: STRIP_TOP_MARGIN,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(TAB_H);
                    ui.spacing_mut().item_spacing.x = STRIP_GAP;
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(&self.repo.label).monospace().size(12.0).color(colors::dim()),
                    );
                    ui.add_space(8.0);
                    let width = layout::tab_width(
                        ui.available_width() - stdusk_egui::widgets::ICON_BTN_W - 12.0,
                        self.tabs.items().len(),
                        STRIP_GAP,
                    );
                    for (i, tab) in self.tabs.items().iter().enumerate() {
                        let (resp, close) = draw_tab(
                            ui,
                            Some(i + 1),
                            tab.id,
                            &tab.title,
                            i == self.tabs.active(),
                            tab.pinned,
                            None,
                            tab.pane.progress(),
                            tab.pane.cmd_state(),
                            &[],
                            None,
                            Some(width),
                        );
                        if close {
                            actions.push(Action::Close(i));
                        } else if resp.clicked() {
                            actions.push(Action::Select(i));
                        }
                    }
                    ui.add_space(6.0);
                    if icon_button(ui, icons::PLUS, "New tab (Cmd+T)").clicked() {
                        actions.push(Action::New);
                    }
                });
            });

        egui::CentralPanel::default().frame(Frame::new().fill(colors::bg())).show(ui, |ui| {
            let areas = layout::split(ui.max_rect());
            let font = egui::FontId::monospace(FONT_SIZE);
            let glyph = ui.painter().layout_no_wrap("M".to_owned(), font.clone(), colors::fg());
            let (cell_w, cell_h) = (glyph.size().x, glyph.size().y);
            let (cols, rows) = layout::grid_dims(areas.terminal, cell_w, cell_h);

            let term = self.tabs.active_pane_mut();
            term.resize(cols, rows);
            let typed = collect_input(ui, false, false, term.app_cursor());
            if !typed.is_empty() {
                term.send(&typed);
            }
            let snap = term.grid_snapshot();
            render_grid(
                ui,
                &[],
                areas.terminal,
                term,
                &snap,
                cell_w,
                cell_h,
                &font,
                None,
                GridStyle {
                    cursor: CursorStyle::Block,
                    dimmed: false,
                    link_active: false,
                    blink: false,
                    ligatures: false,
                    min_contrast: MIN_CONTRAST,
                    app_mouse: false,
                },
                &[],
            );

            panels::placeholder(ui, areas.history, "History");
            panels::placeholder(ui, areas.diff, "Diff");
        });

        for action in actions {
            self.tabs.apply(action, || spawn_shell(&ctx, &self.repo));
        }
        if let Some(shot) = &mut self.screenshot {
            shot.tick(&ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let bg = colors::bg();
        egui::Rgba::from(bg).to_array()
    }
}
