//! The window's side of a picture on the clipboard: a Cmd+V the key monitor took from egui becomes
//! a job, and what it resolves to goes to the tab that was in front.

use std::path::PathBuf;

use tidewatch_core::image_paste::{Target, ctrl_v};
use tidewatch_core::paths::TidewatchPaths;

use super::frame::FrameIntent;
use super::{Tidewatch, wall_ms};
use crate::image_paste::{Delivery, Done, dropped};
use crate::new_tab::{Kind, NewTab};
use crate::pane::{Body, Pane};
use crate::reader::Shown;
use crate::reader_glue;
use stdusk_core::paste::prepare;
use stdusk_core::terminal::PtyTerm;
use stdusk_egui::terminal_view::paste_into;

use super::terminal::paste_rules;

fn composer_focused(ctx: &egui::Context) -> bool {
    crate::reader_view::reply_has_focus(ctx)
        || crate::agent_ready::composer_has_focus(ctx)
        || ctx.memory(|memory| memory.has_focus(crate::new_tab_fields::text_id()))
}

/// The draft of a new-tab form that shows a message box, if `pane` is one.
fn draft_with_box(pane: &Pane<PtyTerm>) -> Option<&NewTab> {
    match &pane.body {
        Body::Draft(form) if form.kind() != Kind::Shell => Some(form),
        _ => None,
    }
}

/// Where the files a tab's message box stages go: its state folder, or for a draft the folder of
/// that draft, which closing the draft without Start removes.
fn staging_dir(pane: &Pane<PtyTerm>, paths: Option<&TidewatchPaths>) -> Option<PathBuf> {
    if let (Body::Draft(form), Some(paths)) = (&pane.body, paths) {
        return Some(paths.draft_dir(form.draft_key()));
    }
    pane.agent
        .as_ref()
        .and_then(|agent| agent.run.notes_file.as_deref())
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
}

pub(crate) fn appended(reply: &str, text: &str) -> String {
    if reply.is_empty() || reply.ends_with(char::is_whitespace) {
        format!("{reply}{text}")
    } else {
        format!("{reply} {text}")
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
        let mut paths = crate::file_drop::dropped(ctx);
        let screen_free = !self.overview && self.settings.is_none() && self.welcome.is_none();
        if intent.keys.modal_open || !screen_free {
            return;
        }
        let pane = self.tabs.active_pane_mut();
        let reader =
            matches!(reader_glue::shown(pane), Some(Shown::Reader)) && pane.agent.is_some();
        let form = draft_with_box(pane).is_some();
        if (reader || form) && paths.is_empty() {
            paths = self.image_paste.take_dev_drop();
        }
        if paths.is_empty() {
            return;
        }
        if reader || form {
            self.attach_files(ctx, intent.active_id, paths);
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
        let reader_shown = matches!(reader_glue::shown(pane), Some(Shown::Reader))
            || draft_with_box(pane).is_some();
        let agent_id = pane.agent.as_ref().map(|agent| agent.run.agent_id.clone());
        let tab_dir = staging_dir(pane, self.paths.as_ref());
        let is_shell = matches!(pane.body, Body::Shell(_));
        let target = if typing || reader_shown {
            Target::TextField
        } else if is_shell {
            let wiring = agent_id.and_then(|id| self.catalog.agent(&id)).map(|agent| agent.wiring);
            Target::Terminal(wiring)
        } else {
            return;
        };
        let Some(dir) = self.pastes_dir(tab_dir) else {
            self.show_toast("No place to save the picture", ctx);
            return;
        };
        self.image_paste.start(ctx, (intent.active_id, target), dir, wall_ms());
    }

    fn pastes_dir(&self, tab_dir: Option<PathBuf>) -> Option<PathBuf> {
        let state = self.paths.as_ref().map(tidewatch_core::paths::TidewatchPaths::state_dir);
        tab_dir.or(state).map(|dir| dir.join(crate::pictures::PASTES_DIR))
    }

    fn attach_files(&mut self, ctx: &egui::Context, tab: u64, paths: Vec<PathBuf>) {
        let tab_dir =
            self.tabs.get_mut(tab).and_then(|tab| staging_dir(&tab.pane, self.paths.as_ref()));
        let copies = paths.iter().any(|path| !tidewatch_core::image_paste::is_image_file(path));
        match self.pastes_dir(tab_dir).filter(|_| copies) {
            Some(dir) => self.image_paste.stage(ctx, tab, dir, paths),
            None => self.attach_to_box(ctx, tab, paths),
        }
    }

    fn attach_to_box(&mut self, ctx: &egui::Context, tab: u64, paths: Vec<PathBuf>) {
        let Some(pane) = self.tabs.get_mut(tab).map(|tab| &mut tab.pane) else { return };
        if let Body::Draft(form) = &mut pane.body {
            form.insert_files(ctx, paths);
        } else if let Some(agent) = pane.agent.as_mut() {
            agent.reader.view.insert_files(ctx, paths);
        }
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
            Delivery::Files(staged) => {
                if !staged.notes.is_empty() {
                    self.show_toast(staged.notes.join(". "), ctx);
                }
                self.attach_to_box(ctx, tab, staged.paths);
            }
        }
    }

    fn insert_into_field(&mut self, ctx: &egui::Context, tab: u64, active_id: u64, text: String) {
        if tab == active_id && ctx.text_edit_focused() {
            crate::image_paste::inject_paste(ctx, text);
            return;
        }
        let agent = self.tabs.agent_mut(tab);
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
        self.attach_files(ctx, tab, paths);
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
        focus: bool,
        pastes: tempfile::TempDir,
    }

    struct Seen {
        rings: Vec<f32>,
        chips: Vec<Rect>,
        sent: Option<String>,
    }

    fn key(pressed: egui::Key) -> egui::Event {
        egui::Event::Key {
            key: pressed,
            physical_key: Some(pressed),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    impl Window {
        fn new() -> Self {
            let ctx = Context::default();
            crate::fonts::install(&ctx);
            Self { ctx, reply: String::new(), focus: true, pastes: tempfile::tempdir().unwrap() }
        }

        fn frame(&mut self, hovered: &[&str], drops: &[&str]) -> Vec<f32> {
            self.run(hovered, drops, Vec::new()).rings
        }

        fn run(&mut self, hovered: &[&str], drops: &[&str], events: Vec<egui::Event>) -> Seen {
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
                events,
                focused: true,
                ..RawInput::default()
            };
            let mut sent = None;
            let out = self.ctx.run_ui(raw, |ui| {
                let paths = crate::file_drop::dropped(ui.ctx());
                if !paths.is_empty() {
                    let staged = crate::attachments::stage(&paths, self.pastes.path());
                    let targets: Vec<_> =
                        staged.paths.into_iter().map(crate::tokens::Target::File).collect();
                    composer::insert_targets(
                        ui.ctx(),
                        egui::Id::new("reply-box"),
                        &mut self.reply,
                        &targets,
                    );
                }
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_size(pos2(8.0, 200.0), vec2(580.0, 190.0))),
                );
                let draft = Draft { text: &mut self.reply, focus: &mut self.focus, recall: None };
                let spec = Spec {
                    id: egui::Id::new("reply-box"),
                    hint: "Reply",
                    enabled: true,
                    commands: &[],
                    ghost: None,
                    mode: composer::Mode::Reply,
                    stop: false,
                    held: 0,
                    files: None,
                };
                sent = composer::show(&mut child, draft, &spec).sent;
            });
            let mut rings = Vec::new();
            let mut chips = Vec::new();
            for clipped in &out.shapes {
                collect_rings(&clipped.shape, &mut rings);
                collect_chips(&clipped.shape, &mut chips);
            }
            Seen { rings, chips, sent }
        }
    }

    fn collect_chips(shape: &egui::Shape, out: &mut Vec<Rect>) {
        match shape {
            egui::Shape::Vec(inner) => inner.iter().for_each(|shape| collect_chips(shape, out)),
            egui::Shape::Rect(rect)
                if rect.fill == crate::brand::SUNKEN
                    && rect.rect.height() == crate::screen_kit::Size::Tag.height() =>
            {
                out.push(rect.rect);
            }
            _ => {}
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

    fn made_file(dir: &std::path::Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn dropped_files_all_become_chips_in_the_order_they_came_with_no_word_between() {
        let mut window = Window::new();
        let from = tempfile::tempdir().unwrap();
        let notes = made_file(from.path(), "notes.txt", b"hello");
        let spaced = made_file(from.path(), "a b.md", b"# hi");
        let note = notes.to_string_lossy().into_owned();
        let spaced_name = spaced.to_string_lossy().into_owned();
        window.frame(&[], &[]);
        window.frame(&[], &[&note, &spaced_name, "/work/shot.png"]);
        let pastes = window.pastes.path();
        assert_eq!(crate::tokens::plain(&window.reply).trim(), "");
        let want =
            [pastes.join("notes.txt"), pastes.join("a b.md"), PathBuf::from("/work/shot.png")];
        assert_eq!(crate::tokens::targets(&window.reply), want.map(crate::tokens::Target::File));
    }

    #[test]
    fn a_dropped_csv_is_a_chip_as_tall_as_a_picture_chip_and_goes_to_the_agent_as_a_path_in_place()
    {
        let mut window = Window::new();
        let from = tempfile::tempdir().unwrap();
        let csv = made_file(from.path(), "sales q3.csv", b"a,b\n1,2\n");
        let csv_name = csv.to_string_lossy().into_owned();
        window.reply = "look at  now".into();
        window.frame(&[], &[]);
        window.frame(&[], &[]);
        let mut state =
            egui::TextEdit::load_state(&window.ctx, egui::Id::new("reply-box")).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(8))));
        state.store(&window.ctx, egui::Id::new("reply-box"));
        window.run(&[], &[&csv_name, "/work/shot.png"], Vec::new());
        let seen = window.run(&[], &[], Vec::new());
        let copy = window.pastes.path().join("sales q3.csv");
        assert_eq!(std::fs::read(&copy).unwrap(), b"a,b\n1,2\n");
        assert_eq!(seen.chips.len(), 2);
        let (file_chip, picture_chip) = (seen.chips[0], seen.chips[1]);
        assert_eq!(file_chip.size().y, picture_chip.size().y);
        assert_eq!(file_chip.height(), crate::screen_kit::Size::Tag.height());
        assert_eq!(file_chip.top(), picture_chip.top());
        assert!(file_chip.right() < picture_chip.left());
        let sent = window.run(&[], &[], vec![key(egui::Key::Enter)]).sent.unwrap();
        let quoted = tidewatch_core::image_paste::path_text(std::slice::from_ref(&copy));
        assert_eq!(sent, format!("look at {quoted} /work/shot.png now"));
        assert_eq!(window.reply, "");
        let (said, chips) = (crate::tokens::from_sent(&sent, &|_| true), crate::tokens::targets);
        assert_eq!(
            chips(&said),
            [
                crate::tokens::Target::File(copy),
                crate::tokens::Target::File("/work/shot.png".into())
            ]
        );
    }

    #[test]
    fn nothing_changes_while_files_only_hover_and_the_box_is_lit_meanwhile() {
        let mut window = Window::new();
        window.reply = "look at".into();
        assert_eq!(window.frame(&[], &[]).len(), 0);
        let lit = window.frame(&["/work/shot.png"], &[]);
        assert_eq!(lit.len(), 1, "the drop highlight is drawn");
        assert_eq!(window.reply, "look at");
        assert_eq!(crate::tokens::targets(&window.reply), Vec::<crate::tokens::Target>::new());
        let dropped_on = window.frame(&[], &["/work/shot.png"]);
        assert_eq!(dropped_on.len(), 0, "the highlight ends with the drop");
        assert_eq!(
            crate::tokens::targets(&window.reply),
            [crate::tokens::Target::File("/work/shot.png".into())]
        );
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
