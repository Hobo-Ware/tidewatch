//! The owner's own face at the right of their messages: one `Me` in egui's memory, a picture or a
//! letter, painted round like an agent's face. Nothing is painted until `me_glue` sets one.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, ColorImage, Context, Id, Painter, Rect, TextureHandle, TextureOptions, pos2};
use image::RgbaImage;
use image::imageops::FilterType;

use crate::agent_avatar::{Face, paint_face, plate, ring};
use crate::brand;
use crate::screen_kit::badge_colors;

const STATE: &str = "me-face";
const TINT_KEY: &str = "me";

static VERSIONS: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Picture {
    rgba: RgbaImage,
    version: u64,
}

#[derive(Clone)]
pub(crate) enum Me {
    Picture(Arc<Picture>),
    Letter(String),
}

impl Me {
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
        (rgba.width() > 0 && rgba.height() > 0).then(|| {
            Self::Picture(Arc::new(Picture {
                rgba,
                version: VERSIONS.fetch_add(1, Ordering::Relaxed),
            }))
        })
    }
}

pub(crate) fn set(ctx: &Context, me: Me) {
    ctx.data_mut(|d| d.insert_temp(Id::new(STATE), me));
    ctx.request_repaint();
}

pub(crate) fn current(ctx: &Context) -> Option<Me> {
    ctx.data(|d| d.get_temp::<Me>(Id::new(STATE)))
}

fn clip_to_circle(source: &RgbaImage) -> ColorImage {
    let (width, height) = (source.width() as usize, source.height() as usize);
    let centre = (width as f32 / 2.0, height as f32 / 2.0);
    let radius = (centre.0.min(centre.1) - 1.0).max(0.5);
    let mut pixels = Vec::with_capacity(width * height);
    for (x, y, pixel) in source.enumerate_pixels() {
        let (dx, dy) = (x as f32 + 0.5 - centre.0, y as f32 + 0.5 - centre.1);
        let cover = (radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0);
        let [r, g, b, a] = pixel.0;
        let alpha = f32::from(a) / 255.0 * cover;
        let premultiply = |channel: u8| (f32::from(channel) * alpha).round() as u8;
        pixels.push(Color32::from_rgba_premultiplied(
            premultiply(r),
            premultiply(g),
            premultiply(b),
            (alpha * 255.0).round() as u8,
        ));
    }
    ColorImage {
        size: [width, height],
        source_size: egui::vec2(width as f32, height as f32),
        pixels,
    }
}

fn texture(ctx: &Context, picture: &Picture, px: u32) -> TextureHandle {
    let id = Id::new(("me-face-texture", picture.version, px));
    if let Some(cached) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return cached;
    }
    let fitted = image::imageops::resize(&picture.rgba, px, px, FilterType::Triangle);
    let made =
        ctx.load_texture(format!("me-face-{px}"), clip_to_circle(&fitted), TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, made.clone()));
    made
}

/// Paints the owner's face round in `rect`, or nothing while none is known yet.
pub(crate) fn paint(ctx: &Context, painter: &Painter, rect: Rect) {
    match current(ctx) {
        None => {}
        Some(Me::Letter(letter)) => {
            let face = Face::Letter { letter, tint: badge_colors(TINT_KEY) };
            paint_face(ctx, painter, rect, &face);
        }
        Some(Me::Picture(picture)) => {
            let px = ((rect.width() * ctx.pixels_per_point()).round() as u32).max(1);
            plate(painter, rect, brand::FACE_PLATE);
            let art = texture(ctx, &picture, px);
            painter.image(
                art.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ring(painter, rect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba};

    fn encoded(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let source = RgbaImage::from_pixel(width, height, Rgba([200, 40, 40, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source).to_rgb8().write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn jpeg_and_png_decode_and_anything_else_is_none() {
        let cases = [
            (encoded(ImageFormat::Jpeg, 8, 8), true),
            (encoded(ImageFormat::Png, 8, 8), true),
            (b"<html>not found</html>".to_vec(), false),
            (Vec::new(), false),
        ];
        for (bytes, want) in cases {
            assert_eq!(Me::decode(&bytes).is_some(), want);
        }
    }

    #[test]
    fn each_decoded_picture_has_its_own_version() {
        let bytes = encoded(ImageFormat::Png, 4, 4);
        let (Some(Me::Picture(first)), Some(Me::Picture(second))) =
            (Me::decode(&bytes), Me::decode(&bytes))
        else {
            panic!("a png decodes");
        };
        assert_ne!(first.version, second.version);
    }

    #[test]
    fn the_picture_is_cut_to_a_circle_with_clear_corners_and_a_solid_middle() {
        let source = RgbaImage::from_pixel(64, 64, Rgba([200, 40, 40, 255]));
        let cut = clip_to_circle(&source);
        let at = |x: usize, y: usize| cut.pixels[y * 64 + x];
        for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63), (2, 2)] {
            assert_eq!(at(x, y).a(), 0, "corner pixel {x},{y}");
        }
        assert_eq!(at(32, 32), Color32::from_rgba_premultiplied(200, 40, 40, 255));
        assert!(at(32, 1).a() >= 250, "the top of the circle is inside");
        assert_eq!(at(0, 32).a(), 0, "the edge is inset like the agent faces");
    }

    #[test]
    fn nothing_is_painted_before_a_face_is_set_and_a_set_face_is_read_back() {
        let ctx = Context::default();
        assert!(current(&ctx).is_none());
        set(&ctx, Me::Letter("V".to_owned()));
        assert!(matches!(current(&ctx), Some(Me::Letter(letter)) if letter == "V"));
    }
}
