//! The table as rows for the Shortcuts page and `docs/shortcuts.md`: the nine tab entries are one row.

#[cfg(test)]
use std::fmt::Write;

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
                    let last = last_tab?.chord(platform);
                    Some(Row {
                        group: entry.group,
                        title: "Switch to tab 1 to 9".to_owned(),
                        keys: range(chord, last),
                    })
                }
                Command::SelectTab(_) => None,
                _ => Some(Row {
                    group: entry.group,
                    title: entry.title.to_owned(),
                    keys: chord.label(),
                }),
            }
        })
        .collect()
}

fn range(first: Chord, last: Chord) -> String {
    format!("{} to {}", first.label(), last.key.symbol_or_name())
}

#[cfg(test)]
pub(crate) fn markdown() -> String {
    let mut out = String::from(
        "# Keyboard shortcuts\n\n\
         Generated from the command table in `crates/tidewatch/src/commands.rs`. A test fails when \
         this file differs from the table. Rewrite it with \
         `TIDEWATCH_BLESS=1 cargo test -p tidewatch shortcuts_doc`.\n\n\
         On Linux a plain Ctrl chord would reach the shell, so Linux uses Ctrl+Shift or Alt where \
         macOS uses Cmd. Enter, Esc and the arrow keys inside dialogs are fixed.\n",
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
            let _ = writeln!(out, "| {} | `{}` | `{}` |", m.title, m.keys, l.keys);
        }
    }
    out
}
