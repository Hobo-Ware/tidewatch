use super::*;

#[test]
fn cards_sit_in_columns_with_the_same_gap_both_ways() {
    let rects = grid(pos2(10.0, 20.0), 408.0, 2, 5, 50.0);
    assert_eq!(rects.len(), 5);
    assert_eq!(rects[0], Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 50.0)));
    assert_eq!(rects[1].left(), 10.0 + 200.0 + GAP);
    assert_eq!(rects[1].right(), 10.0 + 408.0);
    assert_eq!(rects[2].top(), 20.0 + 50.0 + GAP);
    assert_eq!(rects[4].left(), 10.0);
    assert_eq!(grid(pos2(0.0, 0.0), 100.0, 2, 0, 50.0), Vec::<Rect>::new());
}

#[test]
fn the_height_of_a_grid_is_its_rows_and_the_gaps_between_them() {
    let cases =
        [(2, 0, 0.0), (2, 1, 52.0), (2, 2, 52.0), (2, 3, 112.0), (2, 8, 52.0 * 4.0 + 3.0 * GAP)];
    for (columns, count, want) in cases {
        assert_eq!(grid_height(columns, count, 52.0), want, "{count} cards in {columns}");
    }
    assert_eq!(stages_height(4, 4), STAGE_H);
    assert_eq!(stages_height(4, 5), 2.0 * STAGE_H + GAP);
    assert_eq!(stages_height(2, 5), 3.0 * STAGE_H + 2.0 * GAP);
}

#[test]
fn an_empty_list_says_why() {
    assert_eq!(empty_text("zz", true, "profile"), "No profile matches \"zz\".");
    assert!(
        empty_text("", true, "workflow").starts_with("No workflows yet. Add one in Settings (")
    );
    assert_eq!(empty_text("", false, "profile"), "The profiles are still loading...");
}
