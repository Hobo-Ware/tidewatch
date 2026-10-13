# Tidewatch

A desktop app for macOS and Linux that runs AI coding agents side by side, one git repo per window.
Every tab is its own git worktree (a separate checkout on its own branch) with an agent running in
it, so several agents can work on the same repo without touching each other's files. You watch them,
read what they did, comment on their diffs and decide what ships.

The terminal sits on top. Below it are the history of that worktree and the diff of the selected
commit. Built by [HoboWare](https://github.com/Hobo-Ware) from the parts of
[Kelp](https://github.com/Hobo-Ware/kelp) (graph, diff, review) and
[stdusk](https://github.com/Hobo-Ware/stdusk) (terminal, tabs, quake window).

Status: in development. Nothing is released yet, so build from source to try it.

## What it does

- **Worktree tabs, one agent each.** A new tab makes a worktree and starts Claude Code, Codex, Gemini CLI or Copilot CLI in it. Profiles set the agent, model and permission level, from read only up to a YOLO level that asks first. Agent tabs block `git push`; Tidewatch pushes for you on approval.
- **Reader view.** An agent tab shows the agent's answer as readable text on top of the live terminal. One shortcut flips to the raw terminal.
- **Notes on diff lines.** Leave a note on a line of the diff, then send the notes to the agent as one message.
- **Workflows.** Chain stages, each with its own agent profile: Ship a feature, Fix a bug and Review a PR come built in, and there is an editor for your own. Gates (tests pass, no blocking findings, your approval) can send the work back a stage before it hands off.
- **Delivery.** When the work is ready, Tidewatch shows the commits and the PR text. When you approve, it pushes and opens the pull request with `gh`.
- **Overview and focus mode.** Overview lists every tab with its state: working, needs you, done or failed. Focus mode hides the history and diff so the terminal gets the whole window.
- **Session restore.** Quitting saves your tabs. The next launch brings them back and resumes the agents where the CLI can. `--no-restore` or a setting turns it off.
- **Folder trust.** On first launch Tidewatch asks once before it marks its worktree folder as trusted in the agent CLIs' own config files, so they stop asking in every new worktree. It changes nothing without a yes, and Settings can undo it.
- **House style.** One file, `~/.config/tidewatch/house-style.md`, tells every agent how to write its answers (summary first, file references, a Done / Needs you / Next block). Edit it freely: Tidewatch never overwrites it.
- **Quake window, palette, updates.** A drop-down window on a hotkey, a command palette, and update checks; see below.

## Quake window

Tidewatch drops from the top of the screen on `Ctrl+Alt+Grave` (stdusk keeps `Ctrl+Grave`, so both can run) and hides when you click elsewhere. There is a menu-bar item with Show / Hide, Overview, Settings and Quit. Settings, General, Window turns the drop-down off for a plain window and changes the hotkey. `tidewatch <path>` for a repo that already has a window brings that window forward instead of opening another. `tidewatch --toggle` shows or hides the running windows: on Wayland, which has no global hotkeys, bind it to a desktop shortcut.

## Keyboard

`Cmd+Shift+P` (`Ctrl+Shift+P` on Linux) opens the command palette: type to filter every command, the other open tabs, each agent profile and each workflow, Enter runs, Esc closes. Settings, Shortcuts, lists every chord for the platform you run on. [docs/shortcuts.md](docs/shortcuts.md) has both columns; a test keeps it equal to the command table in the code. On Linux a plain Ctrl chord would reach the shell, so it uses Ctrl+Shift or Alt where macOS uses Cmd.

## Updates

Tidewatch checks GitHub for a new release about once an hour and when its window comes back into focus. On macOS, installed from the Homebrew cask, it runs `brew upgrade --cask` in the background and the pill in the top bar asks for a restart. Everywhere else (Linux, a macOS copy that was not installed with Homebrew) it only says "Tidewatch X is out" and the pill opens the release page. Settings, General, Updates has Check now and the two switches (`[updates]` in `settings.toml`: `check`, `install`). A check that fails on its own is silent; Check now shows why. After an install, Restart opens the folder you used last, and the tabs and agents come back through session restore.

## Install

- macOS (after the first release): `brew install hobo-ware/tap/tidewatch`
- Linux (after the first release): the `.tar.gz` or the `.AppImage` from the GitHub Release

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
