//! The Amphetamine menu's body: the twelve-hour track with its no-end cap, Start or Turn off, and
//! the Away band at the bottom. One track starts a session and changes a running one.

use egui::{
    Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use jiff::tz::TimeZone;
use stdusk_egui::ui::icons;
use tidewatch_core::amphetamine::{Choice, State};

use crate::amphetamine_pill::{INFINITY, clock_texts};
use crate::amphetamine_view::{Action, Input};
use crate::brand;
use crate::job_model::clock_label;
use crate::screen_kit::{
    MENU_ROW_H, MENU_ROW_PAD, TEXT_BODY, TEXT_TAG, body, menu_label, menu_row, mono,
};

pub(crate) const FIRST_DRAFT: Choice = Choice::Minutes(60);
pub(crate) const HOURS: u32 = 12;
const SNAP_MINUTES: u32 = 15;
const VALUE_SIZE: f32 = 22.0;
const VALUE_H: f32 = 34.0;
const TRACK_H: f32 = 34.0;
const CELL_GAP: f32 = 3.0;
const CAP_W: f32 = 38.0;
const TICK_H: f32 = 18.0;
const TICK_EVERY: u32 = 3;
const START_H: f32 = 36.0;
const BAND_H: f32 = 54.0;
const SWITCH: egui::Vec2 = vec2(36.0, 20.0);
const INSET: f32 = 6.0;

/// The short words for a length: `1h 30m`, or the infinity sign for no timer.
pub(crate) fn length_text(choice: Choice) -> String {
    choice.seconds().map_or_else(|| INFINITY.to_owned(), |seconds| clock_texts(seconds).0)
}

/// What the line under the value says: the clock time it ends, or that it has none.
pub(crate) fn ends_text(choice: Choice, wall: f64, zone: &TimeZone) -> String {
    match choice.seconds() {
        Some(seconds) => clock_label(((wall + seconds) * 1000.0) as i64, zone)
            .map_or_else(String::new, |at| format!("until {at}")),
        None => "until you turn it off".to_owned(),
    }
}

/// A clock time `hours` from `wall`, for the marks under the track.
pub(crate) fn tick_text(hours: u32, wall: f64, zone: &TimeZone) -> String {
    clock_label(((wall + f64::from(hours) * 3600.0) * 1000.0) as i64, zone).unwrap_or_default()
}

/// A click on the track: the session ends at the end of the hour under the pointer.
pub(crate) fn clicked_choice(fraction: f32) -> Choice {
    let hour = ((fraction.clamp(0.0, 1.0) * HOURS as f32).floor() as u32 + 1).min(HOURS);
    Choice::Minutes(hour * 60)
}

/// A drag on the track: the pointer's spot, snapped to a quarter hour.
pub(crate) fn dragged_choice(fraction: f32) -> Choice {
    let minutes = fraction.clamp(0.0, 1.0) * (HOURS * 60) as f32;
    let snapped = (minutes / SNAP_MINUTES as f32).round() as u32 * SNAP_MINUTES;
    Choice::Minutes(snapped.clamp(Choice::SHORTEST, Choice::LONGEST))
}

/// How many hours of the track glow for a length; `None` when the cap is on.
pub(crate) fn lit_hours(choice: Choice) -> Option<f32> {
    match choice {
        Choice::Minutes(minutes) => Some(minutes.min(Choice::LONGEST) as f32 / 60.0),
        Choice::Open => None,
    }
}

/// Draws the menu's body; `draft` is the length the next start uses and is changed in place.
pub(crate) fn body_ui(ui: &mut Ui, input: &Input, draft: &mut Choice) -> Option<Action> {
    let zone = TimeZone::system();
    let mut action = None;
    if let State::On { until, .. } = input.session {
        menu_label(ui, "Amphetamine is on", "");
        let left = Choice::left(until.map(|until| until - input.wall));
        if let Some(Pick { choice, done: true }) = track(ui, left, input.wall, &zone) {
            action = Some(Action::Retime(choice));
        }
        ui.add_space(2.0);
        if turn_off_row(ui).clicked() {
            action = Some(Action::TurnOff);
        }
    } else {
        menu_label(ui, if input.lid { "Keep awake with the lid closed" } else { "Keep awake" }, "");
        if let Some(pick) = track(ui, *draft, input.wall, &zone) {
            *draft = pick.choice;
        }
        ui.add_space(4.0);
        if start_button(ui, *draft).clicked() {
            action = Some(Action::Pick(*draft));
        }
    }
    ui.add_space(6.0);
    if away_band(ui, input.away).clicked() {
        action = Some(Action::Away);
    }
    action
}

/// A length picked on the track; `done` once the click or drag is over.
#[derive(Clone, Copy)]
struct Pick {
    choice: Choice,
    done: bool,
}

fn track(ui: &mut Ui, value: Choice, wall: f64, zone: &TimeZone) -> Option<Pick> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, VALUE_H + TRACK_H + TICK_H), Sense::hover());
    let row = rect.shrink2(vec2(INSET, 0.0));
    let bar =
        Rect::from_min_size(pos2(row.left(), row.top() + VALUE_H), vec2(row.width(), TRACK_H));
    let cells = Rect::from_min_max(bar.min, pos2(bar.right() - CAP_W - CELL_GAP, bar.bottom()));
    let cap = Rect::from_min_max(pos2(bar.right() - CAP_W, bar.top()), bar.max);
    let id = ui.id().with("amphetamine_track");
    let response = ui.interact(bar, id, Sense::click_and_drag());
    let held = ui.data(|data| data.get_temp::<Option<Choice>>(id)).flatten();
    let at = response.interact_pointer_pos();
    let pick = at.map(|at| {
        let fraction = (at.x - cells.left()) / cells.width();
        let choice = if at.x > cells.right() {
            Choice::Open
        } else if response.dragged() || response.drag_stopped() {
            dragged_choice(fraction)
        } else {
            clicked_choice(fraction)
        };
        Pick { choice, done: response.clicked() || response.drag_stopped() }
    });
    let dragging = pick.filter(|pick| !pick.done).map(|pick| pick.choice);
    ui.data_mut(|data| data.insert_temp(id, dragging));
    let shown = pick.map(|pick| pick.choice).or(held).unwrap_or(value);
    paint_value(ui, row, shown, wall, zone);
    paint_cells(ui.painter(), cells, shown);
    paint_cap(ui.painter(), cap, shown == Choice::Open, response.hovered());
    paint_ticks(ui.painter(), cells, wall, zone);
    response.on_hover_cursor(egui::CursorIcon::PointingHand).widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Keep awake for")
    });
    pick
}

fn paint_value(ui: &Ui, row: Rect, value: Choice, wall: f64, zone: &TimeZone) {
    let y = row.top() + VALUE_H / 2.0 - 2.0;
    let painter = ui.painter();
    painter.text(
        pos2(row.left(), y),
        Align2::LEFT_CENTER,
        length_text(value),
        mono(VALUE_SIZE),
        brand::TEAL_TEXT,
    );
    painter.text(
        pos2(row.right(), y),
        Align2::RIGHT_CENTER,
        ends_text(value, wall, zone),
        body(TEXT_TAG),
        brand::MUTED,
    );
}

fn paint_cells(painter: &egui::Painter, cells: Rect, value: Choice) {
    let width = (cells.width() - CELL_GAP * (HOURS - 1) as f32) / HOURS as f32;
    let lit = lit_hours(value);
    for hour in 0..HOURS {
        let left = cells.left() + hour as f32 * (width + CELL_GAP);
        let cell = Rect::from_min_size(pos2(left, cells.top()), vec2(width, cells.height()));
        let radius = if hour == 0 {
            CornerRadius { nw: 8, sw: 8, ne: 3, se: 3 }
        } else {
            CornerRadius::same(3)
        };
        painter.rect_filled(cell, radius, brand::FIELD_LINE);
        let (share, ink) = match lit {
            None => (1.0, brand::TEAL.gamma_multiply(1.0 - hour as f32 * 0.05)),
            Some(lit) => {
                let share = (lit - hour as f32).clamp(0.0, 1.0);
                let end = lit > hour as f32 && lit <= hour as f32 + 1.0;
                (share, if end { brand::TEAL_TEXT } else { brand::TEAL })
            }
        };
        if share <= 0.0 {
            continue;
        }
        let fill = Rect::from_min_size(cell.min, vec2(cell.width() * share, cell.height()));
        if ink == brand::TEAL_TEXT {
            painter.rect_filled(
                fill.expand(2.0),
                CornerRadius::same(4),
                brand::TEAL.gamma_multiply(0.3),
            );
        }
        painter.rect_filled(fill, radius, ink);
    }
}

fn paint_cap(painter: &egui::Painter, cap: Rect, on: bool, hovered: bool) {
    let radius = CornerRadius { nw: 3, sw: 3, ne: 8, se: 8 };
    let (fill, line, ink) = match (on, hovered) {
        (true, _) => (brand::TEAL, brand::TEAL_TEXT, brand::GROUND),
        (false, true) => (brand::SEGMENT_ON, brand::TEAL, brand::TEAL_TEXT),
        (false, false) => (brand::SUNKEN, brand::FIELD_LINE, brand::MUTED),
    };
    if on {
        painter.rect_filled(
            cap.expand(2.0),
            CornerRadius::same(9),
            brand::TEAL.gamma_multiply(0.3),
        );
    }
    painter.rect(cap, radius, fill, Stroke::new(1.0, line), StrokeKind::Inside);
    painter.text(cap.center(), Align2::CENTER_CENTER, INFINITY, body(18.0), ink);
}

fn paint_ticks(painter: &egui::Painter, cells: Rect, wall: f64, zone: &TimeZone) {
    let y = cells.bottom() + TICK_H / 2.0 + 1.0;
    for hours in (0..=HOURS).step_by(TICK_EVERY as usize) {
        let x = cells.left() + cells.width() * hours as f32 / HOURS as f32;
        let align = match hours {
            0 => Align2::LEFT_CENTER,
            HOURS => Align2::RIGHT_CENTER,
            _ => Align2::CENTER_CENTER,
        };
        painter.text(pos2(x, y), align, tick_text(hours, wall, zone), mono(10.5), brand::FAINT);
    }
}

fn start_button(ui: &mut Ui, draft: Choice) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), START_H), Sense::click());
    let button = rect.shrink2(vec2(INSET, 0.0));
    let fill = if response.hovered() { brand::TEAL_TEXT } else { brand::TEAL };
    let label = match draft {
        Choice::Open => "Start, no timer".to_owned(),
        timed @ Choice::Minutes(_) => format!("Start \u{b7} {}", length_text(timed)),
    };
    let painter = ui.painter();
    painter.rect_filled(button, CornerRadius::same(10), fill);
    painter.text(button.center(), Align2::CENTER_CENTER, label, body(TEXT_BODY), brand::GROUND);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn turn_off_row(ui: &mut Ui) -> Response {
    let (rect, response) = menu_row(ui, MENU_ROW_H, false, true);
    let icon = pos2(rect.left() + MENU_ROW_PAD + 8.0, rect.center().y);
    let painter = ui.painter();
    painter.text(
        icon,
        Align2::CENTER_CENTER,
        icons::X,
        egui::FontId::proportional(16.0),
        brand::SOFT,
    );
    painter.text(
        pos2(icon.x + 18.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Turn off",
        body(TEXT_BODY),
        brand::TEXT,
    );
    response
}

/// The band's words for each Away state: title, then the line under it.
pub(crate) fn band_texts(away: bool) -> (&'static str, &'static str) {
    if away {
        ("You are away", "Agents decide on their own")
    } else {
        ("Away", "Let agents decide on their own")
    }
}

fn away_band(ui: &mut Ui, away: bool) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), BAND_H), Sense::click());
    let hovered = response.hovered();
    let (fill, line, title_ink) = match (away, hovered) {
        (true, _) => (brand::VIOLET_FILL, brand::VIOLET_LINE, brand::VIOLET_TEXT),
        (false, true) => (brand::SEGMENT_ON, brand::FIELD_LINE, brand::TEXT),
        (false, false) => (brand::SUNKEN, brand::CARD_LINE, brand::TEXT),
    };
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(10), fill, Stroke::new(1.0, line), StrokeKind::Inside);
    let moon_at = pos2(rect.left() + 22.0, rect.center().y);
    paint_moon(painter, moon_at, 7.0, if away { brand::VIOLET_TEXT } else { brand::MUTED }, fill);
    let (title, sub) = band_texts(away);
    let text_x = rect.left() + 40.0;
    painter.text(
        pos2(text_x, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        title,
        body(TEXT_BODY),
        title_ink,
    );
    painter.text(
        pos2(text_x, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        sub,
        body(TEXT_TAG),
        brand::MUTED,
    );
    let track =
        Rect::from_center_size(pos2(rect.right() - 14.0 - SWITCH.x / 2.0, rect.center().y), SWITCH);
    let radius = CornerRadius::same((SWITCH.y / 2.0) as u8);
    painter.rect_filled(track, radius, if away { brand::VIOLET } else { brand::FIELD_LINE });
    let knob_x = if away { track.right() - SWITCH.y / 2.0 } else { track.left() + SWITCH.y / 2.0 };
    painter.circle_filled(pos2(knob_x, track.center().y), SWITCH.y / 2.0 - 3.0, brand::TEXT);
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, away, "Away"));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A crescent: a disc with a second disc in `cut` laid over its upper right.
pub(crate) fn paint_moon(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    ink: Color32,
    cut: Color32,
) {
    painter.circle_filled(center, radius, ink);
    painter.circle_filled(center + vec2(radius * 0.45, -radius * 0.4), radius * 0.8, cut);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_read_as_hours_and_minutes_or_infinity() {
        let cases = [
            (Choice::Minutes(15), "15m"),
            (Choice::Minutes(60), "1h"),
            (Choice::Minutes(90), "1h 30m"),
            (Choice::Open, INFINITY),
        ];
        for (choice, want) in cases {
            assert_eq!(length_text(choice), want, "{choice:?}");
        }
    }

    #[test]
    fn the_end_line_names_the_clock_time_or_no_timer() {
        let utc = TimeZone::UTC;
        assert_eq!(ends_text(Choice::Minutes(90), 0.0, &utc), "until 01:30");
        assert_eq!(ends_text(Choice::Open, 0.0, &utc), "until you turn it off");
    }

    #[test]
    fn a_click_ends_on_the_hour_under_the_pointer() {
        let cases = [(0.0, 60), (0.04, 60), (0.09, 120), (0.5, 420), (0.999, 720), (1.4, 720)];
        for (fraction, minutes) in cases {
            assert_eq!(clicked_choice(fraction), Choice::Minutes(minutes), "{fraction}");
        }
    }

    #[test]
    fn a_drag_snaps_to_quarter_hours_between_the_shortest_and_longest() {
        let cases = [(-0.2, 15), (0.0, 15), (0.03, 15), (0.0625, 45), (0.5, 360), (1.0, 720)];
        for (fraction, minutes) in cases {
            assert_eq!(dragged_choice(fraction), Choice::Minutes(minutes), "{fraction}");
        }
    }

    #[test]
    fn the_track_glows_for_the_length_and_the_cap_for_no_timer() {
        assert_eq!(lit_hours(Choice::Minutes(90)), Some(1.5));
        assert_eq!(lit_hours(Choice::Minutes(900)), Some(12.0));
        assert_eq!(lit_hours(Choice::Open), None);
    }
}
