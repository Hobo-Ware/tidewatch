# Keyboard shortcuts

Generated from the command table in `crates/tidewatch/src/commands.rs`. A test fails when this file differs from the table. Rewrite it with `TIDEWATCH_BLESS=1 cargo test -p tidewatch shortcuts_doc`.

On Linux a plain Ctrl chord would reach the shell, so Linux uses Ctrl+Shift or Alt where macOS uses Cmd. Every command except the dialog keys and the bare keys of the file view is also in the command palette. Esc, Up, Down, [ and ] only work while a file's diff is open in place of the terminal, and never while a note is being typed; in the terminal they go to the shell. In the form of a new tab, A, M, W and S pick Agent, Meeseeks, Workflow or Shell, 1 to 9 pick a card, Left and Right change the permission level and Esc closes the tab, all while no text field has the keyboard; Enter or / then moves into the message box, and Esc leaves a text field; Cmd+Enter (Ctrl+Enter on Linux) starts it. Enter, Esc and the arrow keys inside dialogs and the palette are fixed.

## Tabs

| Command | macOS | Linux |
| --- | --- | --- |
| New tab | `Cmd+T` | `Ctrl+Shift+T` |
| Close tab | `Cmd+W` | `Ctrl+Shift+W` |
| Switch to main (1) or tab 2 to 9 | `Cmd+1 to 9` | `Alt+1 to 9` |
| Move tab left | `Cmd+Shift+Left` | `Ctrl+Shift+PageUp` |
| Move tab right | `Cmd+Shift+Right` | `Ctrl+Shift+PageDown` |
| Park tab | `Cmd+Shift+K` | `Ctrl+Shift+K` |
| Next repo | `Cmd+Ctrl+]` | `Alt+]` |
| Previous repo | `Cmd+Ctrl+[` | `Alt+[` |

## View

| Command | macOS | Linux |
| --- | --- | --- |
| Overview | `Cmd+0` | `Alt+0` |
| Focus mode | `Cmd+.` | `Ctrl+.` |
| Reader or raw terminal | `Cmd+Shift+T` | `Ctrl+Shift+R` |
| Terminal or last file | `Cmd+J` | `Ctrl+Shift+J` |
| Back to the terminal | `Esc` | `Esc` |
| Previous file | `Up` | `Up` |
| Next file | `Down` | `Down` |
| Previous commit | `[` | `[` |
| Next commit | `]` | `]` |

## App

| Command | macOS | Linux |
| --- | --- | --- |
| Settings | `Cmd+,` | `Ctrl+,` |
| Keyboard shortcuts | `Cmd+/` | `Ctrl+Shift+/` |
| Open repo | `Cmd+O` | `Ctrl+Shift+O` |
| Command palette | `Cmd+Shift+P` | `Ctrl+Shift+P` |
| Build workflow from this session | palette only | palette only |
| Move sessions to Tidewatch | palette only | palette only |
| Amphetamine | palette only | palette only |

## Dialogs

| Command | macOS | Linux |
| --- | --- | --- |
| Push and open the pull request | `Cmd+Enter` | `Ctrl+Enter` |

## In the terminal

These are not commands: the terminal turns them into the bytes your shell reads, so they edit the line in zsh, bash and most prompts.

| Key | macOS | Linux |
| --- | --- | --- |
| Back one word | `Alt+Left` | `Alt+Left` |
| Forward one word | `Alt+Right` | `Alt+Right` |
| Start of the line | `Cmd+Left` | `Home` |
| End of the line | `Cmd+Right` | `End` |
| Delete the previous word | `Alt+Backspace` | `Alt+Backspace` |
| Delete to the start of the line | `Cmd+Backspace` | `Ctrl+U` |
