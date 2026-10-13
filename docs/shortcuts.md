# Keyboard shortcuts

Generated from the command table in `crates/tidewatch/src/commands.rs`. A test fails when this file differs from the table. Rewrite it with `TIDEWATCH_BLESS=1 cargo test -p tidewatch shortcuts_doc`.

On Linux a plain Ctrl chord would reach the shell, so Linux uses Ctrl+Shift or Alt where macOS uses Cmd. Every command except the dialog keys is also in the command palette. Enter, Esc and the arrow keys inside dialogs and the palette are fixed.

## Tabs

| Command | macOS | Linux |
| --- | --- | --- |
| New tab | `Cmd+T` | `Ctrl+Shift+T` |
| Close tab | `Cmd+W` | `Ctrl+Shift+W` |
| Switch to tab 1 to 9 | `Cmd+1 to 9` | `Alt+1 to 9` |

## View

| Command | macOS | Linux |
| --- | --- | --- |
| Overview | `Cmd+0` | `Alt+0` |
| Focus mode | `Cmd+.` | `Ctrl+.` |
| Reader or raw terminal | `Cmd+Shift+T` | `Ctrl+Shift+R` |

## App

| Command | macOS | Linux |
| --- | --- | --- |
| Settings | `Cmd+,` | `Ctrl+,` |
| Command palette | `Cmd+Shift+P` | `Ctrl+Shift+P` |

## Dialogs

| Command | macOS | Linux |
| --- | --- | --- |
| Push and open the pull request | `Cmd+Enter` | `Ctrl+Enter` |
