//! Pure geometry for the window: the terminal/panel split, grid dimensions and tab widths.

use egui::{Rect, pos2};
use stdusk_egui::ui::{TAB_FIXED_W, TAB_MIN_W};

pub(crate) const TERMINAL_FRACTION: f32 = 0.45;
pub(crate) const GAP: f32 = 1.0;
pub(crate) const TERMINAL_PAD: f32 = 10.0;

pub(crate) struct Layout {
    pub(crate) terminal: Rect,
    pub(crate) history: Rect,
    pub(crate) diff: Rect,
}

pub(crate) fn split(area: Rect) -> Layout {
    let split_y = area.top() + (area.height() * TERMINAL_FRACTION).round();
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
