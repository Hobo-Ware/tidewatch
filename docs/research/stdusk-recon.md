# stdusk recon (read-only), for lib extraction

## 1. Cargo.toml
- package `stdusk` 1.9.0, edition 2024. NO [workspace]. Single bin crate; no [lib], no [features] table.
- `__screenshot` is NOT a feature: it is hard-enabled on eframe: `eframe = { version = "0.35.0", features = ["glow", "__screenshot"] }`.
- deps: alacritty_terminal 0.26.0, anyhow 1.0.104, arboard 3.6.1 (image-data), base64 0.22.1, block2 0.6, eframe 0.35.0, egui 0.35.0, font-kit 0.14.3, global-hotkey 0.8.0, libc 0.2, objc2 0.6, objc2-app-kit 0.3, objc2-service-management 0.3.2, objc2-foundation 0.3, portable-pty 0.9.0, regex 1.13.0, resvg 0.45, rustix 1.1.4 (net, termios), serde 1.0.228, serde_json 1.0.151, sysinfo 0.38.4, toml 1.1.3, tray-icon 0.24.1, ureq 3.4.2, winit 0.30. dev: proptest 1.8.0.
- [lints.rust] `unsafe_code = "deny"` (justified local allows: main set_var, terminal killpg). [lints.clippy] all=deny, pedantic=warn + allow-list (cast_*, module_name_repetitions, missing_errors_doc, too_many_arguments "render_grid threads ui+rect+term+metrics", too_many_lines, etc.). A lib crate would need these lints copied or moved to [workspace.lints].
- Repo root also has build.rs, packaging/, assets/, site/, rustfmt.toml, many md files (PLAN, V1, FUTURE, LEDGER...).

## 2. Modules (lines; crate deps from `use crate::` + inline `crate::` paths)
main.rs 1774 (Stdusk struct, `mod` list main.rs:12-51, `fn main` :1576)
- terminal 3541: deps agent_pane, agent_restore, agent_track, agents, colors, config(Profile, expand_tilde), handoff, links, modes, mouse, osc, procwatch, progress, screen, search, session(SavedAgent), shell, ui(collect_input, pane_notice_text, draw_pane_notice - inside #[cfg(test)] at :3394 only? lines 3394-3397 are in tests region), test_support (tests). Types: CmdState, ExitInfo, OnExit, CellSnap, GridSnap, TabState, SpawnOpts, PtyTerm, Adopted. `PtyTerm::spawn(cols, rows, ctx: egui::Context, opts:&SpawnOpts)` :702, `adopt(...)` :824, `grid_snapshot()->GridSnap` :1289, `send/paste/resize/scroll/selection_*`, `reap_orphaned_session(leader)` :1591.
  SpawnOpts :253 = {detect_progress, shell_integration, autosuggestions, scrollback_lines, word_separators, bold_bright, cwd, profile: Option<config::Profile>, agent_tracking: Option<AgentSetup>}
- ui 2565: deps agent_restore, agents, colors, config(Hotkeys), keys, links, pane, procwatch(Cli), progress, search, shell, terminal. Contains MANY app-specific pure fns (quake geometry, quit confirm text, tab width, right-click) next to the reusable render code. Reusable: `render_grid` :1086, `collect_input` :1033, `draw_tab` :851, `draw_toast` :644, `draw_exit_overlay` :704, `draw_pane_notice` :685, `apply_theme` :596, `tint` :587, `GridStyle` :1076, `pos_to_cell` :117, `ligature_spans`, `normalize_paste`, `ellipsize`.
- pane 578: self-contained generic `Pane<T>` split tree (egui::Rect only). `Pane<T>::{leaf,split,close,layout,splitters,...}`, `Side/SplitDir/Dir`, `GUTTER`.
- procwatch 1340: deps only agent_codex, agent_codex_scan, agents. egui::Context used by `ProcScanner::spawn(ctx, enabled)` :493 (repaint). sysinfo-based `snapshot`, `detect`, `Cli` (label/color Color32), `process_cwd(pid)`, `process_alive`, `pty_victims(leader)`.
- osc 424: only progress. `OscScanner::{new,feed}->Vec<OscEvent>`, `hook_encode`. Pure.
- progress 134: pure (regex LazyLock static). `ProgressScanner`, `Progress`.
- modes 151: pure. `ThemeReportScanner`, `theme_report(dark)`.
- screen 341: pure. `Cell`, `encode(lines, rows, cursor)->Vec<u8>`; MAX_DUMP_BYTES (screen replay for handoff).
- keys 1123: egui keys, deps config::Hotkeys + pane::SplitDir. `key_to_bytes` :11 (pure core: egui::Key -> bytes), but `hotkey_action/matched_hotkeys/consume_hotkey_events` :407-476 are stdusk-specific (Hotkeys, HotkeyAction).
- mouse 445: egui events only. `wheel_sgr`, `sgr_mouse`, `pointer_reports`, `MouseReporting` (alacritty-agnostic). Self-contained.
- links 332: pure + regex OnceLock x3 (stateless). `find_in_row`, `link_at`, `open` (spawns `open`).
- search 172: pure. `find_matches`, `visible_matches`, `Match`.
- shell 669: deps osc(hook), terminal (SpawnOpts?), test_support. `configure(...)` builds shell integration (zsh/bash rc files, OSC 133/7 hooks), `write_files(dir, autosuggest)`, `single_quote`.
- colors 754: deps themes, settings(!). GLOBAL `static THEME: LazyLock<RwLock<Theme>>` :19, `init/set/theme()`, then ~30 free fns reading the global (`bg() fg() accent() elevated() titlebar() border()...`). Also `to_color32_in(&Theme,..)` / `cell_fg_in(&Theme,..)` explicit-theme variants exist :41,:51. Theme comes from alacritty? (struct `Theme` defined in colors.rs; themes.rs has `all_schemes()` LazyLock).
- themes 225: deps colors::Theme. `static ALL: LazyLock` builtin scheme list, `lookup(name)`.
- fonts 303: no crate deps. font-kit `resolve_font`, `build_fonts(...)->egui FontDefinitions`, `installed_families()` OnceLock cache. Self-contained, reusable.
- agents 391, agent_track 708, agent_pane 203, agent_restore 589, agent_codex 543, agent_codex_scan 1098: agent subsystem; only dependency out of the cluster is `session::SavedAgent` (agents, agent_track, agent_pane, agent_restore all import it). agent_track has an OnceLock<Instant> in tests. Pure-ish (std fs + serde), no egui.
- session 710: serde types `SavedSession/SavedTab/SavedPane/SavedAgent/WindowGeom`, `load()/save()` to ~/.config/stdusk/session.toml, global `static WRITER: OnceLock<SaveWriter>` :303 (background writer thread). deps agents, config::lenient (serde helper), pane, procwatch; hardcodes stdusk path. `SavedSession.window` = app-specific.
- repo 622: deps progress, terminal::CmdState, ui, `crate::{Stdusk, colors, tabs}`. Pure grouping fns top (`repo_root`, `Group`, `order`, `roll_up`) + `impl Stdusk` (:212+). APP-COUPLED.
- workspace 642: `Stdusk` + tabs + ui render_grid + widgets. APP GLUE (`central_panel` :115).
- tabs 2138: `Stdusk`, config, Tab struct (`pane::Pane<PtyTerm>`), `spawn_tab(cfg, ctx, cwd)`, `spawn_opts(cfg: &Config, cwd)`. APP GLUE (but Tab/Tab-bar code candidates).
- widgets 529: deps colors (global theme) + ui::icons. egui widgets: color_swatch, text_field, num_field, slider, action_button, toggle_switch, chip, icon_button, icon_toggle, menu_item, style_menu, overlay_frame. Reusable, reads global colors.
- Other modules not on your list: config 964 (Config/Profile/Hotkeys/Terminal/Appearance/Quake; `lenient`, `expand_tilde`), settings 3135 (Stdusk impl), finder 255 + palette 452 (Stdusk impl), handoff 1138, instance 228 (single-instance socket), macos 511, sync 183, tray 75, update 260, updater 224, test_support 95.
- Modules importing `crate::Stdusk`: repo, workspace, tabs, finder, palette, settings (+ handoff `impl crate::Stdusk` :489 `hand_off`). `COLS`/`ROWS` consts live in main.rs (used by tabs, workspace, palette).

## 3. Classification
(a) PURE CORE, no egui in logic: osc, progress, modes, screen, search, links, shell, agents, agent_codex, agent_codex_scan, agent_track, agent_restore, session (types), repo (top half), procwatch (uses egui only for Context repaint + Color32 in Cli::color).
  - Caveat: agent_pane + terminal + keys + mouse + pane use egui types (Rect, Key, Context). pane only needs Rect/Pos2 (could be egui-lite).
(a/b) terminal.rs: THE central piece. Uses egui::Context (repaint) and ui:: fns; spawn takes `&SpawnOpts` whose `profile: config::Profile` and `agent_tracking` tie it to config + agent_pane. Needs: Profile (or a small `ShellProfile` struct) moved into the lib.
(b) egui render/widgets: ui.rs render_grid/collect_input/draw_tab/draw_toast/draw_exit_overlay, widgets.rs, fonts.rs, keys.rs::key_to_bytes, mouse.rs, pane.rs, colors.rs (but global).
(c) APP-SPECIFIC: main.rs, tabs.rs, workspace.rs, settings.rs, finder.rs, palette.rs, config.rs, macos.rs, tray.rs, instance.rs, sync.rs, update.rs, updater.rs, repo.rs impl, handoff.rs `hand_off` (the session-snapshot half), ui.rs quake/quit/tab-width helpers, keys.rs Hotkeys part.

Global state that leaks across a lib boundary:
- colors.rs:19 `static THEME: LazyLock<RwLock<Theme>>` + `init/set/theme()`; ~30 accessors; widgets and ui read it implicitly. Tidewatch would share the one global (can't theme independently). `_in(&Theme,..)` variants exist for cell color. Also colors.rs imports `crate::settings` (cycle-ish: lib -> app!) - check.
- themes.rs `static ALL: LazyLock<Vec<(String,Theme)>>` (read-only, fine).
- fonts.rs `FAMILIES: OnceLock` (cache, fine). links.rs 3 regex OnceLocks (fine). progress.rs PCT LazyLock (fine).
- session.rs:303 `WRITER: OnceLock<SaveWriter>` (global background writer; path hard-coded to stdusk config dir) - would collide/leak.
- handoff.rs:83 `static GAVE_SOCKET: AtomicBool` (process-global, read by main on exit).
- tabs.rs:24 `static NEXT_TAB_ID: AtomicU64` (app).
- ui.rs / settings.rs:1830 `SHOWN: OnceLock<bool>` (app).
- paths: ~/.config/stdusk/... hardcoded in session/config/shell(write_files dir)/instance socket; `update::RUNNING = env!("CARGO_PKG_VERSION")` bakes the CRATE's version (would be the lib's if moved).

## 4. Grid rendering
- `ui::render_grid` ui.rs:1086:
  `fn render_grid(ui:&mut egui::Ui, id_src:&[pane::Side], rect:egui::Rect, term:&PtyTerm, snap:&GridSnap, cw:f32, ch:f32, font:&egui::FontId, bold_font:Option<&egui::FontId>, style:GridStyle, search_marks:&[search::Match]) -> egui::Response`
  `GridStyle {cursor:CursorStyle, dimmed, link_active, blink, ligatures, min_contrast:f32, app_mouse}` :1076
- Input snapshot: `term.grid_snapshot() -> GridSnap {cols,rows,cells:Vec<CellSnap>,cursor:Option<(r,c)>,top_line}` terminal.rs:145/1289. Colors via colors:: global theme (cell_fg_in/to_color32_in/bg()).
- Input path: `ui::collect_input(ui, ..3 bools) ` ui.rs:1033 -> bytes via keys::key_to_bytes; mouse via mouse.rs; `pos_to_cell` ui.rs:117 maps pointer->cell. Callers: workspace.rs central_panel (uses COLS/ROWS, tabs::spawn_opts).
- `ui::draw_tab` ui.rs:851 depends on `procwatch::Cli` badges + resvg SVGs (`cli_icon_svg` :753) -> agent-specific.
- Fonts: cw/ch measured by caller from egui fonts; `fonts::build_fonts` registers family + "term-bold".

## 5. Handoff + agent persistence
Handoff = src/handoff.rs (1138 lines), terminal.rs `adopt`/`handoff_fd`/`mark_handed_off`, main.rs `--adopt SOCK` (:1618-1623; adoption block main.rs:208), `Stdusk::hand_off` handoff.rs:498.
- Mechanism: unix socket + SCM_RIGHTS via `rustix` (safe API, no unsafe). Predecessor binds SOCK, runs `open -n -a <bundle> --args --adopt SOCK`; successor connects. Header = TOML {version, panes, session}; then one message per pane in LEAF ORDER: u32 BE length + JSON `PaneMeta` + exactly ONE fd (the pty MASTER). Successor `PtyTerm::adopt` each, replies ACK "adopted" only when all LIVE; predecessor then `mark_handed_off` + closes. Any failure before ACK = no change. PROTOCOL=2, MAX_META 512K, MAX_PANES 512, IO timeout 10s, LAUNCH_TIMEOUT 12s.
- PaneMeta carries the base64 screen dump (screen::encode) so history is replayed; redraw retry loop with `^L` guarded by `allow_ctrl_l` (terminal.rs ~280).
- Hard couplings: handoff.rs needs `session::SavedSession` (app session model), `update::bundle_path` (.app bundle), `crate::Stdusk` (impl hand_off), `config::Profile`, `procwatch::process_cwd`, `terminal::PtyTerm/SpawnOpts/CmdState`. macOS/unix only.
Agent capture/restore (Claude Code + Codex):
- Sources (no hooks, no shell wrapper): Claude registry (`~/.claude` sessions registry parsed by `agents::parse_claude_registry` :129) and Codex rollout files (`agent_codex_scan` reads `codex_home()` rollouts; `agent_codex::judge` matches thread<->process).
- `procwatch::ProcScanner` background thread (sysinfo snapshot + `read_agent_files`) -> `Scan` -> `pane_seen(scan, shell_root, foreground)->Seen` -> `PtyTerm::step_agent` -> `agent_pane::AgentPane` -> `agent_track::Track` (state machine) -> `saved()` -> `session::SavedAgent` in session.toml.
- Restore: `agent_restore::RestorePlan::resume` builds input bytes `claude --resume <id>` / `codex resume <id>` typed into the shell after prompt (`PROMPT_FALLBACK` 5s), gated by `ResumeMode`/`config::ResumeAgents`; `SkipReason` -> toast/`PaneNotice`. Only exact SessionId resumes.

## 6. Tests
- NO tests/ directory. All tests inline `#[cfg(test)]` (76 test mods; biggest counts: terminal 73, ui 67, procwatch 40, keys 29, agent_track 27, tabs 27, agent_codex_scan 26, settings 24, config 31, session 20, handoff 20, ...). `test_support.rs` = shared SpawnOpts/ShellFixture (spawns real zsh/bash PTYs). proptest for chunk-split in osc/progress etc. UI headless tests via `Context::run_ui` in ui.rs + settings.rs `sections_render_headless`.
- Commands (.agents/rules/project.md:73-78): `cargo build 2>build.log; echo $?` | `cargo test` | `cargo clippy --all-targets -- -D warnings` | `cargo fmt --check` | `cargo run` | `cargo run -- --screenshot /tmp/x.png`.
- Rules files: .agents/rules/{code-principles,implementation,performance,platform,project,terminal,testing,ui}.md; also AGENTS.md/CLAUDE.md/TESTING.md at root.
- Workspace move impacts: build.rs, CI (`workspaces: .`), rule file paths (project.md lists src/ layout at ~:63), test_support is `mod test_support` (cfg(test)) so tests using it must move with it or lib needs a `test-support` feature.

## 7. Release / update
- `update.rs`: `RUNNING = env!("CARGO_PKG_VERSION")`; `latest_release()` uses ureq to read the redirect of github.com/<repo>/releases/latest; `is_newer`; `installed_with_brew()` / `brew_upgrade()`; `pending_for_running_exe()` :54 compares the running exe's .app Info.plist CFBundleShortVersionString vs RUNNING (i.e. brew replaced the bundle on disk = "restart pending").
- `updater.rs` `Updater::new(ctx)` ticked each frame in main (main.rs:130/473). The dot: `ui::update_dot_center` ui.rs:60, painted ui.rs:1506 + tabs.rs:1421-1431 on the gear icon when `pending_update.is_some() || offered.is_some()`. Restart uses handoff (`hand_off`) so shells survive.
- native-release.yml (trigger `stdusk-v*` tag or dispatch, macos-14): checkout -> resolve version -> rust stable + both apple targets + rust-cache (`workspaces: .`) -> `cargo build --release` x2 targets -> lipo universal `stdusk.app` (Info.plist from packaging/Info.plist with __VERSION__) -> optional Developer ID sign + notarize + staple (5 secrets) -> ditto zip -> generate Homebrew cask `stdusk.rb` w/ sha256 -> `gh release create` -> optional push cask to Hobo-Ware/homebrew-tap via HOMEBREW_TAP_DEPLOY_KEY. Binary name `stdusk` hard-coded in lipo paths: still works if the bin stays package `stdusk` in a workspace member (target/ shared at workspace root, paths unchanged).

## Gaps (not checked)
- exact line where colors.rs imports crate::settings (likely `ink_color`/Theme type usage; verify whether `Theme` is defined in colors.rs or settings).
- full pub(crate) signatures for config/handoff/session internals; contents of native.yml; build.rs; whether `pub(crate)` surface count is huge (~hundreds of items need `pub`).
- tests in terminal.rs near :3394 that call `crate::ui` - confirm it is test-only.
- which of terminal/ui import `crate::Stdusk` (none by grep; only repo/workspace/tabs/finder/palette/settings/handoff).
