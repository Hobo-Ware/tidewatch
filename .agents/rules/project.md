---
trigger: always_on
description: 'Tidewatch overview: what it is, sibling repos, layout, commands, dev-run safety, performance rules, showcase and release upkeep, commit standards.'
---

# Tidewatch - project rules

A native app for macOS and Linux in Rust (egui/eframe 0.35) for running AI coding agents in git worktrees,
one repo at a time. Each tab is a fresh worktree with an agent in it. It reuses Kelp (graph,
diff, review) and stdusk (terminal engine, tabs, quake window) through library crates in
those repos, as git dependencies. The plan and decisions live in `PLAN.md`; progress, with
measured numbers, in `LEDGER.md`. The concept mockup is `docs/concept.html`.

## Sibling repos

- Kelp: `../kelp` (github.com/Hobo-Ware/kelp). Rules: `../kelp/.agents/rules/project.md`.
- stdusk: `../stdusk` (github.com/Hobo-Ware/stdusk). Rules: `../stdusk/.agents/rules/`.

Behavior Tidewatch shares with them (quake window, tabs, updates, settings, review, graph)
matches the sibling unless `PLAN.md` says otherwise. Fix a shared bug in the shared crate,
in the sibling repo, never in a local copy.

## Layout

```
crates/tidewatch-core/  no UI: worktrees, profiles, agents and launch, status and hooks, the
                        listener and MCP tools, progress reports, PR and GitHub, guards,
                        sessions and restore, transcripts, services, keep-awake (pure where it can be)
  tests/                end-to-end runs on throwaway repos, a fake gh, real ptys
crates/tidewatch/       the egui app: `app.rs` and `app/*` the frame steps and window glue,
                        `*_glue.rs` hosts that run jobs, `*_model.rs` / `*_state.rs` pure
                        screen data, `*_view.rs` drawing, `dev_*.rs` the dev switches
docs/                   concept mockup, research, shortcuts.md (generated, checked by a test),
                        layout.md (the module map), dev-switches.md (dev switches, live checks)
site/                   the GitHub Pages showcase (tidewatch.hoboware.dev)
packaging/              Info.plist, reference Homebrew cask, .desktop file, release README
scripts/                bundle-macos.sh, package-linux.sh, make-icon.sh, make-icons.py, make-emoji.py
assets/                 icon, tray icon, mascot art, Lucide icon pack, emoji packs
.github/workflows/      ci.yml (fmt, clippy, test on macOS and Ubuntu), release.yml, pages.yml
```

The full module map, one line per file with what it owns, is `docs/layout.md`. Grep it for
the file you are about to touch. Update its line in the same commit that adds, moves or
changes a module.

## Commands

```
cargo run -p tidewatch -- [path]       # open the git repo containing path (default: cwd)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
scripts/bundle-macos.sh                # universal Tidewatch.app and zip in target/dist, ad-hoc signed; never launch the result from a check
scripts/package-linux.sh               # x86_64 Linux only: tarball and AppImage in target/dist
scripts/make-icon.sh                   # rebuild the icon files from assets/icon.svg and assets/tray.svg (inkscape, pngquant, oxipng)
TIDEWATCH_SCREENSHOT=/tmp/x.png cargo run -p tidewatch -- <repo>   # render, save PNG, quit (saves no session)
cargo run -p tidewatch -- --no-restore <repo>   # start with just the main tab; the last session is not read
tidewatch --toggle                     # show or hide every running window, exit 1 if none answers (the way in on Wayland)
```

Rules for every dev or live-check run (the switches themselves are in `docs/dev-switches.md`):

- Screenshot runs open in the background: no Dock icon, never take keyboard focus from the
  developer's active window. `TIDEWATCH_BACKGROUND=1` does the same for any other dev or
  live-check run. Agents must use one of the two for every window they open.
- **Never live-test quake show/hide, the hotkey or the menu-bar item from an agent run: they
  take focus.** A background run gets none of them by construction (`shell_plan::Plan`).
- Use a short throwaway `TIDEWATCH_HOME` (such as `/tmp/x`, not under macOS `$TMPDIR`: the
  socket path must stay under about 100 bytes), and a temp `HOME` with `ZDOTDIR` when a run
  makes worktrees or lists agent sessions, never the real home. Kill everything you started.
- The real GitHub API must never be reached from a check: use `TIDEWATCH_DEV_GH_DIR` with a
  fake `gh`, never a real PR.
- Never click an update pill or Fix in a copy installed with Homebrew.

The sibling crates are git dependencies pinned by commit (`rev`), not by tag (PLAN.md,
Deviations log). To pick up a sibling change, merge it there, then bump `rev`.

Check the real exit code of a build (`cargo build 2>build.log; echo $?`), never a piped
`tail`. Verify UI changes with the screenshot switch instead of asking for a manual check.

## Performance rules

- Idle CPU stays at 0%: repaint only on input, pty output or finished background work.
  Animations run only while something is happening. The mascot animates only while it is drawn in a focused, visible window and asks for a repaint every 33 ms only then (`mascot_motion::should_animate`); otherwise it shows its still frame and asks for nothing.
- Never block the UI thread on git, the network, disk-heavy work or a child process.
- The graph draws only visible rows; per-row work is O(visible lanes).
- Many agents at once is the normal case: a hidden tab costs no paint time.

## Colors

Colors come from `brand.rs`: accent fills and strokes are `brand::TEAL`, accent text on dark is `brand::TEAL_TEXT`, statuses are `brand::WORKING`, `NEEDS`, `DONE` and `FAILED` (a test keeps them apart in lightness, so color-blind readers tell them apart), errors `brand::RED`, warnings `brand::ORANGE`. Never use `colors::accent()`, `colors::yellow()` or Kelp's `theme::accent()` in Tidewatch code: stdusk's accent is the terminal's ANSI blue and Kelp's is green. Kelp's own widgets keep Kelp's colors (diff hunk header and notes, the focus ring, the Below layout's graph); do not copy them. The side layout's graph takes `brand::TEAL` as its accent through `GraphStyle::embedded`.

## Showcase page and releases

Releases follow `packaging/README.md` (bump `Cargo.toml`, tag `v<version>`). The tap job stays off until the
repository variable `PUBLISH_TAP` is `true`, which waits for the repo to go public.

The showcase page is `site/` (plain HTML and CSS, no build step), deployed by `pages.yml` to `tidewatch.hoboware.dev`
whenever `site/**` changes on `main`. Same upkeep as Kelp: a user-visible change updates `README.md` and the page in the
same branch, removed behavior comes off, and the page only describes merged features. The install section says
"not released yet, build from source" until the first release; then it switches to the Homebrew cask and the Linux
downloads. Screenshots come from the screenshot switch on a throwaway demo repo with neutral names (never a private
or work repo, no real home path or user name in any pixel), at 2560x1600, resized to 2200x1375 and compressed
(`pngquant --force --speed 1 --strip 64`, then `oxipng -o 6 --strip safe`, PSNR above 40 dB). A shell prompt shows
user and host: use a throwaway `HOME`, `ZDOTDIR` with a neutral `PROMPT`, and `GIT_PAGER=cat`. Agent tabs without a
real agent: a stand-in `claude` script first on `PATH` that posts the hook events to the listener from its
`--settings` JSON and a hand-written transcript under the throwaway `HOME`'s `.claude/projects`, restored through a
seeded `session.toml`. The og image is a 1200x630 page rendered in headless Chromium. Check the page headless only, at
1280 and 390 wide, with no horizontal scroll.

## Commit standards

- Conventional Commits: `feat: ...`, `fix: ...`, `docs: ...`, `chore: ...`, optional scope.
- Clean author line: no `Co-Authored-By` trailer, no "Generated with" footer.
- Plain hyphens only in commits and GitHub prose, no em or en dashes.
- Commit or push only when asked. One logical change per commit.

## Quick checklist

- [ ] Read `LEDGER.md` + the current phase in `PLAN.md`; follow `workflow.md`.
- [ ] Tests, clippy and fmt pass.
- [ ] UI change verified with a screenshot.
- [ ] `LEDGER.md` updated; `docs/layout.md` still true.
