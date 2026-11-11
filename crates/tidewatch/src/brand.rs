//! The Night tide palette: the one set of colors every Tidewatch view reads, and how it is put on egui.
#![allow(clippy::unreadable_literal)]

use egui::{Color32, Stroke};
use stdusk_egui::colors::{self, Chrome, Theme};

/// Which of the two palettes every color function answers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Dark,
    Light,
}

#[cfg(not(test))]
mod flag {
    use std::sync::atomic::{AtomicBool, Ordering};

    static LIGHT: AtomicBool = AtomicBool::new(false);

    pub(super) fn light() -> bool {
        LIGHT.load(Ordering::Relaxed)
    }

    pub(super) fn set(light: bool) {
        LIGHT.store(light, Ordering::Relaxed);
    }
}

/// Tests each run on their own thread, so each gets its own mode and none can repaint another.
#[cfg(test)]
mod flag {
    use std::cell::Cell;

    thread_local! {
        static LIGHT: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn light() -> bool {
        LIGHT.get()
    }

    pub(super) fn set(light: bool) {
        LIGHT.set(light);
    }
}

pub(crate) fn mode() -> Mode {
    if flag::light() { Mode::Light } else { Mode::Dark }
}

pub(crate) fn set_mode(mode: Mode) {
    flag::set(mode == Mode::Light);
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

fn pick(dark: Color32, light: Color32) -> Color32 {
    if flag::light() { light } else { dark }
}

macro_rules! inks {
    ($($name:ident = $dark:literal / $light:literal;)*) => {
        $(pub(crate) fn $name() -> Color32 {
            pick(hex($dark), hex($light))
        })*
    };
}

inks! {
    ground = 0x10131a / 0xf6f7f9;
    sidebar = 0x0d1016 / 0xeceff3;
    sidebar_line = 0x1f2430 / 0xd8dee5;
    card = 0x161b24 / 0xffffff;
    card_line = 0x222937 / 0xd8dee5;
    item = 0x1a2029 / 0xeef1f4;
    item_line = 0x262e3b / 0xd3dae2;
    bar = 0x12161e / 0xf0f2f5;
    answer_line = 0x333c4b / 0xc3ccd6;
    field_line = 0x2f3747 / 0xc9d1da;
    segment = 0x0f1219 / 0xe8ecf0;
    segment_line = 0x262d39 / 0xd3dae2;
    segment_on = 0x1f2a31 / 0xffffff;
    button = 0x1d2230 / 0xe8ecf1;
    key_edge = 0x0b0e13 / 0xbcc5cf;
    text = 0xe6e8ec / 0x1b2430;
    soft = 0xa7b0bf / 0x5a6776;
    step_text = 0xc4cad4 / 0x36424f;
    muted = 0x8b93a3 / 0x626e7c;
    faint = 0x6b7383 / 0x78828f;
    teal = 0x2dd4bf / 0x0e8a80;
    teal_text = 0x5eead4 / 0x0a7a71;
    teal_nav = 0x16252b / 0xdcf2ef;
    green = 0x8fd16a / 0x2e7d32;
    orange = 0xfb923c / 0xb45309;
    orange_soft = 0xd6a77a / 0x9a5b25;
    blue = 0x60a5fa / 0x1d5fd6;
    blue_text = 0x93c5fd / 0x1e4fb8;
    amber = 0xfbbf24 / 0xa15c00;
    red = 0xf87171 / 0xc62828;
    red_line = 0x3a2530 / 0xf0c4c4;
    red_fill = 0x24171e / 0xfdeeee;
    needs_fill = 0x16130f / 0xfff4e0;
    violet_text = 0xc4b5fd / 0x5b3fd0;
    violet = 0x7c6bd6 / 0x6a58d6;
    violet_fill = 0x221d3d / 0xefecfc;
    violet_line = 0x3b3466 / 0xcdc5f0;
    struggle_fill = 0x1c1610 / 0xfff1dc;
    struggle_line = 0x4a3418 / 0xedcf9f;
    sunken = 0x0f141c / 0xeef1f4;
    row_ink = 0x4a5263 / 0xb9c3ce;
    user_line = 0x1f3a3a / 0xb9ddd8;
    live_fill = 0x12201f / 0xe8f6f4;
    image_chip_line = 0x2f4a4a / 0xb3d6d1;
    gradient_end = 0x15393a / 0xc4e8e3;
    glyph_fill = 0x14191f / 0xf1f4f7;
    done = 0xb2e68a / 0x2f7f3a;
    failed = 0xef4444 / 0xa31515;
    working = 0x2dd4bf / 0x0a6d66;
}

/// Avatar and face art sits on a dark plate in both modes.
pub(crate) fn face_plate() -> Color32 {
    hex(0x222937)
}

pub(crate) fn face_ring() -> Color32 {
    hex(0x333c4b)
}

pub(crate) fn user_code_fill() -> Color32 {
    pick(Color32::from_black_alpha(90), Color32::from_black_alpha(14))
}

pub(crate) fn hover_wash() -> Color32 {
    pick(Color32::from_white_alpha(16), Color32::from_black_alpha(10))
}

pub(crate) fn repo_inks() -> [Color32; 7] {
    [
        pick(hex(0xf472b6), hex(0xdb2777)),
        blue(),
        pick(hex(0xa3e635), hex(0x4d7c0f)),
        amber(),
        pick(hex(0xc084fc), hex(0x9333ea)),
        pick(hex(0x22d3ee), hex(0x0e7490)),
        pick(hex(0xfb7185), hex(0xe11d48)),
    ]
}

/// Each CLI's badge, as its fill and its letter's ink: the one place a hue outside the palette lives.
pub(crate) fn claude_badge() -> (Color32, Color32) {
    (pick(hex(0x3b2418), hex(0xf9e4d4)), orange())
}

pub(crate) fn codex_badge() -> (Color32, Color32) {
    (pick(hex(0x1e2a22), hex(0xe3f1e4)), green())
}

pub(crate) fn gemini_badge() -> (Color32, Color32) {
    (pick(hex(0x1c2740), hex(0xe0e8fb)), pick(hex(0x7aa2f7), hex(0x2f5fd0)))
}

pub(crate) fn copilot_badge() -> (Color32, Color32) {
    (pick(hex(0x2a2333), hex(0xece7f7)), violet_text())
}

/// Agent status inks. They differ in lightness as well as hue (failed, needs you, working, done
/// run darkest to lightest on the dark ground and keep their gaps on the light one), so a
/// color-blind reader still tells them apart.
pub(crate) fn needs() -> Color32 {
    amber()
}

pub(crate) fn waiting() -> Color32 {
    blue()
}

pub(crate) fn blocked() -> Color32 {
    red()
}

pub(crate) fn link_rest() -> Color32 {
    teal_text().gamma_multiply(0.55)
}

/// The terminal's scheme: One Half of the mode's kind on the brand ground, ANSI palette untouched.
pub(crate) fn theme() -> Theme {
    let mut theme = match mode() {
        Mode::Dark => colors::one_half_dark(),
        Mode::Light => colors::by_name("one-half-light"),
    };
    theme.bg = ground();
    theme.fg = text();
    theme.cursor = teal();
    theme
}

pub(crate) fn chrome() -> Chrome {
    Chrome { accent: Some(teal()), red: Some(red()), green: Some(green()), yellow: None }
}

/// Puts a mode on the whole app: the palette flag, stdusk's terminal scheme and chrome, the egui
/// visuals and Kelp's palette, then a repaint.
pub(crate) fn apply(ctx: &egui::Context, mode: Mode) {
    set_mode(mode);
    colors::set(theme());
    colors::set_chrome(chrome());
    stdusk_egui::ui::apply_theme(ctx);
    tint_visuals(ctx);
    kelp_ui::theme::sync(ctx);
    ctx.request_repaint();
}

/// Teal selection, links and active widgets on top of what stdusk derived from the theme.
pub(crate) fn tint_visuals(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.selection.bg_fill = teal().gamma_multiply(0.3);
        v.selection.stroke = Stroke::new(1.0, teal());
        v.hyperlink_color = teal_text();
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, teal().gamma_multiply(0.5));
        v.widgets.active.bg_stroke = Stroke::new(1.0, teal());
        style.spacing.scroll = scroll_style();
        style.interaction.selectable_labels = true;
    });
}

pub(crate) const SCROLL_GAP: f32 = 12.0;
const SCROLL_EDGE: f32 = 2.0;
const SCROLL_BAR_W: f32 = 8.0;
pub(crate) const SCROLL_ROOM: f32 = SCROLL_GAP + SCROLL_BAR_W + SCROLL_EDGE;

fn scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        bar_width: SCROLL_BAR_W,
        bar_outer_margin: SCROLL_EDGE,
        floating_width: 3.0,
        floating_allocated_width: SCROLL_ROOM,
        dormant_handle_opacity: 0.22,
        active_handle_opacity: 0.5,
        interact_handle_opacity: 0.8,
        interact_background_opacity: 0.15,
        ..egui::style::ScrollStyle::floating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color32) -> f32 {
        let lin = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a) + 0.05, luminance(b) + 0.05);
        a.max(b) / a.min(b)
    }

    fn hue(ink: Color32) -> f32 {
        let (red, green, blue) = (f32::from(ink.r()), f32::from(ink.g()), f32::from(ink.b()));
        let (top, bottom) = (red.max(green).max(blue), red.min(green).min(blue));
        let spread = top - bottom;
        let sector = if top == red {
            ((green - blue) / spread).rem_euclid(6.0)
        } else if top == green {
            (blue - red) / spread + 2.0
        } else {
            (red - green) / spread + 4.0
        };
        sector * 60.0
    }

    fn apart(a: Color32, b: Color32) -> f32 {
        let d = (hue(a) - hue(b)).abs();
        d.min(360.0 - d)
    }

    type Role = (&'static str, fn() -> Color32, f32);

    fn in_each_mode(check: impl Fn(Mode)) {
        for mode in [Mode::Dark, Mode::Light] {
            set_mode(mode);
            check(mode);
        }
        set_mode(Mode::Dark);
    }

    #[test]
    fn the_palette_answers_for_the_mode_it_is_in() {
        let cases = [
            (Mode::Dark, [0xe6e8ec, 0x10131a, 0x2dd4bf, 0x161b24]),
            (Mode::Light, [0x1b2430, 0xf6f7f9, 0x0e8a80, 0xffffff]),
        ];
        for (mode, [ink, ground_rgb, teal_rgb, card_rgb]) in cases {
            set_mode(mode);
            assert_eq!(super::mode(), mode);
            let got = [text(), ground(), teal(), card()];
            assert_eq!(got, [ink, ground_rgb, teal_rgb, card_rgb].map(hex), "{mode:?}");
        }
        set_mode(Mode::Dark);
    }

    #[test]
    fn status_inks_differ_in_lightness_and_read_on_the_ground() {
        in_each_mode(|mode| {
            let ramp = [failed(), working(), done()];
            for pair in ramp.windows(2) {
                assert!(
                    contrast(pair[0], pair[1]) >= 1.2,
                    "{mode:?}: {:?} against {:?}",
                    pair[0],
                    pair[1]
                );
            }
            for ink in [failed(), needs(), waiting(), blocked(), working(), done()] {
                assert!(contrast(ink, ground()) >= 4.5, "{mode:?}: {ink:?} on the ground");
            }
        });
    }

    #[test]
    fn text_reads_on_the_ground_and_on_a_card_in_both_modes() {
        in_each_mode(|mode| {
            let roles: [Role; 6] = [
                ("text", text, 4.5),
                ("step_text", step_text, 4.5),
                ("soft", soft, 4.5),
                ("muted", muted, 4.5),
                ("teal_text", teal_text, 4.5),
                ("faint", faint, 3.0),
            ];
            for (name, ink, floor) in roles {
                for (surface, under) in [("ground", ground()), ("card", card())] {
                    let got = contrast(ink(), under);
                    assert!(got >= floor, "{mode:?}: {name} on {surface} is {got:.2}");
                }
            }
        });
    }

    #[test]
    fn the_five_state_inks_are_told_apart_by_hue() {
        in_each_mode(|mode| {
            let inks = [working(), needs(), waiting(), blocked(), done()];
            for (i, a) in inks.iter().enumerate() {
                for b in &inks[i + 1..] {
                    assert!(apart(*a, *b) >= 25.0, "{mode:?}: {a:?} and {b:?}");
                }
            }
        });
    }

    #[test]
    fn the_agent_icon_colors_are_the_brand_inks_they_are_named_for() {
        let named = [
            ("teal", teal()),
            ("blue", blue()),
            ("green", green()),
            ("amber", amber()),
            ("orange", orange()),
            ("red", red()),
            ("purple", violet_text()),
        ];
        for (name, ink) in named {
            let hex = format!("#{:02x}{:02x}{:02x}", ink.r(), ink.g(), ink.b());
            assert_eq!(tidewatch_core::avatar_icons::color_hex(name), Some(hex), "{name}");
        }
    }

    #[test]
    fn the_theme_makes_stdusk_read_teal_for_its_accent() {
        in_each_mode(|mode| {
            let scheme = match mode {
                Mode::Dark => colors::one_half_dark(),
                Mode::Light => colors::by_name("one-half-light"),
            };
            let theme = theme();
            assert_eq!((theme.bg, theme.fg, theme.ansi), (ground(), text(), scheme.ansi));
            assert_eq!(chrome().accent, Some(teal()));
        });
    }

    #[test]
    fn tinting_sets_selection_links_and_active_widgets() {
        let ctx = egui::Context::default();
        tint_visuals(&ctx);
        let v = ctx.global_style().visuals.clone();
        assert_eq!(v.selection.stroke.color, teal());
        assert_eq!(v.hyperlink_color, teal_text());
        assert_eq!(v.widgets.active.bg_stroke.color, teal());
    }

    #[test]
    fn scroll_bars_leave_a_gap_and_take_their_own_room() {
        let ctx = egui::Context::default();
        tint_visuals(&ctx);
        let scroll = ctx.global_style().spacing.scroll;
        assert_eq!(scroll.allocated_width(), SCROLL_ROOM);
        assert!(SCROLL_ROOM - SCROLL_EDGE - scroll.bar_width >= SCROLL_GAP);
    }

    #[test]
    fn a_scroll_area_keeps_its_content_clear_of_the_bar() {
        let ctx = egui::Context::default();
        tint_visuals(&ctx);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 200.0),
            )),
            ..egui::RawInput::default()
        };
        let mut widths = Vec::new();
        for _ in 0..3 {
            let _ = ctx.run_ui(raw.clone(), |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    widths.push(ui.available_width());
                    ui.allocate_space(egui::vec2(10.0, 1000.0));
                });
            });
        }
        assert_eq!(widths.last().copied(), Some(400.0 - SCROLL_ROOM));
    }

    #[test]
    fn labels_are_selectable_by_default() {
        let ctx = egui::Context::default();
        tint_visuals(&ctx);
        assert!(ctx.global_style().interaction.selectable_labels);
    }
}
