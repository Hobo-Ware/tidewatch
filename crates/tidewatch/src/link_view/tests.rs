use super::*;

#[test]
fn chips_fill_a_row_then_wrap_and_never_overlap() {
    let widths = [60.0, 60.0, 80.0, 90.0, 120.0];
    let (at, height) = flow(&widths, 300.0);
    assert_eq!(at[0], pos2(0.0, 0.0));
    assert_eq!(at[1].x, 60.0 + CHIP_GAP);
    assert_eq!(at[3].y, CHIP_H + CHIP_GAP, "the fourth chip does not fit in the first row");
    assert_eq!(height, 2.0 * CHIP_H + CHIP_GAP);
    for (index, a) in at.iter().enumerate() {
        let one = Rect::from_min_size(*a, vec2(widths[index], CHIP_H));
        assert!(one.right() <= 300.0 + 0.01, "chip {index} leaves the row");
        for (other, b) in at.iter().enumerate().skip(index + 1) {
            let two = Rect::from_min_size(*b, vec2(widths[other], CHIP_H));
            assert!(!one.intersects(two), "chips {index} and {other} overlap");
        }
    }
}

#[test]
fn a_chip_wider_than_the_row_still_gets_a_row_of_its_own() {
    let (at, height) = flow(&[50.0, 400.0, 50.0], 300.0);
    assert_eq!(
        at.iter().map(|p| p.y).collect::<Vec<_>>(),
        [0.0, CHIP_H + CHIP_GAP, 2.0 * (CHIP_H + CHIP_GAP)]
    );
    assert_eq!(height, 3.0 * CHIP_H + 2.0 * CHIP_GAP);
}

#[test]
fn no_chips_take_no_height() {
    assert_eq!(flow(&[], 300.0), (Vec::new(), 0.0));
}

#[test]
fn the_dev_switch_reads_the_chips_the_search_and_the_fold() {
    assert!(LinkView::from_dev(None).is_none());
    let view = LinkView::from_dev(Some("prs,done,fix,all")).unwrap();
    assert_eq!(
        (view.filter.kind, view.filter.bucket, view.filter.search.as_str(), view.all),
        (Type::Prs, Some(Bucket::Done), "fix", true)
    );
    let plain = LinkView::from_dev(Some("all,any")).unwrap();
    assert_eq!((plain.filter, plain.all), (Filter::default(), false));
}
