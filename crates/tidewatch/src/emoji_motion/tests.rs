use super::*;

fn input(now: f64, hovered: bool, fresh: bool) -> Input {
    Input { now, hovered, fresh, allowed: true, total: 1.0 }
}

fn run(frames: &[Input]) -> Vec<Option<f64>> {
    let mut state = State::default();
    frames
        .iter()
        .map(|frame| {
            let (next, elapsed) = advance(state, *frame);
            state = next;
            elapsed
        })
        .collect()
}

#[test]
fn a_new_message_plays_once_and_then_rests() {
    let seen = run(&[
        input(10.0, false, true),
        input(10.5, false, true),
        input(10.75, false, true),
        input(11.0, false, true),
        input(30.0, false, true),
    ]);
    assert_eq!(seen, [Some(0.0), Some(0.5), Some(0.75), None, None]);
}

#[test]
fn a_message_loaded_from_history_stays_still() {
    let seen =
        run(&[input(10.0, false, false), input(10.2, false, false), input(60.0, false, false)]);
    assert_eq!(seen, [None, None, None]);
}

#[test]
fn pointing_at_an_emoji_plays_it_again_once_per_visit() {
    let seen = run(&[
        input(0.0, false, false),
        input(5.0, true, false),
        input(5.5, true, false),
        input(6.5, true, false),
        input(7.0, false, false),
        input(8.0, true, false),
    ]);
    assert_eq!(seen, [None, Some(0.0), Some(0.5), None, None, Some(0.0)]);
}

#[test]
fn pointing_at_an_emoji_that_is_playing_starts_it_over() {
    let seen = run(&[input(0.0, false, true), input(0.5, true, true), input(0.75, true, true)]);
    assert_eq!(seen, [Some(0.0), Some(0.0), Some(0.25)]);
}

#[test]
fn a_message_that_was_seen_does_not_play_again_when_it_is_drawn_as_new_again() {
    let seen = run(&[input(0.0, false, true), input(2.0, false, true), input(9.0, false, true)]);
    assert_eq!(seen, [Some(0.0), None, None]);
}

#[test]
fn with_animation_off_nothing_plays_and_turning_it_on_does_not_replay_old_messages() {
    let off = |now, hovered| Input { allowed: false, ..input(now, hovered, true) };
    let mut state = State::default();
    for frame in [off(0.0, false), off(0.5, true)] {
        let (next, elapsed) = advance(state, frame);
        assert_eq!(elapsed, None);
        state = next;
    }
    let (_, later) = advance(state, input(5.0, false, true));
    assert_eq!(later, None);
}

#[test]
fn the_clip_is_asked_for_only_when_something_is_about_to_play() {
    let rest = State::default();
    assert!(rest.wants_clip(false, true));
    assert!(rest.wants_clip(true, false));
    assert!(!rest.wants_clip(false, false));
    let (seen, _) = advance(rest, input(0.0, false, false));
    assert!(!seen.wants_clip(false, true));
    assert!(!seen.wants_clip(false, false));
    let (playing, _) = advance(seen, input(1.0, true, false));
    assert!(playing.wants_clip(true, false));
}

#[test]
fn a_clip_has_the_frames_the_file_holds_and_plays_about_a_second() {
    let ctx = Context::default();
    let clip = clip(&ctx, "1f600").unwrap();
    assert!(clip.ends.len() > 10, "{}", clip.ends.len());
    assert!((0.4..3.0).contains(&clip.total()), "{}", clip.total());
    assert_eq!(clip.frame_at(0.0), Some(0));
    assert_eq!(clip.frame_at(clip.total() + 0.1), None);
    assert_eq!(clip.frame_at(clip.total() - 0.001), Some(clip.ends.len() - 1));
    assert!(clip.ends.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn every_frame_maps_to_its_own_cell_of_the_atlas_inside_the_texture() {
    let ctx = Context::default();
    let clip = clip(&ctx, "1f600").unwrap();
    let mut seen = Vec::new();
    for frame in 0..clip.ends.len() {
        let uv = clip.uv(frame);
        assert!(uv.min.x >= 0.0 && uv.min.y >= 0.0 && uv.max.x <= 1.0 && uv.max.y <= 1.0, "{uv:?}");
        assert!(uv.width() > 0.0 && uv.height() > 0.0);
        let [width, height] = clip.texture.size();
        let cell = (uv.width() * width as f32 + 1.0, uv.height() * height as f32 + 1.0);
        assert_eq!(
            (cell.0.round() as usize, cell.1.round() as usize),
            (clip.cell[0], clip.cell[1])
        );
        seen.push((uv.min.x.to_bits(), uv.min.y.to_bits()));
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), clip.ends.len());
}

#[test]
fn a_clip_is_decoded_once_and_an_emoji_without_one_has_none() {
    let ctx = Context::default();
    let first = clip(&ctx, "2764").unwrap();
    assert!(Arc::ptr_eq(&first, &clip(&ctx, "2764").unwrap()));
    assert!(clip(&ctx, "1f1e7_1f1ea").is_none());
    assert!(!has_clip("1f1e7_1f1ea"));
}

#[test]
fn a_file_that_does_not_decode_is_no_clip_so_the_still_picture_stays() {
    assert!(decode(b"not a webp").is_none());
    assert!(decode(&[]).is_none());
    let mut cut = emoji_pack::moving().get("1f600").unwrap().to_vec();
    cut.truncate(60);
    assert!(decode(&cut).is_none());
}

#[test]
fn the_switch_defaults_on_and_the_fresh_scope_hands_back_what_it_found() {
    let ctx = Context::default();
    assert!(allowed(&ctx));
    set_allowed(&ctx, false);
    assert!(!allowed(&ctx));
    assert!(!is_fresh(&ctx));
    let inside = fresh_scope(&ctx, true, || {
        let inner = fresh_scope(&ctx, false, || is_fresh(&ctx));
        (is_fresh(&ctx), inner)
    });
    assert_eq!(inside, (true, false));
    assert!(!is_fresh(&ctx));
}

#[test]
fn reduce_motion_is_read_again_only_every_two_seconds() {
    let mut reduce = ReduceMotion::default();
    let mut reads = 0;
    let mut ask = |now: f64, answer: bool, reads: &mut i32| {
        reduce.asked(now, || {
            *reads += 1;
            answer
        })
    };
    assert!(ask(0.0, true, &mut reads));
    assert!(ask(1.0, false, &mut reads));
    assert_eq!(reads, 1);
    assert!(!ask(2.5, false, &mut reads));
    assert_eq!(reads, 2);
}
