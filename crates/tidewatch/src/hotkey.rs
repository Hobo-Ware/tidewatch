//! The quake hotkey as a string (`Ctrl+Alt+Grave`, `F13`, `Cmd+Shift+T`) and what it parses to.
//!
//! Strict on purpose: the settings field shows why a string is refused instead of quietly using
//! another one. Pure; the registering itself is in `os`.

use std::fmt;
use std::str::FromStr;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chord {
    pub(crate) mods: Modifiers,
    pub(crate) code: Code,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Problem {
    Empty,
    NoKey,
    UnknownKey(String),
    UnknownModifier(String),
    RepeatedModifier(String),
    NeedsModifier,
    ModifierAsKey(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Type a hotkey such as Ctrl+Alt+Grave"),
            Self::NoKey => write!(f, "Name a key after the modifiers, such as Ctrl+Alt+Grave"),
            Self::UnknownKey(key) => write!(f, "\"{key}\" is not a key name"),
            Self::UnknownModifier(name) => {
                write!(f, "\"{name}\" is not a modifier (Ctrl, Alt, Shift, Cmd)")
            }
            Self::RepeatedModifier(name) => write!(f, "{name} appears twice"),
            Self::NeedsModifier => {
                write!(
                    f,
                    "A key on its own would stop you typing it: add a modifier or use F1 to F24"
                )
            }
            Self::ModifierAsKey(name) => write!(f, "{name} is a modifier, not the main key"),
        }
    }
}

fn modifier(name: &str) -> Option<Modifiers> {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifiers::CONTROL),
        "cmd" | "command" | "super" | "meta" | "win" => Some(Modifiers::META),
        "alt" | "option" | "opt" => Some(Modifiers::ALT),
        "shift" => Some(Modifiers::SHIFT),
        _ => None,
    }
}

fn key(name: &str) -> Option<Code> {
    let lower = name.to_ascii_lowercase();
    let w3c = match lower.as_str() {
        "`" | "grave" | "backquote" => "Backquote".to_owned(),
        "§" | "±" | "section" | "paragraph" | "intlbackslash" => "IntlBackslash".to_owned(),
        "space" => "Space".to_owned(),
        "enter" | "return" => "Enter".to_owned(),
        "tab" => "Tab".to_owned(),
        "esc" | "escape" => "Escape".to_owned(),
        letter if letter.len() == 1 && letter.as_bytes()[0].is_ascii_alphabetic() => {
            format!("Key{}", letter.to_ascii_uppercase())
        }
        digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() => {
            format!("Digit{digit}")
        }
        function if function.starts_with('f') && function[1..].parse::<u8>().is_ok() => {
            format!("F{}", &function[1..])
        }
        _ => name.to_owned(),
    };
    Code::from_str(&w3c).ok()
}

fn is_modifier_code(code: Code) -> bool {
    matches!(
        code,
        Code::ControlLeft
            | Code::ControlRight
            | Code::AltLeft
            | Code::AltRight
            | Code::ShiftLeft
            | Code::ShiftRight
            | Code::MetaLeft
            | Code::MetaRight
            | Code::CapsLock
            | Code::Fn
            | Code::FnLock
    )
}

fn is_function_key(code: Code) -> bool {
    matches!(
        code,
        Code::F1
            | Code::F2
            | Code::F3
            | Code::F4
            | Code::F5
            | Code::F6
            | Code::F7
            | Code::F8
            | Code::F9
            | Code::F10
            | Code::F11
            | Code::F12
            | Code::F13
            | Code::F14
            | Code::F15
            | Code::F16
            | Code::F17
            | Code::F18
            | Code::F19
            | Code::F20
            | Code::F21
            | Code::F22
            | Code::F23
            | Code::F24
    )
}

/// `Ctrl+Alt+Grave` and friends. Modifiers first, one main key last, any case. Without a modifier
/// only F1 to F24 are accepted: a bare letter would be taken from every app.
pub(crate) fn parse(text: &str) -> Result<Chord, Problem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Problem::Empty);
    }
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    if parts.iter().any(|part| part.is_empty()) {
        return Err(Problem::NoKey);
    }
    let (main, names) = parts.split_last().ok_or(Problem::Empty)?;
    let mut mods = Modifiers::empty();
    for name in names {
        let flag = modifier(name).ok_or_else(|| Problem::UnknownModifier((*name).to_owned()))?;
        if mods.contains(flag) {
            return Err(Problem::RepeatedModifier((*name).to_owned()));
        }
        mods |= flag;
    }
    if modifier(main).is_some() {
        return Err(Problem::NoKey);
    }
    let code = key(main).ok_or_else(|| Problem::UnknownKey((*main).to_owned()))?;
    if is_modifier_code(code) {
        return Err(Problem::ModifierAsKey((*main).to_owned()));
    }
    if mods.is_empty() && !is_function_key(code) {
        return Err(Problem::NeedsModifier);
    }
    Ok(Chord { mods, code })
}

impl Chord {
    pub(crate) fn hotkey(self) -> HotKey {
        HotKey::new((!self.mods.is_empty()).then_some(self.mods), self.code)
    }
}

fn key_name(key: egui::Key) -> Option<String> {
    use egui::Key;
    let named = match key {
        Key::Escape => return None,
        Key::Backtick => "Grave",
        Key::Equals | Key::Plus => "Equal",
        Key::OpenBracket => "BracketLeft",
        Key::CloseBracket => "BracketRight",
        Key::Minus => "Minus",
        Key::Comma => "Comma",
        Key::Period => "Period",
        Key::Slash => "Slash",
        Key::Backslash => "Backslash",
        Key::Semicolon => "Semicolon",
        Key::Quote => "Quote",
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Insert => "Insert",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::ArrowUp => "ArrowUp",
        Key::ArrowDown => "ArrowDown",
        Key::ArrowLeft => "ArrowLeft",
        Key::ArrowRight => "ArrowRight",
        other => {
            let name = other.name();
            let plain = name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric();
            let function = name.starts_with('F') && name[1..].parse::<u8>().is_ok();
            return (plain || function).then(|| name.to_owned());
        }
    };
    Some(named.to_owned())
}

/// The text a pressed key stands for, in the spelling `parse` takes (`Ctrl+Alt+Grave`). `None` for
/// Escape, which cancels, and for keys that have no name here.
pub(crate) fn spell(modifiers: egui::Modifiers, key: egui::Key) -> Option<String> {
    let main = key_name(key)?;
    let held = [
        (modifiers.ctrl, "Ctrl"),
        (modifiers.alt, "Alt"),
        (modifiers.shift, "Shift"),
        (modifiers.mac_cmd, "Cmd"),
    ];
    let mut parts: Vec<&str> = held.iter().filter(|(on, _)| *on).map(|(_, name)| *name).collect();
    parts.push(&main);
    Some(parts.join("+"))
}

/// What each key of a hotkey text shows on its cap: `Grave` as a backtick, the rest as typed.
pub(crate) fn cap_labels(text: &str) -> Vec<String> {
    text.split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let symbol = match part.to_ascii_lowercase().as_str() {
                "grave" | "backquote" => "`",
                "escape" => "Esc",
                "minus" => "-",
                "equal" => "=",
                "comma" => ",",
                "period" => ".",
                "slash" => "/",
                "backslash" => "\\",
                "semicolon" => ";",
                "quote" => "'",
                "bracketleft" => "[",
                "bracketright" => "]",
                _ => return part.to_owned(),
            };
            symbol.to_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(mods: Modifiers, code: Code) -> Chord {
        Chord { mods, code }
    }

    #[test]
    fn the_default_and_the_usual_spellings_parse() {
        let cases = [
            ("Ctrl+Alt+Grave", chord(Modifiers::CONTROL | Modifiers::ALT, Code::Backquote)),
            ("ctrl + alt + `", chord(Modifiers::CONTROL | Modifiers::ALT, Code::Backquote)),
            (
                "Control+Option+backquote",
                chord(Modifiers::CONTROL | Modifiers::ALT, Code::Backquote),
            ),
            ("Cmd+Shift+T", chord(Modifiers::META | Modifiers::SHIFT, Code::KeyT)),
            ("super+space", chord(Modifiers::META, Code::Space)),
            ("Ctrl+1", chord(Modifiers::CONTROL, Code::Digit1)),
            ("Alt+Return", chord(Modifiers::ALT, Code::Enter)),
            ("Ctrl+Escape", chord(Modifiers::CONTROL, Code::Escape)),
            ("Ctrl+ArrowUp", chord(Modifiers::CONTROL, Code::ArrowUp)),
            ("Ctrl+Alt+§", chord(Modifiers::CONTROL | Modifiers::ALT, Code::IntlBackslash)),
        ];
        for (text, want) in cases {
            assert_eq!(parse(text), Ok(want), "{text}");
        }
    }

    #[test]
    fn a_function_key_may_stand_alone() {
        for (text, code) in [("F13", Code::F13), ("f1", Code::F1), ("F24", Code::F24)] {
            assert_eq!(parse(text), Ok(chord(Modifiers::empty(), code)), "{text}");
        }
        assert_eq!(parse("Shift+F5"), Ok(chord(Modifiers::SHIFT, Code::F5)));
    }

    #[test]
    fn a_bare_ordinary_key_is_refused_because_it_would_be_taken_from_every_app() {
        for text in ["T", "Space", "Enter", "Grave", "1", "ArrowUp"] {
            assert_eq!(parse(text), Err(Problem::NeedsModifier), "{text}");
        }
    }

    #[test]
    fn mistakes_are_named() {
        let cases = [
            ("", Problem::Empty),
            ("   ", Problem::Empty),
            ("Ctrl+", Problem::NoKey),
            ("+Grave", Problem::NoKey),
            ("Ctrl++Grave", Problem::NoKey),
            ("Ctrl", Problem::NoKey),
            ("Ctrl+Alt", Problem::NoKey),
            ("Hyper+Grave", Problem::UnknownModifier("Hyper".into())),
            ("Ctrl+Ctrl+Grave", Problem::RepeatedModifier("Ctrl".into())),
            ("Ctrl+Banana", Problem::UnknownKey("Banana".into())),
            ("Ctrl+F99", Problem::UnknownKey("F99".into())),
            ("Ctrl+ShiftLeft", Problem::ModifierAsKey("ShiftLeft".into())),
            ("Ctrl+Grave+T", Problem::UnknownModifier("Grave".into())),
        ];
        for (text, want) in cases {
            assert_eq!(parse(text), Err(want), "{text:?}");
        }
    }

    #[test]
    fn every_problem_has_words_for_the_settings_field() {
        let problems = [
            Problem::Empty,
            Problem::NoKey,
            Problem::UnknownKey("x".into()),
            Problem::UnknownModifier("x".into()),
            Problem::RepeatedModifier("x".into()),
            Problem::NeedsModifier,
            Problem::ModifierAsKey("x".into()),
        ];
        for problem in problems {
            assert_ne!(problem.to_string().len(), 0, "{problem:?}");
        }
    }

    #[test]
    fn the_chord_becomes_the_hotkey_the_os_registers() {
        let hotkey = parse("Ctrl+Alt+Grave").unwrap().hotkey();
        assert_eq!(hotkey, HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Backquote));
        assert_eq!(parse("F13").unwrap().hotkey(), HotKey::new(None, Code::F13));
    }

    #[test]
    fn tidewatch_and_stdusk_do_not_share_a_default() {
        let tidewatch = parse(tidewatch_core::prefs::QUAKE_HOTKEY_DEFAULT).unwrap();
        assert_ne!(tidewatch, parse("Ctrl+Grave").unwrap());
    }

    #[test]
    fn a_pressed_chord_is_spelled_the_way_parse_reads_it() {
        let mods = egui::Modifiers { ctrl: true, alt: true, ..egui::Modifiers::NONE };
        assert_eq!(spell(mods, egui::Key::Backtick).as_deref(), Some("Ctrl+Alt+Grave"));
        let mods = egui::Modifiers { mac_cmd: true, shift: true, ..egui::Modifiers::NONE };
        assert_eq!(spell(mods, egui::Key::T).as_deref(), Some("Shift+Cmd+T"));
        assert_eq!(spell(egui::Modifiers::NONE, egui::Key::F13).as_deref(), Some("F13"));
        assert_eq!(spell(egui::Modifiers::NONE, egui::Key::Escape), None);
    }

    #[test]
    fn every_key_that_can_be_spelled_parses_back_to_a_key() {
        let mods = egui::Modifiers { ctrl: true, ..egui::Modifiers::NONE };
        let mut spelled = 0;
        for key in egui::Key::ALL {
            let Some(text) = spell(mods, *key) else { continue };
            spelled += 1;
            assert!(parse(&text).is_ok(), "{key:?} became {text}");
        }
        assert!(spelled > 60, "{spelled}");
    }

    #[test]
    fn caps_show_symbols_where_the_name_is_long() {
        assert_eq!(cap_labels("Ctrl+Alt+Grave"), ["Ctrl", "Alt", "`"]);
        assert_eq!(cap_labels("Cmd + Shift + bracketleft"), ["Cmd", "Shift", "["]);
        assert_eq!(cap_labels("F13"), ["F13"]);
        assert_eq!(cap_labels("Ctrl+"), ["Ctrl"]);
        assert_eq!(cap_labels(""), Vec::<String>::new());
    }
}
