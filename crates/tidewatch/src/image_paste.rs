//! The clipboard picture of a Cmd+V: read it, decide with `tidewatch_core::image_paste`, and turn it
//! into what the target gets (Ctrl+V, or the path of a saved PNG or a copied file). The system
//! clipboard sits behind a seam so the headless tests need no window.

use std::path::{Path, PathBuf};
use stdusk_egui::macos_paste::{ImagePasteHook, post_cmd_v};

use tidewatch_core::image_paste::{Action, Clip, Target, decide, path_text, shot_name};

use crate::jobs::Jobs;

pub(crate) struct Picture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

pub(crate) trait Clipboard {
    fn clip(&mut self) -> Clip;
    fn picture(&mut self) -> Option<Picture>;
    fn image_files(&mut self) -> Vec<PathBuf>;
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    Nothing,
    CtrlV,
    Text(String),
    Failed(String),
}

pub(crate) fn resolve(
    clipboard: &mut dyn Clipboard,
    target: Target,
    dir: &Path,
    now_ms: u128,
) -> Delivery {
    match decide(clipboard.clip(), target) {
        Action::Leave => Delivery::Nothing,
        Action::CtrlV => Delivery::CtrlV,
        Action::PasteFilePaths => Delivery::Text(path_text(&clipboard.image_files())),
        Action::SaveAndPastePath => match save(clipboard, dir, now_ms) {
            Ok(path) => Delivery::Text(path_text(&[path])),
            Err(message) => Delivery::Failed(message),
        },
    }
}

fn save(clipboard: &mut dyn Clipboard, dir: &Path, now_ms: u128) -> Result<PathBuf, String> {
    let picture = clipboard.picture().ok_or("The clipboard picture could not be read")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not save the picture: {e}"))?;
    let path = dir.join(shot_name(now_ms, |name| dir.join(name).exists()));
    image::save_buffer_with_format(
        &path,
        &picture.rgba,
        picture.width,
        picture.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|e| format!("Could not save the picture: {e}"))?;
    Ok(path)
}

pub(crate) fn inject_paste(ctx: &egui::Context, text: String) {
    ctx.input_mut(|input| input.events.push(egui::Event::Paste(text)));
    ctx.request_repaint();
}

pub(crate) struct Done {
    pub(crate) tab: u64,
    pub(crate) target: Target,
    pub(crate) delivery: Delivery,
}

pub(crate) struct Host {
    hook: ImagePasteHook,
    jobs: Jobs<Done>,
    dev_at: Option<f64>,
}

fn dev_seconds(value: Option<&str>) -> Option<f64> {
    value.and_then(|text| text.parse().ok()).filter(|secs: &f64| secs.is_finite() && *secs >= 0.0)
}

impl Host {
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        let hook = ImagePasteHook::default();
        hook.install_monitor(ctx, || {
            crate::macos::clip_has_picture(&mut crate::macos::SystemClipboard)
        });
        let dev_at = dev_seconds(std::env::var("TIDEWATCH_DEV_IMAGE_PASTE").ok().as_deref());
        Self { hook, jobs: Jobs::new(), dev_at }
    }

    pub(crate) fn tick_dev(&mut self, now: f64, ctx: &egui::Context) {
        let Some(at) = self.dev_at else { return };
        if now < at {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(at - now));
            return;
        }
        self.dev_at = None;
        if !post_cmd_v() {
            self.request();
        }
        ctx.request_repaint();
    }

    pub(crate) fn request(&self) {
        self.hook.request();
    }

    pub(crate) fn take_requests(&self) -> usize {
        self.hook.take()
    }

    pub(crate) fn start(
        &self,
        ctx: &egui::Context,
        (tab, target): (u64, Target),
        dir: PathBuf,
        now_ms: u128,
    ) {
        self.jobs.spawn(ctx, move || {
            let delivery = resolve(&mut crate::macos::SystemClipboard, target, &dir, now_ms);
            Done { tab, target, delivery }
        });
    }

    pub(crate) fn drain(&self) -> Vec<Done> {
        self.jobs.drain()
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub(crate) struct Fake {
        pub(crate) clip: Clip,
        pub(crate) picture: Option<Picture>,
        pub(crate) files: Vec<PathBuf>,
    }

    impl Clipboard for Fake {
        fn clip(&mut self) -> Clip {
            self.clip
        }
        fn picture(&mut self) -> Option<Picture> {
            self.picture.take()
        }
        fn image_files(&mut self) -> Vec<PathBuf> {
            self.files.clone()
        }
    }

    pub(crate) fn screenshot() -> Fake {
        let rgba = [10, 20, 30, 255].repeat(4);
        Fake {
            clip: Clip::Image,
            picture: Some(Picture { width: 2, height: 2, rgba }),
            files: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use tidewatch_core::agents::Wiring;

    use super::testing::{Fake, screenshot};
    use super::*;

    #[test]
    fn a_screenshot_for_claude_is_ctrl_v_and_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let target = Target::Terminal(Some(Wiring::Claude));
        let got = resolve(&mut screenshot(), target, &dir.path().join("pastes"), 5);
        assert_eq!(got, Delivery::CtrlV);
        assert!(!dir.path().join("pastes").exists());
    }

    #[test]
    fn a_screenshot_for_a_shell_or_a_text_field_is_saved_as_a_png_and_its_path_returned() {
        for target in [Target::Terminal(None), Target::TextField] {
            let dir = tempfile::tempdir().unwrap();
            let pastes = dir.path().join("pastes");
            let got = resolve(&mut screenshot(), target, &pastes, 7);
            let path = pastes.join("shot-7.png");
            assert_eq!(got, Delivery::Text(path.to_string_lossy().into_owned()), "{target:?}");
            let decoded = image::open(&path).unwrap().to_rgba8();
            assert_eq!(decoded.dimensions(), (2, 2));
            assert_eq!(decoded.get_pixel(1, 1).0, [10, 20, 30, 255]);
        }
    }

    #[test]
    fn two_pastes_in_one_millisecond_keep_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let pastes = dir.path().to_path_buf();
        resolve(&mut screenshot(), Target::TextField, &pastes, 9);
        let second = resolve(&mut screenshot(), Target::TextField, &pastes, 9);
        assert_eq!(
            second,
            Delivery::Text(pastes.join("shot-9-2.png").to_string_lossy().into_owned())
        );
    }

    #[test]
    fn copied_files_are_pasted_by_their_own_paths_whatever_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let files =
            vec![PathBuf::from("/Users/x/Desktop/a.png"), PathBuf::from("/Users/x/My Pics/b.png")];
        for target in [Target::TextField, Target::Terminal(Some(Wiring::Claude))] {
            let mut finder = Fake { clip: Clip::ImageFiles, picture: None, files: files.clone() };
            let got = resolve(&mut finder, target, dir.path(), 1);
            assert_eq!(
                got,
                Delivery::Text("/Users/x/Desktop/a.png '/Users/x/My Pics/b.png'".into())
            );
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn text_with_a_picture_and_a_plain_clipboard_are_left_to_egui() {
        let dir = tempfile::tempdir().unwrap();
        for clip in [Clip::TextAndImage, Clip::Text, Clip::Empty] {
            let mut mixed = Fake { clip, ..screenshot() };
            assert_eq!(resolve(&mut mixed, Target::TextField, dir.path(), 1), Delivery::Nothing);
        }
    }

    #[test]
    fn the_dev_switch_reads_seconds_and_ignores_the_rest() {
        let cases = [
            (Some("3"), Some(3.0)),
            (Some("0.5"), Some(0.5)),
            (Some("x"), None),
            (Some("-1"), None),
            (None, None),
        ];
        for (value, want) in cases {
            assert_eq!(dev_seconds(value), want, "{value:?}");
        }
    }

    #[test]
    fn a_picture_that_cannot_be_read_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut gone = Fake { picture: None, ..screenshot() };
        let got = resolve(&mut gone, Target::TextField, dir.path(), 1);
        assert!(matches!(got, Delivery::Failed(_)));
    }
}
