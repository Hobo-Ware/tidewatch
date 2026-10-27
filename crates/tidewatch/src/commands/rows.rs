//! The table as rows for the Shortcuts page and `docs/shortcuts.md`: the nine tab entries are one row.

#[cfg(test)]
use std::fmt::Write;

#[cfg(test)]
use super::terminal_keys;
use super::{Chord, Command, Group, Platform, TABLE};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) group: Group,
    pub(crate) title: String,
    pub(crate) keys: String,
}

pub(crate) fn rows(platform: Platform) -> Vec<Row> {
    let last_tab = TABLE.iter().rev().find(|entry| matches!(entry.command, Command::SelectTab(_)));
    TABLE
        .iter()
        .filter_map(|entry| {
            let chord = entry.chord(platform);
            match entry.command {
                Command::SelectTab(0) => {
                    let last = last_tab?.chord(platform)?;
                    Some(Row {
                        group: entry.group,
                        title: "Switch to main (1) or tab 2 to 9".to_owned(),
                        keys: range(chord?, last),
                    })
                }
                Command::SelectTab(_) => None,
                _ => Some(Row {
                    group: entry.group,
                    title: entry.title.to_owned(),
                    keys: chord.map_or_else(String::new, Chord::label),
                }),
            }
        })
        .collect()
}

fn range(first: Chord, last: Chord) -> String {
    format!("{} to {}", first.label(), last.key.symbol_or_name())
}

#[cfg(test)]
fn cell(keys: &str) -> String {
    if keys.is_empty() { "palette only".to_owned() } else { format!("`{keys}`") }
}

#[cfg(test)]
pub(crate) fn markdown() -> String {
    let mut out = String::from(
        "# Keyboard shortcuts\n\n\
         Generated from the command table in `crates/tidewatch/src/commands.rs`. A test fails when \
         this file differs from the table. Rewrite it with \
         `TIDEWATCH_BLESS=1 cargo test -p tidewatch shortcuts_doc`.\n\n\
         On Linux a plain Ctrl chord would reach the shell, so Linux uses Ctrl+Shift or Alt where \
         macOS uses Cmd. Every command except the dialog keys and the bare keys of the file view is also in the \
         command palette. Esc, Up, Down, [ and ] only work while a file's diff is open in place of \
         the terminal, and never while a note is being typed; in the terminal they go to the \
         shell. In the form of a new tab, A, M and S pick Agent, Meeseeks or Shell, 1 to 9 pick a \
         card, Left and Right change the permission level and Esc closes the tab, all while no \
         text field has the keyboard; Enter or / then moves into the message box, and Esc leaves \
         a text field; Cmd+Enter (Ctrl+Enter on Linux) starts it. Enter, Esc and the arrow keys inside dialogs and the palette are fixed.\n",
    );
    let mac = rows(Platform::Mac);
    let linux = rows(Platform::Linux);
    for group in Group::ALL {
        let _ = write!(
            out,
            "\n## {}\n\n| Command | {} | {} |\n| --- | --- | --- |\n",
            group.title(),
            Platform::Mac.name(),
            Platform::Linux.name(),
        );
        for (m, l) in mac.iter().zip(&linux).filter(|(m, _)| m.group == group) {
            let _ = writeln!(out, "| {} | {} | {} |", m.title, cell(&m.keys), cell(&l.keys));
        }
    }
    let _ = write!(
        out,
        "\n## In the terminal\n\nThese are not commands: the terminal turns them into the bytes \
         your shell reads, so they edit the line in zsh, bash and most prompts.\n\n\
         | Key | {} | {} |\n| --- | --- | --- |\n",
        Platform::Mac.name(),
        Platform::Linux.name(),
    );
    for key in terminal_keys() {
        let (mac, linux) = (key.keys_on(Platform::Mac), key.keys_on(Platform::Linux));
        let _ = writeln!(out, "| {} | {} | {} |", key.title, cell(&mac), cell(&linux));
    }
    out
}
