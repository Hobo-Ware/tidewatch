# Tidewatch

A native macOS app for running AI coding agents, one repo at a time. Each tab
is a fresh git worktree with an agent in it. The terminal sits on top, the
history of that worktree below it, and the diff of the selected commit on the
right. Built by [HoboWare](https://github.com/Hobo-Ware) from the parts of
[Kelp](https://github.com/Hobo-Ware/kelp) (graph, diff, review) and
[stdusk](https://github.com/Hobo-Ware/stdusk) (terminal, tabs, quake window).

Status: in development, private. The release is packaged but nothing is published yet.

## Quake window

Tidewatch drops from the top of the screen on `Ctrl+Alt+Grave` (stdusk keeps `Ctrl+Grave`, so both can run) and hides when you click elsewhere. There is a menu-bar item with Show / Hide, Overview, Settings and Quit. Settings, General, Window turns the drop-down off for a plain window and changes the hotkey. `tidewatch <path>` for a repo that already has a window brings that window forward instead of opening another. `tidewatch --toggle` shows or hides the running windows: on Wayland, which has no global hotkeys, bind it to a desktop shortcut.

## Keyboard

`Cmd+Shift+P` (`Ctrl+Shift+P` on Linux) opens the command palette: type to filter every command, the other open tabs, each agent profile and each workflow, Enter runs, Esc closes. Settings, Shortcuts, lists every chord for the platform you run on. [docs/shortcuts.md](docs/shortcuts.md) has both columns; a test keeps it equal to the command table in the code. On Linux a plain Ctrl chord would reach the shell, so it uses Ctrl+Shift or Alt where macOS uses Cmd.

## Updates

Tidewatch checks GitHub for a new release about once an hour and when its window comes back into focus. On macOS, installed from the Homebrew cask, it runs `brew upgrade --cask` in the background and the pill in the top bar asks for a restart. Everywhere else (Linux, a macOS copy that was not installed with Homebrew) it only says "Tidewatch X is out" and the pill opens the release page. Settings, General, Updates has Check now and the two switches (`[updates]` in `settings.toml`: `check`, `install`). A check that fails on its own is silent; Check now shows why.

## Install

- macOS (once public): `brew install hobo-ware/tap/tidewatch`
- Linux (once public): the `.tar.gz` or the `.AppImage` from the GitHub Release

## Build from source

```sh
cargo run -p tidewatch -- [path]      # run
scripts/bundle-macos.sh               # universal Tidewatch.app in target/dist
scripts/package-linux.sh              # x86_64 Linux tarball and AppImage in target/dist
```

Release steps: [packaging/README.md](packaging/README.md).

- Concept and mockup: [docs/concept.html](docs/concept.html)
- Plan: [PLAN.md](PLAN.md)
- Progress: [LEDGER.md](LEDGER.md)
