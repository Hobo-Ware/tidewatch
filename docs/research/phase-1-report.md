# Phase 1 report: stdusk workspace and library crates

Worktree: `/Users/vladjerca/Git/Hoboware/stdusk/.claude/worktrees/phase-1-lib-crates`, branch `phase-1-lib-crates` (from main 1.9.1). Nothing pushed.

## Gate

| Check | Result |
|---|---|
| Baseline `cargo test` (before changes) | 610 passed, 0 failed, 4 ignored |
| Final `cargo test --workspace` | 610 passed (243 app + 315 core + 52 egui), 0 failed, 4 ignored |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo build --release`, `target/release/stdusk` | built, 23.4 MB |
| `cargo run -- --screenshot` PSNR vs main | NOT run (unverified) |

## Commits (local, no trailers)

1. `a54674d5` make the repo a Cargo workspace with empty stdusk-core and stdusk-egui crates
2. `2157c7b3` move the terminal engine to stdusk-core and egui rendering to stdusk-egui
3. `245bba4f` narrow library visibility, share deps and lints through the workspace
4. `5330c882` run CI and docs against the workspace (native.yml `--workspace`, `crates/**` path filter, rules/CONTRIBUTING commands)
5. `fe1d7962` expose pos_to_cell and theme lookup for other hosts

## Layout

- Root `Cargo.toml`: `[workspace]` members `.`, `crates/stdusk-core`, `crates/stdusk-egui`; `[workspace.lints]` (added `must_use_candidate = "allow"`); `[workspace.dependencies]` for deps shared by 2+ crates. Every crate has `[lints] workspace = true`.
- App stays at the root, binary still `stdusk`; release workflow untouched (`cargo build --release` at the root builds the app only).
- `stdusk-core` modules: `terminal`, `osc`, `progress`, `modes`, `screen`, `search`, `links`, `shell`, `pane`, `mouse` (pure SGR part), `colors`, `themes`, `procwatch`, `agents`, `agent_track`, `agent_pane`, `agent_restore`, `agent_codex`, `agent_codex_scan`, `session` (only `SavedAgent`), `repaint`, `test_support` (feature `test-support`). It owns `build.rs`, `assets/schemes`, `assets/zsh-autosuggestions.zsh`.
- `stdusk-egui` modules: `ui` (render_grid, collect_input, draw_tab, GridStyle, pos_to_cell, toasts, overlays, CLI badges, apply_theme), `fonts`, `widgets`, `keys` (key_to_bytes, ctrl_letter, alt_scroll_bytes), `mouse`. It owns `assets/{Phosphor.ttf,NotoEmoji-Regular.ttf,icons/}`. Re-exports `colors` and `Repaint`.
- Public items: about 190 `pub fn` plus types and fields; 155 items stayed `pub(crate)`. The visibility was narrowed by compiler errors, so `pub` means "the app or the other crate really uses it", plus `pos_to_cell` and `themes::lookup` for Tidewatch.

## Public API Tidewatch needs

stdusk-core:

```rust
// repaint
pub struct Repaint;                     // Clone
impl Repaint { pub fn new(f: impl Fn(Duration) + Send + Sync + 'static) -> Self;
               pub fn none() -> Self; pub fn now(&self); pub fn after(&self, d: Duration); }

// shell
pub struct ShellProfile { pub shell: Option<String>, pub args: Vec<String>,
                          pub cwd: Option<String>, pub env: BTreeMap<String, String> }

// terminal
pub struct SpawnOpts { pub detect_progress: bool, pub shell_integration: bool, pub autosuggestions: bool,
    pub scrollback_lines: usize, pub word_separators: String, pub bold_bright: bool,
    pub cwd: Option<String>, pub profile: Option<ShellProfile>, pub agent_tracking: Option<AgentSetup> }
PtyTerm::spawn(cols: usize, rows: usize, repaint: Repaint, opts: &SpawnOpts) -> PtyTerm
PtyTerm::adopt(repaint, handover: Adopted, opts) -> io::Result<PtyTerm>
PtyTerm::{send(&mut self, &[u8]), resize(&mut self, cols, rows), scroll(&self, i32), grid_snapshot(&self) -> GridSnap,
          cwd() -> Option<String>, title_osc(), shell_pid() -> Option<u32>, progress() -> Progress,
          cmd_state() -> CmdState, exited() -> Option<ExitInfo>, app_cursor() -> bool,
          mouse_reporting() -> MouseReporting, selection_text() -> Option<String>}
pub struct GridSnap { pub cols: usize, pub rows: usize, pub cells: Vec<CellSnap>,
                      pub cursor: Option<(usize, usize)>, pub top_line: i32 }
pub struct CellSnap { pub c: char, pub fg: Color32, pub bg: Option<Color32>, pub selected: bool,
                      pub wide: bool, pub bold: bool, pub dim: bool }

// colors (process-wide theme, D2)
colors::init(Theme); colors::set(Theme); colors::theme() -> Theme
themes::lookup(name: &str) -> Option<Theme>; themes::all_schemes() -> &'static [(String, Theme)]

// procwatch
ProcScanner::spawn(repaint: Repaint, enabled: bool) -> ProcScanner
```

stdusk-egui:

```rust
repaint_handle(ctx: &egui::Context) -> Repaint
ui::render_grid(ui: &mut Ui, id_src: &[pane::Side], rect: Rect, term: &PtyTerm, snap: &GridSnap,
                cw: f32, ch: f32, font: &FontId, bold_font: Option<&FontId>, style: GridStyle,
                search_marks: &[search::Match]) -> Response
ui::GridStyle { cursor: CursorStyle, dimmed, link_active, blink, ligatures: bool, min_contrast: f32, app_mouse: bool }
ui::collect_input(ui: &Ui, alt_is_meta: bool, intercept_ctrl_c: bool, app_cursor: bool) -> Vec<u8>
ui::pos_to_cell(rel_x: f32, rel_y: f32, cw: f32, ch: f32, cols: usize, rows: usize, top_line: i32) -> (i32, usize, bool)
ui::draw_tab(ui: &mut Ui, idx: Option<usize>, tab_id: u64, title: &str, active: bool, pinned: bool,
             color: Option<Color32>, progress: Progress, cmd: CmdState, layout: &[Rect],
             cli: Option<procwatch::Cli>, width: Option<f32>) -> (Response, bool /*close clicked*/)
ui::apply_theme(ctx: &Context)            // call after colors::init/set
ui::{draw_toast, draw_pane_notice, draw_exit_overlay, TAB_H, TAB_FIXED_W}
fonts::resolve_font(family) -> Option<ResolvedFont>; fonts::resolve_bold_font(family) -> Option<ResolvedFont>
fonts::build_fonts(custom: Option<ResolvedFont>, bold: Option<ResolvedFont>) -> FontDefinitions
keys::key_to_bytes(key: Key, mods: Modifiers, alt_is_meta: bool, app_cursor: bool) -> Option<Vec<u8>>
mouse::{MouseReporting, MouseReportingExt::app_owns_pointer, pointer_reports, WheelAccumulator, GridGeom, PointerTracker}
```

Minimal Tidewatch pane: `colors::init(theme)` -> `ctx.set_fonts(build_fonts(..))` + `apply_theme(ctx)` -> `PtyTerm::spawn(cols, rows, repaint_handle(ctx), &opts)` -> per frame: `collect_input` then `term.send`, `grid_snapshot`, `render_grid`; tab strip with `draw_tab`.

## Deviations from the plan text

- **`Repaint` is `Fn(Duration)`, not `Fn()`**. The reader thread coalesces bursts with `request_repaint_after(4 ms)`. A zero-argument callback would change repaint timing. `Repaint::now()` gives the plain form.
- **Core depends on `emath` and `ecolor`** (the math and color crates egui re-exports, no egui or eframe). `Pane` layout uses `Rect` and the snapshot uses `Color32`, so the types stay identical to egui's and no conversion layer exists.
- **`colors` and `themes` live in core**, re-exported from `stdusk_egui::colors`. `PtyTerm::grid_snapshot` resolves cell colors through the global theme, so colors cannot sit above core. The `colors -> settings` import is gone (`PREVIEW_MIN_CONTRAST` moved into `colors`).
- `session` in core holds only `SavedAgent`. The session file model, load/save and writer stay in the app.
- `mouse` is split: SGR encoders and `MouseReporting` in core; egui event handling in egui. `app_owns_pointer` became an extension trait method.
- `keys` is split: `key_to_bytes` family in egui; the hotkey parser and dispatch stay in the app.
- The app still imports core modules through a root `use stdusk_core::{...}` line in `main.rs`, so existing `crate::colors` paths resolve. Other `crate::x` paths were rewritten to `stdusk_core::x`.
- The tab-bar and hotkey frame tests (they share one harness with the hotkey code) stay in the app's `ui.rs`; pure render/helper tests moved to egui. One terminal test that drives `collect_input` moved to `stdusk-egui`.

## Deferred

- `AppPaths` (config dir, socket path): `~/.config/stdusk` is still hard-coded in `session.rs`, `config.rs`, shell fixtures and `instance.rs`. Tidewatch needs it in Phase 3.
- Handoff wire protocol and `SavedSession` stay in the app (coupled to `Stdusk` and `update::bundle_path`).
- Quake window helpers and `macos.rs` glue stay in the app, as instructed (`stdusk-egui::platform` not created).
- `ui.rs` app helpers (quit messages, tab width modes, quake geometry) remain in the app's `ui.rs`.
- No `//!`-level API list beyond each `lib.rs` header; no per-item docs added.
- Screenshot PSNR check against main and the `stdusk-v1.10.0` tag were not done.
- `cargo test` gating in `native-release.yml` unchanged (the release workflow does not run tests).
