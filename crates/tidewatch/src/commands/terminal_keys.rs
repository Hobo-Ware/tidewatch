//! The line-editing keys the terminal turns into bytes for the shell. They are not commands: the
//! shell owns them. Listed so the Shortcuts page and `docs/shortcuts.md` can say what they do.

use egui::Key;

use super::{Chord, Platform, alt, cmd, ctrl, plain};

#[derive(Clone, Copy, Debug)]
struct Binding {
    chord: Chord,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TerminalKey {
    pub(crate) title: &'static str,
    mac: Binding,
    linux: Binding,
}

impl TerminalKey {
    fn binding(&self, platform: Platform) -> Binding {
        match platform {
            Platform::Mac => self.mac,
            Platform::Linux => self.linux,
        }
    }

    pub(crate) fn keys_on(&self, platform: Platform) -> String {
        self.binding(platform).chord.label()
    }

    #[cfg(test)]
    pub(crate) fn chord_on(&self, platform: Platform) -> Chord {
        self.binding(platform).chord
    }
}

const fn bind(chord: Chord) -> Binding {
    Binding { chord }
}

const fn key(title: &'static str, mac: Binding, linux: Binding) -> TerminalKey {
    TerminalKey { title, mac, linux }
}

const KEYS: [TerminalKey; 6] = [
    key("Back one word", bind(alt(Key::ArrowLeft)), bind(alt(Key::ArrowLeft))),
    key("Forward one word", bind(alt(Key::ArrowRight)), bind(alt(Key::ArrowRight))),
    key("Start of the line", bind(cmd(Key::ArrowLeft)), bind(plain(Key::Home))),
    key("End of the line", bind(cmd(Key::ArrowRight)), bind(plain(Key::End))),
    key("Delete the previous word", bind(alt(Key::Backspace)), bind(alt(Key::Backspace))),
    key("Delete to the start of the line", bind(cmd(Key::Backspace)), bind(ctrl(Key::U))),
];

pub(crate) fn terminal_keys() -> &'static [TerminalKey] {
    &KEYS
}
