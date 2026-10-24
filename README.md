# Tidewatch

A desktop app for macOS and Linux that runs AI coding agents side by side, one git repo per window.
Every tab is its own git worktree (a separate checkout on its own branch) with an agent running in
it, so several agents can work on the same repo without touching each other's files. You watch them,
read what they did, comment on their diffs and decide what ships.

The terminal is on the left and the history of that worktree on the right. Click a commit to open its
files in the tree, then a file to see its diff where the terminal was; the agent keeps running, and
Esc or the Terminal chip takes you back. Built by [HoboWare](https://github.com/Hobo-Ware) from the parts of
[Kelp](https://github.com/Hobo-Ware/kelp) (graph, diff, review) and
[stdusk](https://github.com/Hobo-Ware/stdusk) (terminal, tabs, quake window).

Status: v0.1.0, the first release. Showcase page: [tidewatch.hoboware.dev](https://tidewatch.hoboware.dev).

## What it does

- **Worktree tabs, one agent each.** A new tab is a form in the tab itself: pick an agent, a workflow or a plain shell, name the worktree and the branch it starts from, write the first message, and Start makes the worktree and starts Claude Code, Codex, Gemini CLI or Copilot CLI in it. Profiles set the agent, model and permission level, from read only up to a YOLO level that asks first. Agent tabs block `git push`; Tidewatch pushes for you on approval.
- **Remote control.** A switch in the new tab form (off by default) starts Claude Code with `--remote-control` or Copilot CLI with `--remote`, so the session can be driven from the Claude app or GitHub mobile. A running tab shows a Remote pill with the session name, Copy link (once the CLI has printed one) and Turn off, which applies the next time the agent starts. Codex and Gemini CLI have no per-session flag, so the row is disabled for them.
- **New tab defaults.** Settings, General, New tabs sets what the form opens with: the agent, the permission level (YOLO too, where it is allowed; in Other the form opens at Auto and says so), remote control, whether to start from the current or the default branch, and which side of the right card shows first. They live in the `[new_tab]` table of `settings.toml`, and every tab can still change them.
- **Worktree setup.** Settings, Worktree setup holds a small recipe per repo (in `~/.config/tidewatch/worktree-setup.toml`, never in the repo): gitignored files to copy from the main checkout (`.env` files are offered with one click), folders to link (such as `node_modules`; nothing is ever installed through a link), sharing the main checkout's `target` folder with every Rust worktree, and a generator command that runs in the new worktree and shows as a job. A missing file is a note, never a failed tab. When a tab's pull request merges, its card checks that nothing would be lost (no uncommitted changes, no untracked files, no commits the pull request lacks), lists ignored files that would go, and offers Remove worktree and branch.
- **Reader view.** An agent tab shows the agent's answer as readable text on top of the live terminal. One shortcut flips to the raw terminal.
- **Notes on diff lines.** Leave a note on a line of the diff, then send the notes to the agent as one message.
- **Meeseeks.** The new-tab form's Meeseeks kind takes one task per line and opens a tab, with its own worktree, for each. A Meeseeks has one job: when it says it is done, Tidewatch checks the work (the tree is clean with a commit, the repo's tests pass, and the pull request opens when you ticked it), then ends the agent, removes the worktree folder and keeps the branch. If it fails three checks, runs 40 turns or 60 minutes, the tab turns orange and waits for you: Take over or Give up. What poofed today is listed on the Overview.
- **Done check.** When an agent says it is done, Tidewatch looks at what is really there: the work is committed, the branch is pushed, the pull request is open or merged with its checks passing on the pushed commit, and the agent's ledger lists nothing unverified. A card on the tab says Done, Blocked (each failed check in one line, with Ask agent to fix) or Not verified. A tab with no pull request makes no GitHub call. Settings, General, Done check turns it off.
- **Workflows.** Chain stages, each with its own agent profile: Nine come built in: Ship a feature, Fix a bug and Review a PR, plus six habits that keep work honest (Hunt a root cause, Finish with proof, Change the UI with proof, Port to a platform, Prepare a release, Refactor safely). None of them pushes or tags; each ends at your approval, and the delivery step does the push. There is an editor for your own, and a `workflows.toml` from an older version gains the new ones once (a built-in you delete stays deleted). Gates (tests pass, no blocking findings, your approval) can send the work back a stage before it hands off. The palette's "Build workflow from this session" asks an idle agent to turn what it did into a draft workflow, which opens unsaved in Settings, Workflows.
- **Delivery.** When the work is ready, Tidewatch shows the commits and the PR text. When you approve, it pushes and opens the pull request with `gh`.
- **Overview and focus mode.** Overview lists every tab with its state: working, needs you, done or failed. Focus mode hides the history so the terminal gets the whole window.
- **Session restore.** Quitting saves your tabs. The next launch brings them back and resumes the agents where the CLI can. `--no-restore` or a setting turns it off. `resume_agents` in `settings.toml` (Settings, General, Startup) is `auto` (run the resume command), `prefill` (type it and leave Enter to you) or `off` (a plain shell); after a crash or a kill of Tidewatch or its agent the command is always typed and never run. A YOLO tab resumes with the same permissions it ran at, but its command is always typed and waits for Enter, whatever the setting (`off` still gives a plain shell). On macOS, Settings, General, Startup also has Open at login, which asks the system and keeps no setting of its own.
- **Folder trust.** On first launch Tidewatch asks once before it marks its worktree folder as trusted in the agent CLIs' own config files, so they stop asking in every new worktree. It changes nothing without a yes, and Settings can undo it.
- **House style.** One file, `~/.config/tidewatch/house-style.md`, tells every agent how to write its answers (summary first, file references, a Done / Needs you / Next block). Edit it freely: Tidewatch never overwrites it. Settings, House style opens it in your editor and can put the default text back.
- **Quake window, palette, updates.** A drop-down window on a hotkey, a command palette, and update checks; see below.

## Quake window

Tidewatch drops from the top of the screen on `Ctrl+Alt+Grave` (stdusk keeps `Ctrl+Grave`, so both can run) and hides when you click elsewhere. There is a menu-bar item with Show / Hide, Overview, Settings and Quit. Settings, General, Window turns the drop-down off for a plain window, records a new hotkey (click the key caps and press the keys) and moves the panels: History beside the terminal (the default) or History and the diff below it (`panels = "below"` in `[window]`). `tidewatch <path>` for a repo that already has a window brings that window forward instead of opening another. `tidewatch --toggle` shows or hides the running windows: on Wayland, which has no global hotkeys, bind it to a desktop shortcut. On Linux the plain window draws its own frame: the tab strip is the title bar (drag it to move the window, double-click it to maximize), the window buttons follow your GNOME button layout, and the edges resize.

Settings, General, Terminal picks the terminal's font: Built-in, the default, or any installed family from a search (`font` in `[terminal]`, empty for the built-in one). It applies at once and changes only the terminal; the rest of the window keeps its own fonts. Pick a Nerd Font there if your prompt theme draws icons.

## Keyboard

`Cmd+Shift+P` (`Ctrl+Shift+P` on Linux) opens the command palette: type to filter every command, the other open tabs, each agent profile and each workflow, Enter runs, Esc closes. Settings, Shortcuts, lists every chord for the platform you run on. [docs/shortcuts.md](docs/shortcuts.md) has both columns; a test keeps it equal to the command table in the code. On Linux a plain Ctrl chord would reach the shell, so it uses Ctrl+Shift or Alt where macOS uses Cmd.

## Updates

Tidewatch checks GitHub for a new release about once an hour and when its window comes back into focus. On macOS, installed from the Homebrew cask, it runs `brew upgrade --cask` in the background and the pill in the top bar asks for a restart. Everywhere else (Linux, a macOS copy that was not installed with Homebrew) it only says "Tidewatch X is out" and the pill opens the release page. Settings, General, Updates has Check now and the two switches (`[updates]` in `settings.toml`: `check`, `install`). A check that fails on its own is silent; Check now shows why. After an install, Restart starts the new version and hands it every terminal: shells and agents keep running where they were (no resume), and the new window picks up the same tabs and repos. If the hand-over cannot complete, the old window stays open and says why; session restore still covers a quit or a crash.

Moving tabs from stdusk: its "Move tabs to Tidewatch" hands its live panes over, and each becomes a shell tab here, in the group of the git repo it was in, still running.

Agent sessions running in other terminals: the Overview lists Claude Code, Codex, Gemini and Copilot sessions it finds (Running elsewhere). A process cannot be pulled out of another terminal, so Move here ends it once it is idle and resumes the same conversation in a tab with the same flags; one in stdusk moves live through stdusk's own button. Also in the palette: Move sessions to Tidewatch.

## Install

- macOS: `brew install hobo-ware/tap/tidewatch`
- Linux: the `.tar.gz` or the `.AppImage` from the [latest release](https://github.com/Hobo-Ware/tidewatch/releases/latest)

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
