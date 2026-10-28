use super::*;

fn chars(text: &str) -> f32 {
    text.chars().count() as f32
}

const LONG: &str =
    "projects/client/src/lib/sections/summary/components/recap/SeasonRecapDrawerHost.svelte:58";

#[test]
fn a_path_that_fits_is_left_alone() {
    let cases = [
        ("src/a.rs:3", 10.0),
        ("src/a.rs:3", 80.0),
        (LONG, LONG.chars().count() as f32),
        ("", 1.0),
    ];
    for (text, room) in cases {
        assert_eq!(middle_cut(text, room, &chars), text, "{text} in {room}");
    }
}

#[test]
fn a_long_path_loses_its_middle_folders_and_keeps_the_file_name_and_line() {
    let cases = [
        (58.0, "projects/client/…/recap/SeasonRecapDrawerHost.svelte:58"),
        (55.0, "projects/client/…/recap/SeasonRecapDrawerHost.svelte:58"),
        (54.0, "projects/…/recap/SeasonRecapDrawerHost.svelte:58"),
        (42.0, "projects/…/SeasonRecapDrawerHost.svelte:58"),
        (40.0, "…/recap/SeasonRecapDrawerHost.svelte:58"),
        (34.0, "…/SeasonRecapDrawerHost.svelte:58"),
        (66.0, "projects/client/…/components/recap/SeasonRecapDrawerHost.svelte:58"),
        (80.0, "projects/client/src/…/summary/components/recap/SeasonRecapDrawerHost.svelte:58"),
    ];
    for (room, want) in cases {
        let got = middle_cut(LONG, room, &chars);
        assert_eq!(got, want, "room {room}");
        assert!(chars(&got) <= room, "room {room}: {got}");
    }
}

#[test]
fn a_home_path_keeps_its_tilde_and_an_absolute_one_its_root() {
    assert_eq!(
        middle_cut("~/code/shop/web/src/cart/Totals.tsx:12", 28.0, &chars),
        "~/code/…/cart/Totals.tsx:12"
    );
    assert_eq!(middle_cut("/opt/vendor/lib/deep/Thing.c:4", 20.0, &chars), "/opt/…/Thing.c:4");
}

#[test]
fn a_file_name_wider_than_the_room_is_cut_in_its_own_middle_and_keeps_the_line() {
    let got = middle_cut(LONG, 20.0, &chars);
    assert_eq!(got, "SeasonRec….svelte:58");
    assert!(got.ends_with(":58") && chars(&got) <= 20.0);
    let bare = middle_cut("SeasonRecapDrawerHost.svelte", 12.0, &chars);
    assert_eq!(bare, "Season…velte");
    assert!(chars(&bare) <= 12.0);
}

#[test]
fn a_room_smaller_than_the_line_suffix_still_returns_something_that_fits_or_the_ellipsis() {
    assert_eq!(middle_cut(LONG, 4.0, &chars), "…:58");
    assert_eq!(middle_cut(LONG, 3.0, &chars), "…");
    assert_eq!(middle_cut(LONG, 1.0, &chars), "…");
    assert_eq!(middle_cut("a/b", 0.0, &chars), "…");
}

#[test]
fn a_measure_with_unequal_glyph_widths_is_honored() {
    let wide_m = |text: &str| text.chars().map(|c| if c == 'm' { 3.0 } else { 1.0 }).sum::<f32>();
    let text = "mmmm/mmmm/mmmm/file.rs:1";
    let got = middle_cut(text, 30.0, &wide_m);
    assert_eq!(got, "mmmm/…/file.rs:1");
    assert!(wide_m(&got) <= 30.0, "{got}");
    assert_eq!(middle_cut(text, 18.0, &wide_m), "…/file.rs:1");
}

#[test]
fn the_cut_never_exceeds_the_room_over_every_width() {
    for room in 1..=LONG.chars().count() {
        let got = middle_cut(LONG, room as f32, &chars);
        assert!(chars(&got) <= room as f32 || got == "…", "room {room}: {got}");
    }
}

#[test]
fn keeps_name_tells_a_whole_file_name_from_one_cut_in_its_own_middle() {
    let cases = [
        (LONG, LONG, true),
        ("projects/…/SeasonRecapDrawerHost.svelte:58", LONG, true),
        ("SeasonRec….svelte:58", LONG, false),
        ("…", LONG, false),
        ("a.rs:3", "a.rs:3", true),
    ];
    for (cut, original, want) in cases {
        assert_eq!(keeps_name(cut, original), want, "{cut}");
    }
}
