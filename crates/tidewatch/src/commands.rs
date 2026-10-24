//! The command table: id, title, group, the key chord on macOS and on Linux, and what it runs.
//! Key handling, the palette, the Shortcuts page and `docs/shortcuts.md` all read it.

mod rows;
mod terminal_keys;

#[cfg(test)]
mod tests;

use egui::{Key, Modifiers};

#[cfg(test)]
pub(crate) use rows::markdown;
pub(crate) use rows::rows;
pub(crate) use terminal_keys::terminal_keys;

use crate::tabs::{Action, Step};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Mac,
    Linux,
}

impl Platform {
    pub(crate) const CURRENT: Self =
        if cfg!(target_os = "macos") { Self::Mac } else { Self::Linux };

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Mac => "macOS",
            Self::Linux => "Linux",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Tabs,
    View,
    App,
    Dialogs,
}

impl Group {
    pub(crate) const ALL: [Self; 4] = [Self::Tabs, Self::View, Self::App, Self::Dialogs];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Tabs => "Tabs",
            Self::View => "View",
            Self::App => "App",
            Self::Dialogs => "Dialogs",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    NewTab,
    CloseTab,
    SelectTab(usize),
    MoveTabLeft,
    MoveTabRight,
    ParkTab,
    NextRepo,
    PrevRepo,
    Overview,
    Focus,
    Reader,
    FileView,
    BackToTerminal,
    PrevFile,
    NextFile,
    PrevCommit,
    NextCommit,
    Settings,
    Shortcuts,
    OpenRepo,
    Palette,
    ApproveDelivery,
    BuildWorkflow,
    MoveSessions,
    Amphetamine,
}

impl Command {
    pub(crate) fn action(self) -> Option<Action> {
        match self {
            Self::NewTab => Some(Action::New),
            Self::CloseTab => Some(Action::CloseActive),
            Self::SelectTab(index) => Some(Action::SelectNth(index)),
            Self::MoveTabLeft => Some(Action::MoveActive(Step::Left)),
            Self::MoveTabRight => Some(Action::MoveActive(Step::Right)),
            Self::ParkTab => Some(Action::ParkActive),
            Self::NextRepo => Some(Action::NextGroup),
            Self::PrevRepo => Some(Action::PrevGroup),
            Self::Overview
            | Self::Focus
            | Self::Reader
            | Self::FileView
            | Self::BackToTerminal
            | Self::PrevFile
            | Self::NextFile
            | Self::PrevCommit
            | Self::NextCommit
            | Self::Settings
            | Self::Shortcuts
            | Self::OpenRepo
            | Self::Palette
            | Self::ApproveDelivery
            | Self::BuildWorkflow
            | Self::MoveSessions
            | Self::Amphetamine => None,
        }
    }

    fn global(self) -> bool {
        !matches!(
            self,
            Self::ApproveDelivery | Self::BuildWorkflow | Self::MoveSessions | Self::Amphetamine
        )
    }

    /// A bare key the terminal and the text fields also want: it is only read, and so only taken
    /// from them, while the file view has the keyboard.
    fn in_file_view_only(self) -> bool {
        matches!(
            self,
            Self::BackToTerminal
                | Self::PrevFile
                | Self::NextFile
                | Self::PrevCommit
                | Self::NextCommit
        )
    }

    /// A chord a text field also reads (Cmd+Shift+Left selects to the line start): it is only
    /// taken from the field, and so only read, while no text field is focused.
    fn yields_to_text_fields(self) -> bool {
        matches!(self, Self::MoveTabLeft | Self::MoveTabRight)
    }

    /// A command that does nothing in a window with one repo, so the palette leaves it out there.
    pub(crate) fn needs_several_repos(self) -> bool {
        matches!(self, Self::NextRepo | Self::PrevRepo)
    }

    pub(crate) fn in_palette(self) -> bool {
        !matches!(self, Self::SelectTab(_) | Self::OpenRepo | Self::Palette | Self::ApproveDelivery)
            && !self.in_file_view_only()
    }

    /// Another name the palette finds the command by, shown only when it was the match.
    pub(crate) fn alias(self) -> Option<&'static str> {
        matches!(self, Self::Amphetamine).then_some("cocaine")
    }

    pub(crate) fn id(self) -> &'static str {
        entry_of(self).map_or("", |entry| entry.id)
    }

    pub(crate) fn keys(self) -> String {
        self.keys_on(Platform::CURRENT)
    }

    pub(crate) fn keys_on(self, platform: Platform) -> String {
        entry_of(self)
            .and_then(|entry| entry.chord(platform))
            .map_or_else(String::new, Chord::label)
    }
}

const CMD: u8 = 1;
const CTRL: u8 = 2;
const ALT: u8 = 4;
const SHIFT: u8 = 8;

/// `CMD` is the Command key on macOS; `CTRL` is the physical Control key everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Chord {
    mods: u8,
    key: Key,
}

const fn cmd(key: Key) -> Chord {
    Chord { mods: CMD, key }
}

const fn ctrl(key: Key) -> Chord {
    Chord { mods: CTRL, key }
}

const fn plain(key: Key) -> Chord {
    Chord { mods: 0, key }
}

const fn alt(key: Key) -> Chord {
    Chord { mods: ALT, key }
}

const fn cmd_ctrl(key: Key) -> Chord {
    Chord { mods: CMD | CTRL, key }
}

impl Chord {
    const fn shift(self) -> Self {
        Self { mods: self.mods | SHIFT, ..self }
    }

    fn has(self, flag: u8) -> bool {
        self.mods & flag != 0
    }

    pub(crate) fn modifiers(self) -> Modifiers {
        let mut modifiers = Modifiers::NONE;
        for (flag, egui_flag) in [
            (CMD, Modifiers::COMMAND),
            (CTRL, Modifiers::CTRL),
            (ALT, Modifiers::ALT),
            (SHIFT, Modifiers::SHIFT),
        ] {
            if self.has(flag) {
                modifiers = modifiers.plus(egui_flag);
            }
        }
        modifiers
    }

    /// The modifiers the OS reports while this chord is down on `platform`.
    pub(crate) fn held(self, platform: Platform) -> Modifiers {
        Modifiers {
            alt: self.has(ALT),
            ctrl: self.has(CTRL),
            shift: self.has(SHIFT),
            mac_cmd: platform == Platform::Mac && self.has(CMD),
            command: self.has(CMD) || (platform == Platform::Linux && self.has(CTRL)),
        }
    }

    pub(crate) fn label(self) -> String {
        let names = [(CMD, "Cmd"), (CTRL, "Ctrl"), (ALT, "Alt"), (SHIFT, "Shift")];
        let mut parts: Vec<&str> =
            names.into_iter().filter(|(flag, _)| self.has(*flag)).map(|(_, name)| name).collect();
        parts.push(key_name(self.key));
        parts.join("+")
    }

    /// Shift turns `/` into `?` on most layouts, so that chord arrives as the other key.
    fn keys(self) -> impl Iterator<Item = Key> {
        let shifted = (self.has(SHIFT) && self.key == Key::Slash).then_some(Key::Questionmark);
        std::iter::once(self.key).chain(shifted)
    }

    /// egui-winit leaves the text of a press out only while Ctrl or Cmd is down, so an Alt chord
    /// would type its key into the terminal as well.
    fn types_its_key(self) -> bool {
        self.has(ALT) && !self.has(CMD) && !self.has(CTRL)
    }

    pub(crate) fn press_event(self, platform: Platform) -> egui::Event {
        egui::Event::Key {
            key: self.key,
            physical_key: Some(self.key),
            pressed: true,
            repeat: false,
            modifiers: self.held(platform),
        }
    }

    fn weight(self) -> usize {
        self.mods.count_ones() as usize
    }
}

fn key_name(key: Key) -> &'static str {
    match key {
        Key::Escape => "Esc",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        other => other.symbol_or_name(),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) group: Group,
    pub(crate) command: Command,
    mac: Option<Chord>,
    linux: Option<Chord>,
}

impl Entry {
    /// `None` for a command that is only in the palette.
    pub(crate) fn chord(&self, platform: Platform) -> Option<Chord> {
        match platform {
            Platform::Mac => self.mac,
            Platform::Linux => self.linux,
        }
    }
}

const fn entry(
    id: &'static str,
    title: &'static str,
    group: Group,
    command: Command,
    (mac, linux): (Chord, Chord),
) -> Entry {
    Entry { id, title, group, command, mac: Some(mac), linux: Some(linux) }
}

const fn unbound(id: &'static str, title: &'static str, group: Group, command: Command) -> Entry {
    Entry { id, title, group, command, mac: None, linux: None }
}

const fn tab(id: &'static str, title: &'static str, index: usize, key: Key) -> Entry {
    entry(id, title, Group::Tabs, Command::SelectTab(index), (cmd(key), alt(key)))
}

pub(crate) const TABLE: [Entry; 33] = [
    entry("new-tab", "New tab", Group::Tabs, Command::NewTab, (cmd(Key::T), ctrl(Key::T).shift())),
    entry(
        "close-tab",
        "Close tab",
        Group::Tabs,
        Command::CloseTab,
        (cmd(Key::W), ctrl(Key::W).shift()),
    ),
    tab("tab-1", "Switch to main (tab 1)", 0, Key::Num1),
    tab("tab-2", "Switch to tab 2", 1, Key::Num2),
    tab("tab-3", "Switch to tab 3", 2, Key::Num3),
    tab("tab-4", "Switch to tab 4", 3, Key::Num4),
    tab("tab-5", "Switch to tab 5", 4, Key::Num5),
    tab("tab-6", "Switch to tab 6", 5, Key::Num6),
    tab("tab-7", "Switch to tab 7", 6, Key::Num7),
    tab("tab-8", "Switch to tab 8", 7, Key::Num8),
    tab("tab-9", "Switch to tab 9", 8, Key::Num9),
    entry(
        "move-tab-left",
        "Move tab left",
        Group::Tabs,
        Command::MoveTabLeft,
        (cmd(Key::ArrowLeft).shift(), ctrl(Key::PageUp).shift()),
    ),
    entry(
        "move-tab-right",
        "Move tab right",
        Group::Tabs,
        Command::MoveTabRight,
        (cmd(Key::ArrowRight).shift(), ctrl(Key::PageDown).shift()),
    ),
    entry(
        "park-tab",
        "Park tab",
        Group::Tabs,
        Command::ParkTab,
        (cmd(Key::K).shift(), ctrl(Key::K).shift()),
    ),
    entry(
        "next-repo",
        "Next repo",
        Group::Tabs,
        Command::NextRepo,
        (cmd_ctrl(Key::CloseBracket), alt(Key::CloseBracket)),
    ),
    entry(
        "prev-repo",
        "Previous repo",
        Group::Tabs,
        Command::PrevRepo,
        (cmd_ctrl(Key::OpenBracket), alt(Key::OpenBracket)),
    ),
    entry("overview", "Overview", Group::View, Command::Overview, (cmd(Key::Num0), alt(Key::Num0))),
    entry(
        "focus",
        "Focus mode",
        Group::View,
        Command::Focus,
        (cmd(Key::Period), ctrl(Key::Period)),
    ),
    entry(
        "reader",
        "Reader or raw terminal",
        Group::View,
        Command::Reader,
        (cmd(Key::T).shift(), ctrl(Key::R).shift()),
    ),
    entry(
        "file-view",
        "Terminal or last file",
        Group::View,
        Command::FileView,
        (cmd(Key::J), ctrl(Key::J).shift()),
    ),
    entry(
        "back-to-terminal",
        "Back to the terminal",
        Group::View,
        Command::BackToTerminal,
        (plain(Key::Escape), plain(Key::Escape)),
    ),
    entry(
        "prev-file",
        "Previous file",
        Group::View,
        Command::PrevFile,
        (plain(Key::ArrowUp), plain(Key::ArrowUp)),
    ),
    entry(
        "next-file",
        "Next file",
        Group::View,
        Command::NextFile,
        (plain(Key::ArrowDown), plain(Key::ArrowDown)),
    ),
    entry(
        "prev-commit",
        "Previous commit",
        Group::View,
        Command::PrevCommit,
        (plain(Key::OpenBracket), plain(Key::OpenBracket)),
    ),
    entry(
        "next-commit",
        "Next commit",
        Group::View,
        Command::NextCommit,
        (plain(Key::CloseBracket), plain(Key::CloseBracket)),
    ),
    entry(
        "settings",
        "Settings",
        Group::App,
        Command::Settings,
        (cmd(Key::Comma), ctrl(Key::Comma)),
    ),
    entry(
        "shortcuts",
        "Keyboard shortcuts",
        Group::App,
        Command::Shortcuts,
        (cmd(Key::Slash), ctrl(Key::Slash).shift()),
    ),
    entry(
        "open-repo",
        "Open repo",
        Group::App,
        Command::OpenRepo,
        (cmd(Key::O), ctrl(Key::O).shift()),
    ),
    entry(
        "palette",
        "Command palette",
        Group::App,
        Command::Palette,
        (cmd(Key::P).shift(), ctrl(Key::P).shift()),
    ),
    entry(
        "approve-delivery",
        "Push and open the pull request",
        Group::Dialogs,
        Command::ApproveDelivery,
        (cmd(Key::Enter), ctrl(Key::Enter)),
    ),
    unbound(
        "build-workflow",
        "Build workflow from this session",
        Group::App,
        Command::BuildWorkflow,
    ),
    unbound("move-sessions", "Move sessions to Tidewatch", Group::App, Command::MoveSessions),
    unbound("amphetamine", "Amphetamine", Group::App, Command::Amphetamine),
];

fn entry_of(command: Command) -> Option<&'static Entry> {
    TABLE.iter().find(|entry| entry.command == command)
}

/// Reads the chords pressed this frame and consumes them. Most specific first, because egui
/// matches a chord that asks for fewer Shift or Alt keys against a press that holds more: Cmd+T
/// must not take a Cmd+Shift+T press.
pub(crate) fn consume(
    ctx: &egui::Context,
    platform: Platform,
    wanted: impl Fn(Command) -> bool,
) -> Vec<Command> {
    let mut ordered: Vec<(&Entry, Chord)> = TABLE
        .iter()
        .filter(|entry| wanted(entry.command))
        .filter_map(|entry| Some((entry, entry.chord(platform)?)))
        .collect();
    ordered.sort_by_key(|(_, chord)| std::cmp::Reverse(chord.weight()));
    ctx.input_mut(|input| {
        ordered
            .into_iter()
            .filter(|(_, chord)| {
                let taken = (chord.mods != 0 || !pressed_with_modifiers(input, chord.key))
                    && chord.keys().any(|key| input.consume_key(chord.modifiers(), key));
                if taken && chord.types_its_key() {
                    let typed = chord.key.symbol_or_name();
                    input
                        .events
                        .retain(|event| !matches!(event, egui::Event::Text(text) if text == typed));
                }
                taken
            })
            .map(|(entry, _)| entry.command)
            .collect()
    })
}

/// egui reads a bare key chord as satisfied by the same key with Alt or Shift held, which would
/// take Alt+Down from the diff and Shift+[ from a text field.
fn pressed_with_modifiers(input: &egui::InputState, key: Key) -> bool {
    input.events.iter().any(|event| {
        matches!(event, egui::Event::Key { key: pressed, pressed: true, modifiers, .. }
            if *pressed == key && modifiers.any())
    })
}

pub(crate) fn command_of(id: &str) -> Option<Command> {
    TABLE.iter().find(|entry| entry.id == id).map(|entry| entry.command)
}

/// The key event a person pressing `command`'s chord produces here, for dev drivers and tests.
pub(crate) fn press_event(command: Command) -> Option<egui::Event> {
    Some(entry_of(command)?.chord(Platform::CURRENT)?.press_event(Platform::CURRENT))
}

fn wanted(gate: Gate) -> impl Fn(Command) -> bool {
    move |command| {
        command.global()
            && !((command.in_file_view_only() || command.yields_to_text_fields())
                && !gate.allows(command))
    }
}

pub(crate) fn pressed(ctx: &egui::Context, gate: Gate) -> Vec<Command> {
    consume(ctx, Platform::CURRENT, wanted(gate))
}

pub(crate) fn approve_delivery_pressed(ctx: &egui::Context) -> bool {
    !consume(ctx, Platform::CURRENT, |command| command == Command::ApproveDelivery).is_empty()
}

/// The dialog over the window, if any. The close and take-over dialogs are the ones that also
/// refuse the Settings, Overview and Focus chords.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Dialog {
    #[default]
    None,
    TabChoice,
    Other,
}

/// What is on screen, as far as a key is concerned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Gate {
    pub(crate) dialog: Dialog,
    pub(crate) settings: bool,
    pub(crate) overview: bool,
    pub(crate) palette: bool,
    pub(crate) focus: Focus,
}

/// Who gets the bare keys: the terminal, or the file view, which is the active tab showing a
/// file's diff in place of its terminal with no text field focused. A focused text field takes
/// the keys it reads from either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Focus {
    #[default]
    Terminal,
    FileView,
    TextField,
}

impl Gate {
    pub(crate) fn modal_open(self) -> bool {
        self.dialog != Dialog::None || self.settings || self.palette
    }

    pub(crate) fn allows(self, command: Command) -> bool {
        if self.palette {
            return command == Command::Palette;
        }
        let modal = self.modal_open();
        match command {
            Command::Settings | Command::Shortcuts | Command::Overview | Command::Focus => {
                self.dialog != Dialog::TabChoice
            }
            Command::Reader | Command::CloseTab | Command::FileView | Command::ParkTab => {
                !modal && !self.overview
            }
            Command::BackToTerminal
            | Command::PrevFile
            | Command::NextFile
            | Command::PrevCommit
            | Command::NextCommit => self.focus == Focus::FileView && !modal && !self.overview,
            Command::MoveTabLeft | Command::MoveTabRight => {
                !modal && !self.overview && self.focus != Focus::TextField
            }
            Command::NewTab
            | Command::SelectTab(_)
            | Command::NextRepo
            | Command::PrevRepo
            | Command::OpenRepo => !modal,
            Command::Palette => self.dialog == Dialog::None,
            Command::ApproveDelivery
            | Command::BuildWorkflow
            | Command::MoveSessions
            | Command::Amphetamine => false,
        }
    }
}
