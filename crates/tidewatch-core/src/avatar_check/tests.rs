use super::*;

const LOCK: &str = r##"<svg viewBox="0 0 24 24"><path d="M7 11V8a5 5 0 0 1 10 0v3h-2V8a3 3 0 0 0-6 0v3z" fill="#e6e8ec"/><path d="M6 11h12v9H6z M12 13.5a1.6 1.6 0 1 0 0.01 0z" fill="#ff6b6b"/></svg>"##;
const DARK_LOCK: &str = r##"<svg viewBox="0 0 24 24"><path d="M7 11V8a5 5 0 0 1 10 0v3h1v10H6V11z" fill="#2a2f3a"/><circle cx="12" cy="15" r="1.6" fill="#8a1c1c"/></svg>"##;
const PLAY_SPILLING: &str = r##"<svg viewBox="0 0 24 24"><rect x="0" y="4" width="20" height="16" rx="2" fill="#e8a317"/><path d="M8 8l6 4-6 4z" fill="#141413"/></svg>"##;
const BRIGHT_TILE: &str =
    r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" fill="#2dd4bf"/></svg>"##;
const ROUNDED_TILE: &str = r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#123b38"/><ellipse cx="32" cy="20" rx="16" ry="6" fill="none" stroke="#2dd4bf" stroke-width="3"/></svg>"##;
const BRIGHT_DISC: &str =
    r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="30" fill="#2dd4bf"/></svg>"##;

fn problems(svg: &str) -> Vec<Unreadable> {
    check(svg).err().unwrap_or_default()
}

fn dark(percent_at_least: u32) -> impl Fn(&Unreadable) -> bool {
    move |p| matches!(p, Unreadable::TooDark { percent } if *percent >= percent_at_least)
}

fn spills(p: &Unreadable) -> bool {
    matches!(p, Unreadable::Spills { .. })
}

#[test]
fn bright_art_inside_the_circle_is_kept() {
    for (what, svg) in [
        ("the board's lock", LOCK),
        ("a disc that fills the circle", BRIGHT_DISC),
        (
            "a thin bright stroke",
            r##"<svg viewBox="0 0 64 64"><path d="M12 32h40M32 12v40" stroke="#ffffff" stroke-width="2"/></svg>"##,
        ),
        (
            "a half transparent bright shape",
            r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="20" fill="#ffffff" fill-opacity="0.6"/></svg>"##,
        ),
        (
            "a small bright dot on nothing",
            r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="8" fill="#fbbf24"/></svg>"##,
        ),
    ] {
        assert_eq!(check(svg), Ok(()), "{what}");
    }
}

#[test]
fn dark_on_dark_is_refused_with_the_share_and_the_plate() {
    let found = problems(DARK_LOCK);
    assert!(found.iter().any(dark(90)), "{found:?}");
    let said = found.iter().map(ToString::to_string).collect::<String>();
    assert!(said.contains("#222937") && said.contains("3:1"), "{said}");
}

#[test]
fn a_dark_tile_behind_bright_lines_is_refused_because_most_of_what_is_painted_is_the_tile() {
    let found = problems(ROUNDED_TILE);
    assert!(found.iter().any(dark(50)), "{found:?}");
}

#[test]
fn bright_art_with_a_dark_minority_is_kept() {
    let svg = r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="26" fill="#2dd4bf"/><circle cx="32" cy="32" r="8" fill="#0d1016"/></svg>"##;
    assert_eq!(check(svg), Ok(()));
}

#[test]
fn a_full_bleed_square_is_refused_even_when_it_is_bright() {
    let found = problems(BRIGHT_TILE);
    assert!(found.iter().any(spills), "{found:?}");
    assert!(!found.iter().any(dark(1)), "{found:?}");
}

#[test]
fn art_that_runs_past_the_circle_is_refused_and_says_where_to_keep_it() {
    let found = problems(PLAY_SPILLING);
    let spill = found.iter().find(|p| spills(p)).unwrap_or_else(|| panic!("{found:?}"));
    assert!(spill.to_string().contains("middle circle of the viewBox"), "{spill}");
}

#[test]
fn a_rounded_tile_spills_too_and_both_problems_come_back_together() {
    let found = problems(ROUNDED_TILE);
    assert!(found.iter().any(spills) && found.iter().any(dark(1)), "{found:?}");
}

#[test]
fn a_few_stray_pixels_at_a_corner_are_within_the_tolerance() {
    let svg = r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="22" fill="#2dd4bf"/><rect x="0" y="0" width="3" height="3" fill="#ffffff"/></svg>"##;
    assert_eq!(check(svg), Ok(()));
}

#[test]
fn a_mostly_transparent_picture_is_judged_on_what_is_painted() {
    let faint = r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="20" fill="#ffffff" fill-opacity="0.2"/></svg>"##;
    let invisible = r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="20" fill="#ffffff" opacity="0"/></svg>"##;
    let unpainted =
        r#"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="20" fill="none"/></svg>"#;
    let speck =
        r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="1" fill="#ffffff"/></svg>"##;
    for (what, svg) in [
        ("opacity 0.2", faint),
        ("opacity 0", invisible),
        ("fill none", unpainted),
        ("a 2 unit speck", speck),
    ] {
        assert_eq!(check(svg), Err(vec![Unreadable::NearlyEmpty]), "{what}");
    }
}

#[test]
fn a_picture_that_cannot_be_drawn_is_refused_not_a_panic() {
    assert_eq!(check("<svg"), Err(vec![Unreadable::Undrawable]));
    assert_eq!(check(""), Err(vec![Unreadable::Undrawable]));
}

#[test]
fn a_wide_picture_is_judged_where_it_is_drawn_not_stretched_to_the_square() {
    let wide =
        r##"<svg viewBox="0 0 128 64"><rect width="128" height="64" fill="#2dd4bf"/></svg>"##;
    let bar = r##"<svg viewBox="0 0 128 16"><rect width="128" height="16" fill="#2dd4bf"/></svg>"##;
    assert!(problems(wide).iter().any(spills));
    assert_eq!(check(bar), Ok(()));
}

#[test]
fn the_ratio_is_the_wcag_one() {
    assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
    assert!((contrast([255, 255, 255], [255, 255, 255]) - 1.0).abs() < 1e-9);
    assert!((contrast([0x77, 0x77, 0x77], [255, 255, 255]) - 4.48).abs() < 0.01);
    assert_eq!(contrast([10, 20, 30], [200, 100, 50]), contrast([200, 100, 50], [10, 20, 30]));
}

#[test]
fn the_three_to_one_line_sits_where_it_should_on_the_plate() {
    let gray = |v: u8| [v, v, v];
    assert!(contrast(gray(0x60), PLATE) < MIN_CONTRAST);
    assert!(contrast(gray(0x80), PLATE) > MIN_CONTRAST);
}

#[test]
fn the_fit_is_centered_and_the_inset_shrinks_it() {
    let full = rasterize(BRIGHT_TILE, 40, 1.0).unwrap();
    assert!(full.pixels().iter().all(|p| p.alpha() == 255));
    let inset = rasterize(BRIGHT_TILE, 40, 0.6).unwrap();
    let at = |x: u32, y: u32| inset.pixels()[(y * 40 + x) as usize].alpha();
    assert_eq!((at(0, 0), at(39, 39), at(20, 20)), (0, 0, 255));
    assert_eq!((at(7, 20), at(8, 20), at(31, 20), at(32, 20)), (0, 255, 255, 0));
    let wide = rasterize(
        r##"<svg viewBox="0 0 128 64"><rect width="128" height="64" fill="#fff"/></svg>"##,
        32,
        1.0,
    )
    .unwrap();
    let wide_at = |x: u32, y: u32| wide.pixels()[(y * 32 + x) as usize].alpha();
    assert_eq!(
        (wide_at(16, 0), wide_at(16, 8), wide_at(16, 16), wide_at(16, 31)),
        (0, 255, 255, 0)
    );
}

#[test]
fn the_plate_hex_reads_back_as_the_hex_in_the_message() {
    assert_eq!(hex(PLATE), "#222937");
}

fn reach_of(pixmap: &Pixmap) -> (f32, (f32, f32)) {
    let px = pixmap.width();
    let painted: Vec<(f32, f32)> = pixmap
        .pixels()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.alpha() >= PAINTED_ALPHA)
        .map(|(i, _)| ((i as u32 % px) as f32 + 0.5, (i as u32 / px) as f32 + 0.5))
        .collect();
    let n = painted.len() as f32;
    let center = (
        painted.iter().map(|p| p.0).sum::<f32>() / n,
        painted.iter().map(|p| p.1).sum::<f32>() / n,
    );
    let half = px as f32 / 2.0;
    let reach = painted.iter().map(|&(x, y)| (x - half).hypot(y - half)).fold(0.0, f32::max);
    (reach / half, center)
}

#[test]
fn a_small_centered_disc_is_grown_to_the_fill() {
    let disc =
        r##"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="20" fill="#38e1a0"/></svg>"##;
    let fitted = rasterize_fitted(disc, 96, FACE_FILL).unwrap();
    let (reach, _) = reach_of(&fitted);
    assert!((reach - FACE_FILL).abs() < 0.04, "{reach}");
}

#[test]
fn art_drawn_off_center_is_moved_to_the_middle() {
    let corner =
        r##"<svg viewBox="0 0 64 64"><circle cx="16" cy="16" r="8" fill="#ffffff"/></svg>"##;
    let fitted = rasterize_fitted(corner, 96, FACE_FILL).unwrap();
    let (reach, (x, y)) = reach_of(&fitted);
    assert!((x - 48.0).abs() < 1.0 && (y - 48.0).abs() < 1.0, "{x},{y}");
    assert!(reach <= FACE_FILL + 0.02, "{reach}");
}

#[test]
fn a_square_tile_is_fitted_corners_inside_the_circle() {
    let fitted = rasterize_fitted(BRIGHT_TILE, 96, FACE_FILL).unwrap();
    let (reach, _) = reach_of(&fitted);
    assert!(reach <= FACE_FILL + 0.02, "{reach}");
}

#[test]
fn a_fit_of_nothing_painted_is_none() {
    let empty = r#"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="20" fill="none"/></svg>"#;
    assert!(rasterize_fitted(empty, 48, FACE_FILL).is_none());
    assert!(rasterize_fitted("<svg", 48, FACE_FILL).is_none());
}
