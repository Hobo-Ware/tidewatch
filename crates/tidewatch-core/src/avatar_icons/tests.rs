use super::*;
use crate::avatar;
use crate::avatar_check::{self, MIN_CONTRAST, PLATE};

const SAMPLE: [&str; 12] = [
    "music",
    "shield",
    "refresh-cw",
    "database",
    "bug",
    "rocket",
    "book-open",
    "square",
    "minus",
    "ellipsis",
    "git-branch",
    "triangle-alert",
];

#[test]
fn the_pack_holds_over_a_thousand_icons_including_the_common_ones() {
    assert!(count() > 1000, "{}", count());
    for name in SAMPLE {
        assert!(names().contains(&name), "{name}");
    }
}

#[test]
fn an_older_name_is_an_alias_of_the_icon_it_became() {
    let old = svg("alert-triangle", "#ffffff").unwrap();
    let new = svg("triangle-alert", "#ffffff").unwrap();
    assert_eq!(old, new);
}

#[test]
fn names_are_matched_without_regard_to_case_spaces_and_underscores() {
    let want = svg("book-open", "#ffffff");
    assert_eq!(svg("Book Open", "#ffffff"), want);
    assert_eq!(svg(" book_open ", "#ffffff"), want);
}

#[test]
fn an_unknown_name_has_no_picture() {
    assert_eq!(svg("not-an-icon", "#ffffff"), None);
    assert_eq!(svg("", "#ffffff"), None);
}

#[test]
fn the_color_replaces_current_color_everywhere_and_strokes_stay_lucides() {
    let with_fill = names()
        .iter()
        .find(|name| pack().shapes[**name].contains("currentColor"))
        .expect("some icon fills a dot with the current color");
    let drawn = svg(with_fill, "#2dd4bf").unwrap();
    assert_eq!(drawn.matches("currentColor").count(), 0, "{drawn}");
    assert!(drawn.contains("stroke=\"#2dd4bf\"") && drawn.contains("fill=\"#2dd4bf\""));
    assert!(drawn.contains("stroke-width=\"2\"") && drawn.contains("stroke-linecap=\"round\""));
}

#[test]
fn every_icon_survives_the_cleaner_with_all_its_shapes() {
    for name in names() {
        let drawn = svg(name, "#2dd4bf").unwrap();
        let cleaned = avatar::sanitize(&drawn).unwrap_or_else(|e| panic!("{name}: {e}"));
        let shapes = |text: &str| {
            ["<path", "<circle", "<rect", "<line", "<ellipse", "<polyline", "<polygon"]
                .iter()
                .map(|tag| text.matches(tag).count())
                .sum::<usize>()
        };
        assert_eq!(shapes(&cleaned), shapes(&drawn), "{name}");
    }
}

#[test]
fn every_color_is_readable_on_the_plate() {
    for (name, rgb) in COLORS {
        let ratio = avatar_check::contrast(rgb, PLATE);
        assert!(ratio >= MIN_CONTRAST, "{name} is {ratio:.2}:1");
    }
}

#[test]
fn every_color_passes_the_readability_check_on_a_sample_of_icons() {
    for (color, _) in COLORS {
        let ink = color_hex(color).unwrap();
        for name in SAMPLE {
            let drawn = svg(name, &ink).unwrap();
            assert_eq!(avatar_check::check(&drawn), Ok(()), "{name} in {color}");
        }
    }
}

#[test]
fn every_icon_passes_the_readability_check_in_teal_but_the_two_that_draw_a_single_dot() {
    let ink = color_hex("teal").unwrap();
    let failed: Vec<_> = names()
        .iter()
        .filter_map(|name| {
            avatar_check::check(&svg(name, &ink).unwrap()).err().map(|why| (*name, why))
        })
        .collect();
    let nearly_empty = vec![avatar_check::Unreadable::NearlyEmpty];
    assert_eq!(failed, [("signal-zero", nearly_empty.clone()), ("wifi-zero", nearly_empty)]);
}

#[test]
fn color_names_map_to_hex_whatever_their_case() {
    assert_eq!(color_hex("teal"), Some("#2dd4bf".to_owned()));
    assert_eq!(color_hex(" Purple "), Some("#c4b5fd".to_owned()));
    assert_eq!(color_hex("chartreuse"), None);
    assert_eq!(
        color_names(),
        ["teal", "blue", "green", "amber", "orange", "red", "pink", "purple"]
    );
}

#[test]
fn suggestions_put_names_that_start_with_the_word_first_then_those_that_contain_it() {
    let found = suggest("book");
    assert_eq!(found.len(), 8);
    assert_eq!(found[0], "book");
    let starts = found.iter().take_while(|name| name.starts_with("book")).count();
    assert!(found[starts..].iter().all(|name| name.contains("book")), "{found:?}");
}

#[test]
fn suggestions_fall_back_to_the_words_of_a_made_up_name() {
    assert!(suggest("music-note").contains(&"music"));
    assert_eq!(suggest("zzzz"), Vec::<&str>::new());
    assert_eq!(suggest("  "), Vec::<&str>::new());
}

#[test]
fn a_cut_pack_is_refused_rather_than_read_past_its_end() {
    for cut in [0, 4, 8, 20, PACK.len() - 1] {
        assert!(Pack::parse(&PACK[..cut]).is_none(), "{cut}");
    }
}
