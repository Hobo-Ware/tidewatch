//! Pure geometry for the window: the terminal/panel split, grid dimensions and tab widths.

use egui::{Rect, pos2};
use stdusk_egui::ui::{TAB_FIXED_W, TAB_MIN_W};

pub(crate) const TERMINAL_FRACTION: f32 = 0.45;
pub(crate) const SIDE_FRACTION: f32 = 0.58;
pub(crate) const SIDE_RANGE: std::ops::RangeInclusive<f32> = 0.3..=0.75;
const DIVIDER_GRAB: f32 = 8.0;
pub(crate) const GAP: f32 = 1.0;
pub(crate) const TERMINAL_PAD: f32 = 10.0;
pub(crate) const CARD_MARGIN: f32 = 10.0;
/// The height of the History card's header and of the status line above an agent tab, so both tops
/// line up.
pub(crate) const CHROME_H: f32 = 48.0;

/// In the side layout `diff` is the whole left column, the place a file's diff takes over from
/// the terminal, and `history` is the card on the right: a margin from the terminal's padding, the
/// window edge, the strip above and the bar below.
pub(crate) struct Layout {
    pub(crate) terminal: Rect,
    pub(crate) history: Rect,
    pub(crate) diff: Rect,
}

pub(crate) fn split_side(area: Rect, fraction: f32) -> Layout {
    let left_w = (area.width() * fraction.clamp(*SIDE_RANGE.start(), *SIDE_RANGE.end())).round();
    let left = Rect::from_min_max(area.min, pos2(area.left() + left_w, area.bottom()));
    let history = Rect::from_min_max(
        pos2(left.right(), area.top() + CARD_MARGIN),
        pos2(area.right() - CARD_MARGIN, area.bottom() - CARD_MARGIN),
    );
    Layout { terminal: left.shrink(TERMINAL_PAD), history, diff: left }
}

/// The strip a pointer can grab to move the divider of the side layout.
pub(crate) fn divider_grab(area: Rect, fraction: f32) -> Rect {
    let x = split_side(area, fraction).diff.right();
    Rect::from_min_max(
        pos2(x - DIVIDER_GRAB / 2.0, area.top()),
        pos2(x + DIVIDER_GRAB / 2.0, area.bottom()),
    )
}

/// The share of `area` the left column takes when its divider is at `x`.
pub(crate) fn fraction_at(area: Rect, x: f32) -> f32 {
    ((x - area.left()) / area.width().max(1.0)).clamp(*SIDE_RANGE.start(), *SIDE_RANGE.end())
}

pub(crate) fn split(area: Rect) -> Layout {
    split_at(area, (area.height() * TERMINAL_FRACTION).round())
}

/// The same split with the panels a fixed `panels_h` tall: a tab with nothing to show in them yet
/// gives its room to the terminal area.
pub(crate) fn split_panels(area: Rect, panels_h: f32) -> Layout {
    split_at(area, (area.height() - panels_h - GAP).max(0.0).round())
}

fn split_at(area: Rect, terminal_h: f32) -> Layout {
    let split_y = area.top() + terminal_h;
    let bottom = Rect::from_min_max(pos2(area.left(), split_y + GAP), area.max);
    let mid = bottom.left() + ((bottom.width() - GAP) / 2.0).floor();
    Layout {
        terminal: Rect::from_min_max(area.min, pos2(area.right(), split_y)).shrink(TERMINAL_PAD),
        history: Rect::from_min_max(bottom.min, pos2(mid, bottom.bottom())),
        diff: Rect::from_min_max(pos2(mid + GAP, bottom.top()), bottom.max),
    }
}

/// Takes a band of `height` off the bottom of the terminal area, for a card between the terminal and
/// the panes, and returns it.
pub(crate) fn band_below(terminal: &mut Rect, height: f32) -> Rect {
    let band = Rect::from_min_max(pos2(terminal.left(), terminal.bottom() - height), terminal.max);
    terminal.max.y -= height;
    band
}

/// Takes a band of `height` off the top of the terminal area, for the stepper and its cards, and
/// returns it.
pub(crate) fn band_above(terminal: &mut Rect, height: f32) -> Rect {
    let band = Rect::from_min_max(terminal.min, pos2(terminal.right(), terminal.top() + height));
    terminal.min.y += height;
    band
}

/// The Other group has no graph or diff: the terminal takes the whole area.
pub(crate) fn terminal_only(area: Rect) -> Rect {
    area.shrink(TERMINAL_PAD)
}

pub(crate) fn grid_dims(rect: Rect, cell_w: f32, cell_h: f32) -> (usize, usize) {
    let cols = (rect.width() / cell_w).floor().max(1.0) as usize;
    let rows = (rect.height() / cell_h).floor().max(1.0) as usize;
    (cols, rows)
}

pub(crate) fn tab_width(available: f32, count: usize, gap: f32) -> f32 {
    let count = count.max(1) as f32;
    ((available - gap * (count - 1.0)) / count).clamp(TAB_MIN_W, TAB_FIXED_W)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_max(pos2(0.0, 40.0), pos2(1000.0, 840.0))
    }

    #[test]
    fn terminal_takes_the_top_45_percent_of_the_full_width() {
        let l = split(area());
        let outer = l.terminal.expand(TERMINAL_PAD);
        assert_eq!(outer.left(), 0.0);
        assert_eq!(outer.right(), 1000.0);
        assert_eq!(outer.height(), 360.0);
    }

    #[test]
    fn a_fixed_panel_height_gives_the_rest_to_the_terminal() {
        let l = split_panels(area(), 200.0);
        let outer = l.terminal.expand(TERMINAL_PAD);
        assert_eq!(outer.height(), 799.0 - 200.0);
        assert_eq!(l.history.height(), 200.0);
        assert_eq!(l.diff.bottom(), area().bottom());
    }

    #[test]
    fn bottom_panels_share_the_rest_without_overlap() {
        let l = split(area());
        let outer = l.terminal.expand(TERMINAL_PAD);
        assert_eq!(l.history.top(), outer.bottom() + GAP);
        assert_eq!(l.diff.top(), l.history.top());
        assert_eq!(l.history.bottom(), 840.0);
        assert_eq!(l.diff.bottom(), 840.0);
        assert_eq!(l.history.left(), 0.0);
        assert_eq!(l.diff.right(), 1000.0);
        assert_eq!(l.diff.left() - l.history.right(), GAP);
        assert!((l.history.width() - l.diff.width()).abs() <= 1.0);
    }

    #[test]
    fn the_side_layout_gives_the_left_share_to_the_terminal_and_the_rest_to_history() {
        let l = split_side(area(), SIDE_FRACTION);
        assert_eq!(l.diff, Rect::from_min_max(pos2(0.0, 40.0), pos2(580.0, 840.0)));
        assert_eq!(l.terminal, l.diff.shrink(TERMINAL_PAD));
        assert_eq!(l.history, Rect::from_min_max(pos2(580.0, 50.0), pos2(990.0, 830.0)));
        assert_eq!(l.history.left() - l.terminal.right(), CARD_MARGIN);
    }

    #[test]
    fn the_divider_stays_inside_its_range() {
        let wide = split_side(area(), 0.99);
        assert_eq!(wide.diff.width(), 750.0);
        let narrow = split_side(area(), 0.0);
        assert_eq!(narrow.diff.width(), 300.0);
        assert_eq!(fraction_at(area(), 5000.0), 0.75);
        assert_eq!(fraction_at(area(), 500.0), 0.5);
    }

    #[test]
    fn the_grab_strip_sits_on_the_divider() {
        let grab = divider_grab(area(), SIDE_FRACTION);
        assert_eq!(grab.center().x, 580.0);
        assert_eq!(grab.height(), 800.0);
    }

    #[test]
    fn terminal_only_fills_the_area_minus_the_padding() {
        let r = terminal_only(area());
        assert_eq!(r, area().shrink(TERMINAL_PAD));
    }

    #[test]
    fn a_band_comes_off_the_bottom_of_the_terminal() {
        let mut terminal = Rect::from_min_max(pos2(10.0, 50.0), pos2(990.0, 400.0));
        let band = band_below(&mut terminal, 40.0);
        assert_eq!(terminal.bottom(), 360.0);
        assert_eq!(band, Rect::from_min_max(pos2(10.0, 360.0), pos2(990.0, 400.0)));
    }

    #[test]
    fn a_band_comes_off_the_top_of_the_terminal() {
        let mut terminal = Rect::from_min_max(pos2(10.0, 50.0), pos2(990.0, 400.0));
        let band = band_above(&mut terminal, 40.0);
        assert_eq!(terminal.top(), 90.0);
        assert_eq!(band, Rect::from_min_max(pos2(10.0, 50.0), pos2(990.0, 90.0)));
        let second = band_above(&mut terminal, 10.0);
        assert_eq!(second.top(), 90.0);
        assert_eq!(terminal.top(), 100.0);
    }

    #[test]
    fn grid_dims_floor_and_never_drop_below_one() {
        let r = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 50.0));
        assert_eq!(grid_dims(r, 8.0, 16.0), (12, 3));
        assert_eq!(grid_dims(r, 500.0, 500.0), (1, 1));
    }

    #[test]
    fn tab_width_caps_at_fixed_and_shrinks_to_the_floor() {
        assert_eq!(tab_width(2000.0, 2, 4.0), TAB_FIXED_W);
        assert_eq!(tab_width(10.0, 5, 4.0), TAB_MIN_W);
        let shared = tab_width(504.0, 5, 4.0);
        assert!((shared - 97.6).abs() < 0.01);
    }
}
