//! What Cmd+V does when the clipboard holds a picture: egui sends nothing for it, so the app hooks
//! the key and asks this table (pure).

use std::path::{Path, PathBuf};

use crate::agents::Wiring;

const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "heic", "tif", "tiff"];
const CTRL_V: u8 = 0x16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    Empty,
    Text,
    Image,
    TextAndImage,
    /// Finder adds the file name as text and the icon as a picture; neither is used.
    ImageFiles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    TextField,
    /// `None` is a plain shell.
    Terminal(Option<Wiring>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Leave,
    CtrlV,
    SaveAndPastePath,
    PasteFilePaths,
}

#[must_use]
pub fn clip_of(has_text: bool, has_image: bool, image_files: usize) -> Clip {
    match (image_files > 0, has_text, has_image) {
        (true, _, _) => Clip::ImageFiles,
        (false, true, true) => Clip::TextAndImage,
        (false, true, false) => Clip::Text,
        (false, false, true) => Clip::Image,
        (false, false, false) => Clip::Empty,
    }
}

#[must_use]
pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(ext)))
}

/// Ctrl+V attaches the clipboard picture in Claude Code, Codex 0.162.1, Gemini CLI 0.46.0 and Copilot CLI 1.0.103.
#[must_use]
pub fn reads_clipboard_on_ctrl_v(wiring: Wiring) -> bool {
    match wiring {
        Wiring::Claude | Wiring::Codex | Wiring::Gemini | Wiring::Copilot => true,
        Wiring::Custom => false,
    }
}

#[must_use]
pub fn decide(clip: Clip, target: Target) -> Action {
    match (clip, target) {
        (Clip::Empty | Clip::Text | Clip::TextAndImage, _) => Action::Leave,
        (Clip::ImageFiles, _) => Action::PasteFilePaths,
        (Clip::Image, Target::Terminal(Some(wiring))) if reads_clipboard_on_ctrl_v(wiring) => {
            Action::CtrlV
        }
        (Clip::Image, _) => Action::SaveAndPastePath,
    }
}

#[must_use]
pub fn takes_the_key(clip: Clip, target: Target) -> bool {
    decide(clip, target) != Action::Leave
}

#[must_use]
pub fn ctrl_v() -> &'static [u8] {
    &[CTRL_V]
}

#[must_use]
pub fn shot_name(now_ms: u128, taken: impl Fn(&str) -> bool) -> String {
    let plain = format!("shot-{now_ms}.png");
    if !taken(&plain) {
        return plain;
    }
    (2..10_000).map(|n| format!("shot-{now_ms}-{n}.png")).find(|name| !taken(name)).unwrap_or(plain)
}

#[must_use]
pub fn path_text(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| shell_words::quote(&path.to_string_lossy()).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_WIRINGS: [Wiring; 5] =
        [Wiring::Claude, Wiring::Codex, Wiring::Gemini, Wiring::Copilot, Wiring::Custom];

    #[test]
    fn clip_kind_follows_what_the_clipboard_holds() {
        let cases = [
            ((false, false, 0), Clip::Empty),
            ((true, false, 0), Clip::Text),
            ((false, true, 0), Clip::Image),
            ((true, true, 0), Clip::TextAndImage),
            ((false, false, 1), Clip::ImageFiles),
            ((true, true, 2), Clip::ImageFiles),
            ((true, false, 1), Clip::ImageFiles),
        ];
        for ((text, image, files), want) in cases {
            assert_eq!(clip_of(text, image, files), want, "{text} {image} {files}");
        }
    }

    #[test]
    fn the_action_follows_clipboard_target_and_cli() {
        let terminal = |w| Target::Terminal(w);
        let cases = [
            (Clip::Image, terminal(Some(Wiring::Claude)), Action::CtrlV),
            (Clip::Image, terminal(Some(Wiring::Codex)), Action::CtrlV),
            (Clip::Image, terminal(Some(Wiring::Gemini)), Action::CtrlV),
            (Clip::Image, terminal(Some(Wiring::Copilot)), Action::CtrlV),
            (Clip::Image, terminal(Some(Wiring::Custom)), Action::SaveAndPastePath),
            (Clip::Image, terminal(None), Action::SaveAndPastePath),
            (Clip::Image, Target::TextField, Action::SaveAndPastePath),
            (Clip::ImageFiles, terminal(Some(Wiring::Claude)), Action::PasteFilePaths),
            (Clip::ImageFiles, terminal(None), Action::PasteFilePaths),
            (Clip::ImageFiles, Target::TextField, Action::PasteFilePaths),
            (Clip::TextAndImage, terminal(Some(Wiring::Claude)), Action::Leave),
            (Clip::TextAndImage, terminal(None), Action::Leave),
            (Clip::TextAndImage, Target::TextField, Action::Leave),
            (Clip::Text, terminal(Some(Wiring::Claude)), Action::Leave),
            (Clip::Text, Target::TextField, Action::Leave),
            (Clip::Empty, terminal(None), Action::Leave),
            (Clip::Empty, Target::TextField, Action::Leave),
        ];
        for (clip, target, want) in cases {
            assert_eq!(decide(clip, target), want, "{clip:?} into {target:?}");
        }
    }

    #[test]
    fn every_built_in_cli_reads_the_picture_on_ctrl_v_and_only_custom_does_not() {
        let reading: Vec<_> =
            ALL_WIRINGS.iter().filter(|w| reads_clipboard_on_ctrl_v(**w)).collect();
        assert_eq!(reading.len(), 4);
        assert!(!reads_clipboard_on_ctrl_v(Wiring::Custom));
    }

    #[test]
    fn the_key_is_taken_from_egui_only_when_a_picture_is_to_be_handled() {
        assert!(takes_the_key(Clip::Image, Target::TextField));
        assert!(takes_the_key(Clip::ImageFiles, Target::Terminal(None)));
        assert!(!takes_the_key(Clip::TextAndImage, Target::TextField));
        assert!(!takes_the_key(Clip::Text, Target::Terminal(Some(Wiring::Claude))));
    }

    #[test]
    fn image_files_are_told_by_extension_ignoring_case() {
        let cases = [
            ("/a/Screenshot 2026-10-24.PNG", true),
            ("/a/photo.jpeg", true),
            ("/a/anim.gif", true),
            ("/a/notes.txt", false),
            ("/a/png", false),
            ("/a/dir.png/readme", false),
        ];
        for (path, want) in cases {
            assert_eq!(is_image_file(Path::new(path)), want, "{path}");
        }
    }

    #[test]
    fn shot_names_are_timed_and_never_reuse_a_taken_one() {
        assert_eq!(shot_name(1700, |_| false), "shot-1700.png");
        assert_eq!(shot_name(1700, |n| n == "shot-1700.png"), "shot-1700-2.png");
        assert_eq!(
            shot_name(1700, |n| n == "shot-1700.png" || n == "shot-1700-2.png"),
            "shot-1700-3.png"
        );
    }

    #[test]
    fn paths_are_quoted_only_when_a_name_needs_it() {
        let paths = [PathBuf::from("/s/shot-1.png"), PathBuf::from("/My Pics/a b.png")];
        assert_eq!(path_text(&paths), "/s/shot-1.png '/My Pics/a b.png'");
        assert_eq!(path_text(&[]), "");
    }

    #[test]
    fn ctrl_v_is_the_single_byte_claude_code_reads_as_its_image_paste() {
        assert_eq!(ctrl_v(), [0x16]);
    }
}
