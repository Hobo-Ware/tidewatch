//! The window shell: what other launches, the hotkey, the menu-bar item and the settings ask of
//! the window, and the quake drop-down they drive. One `tick` per frame turns those requests into
//! viewport commands and OS calls, all through the `Plan`, so a background run does none of it.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use egui::{Context, ViewportCommand};
use tidewatch_core::instance::{self, Claim, Command, Guard, Primary};
use tidewatch_core::prefs::WindowPrefs;

use crate::hotkey::Chord;
use crate::os::{Os, RealOs};
use crate::quake::{self, Frame, Op, Prompt, Quake};
use crate::shell_plan::{Env, Hotkey, Plan};
use crate::tray::Click;
use crate::window_chrome::{Chrome, Desktop};

pub(crate) const WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];
const MIN_WINDOW_SIZE: [f32; 2] = [600.0, 480.0];

pub(crate) fn min_inner_size(quake: bool) -> [f32; 2] {
    if quake { [0.0, 0.0] } else { MIN_WINDOW_SIZE }
}

/// What the app should do about a request the shell cannot answer by itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Opened(PathBuf),
    /// stdusk is handing its live panes over on this socket.
    AdoptStdusk(PathBuf),
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
    prompt: Prompt,
    inbox: Receiver<Command>,
    guard: Option<Guard>,
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
            prompt: Prompt::new(),
            inbox,
            guard,
            registered: None,
            attempted: None,
            problem: None,
            tray_built: false,
            dock: None,
            spaces: None,
        }
    }

    /// Tells launches that this window is leaving, so they wait for its socket instead of handing
    /// it a folder to open.
    pub(crate) fn set_closing(&self, closing: bool) {
        if let Some(guard) = &self.guard {
            guard.set_closing(closing);
        }
    }

    pub(crate) fn prefs(&self) -> &WindowPrefs {
        &self.prefs
    }

    /// The quake window is parked out of sight: nothing on screen needs fresh data.
    pub(crate) fn window_hidden(&self) -> bool {
        self.plan.quake && self.quake.hidden()
    }

    pub(crate) fn chrome(&self) -> Chrome {
        self.plan.chrome
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
    /// change between quake and plain window changes the window's chrome now, and a new height
    /// resizes a shown quake window now (a hidden one takes it on its next drop).
    pub(crate) fn apply_prefs(&mut self, ctx: &Context, prefs: WindowPrefs) {
        let was_quake = self.plan.quake;
        let was_height = self.prefs.height_pct;
        self.plan = Plan::new(self.env, &prefs);
        self.prefs = prefs;
        if self.plan.activates
            && self.plan.quake
            && was_quake
            && !self.quake.hidden()
            && was_height != self.prefs.height_pct
        {
            quake::resize(ctx, &self.os, self.prefs.height_pct);
        }
        if self.plan.activates && was_quake != self.plan.quake {
            self.switch_mode(ctx);
        }
        ctx.request_repaint();
    }

    fn switch_mode(&mut self, ctx: &Context) {
        let quake = self.plan.quake;
        ctx.send_viewport_cmd(ViewportCommand::Decorations(self.plan.chrome.decorations()));
        ctx.send_viewport_cmd(ViewportCommand::Resizable(!quake));
        let [min_w, min_h] = min_inner_size(quake);
        ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(egui::vec2(min_w, min_h)));
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

    /// Once per frame, before the UI draws. `prompt_open` is true while a system prompt that
    /// Tidewatch started (the Amphetamine password dialog or Touch ID sheet) is on screen.
    pub(crate) fn tick(&mut self, ctx: &Context, prompt_open: bool) -> Vec<Event> {
        self.reconcile();
        let mut events = Vec::new();
        let commands: Vec<Command> = self.inbox.try_iter().collect();
        for command in commands {
            match command {
                Command::Open(path) => {
                    events.push(Event::Opened(path));
                    self.surface(ctx);
                }
                Command::AdoptStdusk(socket) => {
                    events.push(Event::AdoptStdusk(socket));
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
        if self.plan.activates {
            let (focused, now) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.time));
            let app_active = self.os.app_active();
            let step = self.prompt.tick(prompt_open, app_active, focused, now);
            if step.refocus && !(self.plan.quake && self.quake.hidden()) {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            if self.plan.quake {
                let monitor_known = ctx.input(|i| i.viewport().monitor_size.is_some());
                let frame = Frame { monitor_known, focused, app_active, prompt_hold: step.hold };
                for op in self.quake.tick(&frame) {
                    self.run(ctx, op);
                }
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
            Op::Present(visible) => {
                quake::present(ctx, &mut self.os, visible, self.prefs.height_pct);
            }
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
    /// The app socket of the window this one takes over from. It is claimed only once that window
    /// has handed its terminals over and is on its way out.
    pub(crate) replacing: Option<PathBuf>,
    /// The desktop's title bar settings, for the frame Tidewatch draws itself.
    pub(crate) desktop: Desktop,
}

impl Setup {
    /// Claims the socket of the window being replaced, once it stops answering. Nothing is sent to
    /// it, so it never gets a folder it is about to lose.
    pub(crate) fn claim_from_old_window(&mut self) {
        let Some(socket) = self.replacing.take() else { return };
        match instance::claim_replacing(&socket) {
            Claim::Primary(primary) => {
                self.primary = Some(primary);
                self.own_socket = Some(socket);
            }
            Claim::Secondary => eprintln!("tidewatch: another window answers on the app socket"),
            Claim::Unavailable(reason) => {
                eprintln!(
                    "tidewatch: no single-instance socket ({reason}); opening a window anyway"
                );
            }
        }
    }
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
        Event::AdoptStdusk(socket) => format!("adopt-stdusk {}", socket.display()),
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

    const X11: Env =
        Env { background: false, wayland: false, tray_supported: true, own_frame: false };
    const BACKGROUND: Env = Env { background: true, ..X11 };

    struct Rig {
        shell: Shell<Recorder>,
        ctx: Context,
        tx: Sender<Command>,
        prompt_open: bool,
    }

    fn rig(env: Env, prefs: WindowPrefs) -> Rig {
        let (tx, rx) = channel();
        let shell = Shell::new(Recorder::default(), env, prefs, "web".to_owned(), rx, None);
        Rig { shell, ctx: Context::default(), tx, prompt_open: false }
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
            let prompt_open = self.prompt_open;
            let output = self.ctx.run_ui(input, |ui| events = shell.tick(ui.ctx(), prompt_open));
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
        WindowPrefs { quake, hotkey: hotkey.to_owned(), menu_bar, ..WindowPrefs::default() }
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

    fn with_height(height_pct: f32) -> WindowPrefs {
        WindowPrefs { height_pct, ..WindowPrefs::default() }
    }

    #[test]
    fn a_new_height_resizes_a_shown_quake_window_at_once_and_a_drop_uses_it() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.settle();
        rig.shell.apply_prefs(&rig.ctx, with_height(0.8));
        let (_, resized) = rig.quiet_frame();
        assert_eq!(resized, vec![ViewportCommand::InnerSize(vec2(1440.0, 720.0))]);
        rig.shell.apply_prefs(&rig.ctx, with_height(0.8));
        let (_, unchanged) = rig.quiet_frame();
        assert_eq!(unchanged, vec![]);
        rig.tx.send(Command::Toggle).unwrap();
        rig.quiet_frame();
        rig.tx.send(Command::Toggle).unwrap();
        let (_, shown) = rig.quiet_frame();
        assert!(shown.contains(&ViewportCommand::InnerSize(vec2(1440.0, 720.0))));
    }

    #[test]
    fn a_new_height_waits_for_the_next_drop_while_the_window_is_hidden() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.settle();
        rig.tx.send(Command::Toggle).unwrap();
        rig.quiet_frame();
        rig.shell.apply_prefs(&rig.ctx, with_height(0.3));
        let (_, quiet) = rig.quiet_frame();
        assert!(!quiet.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_))));
        rig.tx.send(Command::Toggle).unwrap();
        let (_, shown) = rig.quiet_frame();
        assert!(shown.contains(&ViewportCommand::InnerSize(vec2(1440.0, 270.0))));
    }

    #[test]
    fn a_new_height_leaves_a_plain_window_and_a_background_run_alone() {
        let mut plain = rig(X11, prefs(false, "F13", true));
        plain.settle();
        plain
            .shell
            .apply_prefs(&plain.ctx, WindowPrefs { height_pct: 0.8, ..prefs(false, "F13", true) });
        assert_eq!(plain.quiet_frame().1, vec![]);
        let mut background = rig(BACKGROUND, WindowPrefs::default());
        background.shell.apply_prefs(&background.ctx, with_height(0.8));
        assert_eq!(background.quiet_frame().1, vec![]);
    }

    #[test]
    fn leaving_quake_gives_the_window_its_chrome_and_size_back_and_entering_takes_them() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.settle();
        rig.shell.apply_prefs(&rig.ctx, prefs(false, "Ctrl+Alt+Grave", true));
        let (_, commands) = rig.quiet_frame();
        assert!(commands.contains(&ViewportCommand::Decorations(true)));
        assert!(commands.contains(&ViewportCommand::Resizable(true)));
        assert!(commands.contains(&ViewportCommand::MinInnerSize(vec2(600.0, 480.0))));
        assert!(commands.contains(&ViewportCommand::InnerSize(vec2(1280.0, 800.0))));
        assert!(rig.shell.os.calls.contains(&Call::Alpha(1.0)));
        assert!(rig.shell.os.calls.contains(&Call::DockIcon(true)));

        rig.shell.apply_prefs(&rig.ctx, WindowPrefs::default());
        let (_, commands) = rig.quiet_frame();
        assert!(commands.contains(&ViewportCommand::Decorations(false)));
        assert!(commands.contains(&ViewportCommand::Resizable(false)));
        assert!(commands.contains(&ViewportCommand::MinInnerSize(vec2(0.0, 0.0))));
        assert!(rig.shell.os.calls.contains(&Call::DockIcon(false)));
    }

    #[test]
    fn only_the_normal_window_has_a_least_size_and_it_fits_the_default_and_the_dev_sizes() {
        assert_eq!(min_inner_size(true), [0.0, 0.0]);
        let [min_w, min_h] = min_inner_size(false);
        assert!(min_w <= WINDOW_SIZE[0] && min_h <= WINDOW_SIZE[1]);
        assert!(min_w <= 600.0 && min_h <= 600.0, "TIDEWATCH_DEV_WINDOW starts at 600x600");
    }

    #[test]
    fn leaving_quake_on_linux_keeps_the_system_frame_off_for_the_drawn_one() {
        let linux = Env { tray_supported: false, own_frame: true, ..X11 };
        let mut rig = rig(linux, WindowPrefs::default());
        rig.settle();
        rig.shell.apply_prefs(&rig.ctx, prefs(false, "Ctrl+Alt+Grave", false));
        let (_, commands) = rig.quiet_frame();
        assert!(commands.contains(&ViewportCommand::Decorations(false)));
        assert!(commands.contains(&ViewportCommand::Resizable(true)));
        assert_eq!(rig.shell.chrome(), Chrome::Drawn);
    }

    fn focus_count(commands: &[ViewportCommand]) -> usize {
        commands.iter().filter(|c| **c == ViewportCommand::Focus).count()
    }

    fn hides(commands: &[ViewportCommand]) -> bool {
        commands.contains(&ViewportCommand::Visible(false))
            || commands.iter().any(|c| matches!(c, ViewportCommand::OuterPosition(p) if p.y > 0.0))
    }

    #[test]
    fn a_prompt_started_from_the_active_app_keeps_the_quake_window_up_and_brings_it_back_once() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.shell.os.parks = true;
        rig.shell.os.active = true;
        rig.settle();
        rig.prompt_open = true;
        rig.quiet_frame();
        rig.shell.os.active = false;
        for _ in 0..4 {
            let (_, commands) = rig.frame(windowed(true, false));
            assert!(!hides(&commands), "the prompt took the focus, the user did not");
            assert_eq!(focus_count(&commands), 0);
        }
        rig.prompt_open = false;
        let (_, back) = rig.frame(windowed(true, false));
        assert_eq!(focus_count(&back), 1);
        assert!(!hides(&back), "the window is held while the focus comes back");
        let (_, waiting) = rig.frame(windowed(true, false));
        assert_eq!((focus_count(&waiting), hides(&waiting)), (0, false));
        rig.shell.os.active = true;
        for _ in 0..3 {
            let (_, quiet) = rig.frame(windowed(true, true));
            assert_eq!(quiet, vec![]);
        }
    }

    #[test]
    fn the_focus_is_asked_for_exactly_once_however_many_frames_follow_the_prompt() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.shell.os.active = true;
        rig.settle();
        rig.prompt_open = true;
        rig.quiet_frame();
        rig.shell.os.active = false;
        rig.frame(windowed(true, false));
        rig.prompt_open = false;
        let mut focuses = 0;
        for _ in 0..6 {
            let (_, commands) = rig.frame(windowed(true, false));
            focuses += focus_count(&commands);
        }
        assert_eq!(focuses, 1);
    }

    #[test]
    fn a_prompt_started_while_another_app_was_active_never_raises_the_window() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.shell.os.parks = true;
        rig.settle();
        rig.prompt_open = true;
        for _ in 0..3 {
            let (_, commands) = rig.frame(windowed(true, false));
            assert_eq!(focus_count(&commands), 0);
        }
        rig.prompt_open = false;
        let (_, after) = rig.frame(windowed(true, false));
        assert_eq!(focus_count(&after), 0);
    }

    #[test]
    fn a_window_hidden_while_the_prompt_was_open_stays_hidden_when_it_ends() {
        let mut rig = rig(X11, WindowPrefs::default());
        rig.shell.os.active = true;
        rig.settle();
        rig.prompt_open = true;
        rig.quiet_frame();
        rig.tx.send(Command::Toggle).unwrap();
        let (_, hide) = rig.frame(windowed(true, false));
        assert!(hide.contains(&ViewportCommand::Visible(false)));
        rig.prompt_open = false;
        let (_, after) = rig.frame(windowed(true, false));
        assert_eq!(focus_count(&after), 0);
    }

    #[test]
    fn on_a_plain_window_the_end_of_a_prompt_raises_it_once() {
        let mut rig = rig(X11, prefs(false, "F13", true));
        rig.shell.os.active = true;
        rig.settle();
        rig.prompt_open = true;
        rig.quiet_frame();
        rig.shell.os.active = false;
        rig.prompt_open = false;
        let (_, back) = rig.frame(windowed(true, false));
        assert_eq!(back, vec![ViewportCommand::Focus]);
        let (_, later) = rig.frame(windowed(true, true));
        assert_eq!(later, vec![]);
    }

    #[test]
    fn a_background_or_screenshot_run_never_raises_the_window_for_a_prompt() {
        let mut rig = rig(BACKGROUND, WindowPrefs::default());
        rig.shell.os.active = true;
        rig.quiet_frame();
        rig.prompt_open = true;
        rig.quiet_frame();
        rig.shell.os.active = false;
        rig.prompt_open = false;
        for _ in 0..3 {
            let (_, commands) = rig.frame(windowed(true, false));
            assert_eq!(commands, vec![]);
        }
        assert_eq!(rig.shell.os.calls, vec![]);
    }

    #[test]
    fn the_dev_log_names_each_request() {
        assert_eq!(describe(&Event::Opened(PathBuf::from("/a b"))), "open /a b");
        assert_eq!(describe(&Event::Toggled), "toggle");
    }
}
