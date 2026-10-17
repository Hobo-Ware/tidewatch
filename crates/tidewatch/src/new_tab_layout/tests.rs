use super::*;

fn left_column(window: (f32, f32)) -> Rect {
    let (width, height) = window;
    Rect::from_min_size(pos2(0.0, 0.0), vec2((width * 0.58).round(), height - 78.0))
}

fn sized(width: f32, height: f32) -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(width, height))
}

fn areas() -> [(&'static str, Rect); 4] {
    [
        ("1280x720", left_column((1280.0, 720.0))),
        ("1440x900", left_column((1440.0, 900.0))),
        ("1100x640", left_column((1100.0, 640.0))),
        ("short 980x560 column", sized(980.0, 560.0)),
    ]
}

fn disjoint(rects: &[(&str, Rect)]) {
    for (index, (a_name, a)) in rects.iter().enumerate() {
        for (b_name, b) in &rects[index + 1..] {
            assert!(!a.intersects(*b), "{a_name} {a:?} meets {b_name} {b:?}");
        }
    }
}

#[test]
fn the_header_lane_and_pane_share_the_form_without_meeting() {
    for (name, area) in areas() {
        let form = form_rect(area);
        let columns = columns(form);
        assert!(form.contains_rect(columns.lane), "{name}: lane in form");
        assert!(form.contains_rect(columns.pane), "{name}: pane in form");
        assert_eq!(columns.header.bottom() + HEADER_GAP, columns.lane.top(), "{name}");
        assert_eq!(columns.lane.top(), columns.pane.top(), "{name}");
        assert_eq!(columns.lane.bottom(), form.bottom(), "{name}: the lane is as tall as the form");
        assert_eq!(columns.pane.bottom(), form.bottom(), "{name}");
        assert_eq!(columns.pane.left() - columns.lane.right(), COLUMN_GAP, "{name}");
        disjoint(&[("header", columns.header), ("lane", columns.lane), ("pane", columns.pane)]);
    }
}

#[test]
fn the_lane_is_tight_and_never_more_than_38_percent_of_the_form() {
    let cases = [
        (400.0, 152.0),
        (600.0, 228.0),
        (690.0, 240.0),
        (800.0, 240.0),
        (900.0, 270.0),
        (1100.0, 280.0),
    ];
    for (width, want) in cases {
        let got = lane_width(width);
        assert!((got - want).abs() < 0.01, "form {width} wide: {got}");
        assert!(got <= width * 0.38 + 0.01, "form {width} wide");
    }
}

#[test]
fn the_form_stops_growing_on_a_very_wide_column() {
    assert_eq!(form_rect(sized(3000.0, 800.0)).width(), COLUMN_MAX_W);
    assert_eq!(form_rect(sized(300.0, 800.0)).width(), 300.0 - 2.0 * SIDE_PAD);
    assert_eq!(form_rect(sized(40.0, 800.0)).width(), 0.0);
}

#[test]
fn the_lane_gives_its_top_to_the_filter_and_the_rest_to_the_list() {
    let lane = sized(260.0, 500.0);
    let (top, list) = lane_parts(lane);
    assert_eq!(top.height(), FILTER_H);
    assert_eq!(list.top(), top.bottom() + LANE_LIST_GAP);
    assert_eq!(list.bottom(), lane.bottom());
    let (top, list) = lane_parts(sized(260.0, 10.0));
    assert_eq!((top.height(), list.height()), (10.0, 0.0));
}

#[test]
fn lane_slots_stack_groups_and_rows_with_their_gaps() {
    use Slot::{Group, Row};
    let (placed, total) = lane_slots(&[Group, Row, Row, Group, Row]);
    let want = [
        (0.0, GROUP_H),
        (GROUP_H + ROW_GAP, ROW_H),
        (GROUP_H + 2.0 * ROW_GAP + ROW_H, ROW_H),
        (GROUP_H + 2.0 * ROW_GAP + 2.0 * ROW_H + GROUP_GAP, GROUP_H),
        (2.0 * GROUP_H + 3.0 * ROW_GAP + 2.0 * ROW_H + GROUP_GAP, ROW_H),
    ];
    assert_eq!(placed, want);
    assert_eq!(total, 2.0 * GROUP_H + 3.0 * ROW_GAP + 3.0 * ROW_H + GROUP_GAP);
    assert_eq!(lane_slots(&[]), (Vec::new(), 0.0));
    assert_eq!(lane_slots(&[Row]), (vec![(0.0, ROW_H)], ROW_H));
}

#[test]
fn name_and_branch_sit_side_by_side_until_the_pane_is_narrow() {
    let cases = [(500.0, false), (380.0, false), (379.0, true), (200.0, true)];
    for (width, stacked) in cases {
        assert_eq!(fields_stacked(width), stacked, "{width}");
        for problem in [false, true] {
            let area =
                Rect::from_min_size(pos2(5.0, 7.0), vec2(width, fields_height(width, problem)));
            let (name, base) = field_rects(area, problem);
            disjoint(&[("name", name), ("base", base)]);
            assert!(area.contains_rect(name) && area.contains_rect(base), "{width} {problem}");
            assert_eq!(base.top() > name.top(), stacked, "{width}");
            let lowest = name.bottom().max(base.bottom());
            assert_eq!(lowest, area.bottom(), "{width} {problem}: the height is the blocks'");
        }
    }
}

#[test]
fn a_taken_name_makes_its_block_taller_by_the_problem_line() {
    for width in [300.0, 500.0] {
        assert_eq!(fields_height(width, true) - fields_height(width, false), 2.0 + PROBLEM_H);
    }
}

#[test]
fn stage_cards_use_as_many_columns_as_fit() {
    let cases = [
        (100.0, 1),
        (247.0, 1),
        (248.0, 2),
        (375.0, 2),
        (376.0, 3),
        (503.0, 3),
        (504.0, 4),
        (900.0, 4),
    ];
    for (width, want) in cases {
        assert_eq!(stage_columns(width), want, "{width}");
    }
}

#[test]
fn what_stands_above_the_message_is_the_blocks_present_and_a_gap_after_each() {
    assert_eq!(Above::default().height(), 0.0);
    let above = Above { permission: 70.0, stages: 0.0, warnings: 0.0, fields: 54.0 };
    assert_eq!(above.height(), 70.0 + 54.0 + 2.0 * SECTION_GAP);
    let all = Above { permission: 1.0, stages: 2.0, warnings: 3.0, fields: 4.0 };
    assert_eq!(all.height(), 10.0 + 4.0 * SECTION_GAP);
}

fn measure(above: Above, hint: f32) -> impl Fn(f32) -> (Above, f32) {
    move |_| (above, hint)
}

#[test]
fn the_message_box_takes_the_rest_of_the_pane_above_the_pinned_footer() {
    let above = Above { permission: 80.0, stages: 0.0, warnings: 0.0, fields: 54.0 };
    for (name, area) in areas() {
        let pane_rect = columns(form_rect(area)).pane;
        let plan = pane(pane_rect, measure(above, 34.0));
        assert!(!plan.scrolls, "{name}");
        assert_eq!(plan.content_w, pane_rect.width(), "{name}");
        assert!(plan.message_h >= MESSAGE_MIN_H, "{name}");
        let content = above.height() + LABEL_H + LABEL_GAP + plan.message_h + 34.0;
        assert!((content - plan.body.height()).abs() < 0.01, "{name}: the box fills what is left");
        assert!(plan.body.bottom() <= plan.footer.top(), "{name}: body over footer");
        assert_eq!(plan.footer.height(), FOOTER_H, "{name}");
        assert_eq!(plan.footer.bottom(), pane_rect.bottom() - BOTTOM_PAD, "{name}");
        assert_eq!(
            (plan.footer.left(), plan.footer.right()),
            (pane_rect.left(), pane_rect.right())
        );
    }
}

#[test]
fn a_pane_too_short_for_the_minimum_box_scrolls_as_a_whole_and_keeps_its_footer() {
    let above = Above { permission: 80.0, stages: 0.0, warnings: 30.0, fields: 54.0 };
    let pane_rect = columns(form_rect(sized(900.0, 330.0))).pane;
    let plan = pane(pane_rect, measure(above, 34.0));
    assert!(plan.scrolls);
    assert_eq!(plan.message_h, MESSAGE_MIN_H);
    assert_eq!(plan.content_w, pane_rect.width() - SCROLL_GUTTER);
    assert!(plan.body.bottom() <= plan.footer.top());
    assert_eq!(plan.footer.bottom(), pane_rect.bottom() - BOTTOM_PAD);
    let content = above.height() + LABEL_H + LABEL_GAP + MESSAGE_MIN_H + 34.0;
    assert!(content > plan.body.height());
}

#[test]
fn the_content_is_measured_again_at_the_width_a_scrollbar_leaves() {
    let measured = std::cell::RefCell::new(Vec::new());
    let pane_rect = columns(form_rect(sized(900.0, 300.0))).pane;
    let plan = pane(pane_rect, |width| {
        measured.borrow_mut().push(width);
        (Above { permission: 200.0, ..Above::default() }, 20.0)
    });
    assert!(plan.scrolls);
    assert_eq!(*measured.borrow(), [pane_rect.width(), pane_rect.width() - SCROLL_GUTTER]);
}

#[test]
fn the_footer_never_leaves_the_pane_however_short_it_is() {
    for height in [0.0, 20.0, 60.0, 120.0, 200.0, 400.0] {
        let rect = Rect::from_min_size(pos2(10.0, 30.0), vec2(400.0, height));
        let plan = pane(rect, measure(Above { fields: 54.0, ..Above::default() }, 30.0));
        assert!(rect.contains_rect(plan.footer), "pane {height} tall: {:?}", plan.footer);
        assert!(plan.body.bottom() <= plan.footer.top() + 0.001, "pane {height} tall");
    }
}
