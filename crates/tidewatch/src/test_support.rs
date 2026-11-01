//! Test only: the walkers the view tests share to read back what a frame painted, and a git runner.

use std::path::Path;
use std::process::Command;

use egui::epaint::TextShape;
use egui::{Rect, Shape};

fn for_each_text(shape: &Shape, visit: &mut impl FnMut(&TextShape)) {
    match shape {
        Shape::Text(text) => visit(text),
        Shape::Vec(shapes) => shapes.iter().for_each(|shape| for_each_text(shape, visit)),
        _ => {}
    }
}

pub(crate) fn painted_strings(shape: &Shape, out: &mut Vec<String>) {
    for_each_text(shape, &mut |text| out.push(text.galley.text().to_owned()));
}

pub(crate) fn painted_text_rects(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    for_each_text(shape, &mut |text| {
        out.push((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2())));
    });
}

pub(crate) fn painted_text_rects_nonblank(shape: &Shape, out: &mut Vec<(String, Rect)>) {
    for_each_text(shape, &mut |text| {
        if !text.galley.text().trim().is_empty() {
            out.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            ));
        }
    });
}

pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[cfg(test)]
mod tests {
    use egui::{Color32, FontId, RawInput, pos2, vec2};

    use super::*;

    fn frame_with_nested_text() -> Vec<egui::epaint::ClippedShape> {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0))),
            ..RawInput::default()
        };
        ctx.run_ui(raw, |ui| {
            let painter = ui.painter();
            let galley = |text: &str| {
                painter.layout_no_wrap(text.to_owned(), FontId::proportional(14.0), Color32::WHITE)
            };
            let inner = Shape::Vec(vec![
                Shape::galley(pos2(10.0, 20.0), galley("inner"), Color32::WHITE),
                Shape::galley(pos2(10.0, 40.0), galley("  "), Color32::WHITE),
            ]);
            painter.add(Shape::Vec(vec![
                inner,
                Shape::galley(pos2(0.0, 60.0), galley("outer"), Color32::WHITE),
            ]));
        })
        .shapes
    }

    #[test]
    fn git_commits_with_its_own_identity_and_returns_stdout() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "first"]);
        assert_eq!(git(dir.path(), &["log", "-1", "--format=%an <%ae>"]), "T <t@example.com>\n");
    }

    #[test]
    #[should_panic(expected = "git [\"no-such-subcommand\"]")]
    fn git_panics_with_the_arguments_when_it_fails() {
        git(Path::new("."), &["no-such-subcommand"]);
    }

    #[test]
    fn nested_shape_lists_are_walked_in_paint_order() {
        let shapes = frame_with_nested_text();
        let mut all = Vec::new();
        for clipped in &shapes {
            painted_strings(&clipped.shape, &mut all);
        }
        assert_eq!(all, ["inner", "  ", "outer"]);
    }

    #[test]
    fn rects_sit_where_the_text_was_painted_and_nonblank_drops_whitespace() {
        let shapes = frame_with_nested_text();
        let (mut every, mut visible) = (Vec::new(), Vec::new());
        for clipped in &shapes {
            painted_text_rects(&clipped.shape, &mut every);
            painted_text_rects_nonblank(&clipped.shape, &mut visible);
        }
        let names = |found: &[(String, Rect)]| {
            found.iter().map(|(text, _)| text.clone()).collect::<Vec<_>>()
        };
        assert_eq!(names(&every), ["inner", "  ", "outer"]);
        assert_eq!(names(&visible), ["inner", "outer"]);
        assert_eq!(every[0].1.min, pos2(10.0, 20.0));
        assert_eq!(every[2].1.min, pos2(0.0, 60.0));
    }
}
