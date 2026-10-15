use std::time::Duration;

use egui::{RawInput, ViewportId, ViewportInfo, pos2};

use super::*;

const POSES: [Pose; 3] = [Pose::Peek, Pose::Asleep, Pose::Caught];

fn opaque_pixels(image: &ColorImage) -> usize {
    image.pixels.iter().filter(|p| p.a() > 0).count()
}

fn input(focused: bool, minimized: bool) -> RawInput {
    let info = ViewportInfo {
        focused: Some(focused),
        minimized: Some(minimized),
        ..ViewportInfo::default()
    };
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0))),
        viewports: std::iter::once((ViewportId::ROOT, info)).collect(),
        ..RawInput::default()
    }
}

fn repaint_after(ctx: &Context, raw: RawInput) -> Duration {
    let output = ctx.run_ui(raw, |ui| {
        let area = ui.max_rect();
        show(ui, area, Pose::Peek);
    });
    output.viewport_output[&ViewportId::ROOT].repaint_delay
}

#[test]
fn every_pose_names_eyes_that_exist_and_draws_something() {
    for pose in POSES {
        let spec = pose.spec();
        let extent = spec.extent();
        let base = render(pose.base(), extent).expect("the base renders");
        assert!(opaque_pixels(&base) > 1000, "{pose:?}");
        for eye in &spec.eyes {
            let svg = eye_svg(&eye.kind).expect("the eye kind is bundled");
            let image = render(svg, Vec2::splat(eye.size)).expect("the eye renders");
            assert!(opaque_pixels(&image) > 20, "{pose:?} {}", eye.kind);
        }
    }
}

#[test]
fn eyes_sit_inside_the_picture() {
    for pose in POSES {
        let spec = pose.spec();
        let extent = spec.extent();
        for eye in &spec.eyes {
            assert!(eye.x >= 0.0 && eye.y >= 0.0, "{pose:?}");
            assert!(eye.x + eye.size <= extent.x && eye.y + eye.size <= extent.y, "{pose:?}");
        }
    }
}

#[test]
fn a_texture_is_rasterized_again_only_when_the_pixel_size_changes() {
    let ctx = Context::default();
    let id = |pixels| raster(&ctx, Pose::Peek, pixels).expect("rasterizes").base.id();
    let first = id(220.0);
    assert_eq!(id(220.0), first);
    assert_eq!(id(219.8), first);
    assert_ne!(id(330.0), first);
}

#[test]
fn a_focused_visible_window_asks_for_frames_at_a_modest_rate() {
    let ctx = Context::default();
    let delay = repaint_after(&ctx, input(true, false));
    assert!(delay <= Duration::from_millis(33), "{delay:?}");
}

#[test]
fn a_window_without_focus_or_out_of_sight_asks_for_no_frames() {
    for (focused, minimized) in [(false, false), (true, true), (false, true)] {
        let ctx = Context::default();
        repaint_after(&ctx, input(focused, minimized));
        let delay = repaint_after(&ctx, input(focused, minimized));
        assert!(delay > Duration::from_secs(60), "{focused} {minimized}: {delay:?}");
    }
}
