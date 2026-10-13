//! The window shell: what other launches, the hotkey, the menu-bar item and the settings ask of
//! the window, and the quake drop-down they drive. One `tick` per frame turns those requests into
//! viewport commands and OS calls, all through the `Plan`, so a background run does none of it.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use egui::{Context, ViewportCommand};
use tidewatch_core::instance::{self, Command, Guard, Primary};
use tidewatch_core::prefs::WindowPrefs;

use crate::hotkey::Chord;
use crate::os::{Os, RealOs};
use crate::quake::{self, Frame, Op, Quake};
use crate::shell_plan::{Env, Hotkey, Plan};
use crate::tray::Click;

pub(crate) const WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];

/// What the app should do about a request the shell cannot answer by itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Opened(PathBuf),
    Toggled,
    Overview,
    Settings,
    Quit,
}

/// How the global hotkey stands, for the settings page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HotkeyStatus {
    Off,
    Registered,
    NoGlobalHotkeys,
    Problem(String),
}

pub(crate) struct Shell<O: Os> {
    pub(crate) os: O,
    env: Env,
    plan: Plan,
    prefs: WindowPrefs,
    label: String,
    quake: Quake,
    inbox: Receiver<Command>,
    _guard: Option<Guard>,
    registered: Option<Chord>,
    attempted: Option<Chord>,
    problem: Option<String>,
    tray_built: bool,
    dock: Option<bool>,
    spaces: Option<bool>,
}

impl<O: Os> Shell<O> {
    pub(crate) fn new(
        os: O,
        env: Env,
        prefs: WindowPrefs,
        label: String,
        inbox: Receiver<Command>,
        guard: Option<Guard>,
    ) -> Self {
        Self {
            os,
            env,
            plan: Plan::new(env, &prefs),
            prefs,
            label,
            quake: Quake::new(),
            inbox,
            _guard: guard,
            registered: None,
            attempted: None,
            problem: None,
            tray_built: false,
            dock: None,
            spaces: None,
        }
    }

    pub(crate) fn prefs(&self) -> &WindowPrefs {
        &self.prefs
    }

    /// Whether this system can show a menu-bar item at all.
    pub(crate) fn tray_supported(&self) -> bool {
        self.env.tray_supported
    }

    pub(crate) fn hotkey_status(&self) -> HotkeyStatus {
        match &self.plan.hotkey {
            Hotkey::Off => HotkeyStatus::Off,
            Hotkey::NoGlobalHotkeys => HotkeyStatus::NoGlobalHotkeys,
            Hotkey::Invalid(problem) => HotkeyStatus::Problem(problem.to_string()),
            Hotkey::Register(chord) if self.registered == Some(*chord) => HotkeyStatus::Registered,
            Hotkey::Register(_) => HotkeyStatus::Problem(
                self.problem.clone().unwrap_or_else(|| "Not registered yet".to_owned()),
            ),
        }
    }

    /// Takes new settings live: the hotkey and the menu-bar item reconcile on the next frame, a
    /// change between quake and plain window changes the window's chrome now.
    pub(crate) fn apply_prefs(&mut self, ctx: &Context, prefs: WindowPrefs) {
        let was_quake = self.plan.quake;
        self.plan = Plan::new(self.env, &prefs);
        self.prefs = prefs;
        if self.plan.activates && was_quake != self.plan.quake {
            self.switch_mode(ctx);
        }
        ctx.request_repaint();
    }

    fn switch_mode(&mut self, ctx: &Context) {
        let quake = self.plan.quake;
        ctx.send_viewport_cmd(ViewportCommand::Decorations(!quake));
        ctx.send_viewport_cmd(ViewportCommand::Resizable(!quake));
        self.quake.restart();
        if quake {
            return;
        }
        self.os.set_alpha(quake::alpha(true));
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(
            WINDOW_SIZE[0],
            WINDOW_SIZE[1],
        )));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    /// Once per frame, before the UI draws.
    pub(crate) fn tick(&mut self, ctx: &Context) -> Vec<Event> {
        self.reconcile();
        let mut events = Vec::new();
        let commands: Vec<Command> = self.inbox.try_iter().collect();
        for command in commands {
            match command {
                Command::Open(path) => {
                    events.push(Event::Opened(path));
                    self.surface(ctx);
                }
                Command::Toggle => {
                    events.push(Event::Toggled);
                    self.toggle(ctx);
                }
            }
        }
        for click in self.os.tray_clicks() {
            match click {
                Click::ShowHide => self.toggle(ctx),
                Click::Overview => {
                    events.push(Event::Overview);
                    self.surface(ctx);
                }
                Click::Settings => {
                    events.push(Event::Settings);
                    self.surface(ctx);
                }
                Click::Quit => events.push(Event::Quit),
            }
        }
        if self.plan.activates && self.plan.quake {
            let frame = ctx.input(|i| {
                let viewport = i.viewport();
                Frame {
                    monitor_known: viewport.monitor_size.is_some(),
                    focused: viewport.focused.unwrap_or(true),
                    app_active: false,
                }
            });
            let frame = Frame { app_active: self.os.app_active(), ..frame };
            for op in self.quake.tick(&frame) {
                self.run(ctx, op);
            }
        }
        events
    }

    fn surface(&mut self, ctx: &Context) {
        if !self.plan.activates {
            return;
        }
        if self.plan.quake {
            let op = self.quake.surface();
            self.run(ctx, op);
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    fn toggle(&mut self, ctx: &Context) {
        if !self.plan.activates {
            return;
        }
        if self.plan.quake {
            let op = self.quake.toggle();
            self.run(ctx, op);
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    fn run(&mut self, ctx: &Context, op: Op) {
        match op {
            Op::Present(visible) => quake::present(ctx, &mut self.os, visible),
            Op::Repaint => ctx.request_repaint(),
            Op::RepaintAfter(after) => ctx.request_repaint_after(after),
        }
    }

    fn reconcile(&mut self) {
        self.reconcile_hotkey();
        self.reconcile_tray();
        if self.plan.activates {
            let regular = !self.plan.quake;
            if self.dock != Some(regular) {
                self.dock = Some(regular);
                self.os.set_dock_icon(regular);
            }
            if self.spaces != Some(self.plan.quake) {
                self.spaces = Some(self.plan.quake);
                self.os.set_all_spaces(self.plan.quake);
            }
        }
    }

    fn reconcile_hotkey(&mut self) {
        let want = match &self.plan.hotkey {
            Hotkey::Register(chord) => Some(*chord),
            _ => None,
        };
        if self.registered.is_some() && self.registered != want {
            self.os.unregister_hotkey();
            self.registered = None;
        }
        let Some(chord) = want else {
            self.attempted = None;
            self.problem = None;
            return;
        };
        if self.registered.is_some() || self.attempted == Some(chord) {
            return;
        }
        self.attempted = Some(chord);
        match self.os.register_hotkey(chord) {
            Ok(()) => {
                self.registered = Some(chord);
                self.problem = None;
            }
            Err(reason) => {
                eprintln!(
                    "tidewatch: no global hotkey ({reason}); `tidewatch --toggle` still works"
                );
                self.problem = Some(reason);
            }
        }
    }

    fn reconcile_tray(&mut self) {
        if self.plan.tray && !self.tray_built {
            self.tray_built = self.os.show_tray(true, &self.label);
        } else if !self.plan.tray && self.tray_built {
            self.os.show_tray(false, &self.label);
            self.tray_built = false;
        }
    }
}

/// Writes the window settings off the UI thread; a failure goes to stderr.
pub(crate) fn save(paths: tidewatch_core::paths::TidewatchPaths, prefs: WindowPrefs) {
    std::thread::spawn(move || {
        if let Err(e) = tidewatch_core::prefs::Prefs::update(&paths, |p| p.window = prefs) {
            eprintln!("tidewatch: cannot save settings.toml: {e}");
        }
    });
}

/// Everything `main` settled before the window existed.
pub(crate) struct Setup {
    pub(crate) env: Env,
    pub(crate) prefs: WindowPrefs,
    pub(crate) label: String,
    pub(crate) primary: Option<Primary>,
    pub(crate) instance_dir: Option<PathBuf>,
    pub(crate) own_socket: Option<PathBuf>,
}

pub(crate) fn start(ctx: &Context, setup: Setup) -> Shell<RealOs> {
    let (tx, rx) = channel();
    let guard = setup.primary.map(|primary| {
        let (tx, ctx) = (tx.clone(), ctx.clone());
        primary.serve(move |command| {
            let _ = tx.send(command);
            ctx.request_repaint();
        })
    });
    let press = {
        let (tx, ctx) = (tx, ctx.clone());
        let (dir, own) = (setup.instance_dir, setup.own_socket);
        move || {
            let _ = tx.send(Command::Toggle);
            if let Some(dir) = &dir {
                let _ = instance::toggle_all(dir, own.as_deref());
            }
            ctx.request_repaint();
        }
    };
    let wake = {
        let ctx = ctx.clone();
        move || ctx.request_repaint()
    };
    Shell::new(RealOs::new(press, wake), setup.env, setup.prefs, setup.label, rx, guard)
}

/// The line the dev log prints for a request.
pub(crate) fn describe(event: &Event) -> String {
    match event {
        Event::Opened(path) => format!("open {}", path.display()),
        Event::Toggled => "toggle".to_owned(),
        Event::Overview => "overview".to_owned(),
        Event::Settings => "settings".to_owned(),
        Event::Quit => "quit".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey;
    use crate::os::fake::{Call, Recorder};
    use egui::{RawInput, ViewportId, ViewportInfo, vec2};
    use std::sync::mpsc::Sender;

    const X11: Env = Env { background: false, wayland: false, tray_supported: true };
    const BACKGROUND: Env = Env { background: true, wayland: false, tray_supported: true };

    struct Rig {
        shell: Shell<Recorder>,
        ctx: Context,
        tx: Sender<Command>,
    }

    fn rig(env: Env, prefs: WindowPrefs) -> Rig {
        let (tx, rx) = channel();
        let shell = Shell::new(Recorder::default(), env, prefs, "web".to_owned(), rx, None);
        Rig { shell, ctx: Context::default(), tx }
    }

    fn windowed(monitor: bool, focused: bool) -> RawInput {
        let info = ViewportInfo {
            monitor_size: monitor.then(|| vec2(1440.0, 900.0)),
            focused: Some(focused),
            ..ViewportInfo::default()
        };
        RawInput {
            viewports: [(ViewportId::ROOT, info)].into_iter().collect(),
            ..RawInput::default()
        }
    }

    impl Rig {
        fn frame(&mut self, input: RawInput) -> (Vec<Event>, Vec<ViewportCommand>) {
            let mut events = Vec::new();
            let shell = &mut self.shell;
            let output = self.ctx.run_ui(input, |ui| events = shell.tick(ui.ctx()));
            let commands = output
                .viewport_output
                .get(&ViewportId::ROOT)
                .map(|v| v.commands.clone())
                .unwrap_or_default();
            (events, commands)
        }

        fn quiet_frame(&mut self) -> (Vec<Event>, Vec<ViewportCommand>) {
            self.frame(windowed(true, true))
        }

        fn settle(&mut self) {
            self.quiet_frame();
            self.quiet_frame();
            self.shell.os.calls.clear();
        }
    }

    fn chord(text: &str) -> Chord {
        hotkey::parse(text).unwrap()
    }

    fn prefs(quake: bool, hotkey: &str, menu_bar: bool) -> WindowPrefs {
        WindowPrefs { quake, hotkey: hotkey.to_owned(), menu_bar }
    }

    #[test]
    fn a_background_run_touches_nothing_whatever_arrives_and_whatever_is_saved() {
        let mut rig = rig(BACKGROUND, WindowPrefs::default());
        rig.shell.os.clicks = vec![Click::ShowHide, Click::Overview, Click::Settings];
        rig.tx.send(Command::Open(PathBuf::from("/work/web"))).unwrap();
        rig.tx.send(Command::Toggle).unwrap();
        let mut events = Vec::new();
        let mut commands = Vec::new();
        for _ in 0..3 {
            let (e, c) = rig.frame(windowed(true, false));
            events.extend(e);
            commands.extend(c);
        }
        rig.shell.apply_prefs(&rig.ctx, prefs(true, "Ctrl+Alt+Grave", true));
        rig.shell.apply_prefs(&rig.ctx, prefs(false, "F13", true));
        let (e, c) = rig.quiet_frame();
        events.extend(e);
        commands.extend(c);

        assert_eq!(rig.shell.os.calls, vec![], "no hotkey, no menu-bar item, no activation policy");
        assert_eq!(commands, vec![], "no focus, show, hide, move or resize");
        assert_eq!(rig.shell.hotkey_status(), HotkeyStatus::Off);
        assert!(events.contains(&Event::Opened(PathBuf::from("/work/web"))));
        assert!(events.contains(&Event::Toggled), "the request is still reported, for the dev log");
    }

    #[test]
    fn a_foreground_run_registers_the_hotkey_builds_the_item_and_hides_the_dock_icon() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.quiet_frame();
        assert_eq!(
            rig.shell.os.calls,
            vec![
                Call::Register(chord("Ctrl+Alt+Grave")),
                Call::Tray(true),
                Call::DockIcon(false),
                Call::AllSpaces(true),
                Call::Alpha(1.0),
            ]
        );
        assert_eq!(rig.shell.hotkey_status(), HotkeyStatus::Registered);
        rig.shell.os.calls.clear();
        rig.quiet_frame();
        assert_eq!(rig.shell.os.calls, vec![], "once, not every frame");
    }

    #[test]
    fn a_plain_window_registers_nothing_and_keeps_its_dock_icon() {
        let mut rig = rig(X11, prefs(false, "Ctrl+Alt+Grave", true));
        rig.quiet_frame();
        assert_eq!(
            rig.shell.os.calls,
            vec![Call::Tray(true), Call::DockIcon(true), Call::AllSpaces(false)]
        );
        assert_eq!(rig.shell.hotkey_status(), HotkeyStatus::Off);
    }

    #[test]
    fn on_wayland_the_hotkey_is_skipped_and_toggle_is_the_way_in() {
        let mut rig = rig(Env { wayland: true, ..X11 }, WindowPrefs::default());
        rig.settle();
        assert_eq!(rig.shell.hotkey_status(), HotkeyStatus::NoGlobalHotkeys);
        rig.tx.send(Command::Toggle).unwrap();
        let (_, commands) = rig.quiet_frame();
        assert!(commands.contains(&ViewportCommand::Visible(false)), "the toggle still works");
    }

    #[test]
    fn a_refused_hotkey_is_reported_and_not_retried_every_frame_but_a_new_one_is_tried() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.shell.os.refuse_hotkey = Some("already taken".to_owned());
        rig.settle();
        assert_eq!(rig.shell.hotkey_status(), HotkeyStatus::Problem("already taken".to_owned()));
        for _ in 0..3 {
            rig.quiet_frame();
        }
        assert_eq!(rig.shell.os.calls, vec![], "not retried");

        rig.shell.os.refuse_hotkey = None;
        rig.shell.apply_prefs(&rig.ctx, prefs(true, "F13", true));
        rig.quiet_frame();
        assert_eq!(rig.shell.os.calls, vec![Call::Register(chord("F13"))]);
        assert_eq!(rig.shell.hotkey_status(), HotkeyStatus::Registered);
    }

    #[test]
    fn changing_the_hotkey_swaps_the_registration_and_turning_quake_off_drops_it() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.settle();
        rig.shell.apply_prefs(&rig.ctx, prefs(true, "Cmd+Shift+T", true));
        rig.quiet_frame();
        assert_eq!(
            rig.shell.os.calls,
            vec![Call::Unregister, Call::Register(chord("Cmd+Shift+T"))]
        );
        rig.shell.os.calls.clear();
        rig.shell.apply_prefs(&rig.ctx, prefs(false, "Cmd+Shift+T", true));
        rig.quiet_frame();
        assert_eq!(
            rig.shell.os.calls,
            vec![Call::Alpha(1.0), Call::Unregister, Call::DockIcon(true), Call::AllSpaces(false)]
        );
        assert_eq!(rig.shell.hotkey_status(), HotkeyStatus::Off);
    }

    #[test]
    fn a_hotkey_that_does_not_parse_registers_nothing_and_says_why() {
        let mut rig = rig(X11, prefs(true, "Ctrl+Banana", true));
        rig.quiet_frame();
        assert!(!rig.shell.os.calls.iter().any(|call| matches!(call, Call::Register(_))));
        assert_eq!(
            rig.shell.hotkey_status(),
            HotkeyStatus::Problem("\"Banana\" is not a key name".to_owned())
        );
    }

    #[test]
    fn the_menu_bar_item_follows_its_switch_live() {
        let mut rig = rig(X11, prefs(true, "F13", false));
        rig.settle();
        rig.shell.apply_prefs(&rig.ctx, prefs(true, "F13", true));
        rig.quiet_frame();
        assert_eq!(rig.shell.os.calls, vec![Call::Tray(true)]);
        rig.shell.os.calls.clear();
        rig.shell.apply_prefs(&rig.ctx, prefs(true, "F13", false));
        rig.quiet_frame();
        assert_eq!(rig.shell.os.calls, vec![Call::Tray(false)]);
    }

    #[test]
    fn a_launch_for_the_same_folder_surfaces_a_hidden_quake_window() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.settle();
        rig.tx.send(Command::Toggle).unwrap();
        let (_, hide) = rig.quiet_frame();
        assert!(hide.contains(&ViewportCommand::Visible(false)));
        rig.tx.send(Command::Open(PathBuf::from("/work/web"))).unwrap();
        let (events, show) = rig.quiet_frame();
        assert_eq!(events, vec![Event::Opened(PathBuf::from("/work/web"))]);
        assert!(show.contains(&ViewportCommand::Focus));
    }

    #[test]
    fn a_toggle_hides_a_shown_window_and_shows_it_again() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.settle();
        rig.tx.send(Command::Toggle).unwrap();
        let (events, hide) = rig.quiet_frame();
        assert_eq!(events, vec![Event::Toggled]);
        assert!(hide.contains(&ViewportCommand::Visible(false)));
        assert!(!hide.contains(&ViewportCommand::Focus));
        rig.tx.send(Command::Toggle).unwrap();
        let (_, show) = rig.quiet_frame();
        assert!(show.contains(&ViewportCommand::Visible(true)));
        assert!(show.contains(&ViewportCommand::Focus));
    }

    #[test]
    fn on_a_plain_window_a_toggle_or_a_launch_just_raises_it() {
        let mut rig = rig(X11, prefs(false, "F13", true));
        rig.settle();
        rig.tx.send(Command::Toggle).unwrap();
        let (_, raised) = rig.quiet_frame();
        assert_eq!(raised, vec![ViewportCommand::Focus]);
        rig.tx.send(Command::Open(PathBuf::from("/work/web"))).unwrap();
        let (_, raised) = rig.quiet_frame();
        assert_eq!(raised, vec![ViewportCommand::Focus]);
    }

    #[test]
    fn menu_bar_clicks_become_requests_and_surface_the_window_except_quit() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.settle();
        rig.shell.os.clicks = vec![Click::Overview, Click::Settings, Click::Quit];
        let (events, commands) = rig.quiet_frame();
        assert_eq!(events, vec![Event::Overview, Event::Settings, Event::Quit]);
        assert_eq!(commands.iter().filter(|c| **c == ViewportCommand::Focus).count(), 2);
        rig.shell.os.clicks = vec![Click::ShowHide];
        let (events, hide) = rig.quiet_frame();
        assert_eq!(events, vec![]);
        assert!(hide.contains(&ViewportCommand::Visible(false)));
    }

    #[test]
    fn the_first_frame_with_a_known_monitor_sizes_the_quake_window() {
        let mut rig = rig(X11, WindowPrefs::default());
        let (_, before) = rig.frame(windowed(false, true));
        assert_eq!(before, vec![]);
        let (_, sized) = rig.frame(windowed(true, true));
        assert!(sized.contains(&ViewportCommand::InnerSize(vec2(1440.0, 450.0))));
        let (_, after) = rig.frame(windowed(true, true));
        assert_eq!(after, vec![]);
    }

    #[test]
    fn leaving_quake_gives_the_window_its_chrome_and_size_back_and_entering_takes_them() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.settle();
        rig.shell.apply_prefs(&rig.ctx, prefs(false, "Ctrl+Alt+Grave", true));
        let (_, commands) = rig.quiet_frame();
        assert!(commands.contains(&ViewportCommand::Decorations(true)));
        assert!(commands.contains(&ViewportCommand::Resizable(true)));
        assert!(commands.contains(&ViewportCommand::InnerSize(vec2(1280.0, 800.0))));
        assert!(rig.shell.os.calls.contains(&Call::Alpha(1.0)));
        assert!(rig.shell.os.calls.contains(&Call::DockIcon(true)));

        rig.shell.apply_prefs(&rig.ctx, WindowPrefs::default());
        let (_, commands) = rig.quiet_frame();
        assert!(commands.contains(&ViewportCommand::Decorations(false)));
        assert!(commands.contains(&ViewportCommand::Resizable(false)));
        assert!(rig.shell.os.calls.contains(&Call::DockIcon(false)));
    }

    #[test]
    fn the_dev_log_names_each_request() {
        assert_eq!(describe(&Event::Opened(PathBuf::from("/a b"))), "open /a b");
        assert_eq!(describe(&Event::Toggled), "toggle");
    }
}
