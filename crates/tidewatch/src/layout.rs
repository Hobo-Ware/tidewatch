//! Pure geometry for the window: the terminal/panel split, grid dimensions and tab widths.

use egui::{Rect, pos2};
use stdusk_egui::ui::TAB_FIXED_W;

pub(crate) const TERMINAL_FRACTION: f32 = 0.45;
pub(crate) const SIDE_FRACTION: f32 = 0.58;
pub(crate) const SIDE_RANGE: std::ops::RangeInclusive<f32> = 0.3..=0.75;
const DIVIDER_GRAB: f32 = 8.0;
pub(crate) const GAP: f32 = 1.0;
pub(crate) const TERMINAL_PAD: f32 = 10.0;
pub(crate) const CARD_MARGIN: f32 = 10.0;
/// The one left edge of an agent tab: its status line, its content and the bar under them.
pub(crate) const EDGE: f32 = 24.0;
/// The height of the History card's header and of the status line above an agent tab, so both tops
/// line up.
pub(crate) const CHROME_H: f32 = 48.0;
pub(crate) const MIN_TERMINAL_H: f32 = 260.0;

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

fn split_at(area: Rect, wanted_h: f32) -> Layout {
    let terminal_h = wanted_h.max(MIN_TERMINAL_H).min(area.height().max(0.0));
    let split_y = area.top() + terminal_h;
    let bottom =
        Rect::from_min_max(pos2(area.left(), (split_y + GAP).min(area.bottom())), area.max);
    let cards = Rect::from_min_max(
        pos2(bottom.left() + CARD_MARGIN, bottom.top()),
        pos2(bottom.right() - CARD_MARGIN, (bottom.bottom() - CARD_MARGIN).max(bottom.top())),
    );
    let mid = cards.center().x.round();
    Layout {
        terminal: Rect::from_min_max(area.min, pos2(area.right(), split_y)).shrink(TERMINAL_PAD),
        history: Rect::from_min_max(cards.min, pos2(mid - CARD_MARGIN / 2.0, cards.bottom())),
        diff: Rect::from_min_max(pos2(mid + CARD_MARGIN / 2.0, cards.top()), cards.max),
    }
}

/// Takes a band of `height` off the bottom of the terminal area, for a card between the terminal and
/// the panes, and returns it.
pub(crate) fn band_below(terminal: &mut Rect, height: f32) -> Rect {
    let top = (terminal.bottom() - height).max(terminal.top());
    let band = Rect::from_min_max(pos2(terminal.left(), top), terminal.max);
    terminal.max.y = top;
    band
}

/// Takes a band of `height` off the top of the terminal area, for the stepper and its cards, and
/// returns it.
pub(crate) fn band_above(terminal: &mut Rect, height: f32) -> Rect {
    let bottom = (terminal.top() + height).min(terminal.bottom());
    let band = Rect::from_min_max(terminal.min, pos2(terminal.right(), bottom));
    terminal.min.y = bottom;
    band
}

/// The Other group has no graph or diff: the terminal takes the whole area.
pub(crate) fn terminal_only(area: Rect) -> Rect {
    area.shrink(TERMINAL_PAD)
}

pub(crate) fn agent_grid(terminal: Rect) -> Rect {
    Rect::from_min_max(pos2(terminal.left() + EDGE - TERMINAL_PAD, terminal.top()), terminal.max)
}

pub(crate) fn grid_dims(rect: Rect, cell_w: f32, cell_h: f32) -> (usize, usize) {
    let cols = (rect.width() / cell_w).floor().max(1.0) as usize;
    let rows = (rect.height() / cell_h).floor().max(1.0) as usize;
    (cols, rows)
}

pub(crate) const MIN_TAB_W: f32 = 140.0;

pub(crate) fn tab_width(available: f32, count: usize, gap: f32) -> f32 {
    let count = count.max(1) as f32;
    ((available - gap * (count - 1.0)) / count).clamp(MIN_TAB_W, TAB_FIXED_W)
}

pub(crate) fn tab_capacity(available: f32, gap: f32) -> usize {
    (((available + gap) / (MIN_TAB_W + gap)).floor().max(0.0)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_max(pos2(0.0, 40.0), pos2(1000.0, 840.0))
    }

    #[test]
    fn an_agent_grid_starts_at_the_edge_from_the_window() {
        let l = split_side(area(), SIDE_FRACTION);
        assert_eq!(l.terminal.left(), TERMINAL_PAD);
        let grid = agent_grid(l.terminal);
        assert_eq!(grid.left(), EDGE);
        assert_eq!((grid.top(), grid.max), (l.terminal.top(), l.terminal.max));
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
        assert_eq!(l.history.height(), 200.0 - CARD_MARGIN);
        assert_eq!(l.diff.bottom(), area().bottom() - CARD_MARGIN);
    }

    #[test]
    fn the_terminal_area_keeps_its_least_height_before_the_panels_shrink() {
        let heights = [840.0, 600.0, 400.0, 300.0, 100.0, 0.0];
        for height in heights {
            let area = Rect::from_min_max(pos2(0.0, 40.0), pos2(1000.0, 40.0 + height));
            for l in [split(area), split_panels(area, 200.0), split_panels(area, 900.0)] {
                let outer = l.terminal.expand(TERMINAL_PAD);
                assert!(outer.height() >= MIN_TERMINAL_H.min(height), "{height}");
                assert!(outer.bottom() <= area.bottom(), "{height}");
                assert!(l.history.height() >= 0.0 && l.diff.height() >= 0.0, "{height}");
                assert!(l.history.top() >= outer.bottom().min(area.bottom()), "{height}");
            }
        }
        let l = split_panels(area(), 900.0);
        assert_eq!(l.terminal.expand(TERMINAL_PAD).height(), MIN_TERMINAL_H);
        let short = Rect::from_min_max(pos2(0.0, 40.0), pos2(1000.0, 440.0));
        assert_eq!(split(short).terminal.expand(TERMINAL_PAD).height(), MIN_TERMINAL_H);
    }

    #[test]
    fn bottom_cards_share_the_rest_with_a_card_margin_between_them() {
        let l = split(area());
        let outer = l.terminal.expand(TERMINAL_PAD);
        assert_eq!(l.history.top(), outer.bottom() + GAP);
        assert_eq!(l.diff.top(), l.history.top());
        assert_eq!(l.history.bottom(), 840.0 - CARD_MARGIN);
        assert_eq!(l.diff.bottom(), 840.0 - CARD_MARGIN);
        assert_eq!(l.history.left(), CARD_MARGIN);
        assert_eq!(l.diff.right(), 1000.0 - CARD_MARGIN);
        assert_eq!(l.diff.left() - l.history.right(), CARD_MARGIN);
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
    fn a_band_taller_than_the_terminal_leaves_it_empty_not_upside_down() {
        let mut above = Rect::from_min_max(pos2(10.0, 50.0), pos2(990.0, 80.0));
        let band = band_above(&mut above, 48.0);
        assert_eq!(band.height(), 30.0);
        assert_eq!((above.height(), above.top()), (0.0, 80.0));
        let mut below = Rect::from_min_max(pos2(10.0, 50.0), pos2(990.0, 80.0));
        let band = band_below(&mut below, 48.0);
        assert_eq!(band.height(), 30.0);
        assert_eq!((below.height(), below.bottom()), (0.0, 50.0));
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
        assert_eq!(tab_width(10.0, 5, 4.0), MIN_TAB_W);
        assert_eq!(tab_width(504.0, 5, 4.0), MIN_TAB_W);
        let shared = tab_width(900.0, 5, 4.0);
        assert!((shared - 176.8).abs() < 0.01);
    }

    #[test]
    fn a_tab_never_gets_less_than_the_least_width_however_many_there_are() {
        for count in [1, 2, 6, 12, 40] {
            for available in [0.0, 100.0, 700.0, 1400.0] {
                let width = tab_width(available, count, 4.0);
                assert!((MIN_TAB_W..=TAB_FIXED_W).contains(&width), "{count} in {available}");
            }
        }
    }

    #[test]
    fn the_strip_holds_as_many_least_width_tabs_as_fit_with_their_gaps() {
        let cases = [
            (-10.0, 0),
            (0.0, 0),
            (139.0, 0),
            (140.0, 1),
            (283.0, 1),
            (284.0, 2),
            (572.0, 4),
            (571.0, 3),
            (1000.0, 6),
            (1008.0, 7),
        ];
        for (available, want) in cases {
            assert_eq!(tab_capacity(available, 4.0), want, "{available}");
        }
    }

    #[test]
    fn what_tab_capacity_counts_really_fits_and_one_more_does_not() {
        for available in [140.0, 300.0, 575.5, 880.0, 1337.0] {
            let fits = tab_capacity(available, 4.0) as f32;
            assert!(fits * MIN_TAB_W + (fits - 1.0).max(0.0) * 4.0 <= available + 0.001);
            let more = fits + 1.0;
            assert!(more * MIN_TAB_W + (more - 1.0) * 4.0 > available);
        }
    }
}
