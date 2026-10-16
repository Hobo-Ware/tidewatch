//! The window's frame. On Linux the system's title bar (GNOME draws a grey Adwaita bar over a
//! Wayland client) clashes with the Night tide strip under it, so Tidewatch draws its own: the tab
//! strip is the title bar (drag to move, double-click to maximize), the window buttons sit at its
//! right end, thin edges resize and a 1 px line marks the window. macOS keeps its native title
//! bar, and the quake drop-down has no frame at all.

use egui::viewport::ResizeDirection;
use egui::{
    Context, CursorIcon, Id, LayerId, Order, PointerButton, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, UiBuilder, ViewportCommand, pos2, vec2,
};

use crate::brand;

/// How the window is framed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chrome {
    /// The system's title bar and borders.
    Native,
    /// No system frame: the strip is the title bar, with Tidewatch's own buttons and edges.
    Drawn,
    /// No frame and nothing in its place (the quake drop-down).
    Borderless,
}

impl Chrome {
    pub(crate) fn pick(draws_own_frame: bool, quake: bool) -> Self {
        match (quake, draws_own_frame) {
            (true, _) => Self::Borderless,
            (false, true) => Self::Drawn,
            (false, false) => Self::Native,
        }
    }

    /// Whether the system draws a title bar and borders.
    pub(crate) fn decorations(self) -> bool {
        self == Self::Native
    }
}

/// How far into the window an edge still resizes it.
const GRIP: f32 = 5.0;
/// How far along each side a corner reaches.
const CORNER: f32 = 14.0;

/// The resize zones along the window's border: four edges and, at each corner, an L of two arms
/// `CORNER` long. They never overlap and leave the inside of the window alone.
fn edge_zones(window: Rect) -> [(ResizeDirection, Rect); 12] {
    use ResizeDirection::{East, North, NorthEast, NorthWest, South, SouthEast, SouthWest, West};
    let (l, r, t, b) = (window.left(), window.right(), window.top(), window.bottom());
    let zone = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    [
        (NorthWest, zone(l, t, l + CORNER, t + GRIP)),
        (NorthWest, zone(l, t + GRIP, l + GRIP, t + CORNER)),
        (North, zone(l + CORNER, t, r - CORNER, t + GRIP)),
        (NorthEast, zone(r - CORNER, t, r, t + GRIP)),
        (NorthEast, zone(r - GRIP, t + GRIP, r, t + CORNER)),
        (East, zone(r - GRIP, t + CORNER, r, b - CORNER)),
        (SouthEast, zone(r - CORNER, b - GRIP, r, b)),
        (SouthEast, zone(r - GRIP, b - CORNER, r, b - GRIP)),
        (South, zone(l + CORNER, b - GRIP, r - CORNER, b)),
        (SouthWest, zone(l, b - GRIP, l + CORNER, b)),
        (SouthWest, zone(l, b - CORNER, l + GRIP, b - GRIP)),
        (West, zone(l, t + CORNER, l + GRIP, b - CORNER)),
    ]
}

/// A window button the desktop can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Minimize,
    Maximize,
    Close,
}

/// What a double click on the title bar does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DoubleClick {
    ToggleMaximize,
    Minimize,
    Nothing,
}

/// The desktop's title bar settings, so the drawn bar behaves like every other window's: GNOME's
/// `button-layout` (Fedora shows only Close) and `action-double-click-titlebar`. Anywhere else, or
/// when they cannot be read: minimize, maximize and close, and a double click maximizes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Desktop {
    buttons: Vec<Kind>,
    double_click: DoubleClick,
}

impl Default for Desktop {
    fn default() -> Self {
        Self {
            buttons: vec![Kind::Minimize, Kind::Maximize, Kind::Close],
            double_click: DoubleClick::ToggleMaximize,
        }
    }
}

impl Desktop {
    /// Reads GNOME's settings once, before the window opens. Without `gsettings` (or off Linux)
    /// the defaults stand.
    pub(crate) fn read() -> Self {
        let get = |key: &str| {
            let output = std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.wm.preferences", key])
                .output()
                .ok()
                .filter(|output| output.status.success())?;
            String::from_utf8(output.stdout).ok()
        };
        Self::from_settings(
            get("button-layout").as_deref(),
            get("action-double-click-titlebar").as_deref(),
        )
    }

    /// `layout` is GNOME's `left:right` list (`'appmenu:minimize,maximize,close'`); the buttons
    /// it names keep their order and all sit at the strip's right end. Names it does not know
    /// (`appmenu`, `spacer`) are skipped, a layout with none of the three shows none.
    fn from_settings(layout: Option<&str>, double_click: Option<&str>) -> Self {
        let defaults = Self::default();
        let unquote = |value: &str| value.trim().trim_matches('\'').to_owned();
        let buttons = layout.map(unquote).map_or(defaults.buttons, |layout| {
            let mut buttons = Vec::new();
            for name in layout.split([':', ',']).map(str::trim) {
                let kind = match name {
                    "minimize" => Kind::Minimize,
                    "maximize" => Kind::Maximize,
                    "close" => Kind::Close,
                    _ => continue,
                };
                if !buttons.contains(&kind) {
                    buttons.push(kind);
                }
            }
            buttons
        });
        let double_click = match double_click.map(unquote).as_deref() {
            Some("minimize") => DoubleClick::Minimize,
            Some("none" | "lower" | "menu") => DoubleClick::Nothing,
            _ => DoubleClick::ToggleMaximize,
        };
        Self { buttons, double_click }
    }

    /// The room the window buttons take at the strip's right end, with the divider before them.
    pub(crate) fn controls_width(&self) -> f32 {
        if self.buttons.is_empty() { 0.0 } else { DIVIDER_W + BUTTON_W * self.buttons.len() as f32 }
    }

    fn row(&self, maximized: bool) -> impl Iterator<Item = Button> + '_ {
        self.buttons.iter().map(move |kind| match kind {
            Kind::Minimize => Button::Minimize,
            Kind::Maximize if maximized => Button::Restore,
            Kind::Maximize => Button::Maximize,
            Kind::Close => Button::Close,
        })
    }
}

/// A window button as drawn: maximize turns into restore while the window is maximized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl Button {
    fn command(self) -> ViewportCommand {
        match self {
            Self::Minimize => ViewportCommand::Minimized(true),
            Self::Maximize => ViewportCommand::Maximized(true),
            Self::Restore => ViewportCommand::Maximized(false),
            Self::Close => ViewportCommand::Close,
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Self::Minimize => "Minimize",
            Self::Maximize => "Maximize",
            Self::Restore => "Restore",
            Self::Close => "Close",
        }
    }
}

fn cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
    }
}

const BUTTON_W: f32 = stdusk_egui::widgets::ICON_BTN_W;
const BUTTON_H: f32 = 30.0;
const DIVIDER_W: f32 = 13.0;
const DIVIDER_H: f32 = 16.0;
const GLYPH: f32 = 10.0;
const GLYPH_STROKE: f32 = 1.3;

fn maximized(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// The strip's empty space as the title bar: a drag moves the window, a double click does what the
/// desktop says. Called before the strip's widgets, so they sit on top and keep their clicks.
pub(crate) fn title_bar(ui: &Ui, rect: Rect, desktop: &Desktop) {
    let bar = ui.interact(rect, Id::new("window_title_bar"), Sense::click_and_drag());
    if bar.double_clicked() {
        let command = match desktop.double_click {
            DoubleClick::ToggleMaximize => Some(ViewportCommand::Maximized(!maximized(ui.ctx()))),
            DoubleClick::Minimize => Some(ViewportCommand::Minimized(true)),
            DoubleClick::Nothing => None,
        };
        if let Some(command) = command {
            ui.ctx().send_viewport_cmd(command);
        }
    } else if bar.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// The desktop's window buttons after a short divider. Each looks like the strip's icon buttons;
/// close turns red under the pointer.
pub(crate) fn controls(ui: &mut Ui, desktop: &Desktop) {
    let width = desktop.controls_width();
    if width == 0.0 {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(vec2(width, BUTTON_H), Sense::hover());
    let painter = ui.painter();
    let x = rect.left() + DIVIDER_W / 2.0;
    let half = DIVIDER_H / 2.0;
    painter.vline(x, rect.center().y - half..=rect.center().y + half, (1.0, brand::SIDEBAR_LINE));
    for (i, button) in desktop.row(maximized(ui.ctx())).enumerate() {
        let min = pos2(rect.left() + DIVIDER_W + BUTTON_W * i as f32, rect.top());
        let slot = Rect::from_min_size(min, vec2(BUTTON_W, BUTTON_H));
        let resp = ui.interact(slot, Id::new(("window_button", i)), Sense::click());
        let hovered = resp.hovered();
        let (fill, ink) = match (button, hovered) {
            (Button::Close, true) => (brand::RED_FILL, brand::RED),
            (_, true) => (brand::BUTTON, brand::TEXT),
            (_, false) => (egui::Color32::TRANSPARENT, brand::MUTED),
        };
        painter.rect_filled(slot, 6.0, fill);
        paint_glyph(painter, slot.center(), button, ink);
        if resp.on_hover_text(button.tip()).clicked() {
            ui.ctx().send_viewport_cmd(button.command());
        }
    }
}

fn paint_glyph(painter: &egui::Painter, center: Pos2, button: Button, ink: egui::Color32) {
    let stroke = Stroke::new(GLYPH_STROKE, ink);
    let h = GLYPH / 2.0;
    let at = |x: f32, y: f32| center + vec2(x, y);
    match button {
        Button::Minimize => {
            painter.line_segment([at(-h, 0.5), at(h, 0.5)], stroke);
        }
        Button::Maximize => {
            let square = Rect::from_center_size(center, vec2(GLYPH, GLYPH));
            painter.rect_stroke(square, 1.5, stroke, StrokeKind::Middle);
        }
        Button::Restore => {
            let front = Rect::from_min_max(at(-h, -h + 2.5), at(h - 2.5, h));
            painter.rect_stroke(front, 1.5, stroke, StrokeKind::Middle);
            let back = vec![
                at(-h + 2.5, -h + 2.5),
                at(-h + 2.5, -h),
                at(h, -h),
                at(h, h - 2.5),
                at(h - 2.5, h - 2.5),
            ];
            painter.add(Shape::line(back, stroke));
        }
        Button::Close => {
            painter.line_segment([at(-h, -h), at(h, h)], stroke);
            painter.line_segment([at(-h, h), at(h, -h)], stroke);
        }
    }
}

/// The window's edges while it is neither maximized nor full screen: a 1 px line, and the zones
/// that resize it, on top of everything so a press there never reaches the terminal under them.
pub(crate) fn edges(ctx: &Context) {
    let full = ctx.input(|i| {
        let viewport = i.viewport();
        viewport.maximized.unwrap_or(false) || viewport.fullscreen.unwrap_or(false)
    });
    if full {
        return;
    }
    let window = ctx.content_rect();
    let layer = LayerId::new(Order::Foreground, Id::new("window_edges"));
    ctx.layer_painter(layer).rect_stroke(
        window,
        0.0,
        Stroke::new(1.0, brand::SIDEBAR_LINE),
        StrokeKind::Inside,
    );
    let ui = Ui::new(
        ctx.clone(),
        Id::new("window_edges"),
        UiBuilder::new().layer_id(layer).max_rect(window),
    );
    for (i, (direction, zone)) in edge_zones(window).into_iter().enumerate() {
        let resp = ui.interact(zone, Id::new(("window_edge", i)), Sense::drag());
        if resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
        let _ = resp.on_hover_cursor(cursor(direction));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_linux_outside_quake_draws_its_own_frame() {
        let cases = [
            (true, false, Chrome::Drawn),
            (false, false, Chrome::Native),
            (true, true, Chrome::Borderless),
            (false, true, Chrome::Borderless),
        ];
        for (own, quake, want) in cases {
            assert_eq!(Chrome::pick(own, quake), want, "own frame {own}, quake {quake}");
        }
        assert!(Chrome::Native.decorations());
        assert!(!Chrome::Drawn.decorations());
        assert!(!Chrome::Borderless.decorations());
    }

    const WINDOW: Rect = Rect { min: Pos2::new(0.0, 0.0), max: Pos2::new(800.0, 600.0) };

    fn at(pos: Pos2) -> Vec<ResizeDirection> {
        edge_zones(WINDOW)
            .into_iter()
            .filter(|(_, zone)| zone.contains(pos))
            .map(|(direction, _)| direction)
            .collect()
    }

    #[test]
    fn each_edge_and_corner_resizes_its_own_way() {
        use ResizeDirection::{
            East, North, NorthEast, NorthWest, South, SouthEast, SouthWest, West,
        };
        let cases = [
            ((400.0, 1.0), North),
            ((400.0, 598.5), South),
            ((1.0, 300.0), West),
            ((799.0, 300.0), East),
            ((2.0, 2.0), NorthWest),
            ((10.0, 1.0), NorthWest),
            ((1.0, 10.0), NorthWest),
            ((798.0, 2.0), NorthEast),
            ((799.0, 12.0), NorthEast),
            ((798.0, 598.0), SouthEast),
            ((790.0, 599.0), SouthEast),
            ((1.0, 599.0), SouthWest),
            ((2.0, 590.0), SouthWest),
        ];
        for ((x, y), want) in cases {
            assert_eq!(at(pos2(x, y)), vec![want], "at {x},{y}");
        }
    }

    #[test]
    fn the_inside_of_the_window_never_resizes() {
        for pos in [pos2(400.0, 300.0), pos2(GRIP + 1.0, GRIP + 1.0), pos2(20.0, 20.0)] {
            assert_eq!(at(pos), vec![], "at {pos:?}");
        }
    }

    #[test]
    fn the_whole_border_band_resizes_and_no_point_is_in_two_zones() {
        let mut y = 0.25;
        while y < WINDOW.height() {
            let mut x = 0.25;
            while x < WINDOW.width() {
                let p = pos2(x, y);
                let in_band = x < GRIP || y < GRIP || x > 800.0 - GRIP || y > 600.0 - GRIP;
                let hits = at(p).len();
                assert_eq!(hits, usize::from(in_band), "at {p:?}");
                x += if (GRIP..800.0 - GRIP).contains(&x) { 37.0 } else { 0.5 };
            }
            y += if (GRIP..600.0 - GRIP).contains(&y) { 29.0 } else { 0.5 };
        }
    }

    #[test]
    fn maximize_turns_into_restore_on_a_maximized_window() {
        let desktop = Desktop::default();
        let row = |maximized| desktop.row(maximized).collect::<Vec<_>>();
        assert_eq!(row(false), [Button::Minimize, Button::Maximize, Button::Close]);
        assert_eq!(row(true), [Button::Minimize, Button::Restore, Button::Close]);
    }

    #[test]
    fn gnomes_button_layout_picks_the_buttons_and_their_order() {
        use Kind::{Close, Maximize, Minimize};
        let cases: [(Option<&str>, Vec<Kind>); 7] = [
            (Some("'appmenu:close'\n"), vec![Close]),
            (Some("'appmenu:minimize,maximize,close'"), vec![Minimize, Maximize, Close]),
            (Some("'close,minimize,maximize:'"), vec![Close, Minimize, Maximize]),
            (Some("'appmenu:spacer,close,close'"), vec![Close]),
            (Some("'appmenu:'"), vec![]),
            (Some("''"), vec![]),
            (None, vec![Minimize, Maximize, Close]),
        ];
        for (layout, want) in cases {
            assert_eq!(Desktop::from_settings(layout, None).buttons, want, "{layout:?}");
        }
    }

    #[test]
    fn gnomes_double_click_action_is_followed() {
        let cases = [
            (Some("'toggle-maximize'\n"), DoubleClick::ToggleMaximize),
            (Some("'minimize'"), DoubleClick::Minimize),
            (Some("'none'"), DoubleClick::Nothing),
            (Some("'menu'"), DoubleClick::Nothing),
            (Some("'something-new'"), DoubleClick::ToggleMaximize),
            (None, DoubleClick::ToggleMaximize),
        ];
        for (action, want) in cases {
            assert_eq!(Desktop::from_settings(None, action).double_click, want, "{action:?}");
        }
    }

    #[test]
    fn no_buttons_take_no_room() {
        assert_eq!(Desktop::from_settings(Some("'appmenu:'"), None).controls_width(), 0.0);
        let close = Desktop::from_settings(Some("'appmenu:close'"), None).controls_width();
        assert_eq!(close, DIVIDER_W + BUTTON_W);
    }

    #[test]
    fn the_buttons_minimize_maximize_restore_and_close() {
        let cases = [
            (Button::Minimize, ViewportCommand::Minimized(true)),
            (Button::Maximize, ViewportCommand::Maximized(true)),
            (Button::Restore, ViewportCommand::Maximized(false)),
            (Button::Close, ViewportCommand::Close),
        ];
        for (button, want) in cases {
            assert_eq!(button.command(), want, "{button:?}");
        }
    }

    struct Strip {
        ctx: Context,
        time: f64,
        tab_clicks: usize,
        desktop: Desktop,
    }

    impl Strip {
        fn new() -> Self {
            Self::with(Desktop::default())
        }

        fn with(desktop: Desktop) -> Self {
            let mut strip = Self { ctx: Context::default(), time: 0.0, tab_clicks: 0, desktop };
            strip.frame(vec![]);
            strip.frame(vec![]);
            strip
        }

        /// One frame of the strip's real structure: the title bar under the row, a tab-like button,
        /// the window buttons at the right end, then the edges on top of everything.
        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<ViewportCommand> {
            self.time += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(WINDOW),
                time: Some(self.time),
                events,
                focused: true,
                ..egui::RawInput::default()
            };
            let mut clicked = false;
            let output = self.ctx.run_ui(raw, |ui| {
                egui::Panel::top("tabbar").exact_size(40.0).frame(egui::Frame::NONE).show(
                    ui,
                    |ui| {
                        title_bar(ui, ui.max_rect(), &self.desktop);
                        ui.horizontal(|ui| {
                            ui.set_min_height(BUTTON_H);
                            clicked = ui
                                .add_sized(vec2(120.0, BUTTON_H), egui::Button::new("main"))
                                .clicked();
                            let width = self.desktop.controls_width();
                            ui.add_space((ui.available_width() - width).max(0.0));
                            controls(ui, &self.desktop);
                        });
                    },
                );
                edges(ui.ctx());
            });
            self.tab_clicks += usize::from(clicked);
            output
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .map(|viewport| viewport.commands.clone())
                .unwrap_or_default()
        }

        fn press(&mut self, pos: Pos2, pressed: bool) -> Vec<ViewportCommand> {
            self.frame(vec![egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }])
        }

        fn click(&mut self, pos: Pos2) -> Vec<ViewportCommand> {
            let mut commands = self.frame(vec![egui::Event::PointerMoved(pos)]);
            commands.extend(self.press(pos, true));
            commands.extend(self.press(pos, false));
            commands
        }

        fn drag(&mut self, from: Pos2, by: egui::Vec2) -> Vec<ViewportCommand> {
            let mut commands = self.frame(vec![egui::Event::PointerMoved(from)]);
            commands.extend(self.press(from, true));
            for step in 1..=4 {
                let pos = from + by * (step as f32 / 4.0);
                commands.extend(self.frame(vec![egui::Event::PointerMoved(pos)]));
            }
            commands.extend(self.press(from + by, false));
            commands
        }

        fn button_center(&self, i: usize) -> Pos2 {
            let resp = self.ctx.read_response(Id::new(("window_button", i)));
            resp.expect("the window buttons were drawn in the warm-up frames").rect.center()
        }
    }

    fn moves_window(command: &ViewportCommand) -> bool {
        matches!(
            command,
            ViewportCommand::StartDrag
                | ViewportCommand::Maximized(_)
                | ViewportCommand::BeginResize(_)
        )
    }

    #[test]
    fn a_click_on_a_strip_widget_stays_with_the_widget() {
        let mut strip = Strip::new();
        let commands = strip.click(pos2(60.0, 20.0));
        assert_eq!(strip.tab_clicks, 1);
        assert!(!commands.iter().any(moves_window), "{commands:?}");
    }

    #[test]
    fn a_drag_on_the_empty_strip_moves_the_window() {
        let mut strip = Strip::new();
        let commands = strip.drag(pos2(400.0, 20.0), vec2(40.0, 10.0));
        assert_eq!(commands.iter().filter(|c| **c == ViewportCommand::StartDrag).count(), 1);
        assert_eq!(strip.tab_clicks, 0);
    }

    #[test]
    fn a_double_click_on_the_empty_strip_maximizes() {
        let mut strip = Strip::new();
        let mut commands = strip.click(pos2(400.0, 20.0));
        commands.extend(strip.click(pos2(400.0, 20.0)));
        assert_eq!(commands, vec![ViewportCommand::Maximized(true)]);
    }

    #[test]
    fn a_double_click_does_what_the_desktop_says() {
        let mut strip = Strip::with(Desktop::from_settings(None, Some("'minimize'")));
        let mut commands = strip.click(pos2(400.0, 20.0));
        commands.extend(strip.click(pos2(400.0, 20.0)));
        assert_eq!(commands, vec![ViewportCommand::Minimized(true)]);
        let mut strip = Strip::with(Desktop::from_settings(None, Some("'none'")));
        let mut commands = strip.click(pos2(400.0, 20.0));
        commands.extend(strip.click(pos2(400.0, 20.0)));
        assert_eq!(commands, vec![]);
    }

    #[test]
    fn a_close_only_layout_draws_close_alone() {
        let strip = Strip::with(Desktop::from_settings(Some("'appmenu:close'"), None));
        let close = strip.ctx.read_response(Id::new(("window_button", 0)));
        assert!(close.is_some_and(|resp| resp.rect.right() == WINDOW.right()));
        assert!(strip.ctx.read_response(Id::new(("window_button", 1))).is_none());
    }

    #[test]
    fn the_window_buttons_minimize_maximize_and_close() {
        let mut strip = Strip::new();
        for (i, want) in [
            ViewportCommand::Minimized(true),
            ViewportCommand::Maximized(true),
            ViewportCommand::Close,
        ]
        .into_iter()
        .enumerate()
        {
            let center = strip.button_center(i);
            assert_eq!(strip.click(center), vec![want], "button {i}");
        }
    }

    #[test]
    fn a_press_on_an_edge_resizes_and_one_inside_does_nothing() {
        let mut strip = Strip::new();
        let edge = strip.drag(pos2(400.0, 598.0), vec2(0.0, 20.0));
        assert_eq!(edge, vec![ViewportCommand::BeginResize(ResizeDirection::South)]);
        let corner = strip.drag(pos2(2.0, 2.0), vec2(-20.0, -20.0));
        assert_eq!(corner, vec![ViewportCommand::BeginResize(ResizeDirection::NorthWest)]);
        let inside = strip.drag(pos2(400.0, 580.0), vec2(0.0, 20.0));
        assert_eq!(inside, vec![]);
    }

    #[test]
    fn resize_cursors_point_along_the_drag() {
        assert_eq!(cursor(ResizeDirection::North), CursorIcon::ResizeVertical);
        assert_eq!(cursor(ResizeDirection::West), CursorIcon::ResizeHorizontal);
        assert_eq!(cursor(ResizeDirection::NorthEast), CursorIcon::ResizeNeSw);
        assert_eq!(cursor(ResizeDirection::SouthEast), CursorIcon::ResizeNwSe);
    }
}
