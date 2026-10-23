//! Worktree setup page: renders, offers the ignored env files, and a click returns the recipe.

use std::path::Path;
use std::process::Command;

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2, pos2};
use tidewatch_core::paths::TidewatchPaths;

use super::*;
use crate::setup_editor::RepoEntry;

fn input(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1000.0, 1100.0))),
        focused: true,
        events,
        ..RawInput::default()
    }
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

struct Harness {
    _home: tempfile::TempDir,
    ctx: egui::Context,
    editor: Editor,
}

impl Harness {
    fn new(with_repo: bool) -> Self {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().canonicalize().unwrap();
        let web = root.join("acme-web");
        std::fs::create_dir_all(&web).unwrap();
        git(&web, &["init", "-q", "-b", "main"]);
        std::fs::write(web.join(".gitignore"), ".env*\n").unwrap();
        std::fs::write(web.join(".env.local"), "A=1\n").unwrap();
        let repos = if with_repo {
            vec![RepoEntry { name: "acme-web".into(), dir: web.clone() }]
        } else {
            Vec::new()
        };
        let editor = Editor::new(repos, Some(TidewatchPaths::new(&root)), &web);
        Self { _home: home, ctx: crate::fonts::test_context(), editor }
    }

    fn frame(&mut self, events: Vec<Event>) -> Option<Action> {
        let mut answer = None;
        let editor = &mut self.editor;
        let _ =
            self.ctx.run_ui(input(events), |ui| answer = page(ui, editor, "worktree-setup.toml"));
        answer
    }

    fn click(&mut self, at: Pos2) -> Option<Action> {
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![press(at, true)]);
        self.frame(vec![press(at, false)])
    }

    fn click_scan(&mut self, ys: std::ops::Range<i32>) -> Option<Action> {
        for y in ys.step_by(4) {
            for x in (800..990).step_by(6) {
                if let Some(action) = self.click(pos2(x as f32, y as f32)) {
                    return Some(action);
                }
            }
        }
        None
    }

    fn until_offered(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.editor.offered().is_empty() {
            self.frame(vec![]);
            assert!(std::time::Instant::now() < deadline, "no suggestion arrived");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[test]
fn the_page_renders_with_and_without_a_repo_and_asks_for_nothing_by_itself() {
    for with_repo in [false, true] {
        let mut harness = Harness::new(with_repo);
        for _ in 0..3 {
            assert!(harness.frame(vec![]).is_none());
        }
    }
}

#[test]
fn a_click_on_a_suggestion_returns_the_repo_and_the_whole_recipe() {
    let mut harness = Harness::new(true);
    harness.until_offered();
    assert_eq!(harness.editor.offered(), [".env.local"]);
    let Some(Action::WorktreeSetup { repo, recipe }) = harness.click_scan(130..400) else {
        panic!("no Add button answered")
    };
    assert!(repo.ends_with("acme-web"));
    assert_eq!(recipe.copy, [".env.local"]);
    assert_eq!(harness.editor.offered().len(), 0);
}

#[test]
fn a_remove_button_drops_that_entry_only() {
    let mut harness = Harness::new(true);
    harness.until_offered();
    harness.editor.add_copy(".env.local");
    harness.editor.add_copy(".env");
    harness.frame(vec![]);
    let Some(Action::WorktreeSetup { recipe, .. }) = harness.click_scan(130..400) else {
        panic!("no Remove button answered")
    };
    assert_eq!(recipe.copy, [".env"], "the first row went");
}
