//! The window's side of a picture on the clipboard: a Cmd+V the key monitor took from egui becomes
//! a job, and what it resolves to goes to the tab that was in front.

use std::path::PathBuf;

use tidewatch_core::image_paste::{Target, ctrl_v};

use super::frame::FrameIntent;
use super::{Tidewatch, wall_ms};
use crate::image_paste::{Delivery, Done, dropped};
use crate::pane::Body;
use crate::reader::Shown;
use crate::reader_glue;
use stdusk_core::paste::prepare;
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::terminal_view::paste_into;

use super::terminal::paste_rules;

const PASTES_DIR: &str = "pastes";

fn composer_focused(ctx: &egui::Context) -> bool {
    crate::reader_view::reply_has_focus(ctx) || crate::agent_ready::composer_has_focus(ctx)
}

pub(crate) fn appended(reply: &str, text: &str) -> String {
    if reply.is_empty() || reply.ends_with(char::is_whitespace) {
        format!("{reply}{text}")
    } else {
        format!("{reply} {text}")
    }
}

pub(crate) fn fill_reply(reply: &mut String, pictures: &mut Vec<PathBuf>, delivery: Delivery) {
    match delivery {
        Delivery::Text(text) => *reply = appended(reply, &text),
        Delivery::Pictures(paths) => crate::pictures::attach(pictures, paths),
        Delivery::Nothing | Delivery::CtrlV | Delivery::Failed(_) => {}
    }
}

fn type_dropped(term: &mut PtyTerm, paths: &[PathBuf]) {
    for delivery in dropped(paths, Target::Terminal(None)) {
        if let Delivery::Text(text) = delivery {
            term.paste(&text);
        }
    }
}

impl Tidewatch {
    pub(super) fn drive_file_drop(&mut self, ctx: &egui::Context, intent: &FrameIntent) {
        let paths = crate::file_drop::dropped(ctx);
        let screen_free = !self.overview && self.settings.is_none() && self.welcome.is_none();
        if paths.is_empty() || intent.keys.modal_open || !screen_free {
            return;
        }
        let pane = self.tabs.active_pane_mut();
        if matches!(reader_glue::shown(pane), Some(Shown::Reader)) {
            if let Some(agent) = pane.agent.as_mut() {
                let view = &mut agent.reader.view;
                for delivery in dropped(&paths, Target::TextField) {
                    fill_reply(&mut view.reply, &mut view.pictures, delivery);
                }
                view.request_reply_focus();
                ctx.request_repaint();
            }
        } else if let Body::Shell(term) = &mut pane.body {
            type_dropped(term, &paths);
        }
    }

    pub(super) fn drive_image_paste(
        &mut self,
        ctx: &egui::Context,
        now: f64,
        intent: &FrameIntent,
    ) {
        self.image_paste.tick_dev(now, ctx);
        for done in self.image_paste.drain() {
            self.deliver_picture(done, ctx, intent.active_id);
        }
        if self.image_paste.take_requests() > 0 {
            self.start_picture_paste(ctx, intent);
        }
    }

    fn start_picture_paste(&mut self, ctx: &egui::Context, intent: &FrameIntent) {
        let screen_free = !self.overview && self.settings.is_none() && self.welcome.is_none();
        let typing = intent.keys.typing_in_a_panel;
        if !typing && (intent.keys.modal_open || !screen_free) {
            return;
        }
        let pane = self.tabs.active_pane_mut();
        let reader_shown = matches!(reader_glue::shown(pane), Some(Shown::Reader));
        let agent_id = pane.agent.as_ref().map(|agent| agent.run.agent_id.clone());
        let tab_dir = pane
            .agent
            .as_ref()
            .and_then(|agent| agent.run.notes_file.as_deref())
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf);
        let is_shell = matches!(pane.body, Body::Shell(_));
        let target = if typing || reader_shown {
            Target::TextField
        } else if is_shell {
            let wiring = agent_id.and_then(|id| self.catalog.agent(&id)).map(|agent| agent.wiring);
            Target::Terminal(wiring)
        } else {
            return;
        };
        let state = self.paths.as_ref().map(tidewatch_core::paths::TidewatchPaths::state_dir);
        let Some(dir) = tab_dir.or(state).map(|dir| dir.join(PASTES_DIR)) else {
            self.show_toast("No place to save the picture", ctx);
            return;
        };
        self.image_paste.start(ctx, (intent.active_id, target), dir, wall_ms());
    }

    fn deliver_picture(&mut self, done: Done, ctx: &egui::Context, active_id: u64) {
        let Done { tab, target, delivery } = done;
        match delivery {
            Delivery::Nothing => {}
            Delivery::Failed(message) => self.show_toast(message, ctx),
            Delivery::CtrlV => {
                if let Some(Body::Shell(term)) = self.tab_body_mut(tab) {
                    term.send(ctrl_v());
                }
            }
            Delivery::Text(text) => match target {
                Target::Terminal(_) => {
                    let rules = paste_rules(&self.terminal_config);
                    if let (Some(Body::Shell(term)), Some(text)) =
                        (self.tab_body_mut(tab), prepare(&text, rules))
                    {
                        paste_into(term, &text);
                    }
                }
                Target::TextField => self.insert_into_field(ctx, tab, active_id, text),
            },
            Delivery::Pictures(paths) => self.attach_pictures(ctx, tab, active_id, paths),
        }
    }

    fn insert_into_field(&mut self, ctx: &egui::Context, tab: u64, active_id: u64, text: String) {
        if tab == active_id && ctx.text_edit_focused() {
            crate::image_paste::inject_paste(ctx, text);
            return;
        }
        let agent = self.tabs.get_mut(tab).and_then(|tab| tab.pane.agent.as_mut());
        if let Some(agent) = agent {
            agent.reader.view.reply = appended(&agent.reader.view.reply, &text);
            agent.reader.view.request_reply_focus();
            ctx.request_repaint();
        }
    }

    fn attach_pictures(
        &mut self,
        ctx: &egui::Context,
        tab: u64,
        active_id: u64,
        paths: Vec<std::path::PathBuf>,
    ) {
        let other_field_focused =
            tab == active_id && ctx.text_edit_focused() && !composer_focused(ctx);
        if other_field_focused {
            crate::image_paste::inject_paste(ctx, tidewatch_core::image_paste::path_text(&paths));
            return;
        }
        let agent = self.tabs.get_mut(tab).and_then(|tab| tab.pane.agent.as_mut());
        if let Some(agent) = agent {
            crate::pictures::attach(&mut agent.reader.view.pictures, paths);
            agent.reader.view.request_reply_focus();
            ctx.request_repaint();
        }
    }

    fn tab_body_mut(&mut self, tab: u64) -> Option<&mut Body<PtyTerm>> {
        self.tabs.get_mut(tab).map(|tab| &mut tab.pane.body)
    }
}

#[cfg(test)]
mod tests {
    use egui::{Context, DroppedFile, HoveredFile, RawInput, Rect, UiBuilder, pos2, vec2};

    use super::*;
    use crate::composer::{self, Draft, Spec};

    #[test]
    fn a_path_is_added_after_the_text_already_typed() {
        let cases = [
            ("", "/s/shot-1.png", "/s/shot-1.png"),
            ("look at ", "/s/shot-1.png", "look at /s/shot-1.png"),
            ("look at", "/s/shot-1.png", "look at /s/shot-1.png"),
            ("a\n", "/s/shot-1.png", "a\n/s/shot-1.png"),
        ];
        for (reply, text, want) in cases {
            assert_eq!(appended(reply, text), want, "{reply:?}");
        }
    }

    struct Window {
        ctx: Context,
        reply: String,
        pictures: Vec<PathBuf>,
        focus: bool,
    }

    impl Window {
        fn new() -> Self {
            let ctx = Context::default();
            crate::fonts::install(&ctx);
            Self { ctx, reply: String::new(), pictures: Vec::new(), focus: false }
        }

        fn frame(&mut self, hovered: &[&str], drops: &[&str]) -> Vec<f32> {
            let files =
                |names: &[&str]| -> Vec<PathBuf> { names.iter().map(PathBuf::from).collect() };
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0))),
                hovered_files: files(hovered)
                    .into_iter()
                    .map(|path| HoveredFile { path: Some(path), ..HoveredFile::default() })
                    .collect(),
                dropped_files: files(drops)
                    .into_iter()
                    .map(|path| DroppedFile { path: Some(path), ..DroppedFile::default() })
                    .collect(),
                focused: true,
                ..RawInput::default()
            };
            let mut rings = Vec::new();
            let out = self.ctx.run_ui(raw, |ui| {
                let paths = crate::file_drop::dropped(ui.ctx());
                for delivery in dropped(&paths, Target::TextField) {
                    fill_reply(&mut self.reply, &mut self.pictures, delivery);
                }
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_size(pos2(8.0, 300.0), vec2(580.0, 90.0))),
                );
                let draft = Draft {
                    text: &mut self.reply,
                    pictures: &mut self.pictures,
                    focus: &mut self.focus,
                };
                let spec = Spec {
                    id: egui::Id::new("reply-box"),
                    hint: "Reply",
                    enabled: true,
                    commands: &[],
                    ghost: None,
                };
                composer::show(&mut child, draft, &spec);
            });
            for clipped in &out.shapes {
                collect_rings(&clipped.shape, &mut rings);
            }
            rings
        }
    }

    fn collect_rings(shape: &egui::Shape, out: &mut Vec<f32>) {
        match shape {
            egui::Shape::Vec(inner) => inner.iter().for_each(|shape| collect_rings(shape, out)),
            egui::Shape::Rect(rect)
                if rect.fill == egui::Color32::TRANSPARENT && rect.stroke.width > 4.0 =>
            {
                out.push(rect.stroke.width);
            }
            _ => {}
        }
    }

    #[test]
    fn dropped_files_fill_the_reply_box_paths_and_picture_chips() {
        let mut window = Window::new();
        window.frame(&[], &[]);
        window.frame(&[], &["/work/notes.txt", "/work/My Docs/a b.md", "/work/shot.png"]);
        assert_eq!(window.reply, "/work/notes.txt '/work/My Docs/a b.md'");
        assert_eq!(window.pictures, [PathBuf::from("/work/shot.png")]);
        window.frame(&[], &["/work/more.rs", "/work/shot.png"]);
        assert_eq!(window.reply, "/work/notes.txt '/work/My Docs/a b.md' /work/more.rs");
        assert_eq!(window.pictures, [PathBuf::from("/work/shot.png")], "one chip per picture");
    }

    #[test]
    fn nothing_changes_while_files_only_hover_and_the_box_is_lit_meanwhile() {
        let mut window = Window::new();
        window.reply = "look at".into();
        assert_eq!(window.frame(&[], &[]).len(), 0);
        let lit = window.frame(&["/work/shot.png"], &[]);
        assert_eq!(lit.len(), 1, "the drop highlight is drawn");
        assert_eq!(window.reply, "look at");
        assert_eq!(window.pictures.len(), 0);
        let dropped_on = window.frame(&[], &["/work/shot.png"]);
        assert_eq!(dropped_on.len(), 0, "the highlight ends with the drop");
        assert_eq!(window.pictures, [PathBuf::from("/work/shot.png")]);
    }

    #[test]
    fn files_dropped_into_a_terminal_are_typed_as_one_line_of_paths() {
        let paths = [PathBuf::from("/work/a.png"), PathBuf::from("/work/My Docs/b.txt")];
        let got = crate::image_paste::dropped(&paths, Target::Terminal(None));
        assert_eq!(got, [Delivery::Text("/work/a.png '/work/My Docs/b.txt' ".into())]);
        assert_eq!(crate::image_paste::dropped(&[], Target::Terminal(None)), []);
        assert_eq!(crate::image_paste::dropped(&[], Target::TextField), []);
    }

    fn echoing_terminal() -> PtyTerm {
        use std::collections::BTreeMap;
        use stdusk_core::repaint::Repaint;
        use stdusk_core::shell::ShellProfile;
        use stdusk_core::terminal::SpawnOpts;
        PtyTerm::spawn(
            80,
            24,
            Repaint::none(),
            &SpawnOpts {
                detect_progress: false,
                shell_integration: false,
                autosuggestions: false,
                scrollback_lines: 200,
                word_separators: " ".into(),
                bold_bright: false,
                agent_tracking: None,
                cwd: None,
                profile: Some(ShellProfile {
                    shell: Some("/bin/sh".into()),
                    args: vec!["-c".into(), "cat".into()],
                    cwd: None,
                    env: BTreeMap::new(),
                }),
            },
        )
    }

    fn screen_text(term: &PtyTerm) -> String {
        crate::job_glue::screen_rows(&term.grid_snapshot()).join("\n")
    }

    #[test]
    fn files_dropped_on_a_terminal_reach_its_pty_as_typed_paths() {
        let mut term = echoing_terminal();
        let paths = [PathBuf::from("/work/a.png"), PathBuf::from("/work/My Docs/b.txt")];
        type_dropped(&mut term, &paths);
        let want = "/work/a.png '/work/My Docs/b.txt'";
        let started = std::time::Instant::now();
        while !screen_text(&term).contains(want) {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(10),
                "{}",
                screen_text(&term)
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
