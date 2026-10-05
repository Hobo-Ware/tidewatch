# Kelp recon (read-only, ~14 tool calls)

## 1. Cargo
- /kelp/Cargo.toml:1-17 workspace, resolver 3, members crates/kelp-core + crates/kelp. version 0.8.17, edition 2024, MIT. `kelp-core = { path = "crates/kelp-core" }` (path-only, not published; no publish metadata, no `[lints]` anywhere).
- kelp-core/Cargo.toml: gix 0.88.0 (default-features=false: sha1, revision, max-performance-safe, blob-diff, index), pulldown-cmark 0.13.4, image 0.25.10, notify 8.2.0, serde 1.0.229, serde_json, sha2 0.11, similar 3.2, ureq 3.4.2, anyhow. dev: criterion 0.8.2 (bench "layout").
- kelp/Cargo.toml: eframe 0.35.0 (accesskit, default_fonts, glow, wayland, x11), egui_extras 0.35.0, egui_commonmark 0.24, resvg 0.48, rfd 0.17, gix 0.88.0 (sha1 only), objc2* (macOS only).
- kelp-core has NO egui dependency. Good for git dep. Tidewatch must pin same egui 0.35 if it shares UI.
- Git dep via workspace: `kelp-core = { git = "<repo>", tag = "v0.8.x" }` works (path inside workspace is resolved by cargo). Release tags are v* so tag-pinning is natural. Repo is Hobo-Ware/kelp (update.rs:7).

## 2. kelp-core API (crates/kelp-core/src, lib.rs = flat `pub mod` list, 35 modules)
Sync everywhere (no async). IO is blocking; caller must thread it.
- history.rs (IO gix): `History::load(&gix::Repository)`, `load_filtered(repo,&ViewFilter)`, `History::open(&Path)->Result<(gix::Repository, Self)>`, `len/ids/id(row)/row(&ObjectId)/parents(row)->&[u32]`; `Parents::from_lists`; `date_order(times,&Parents)->Vec<u32>`; `LANE_COLORS=8`.
- graph.rs (pure CPU): `layout<'a>(rows, parents: impl Fn(usize)->&'a [u32], colors:u8)->Layout`; `Layout::{rows,node_lane,node_color,edges(row)->Vec<Edge>,lane_count}`; `Edge{kind:EdgeKind(Pass|Top|Bottom|JoinIn|MergeOut),lane:u16,color:u8,from:u32}`. Bench: benches/layout.rs.
- diff.rs 650 lines (IO gix + similar): `commit_file(...)`, `staged_file(repo,path)`, `unstaged_file`, `range_file`, `file_at`, `build(path, old:Option<&[u8]>, new:Option<&[u8]>)->FileDiff` (pure, works on any two byte blobs - Tidewatch can feed it), `hunk_patch`, `lines_patch`, `discard_lines_patch`. Types: `FileDiff{path,body:Body,added,removed,old_text,new_text,preview,lfs}`, `Body{Text(Vec<Hunk>),Binary,TooLarge,Submodule}`, `Hunk{header,lines}`, `Line{kind,old:Option<u32>,new:Option<u32>,text,raw,emphasis:Vec<(usize,usize)>}` (word-highlight spans computed in core: diff.rs:444 `word_emphasis`). 8 MiB cap (diff.rs:4).
- status.rs: `working_status(&Path)->Result<WorkingStatus>` (git CLI), `parse_porcelain`.
- commit.rs (gix): `summary`, `details`, `list_dir`, `FileChange`, `relative_time`, `calendar_time`, `reflow`. refs.rs: `Refs::load(&Repository)`, `RefLabel`, `attach_pulls`. 
- ops.rs 972 lines: `enum Op` (all writes, shells out to git CLI), `Op::args()/label()/command_line()/run(&Path)->Result<String>`. git_cli.rs: `run(dir,&[&str])`, `run_with_stdin`.
- review.rs 300 lines (file IO, no gix except `author_name(&gix::Repository)`): see section 4.
- avatar.rs (network via ureq + disk cache + image decode): `Resolver::new(Option<GitHubRepo>, Option<token>)`, `resolve(email,commit)->Option<Image>`, `resolve_owner`, `initials`, `color_index`, `gh_token()`, `cache_dir()`, `decode(bytes,Shape)`. Blocking.
- search.rs 33 lines: `matching_rows(dir,&[ObjectId],query)->Result<Vec<usize>>` (git CLI, blocking).
- update.rs 175 lines (network+brew): hardcoded `REPO="Hobo-Ware/kelp"`, `CASK="hobo-ware/tap/kelp"` (update.rs:7-8) -> Tidewatch needs its own or parametrize.
- workspace.rs: `worktrees(&Path)->Result<Vec<Worktree>>` (git worktree list --porcelain), `stashes`, `ahead_behind`, `change_count`. agents.rs: see 5.
- Also: pulls.rs (gh CLI, 20s timeout), checks.rs, watch.rs (notify file watcher), blame, conflict, rebase, undo, reflog, compare, signing, console (command log), view (ViewFilter), sidebar_prefs, markdown, preview, lfs, submodules.
- Note: core shells out to `git` and `gh` binaries; Tidewatch inherits that runtime requirement. Some core modules log to `console` global (update.rs:105).

## 3. UI in crates/kelp/src (NOT reusable as-is)
Graph:
- graph_view.rs 2024 lines: `GraphView` struct (:44) holds state; `GraphInput<'a>` (:61) = `&gix::Repository, &History, Selection, head_row, Wip, lit, other_wips: &[OtherWip{tree:&Worktree}], GraphColumns, compare_view::Marks, checks map`. `GraphView::show(..., avatars:&mut AvatarStore)` (:140). Returns `Action{Select,Compare,Command(crate::commands::Command)}` (:98) so it is coupled to the app command enum (OpenWorktree, OpenChecks, ref label drag/drop -> DropPlan, menus). `paint_row` :949 is a free fn taking `RowPaint`/`RowGeo` structs - closest to reusable but private. Loads commit summaries lazily from the repo inside paint (`load_summary` :851) = reads gix on UI thread (cached per row).
- graph_rows.rs 245 (Row enum incl WIP rows, RowMap), graph_hover.rs 299 (HoverPath), ref_labels.rs 837 (labels, drag/drop onto branches -> git ops), columns.rs 296, avatars.rs (AvatarStore wraps core Resolver + egui textures; `texture(email, ObjectId)`).
- Imports from crate: fonts, icons, widgets, compare_view, commands, columns, avatars, theme (58 `theme::` uses in graph_view). `theme.rs` 643 lines = global color functions (`theme::panel()`, `accent()`, ... global state, not a parameter).
- Pub reusable-ish: `commit_tooltip(ui,&TooltipCommit,now)` :1415, `paint_avatar` :1480, `draw_avatar` :1508, `selection_halo` :1499, `truncated` :1580 (take plain args).
Diff:
- diff_view.rs 1949 lines. `DiffView` (:88) fields: source:DiffSource(Commit/Unstaged/Staged/File/Range, :41), diff:FileDiff, layout Unified|Split (:58), composer, draft, replies, pending:Option<Op>, ask:Option<Dialog>, blame, preview, jump. `DiffView::load(repo, workdir, source, path)` (:135) reads gix itself. `ui(&mut self, ui, review:&mut Review, author:&str, avatars:&mut AvatarStore)->Event` (:427) and `stacked_ui` (:439). `Event{None,Close,Changed,Run(Op),Ask(Dialog),OpenInEditor,FileHistory,Reveal}` (:118): couples to `kelp_core::ops::Op` (stage/discard hunks, :326,:356), `crate::dialogs::Dialog`, blame_view, preview_view, fonts, theme, widgets, icons.
- Unified/split: :1124 `split_side`, `Layout` enum. Jump between changes: `jump_to_change(next)` :393, `change_starts` :1584, Alt+Up/Down :461 (not shown above, consumed in `show`). Word highlight comes from core `Line.emphasis`.
- multi_diff_view.rs (stacked multi-file), details.rs 888 (commit details pane), file_history_view.rs.
Review threads:
- Rendered inside diff_view.rs: `Item::Thread(u64)` row type (:76), `items()` :750 places threads via `Anchor::locate` (:762), `fn thread(...)` :1296 and `composer_ui` :1460. Takes `&mut Review` + `author` (plain data, good) but is a private method of DiffView, interleaved with diff row layout. No standalone widget.
- Other users of Review: repo_view.rs, details.rs, pulls_view.rs, stash_view.rs, reflog_view.rs, file_history_view.rs, preview_view.rs, multi_diff_view.rs.
Verdict: no graph/diff/thread widget is a plain-data widget. All reach into: AvatarStore, theme globals, Command/Dialog/Op enums, and repo handles. No `app.rs` main struct is passed in directly (GraphInput/args are narrow), which makes extraction feasible but non-trivial.

## 4. Review comments (kelp-core/src/review.rs)
- Path: `<git_common_dir>/kelp/comments.json` (review.rs:97), `Review::load(git_common_dir:&Path)` (infallible, defaults on parse error), `save()` atomic via tmp+rename (:106). Shared across worktrees (common dir). No locking: two processes saving = last write wins (Tidewatch + Kelp concurrently on same repo is a risk).
- Types (serde, JSON): `Side{Old,New}`; `Anchor{side,line_hint:u32,text:String,before:Vec<String>,after:Vec<String>}` (CONTEXT=2 lines each side); `Comment{author,time:i64,body}`; `Thread{id:u64,path,commit:Option<String>,anchor,resolved,comments}`; `Review{threads, next_id, #[serde(skip)] file}`. Methods: add/reply/set_resolved/delete/for_path/count_for/open_count/to_markdown; `author_name(&gix::Repository)`.
- Line following: `Anchor::locate(&[&str])->Option<usize>` (:63): finds all lines equal to anchor.text, scores by matching context lines before/after, tie-breaks by distance to line_hint. Called at render time (diff_view.rs:762), so threads re-anchor on current text, orphaned threads go to the "outside"/hidden section (`OutsideHeader` item, :80, :852). No line number rewriting persisted.
- `commit: Option<String>` ties thread to a commit; None = working tree. Tidewatch (PR-ish review of agent worktrees) can use this core as-is; the schema has no PR/branch id.

## 5. Worktrees
- List: kelp-core/src/workspace.rs:100 `worktrees(dir)` -> `git worktree list --porcelain`, parser :105 (`Worktree{path,branch,locked,prunable,detached...}`, `name()` :22).
- Create/remove/move/prune: kelp-core/src/ops.rs:265-273,334 `Op::WorktreeAdd{path,new_branch,start}`, `WorktreeRemove{path,force}`, `WorktreePrune`, `WorktreeMove{from,to}` (git CLI via Op::run).
- Agent detection: kelp-core/src/agents.rs: `enum Agent{Claude,Codex,Gemini,Copilot,Aider,Cursor}` :6, `label()`, `classify(args)` :71 (matches process args incl. node/bun/python interpreters), `parse_ps` :91 (runs `ps`), `parse_lsof_cwd` :101 (runs `lsof` for cwd), `sessions()` :118, `sessions_cached(&mut HashMap<pid,PathBuf>)` :122, `by_worktree(&[PathBuf], &[Session])->HashMap<PathBuf,Vec<Agent>>` :168 (deepest containing worktree wins, handles `.claude/worktrees/*`). Fake: `fake_sessions(spec)` :155 (env KELP_FAKE_AGENTS).
- App side: crates/kelp/src/agent_watch.rs (199 lines): background scanner thread every 10s (SCAN_EVERY), only runs when focused and wanted, `set_wanted`, `in_worktrees(&paths)`, `paint_badge`, `summary`. worktrees_view.rs (627 lines) UI: :95-173 badges and hover; sidebar.rs `worktree_rank`. Process-based detection only (ps+lsof): no hook/transcript parsing, no session content.

## 6. Self-update + release
- Core: kelp-core/src/update.rs `latest_release()` :40 (ureq to GitHub releases), `is_newer`, `installed_version()` :75 (reads installed bundle plist), `installed_with_brew()` :93, `brew_upgrade()` :99 (git pull tap `hobo-ware/tap` ff-only, then `brew upgrade --cask hobo-ware/tap/kelp` with HOMEBREW_NO_AUTO_UPDATE), `app_bundle()` :126.
- App: crates/kelp/src/updater.rs 329 lines: `Updater` state machine `Status{Idle,Checking,UpToDate,Available,Installing,Ready,Failed}`; first check 5s after launch, hourly (CHECK_EVERY), retry 15 min, check on focus (60s gap); mpsc channel + thread; `tick(enabled, auto_install)`; auto-install only when brew-installed; after install shows "restart" pill; `restart(ctx)` :260. Settings section :206. Only works if installed via Homebrew.
- .github/workflows/release.yml: on tag `v*` (or manual dispatch) on macos-14. Steps: 1 verify tag == Cargo.toml version; 2 rust stable + both mac targets + cache; 3 `cargo test --workspace`; 4 build release for aarch64 and x86_64; 5 lipo universal Kelp.app, copy icns/Assets.car, stamp Info.plist from packaging/Info.plist; 6 optional codesign+notarize if 5 secrets (MACOS_CERT_P12, MACOS_CERT_PASSWORD, NOTARY_KEY_ID, NOTARY_ISSUER, NOTARY_KEY) else ad-hoc; 7 ditto zip + sha256; 8 generate cask kelp.rb (adds xattr quarantine-removal postflight if unsigned); 9 (rest, not read) publishes GitHub release and pushes cask to Hobo-Ware homebrew tap. Details in packaging/README.md.

## 7. Performance rules + harness
- .agents/rules/project.md "Performance rules": idle CPU 0% (repaint only on input or finished work), never block UI on git/network/disk (use job runner), graph draws visible rows only, O(visible lanes) per row, re-run KELP_BENCH_SCROLL on trakt-web and git/git, log in LEDGER.md.
- Job runner: crates/kelp/src/jobs.rs (60 lines, tiny): `Jobs<T>::new(egui::Context)`, `spawn(label, FnOnce()->T + Send)` = std::thread::spawn, sends (id, T) over mpsc and `ctx.request_repaint()`; `finished()->Vec<T>` polled each frame; `running()`, `is_running(label)`. No pool, no cancellation, depends on egui. Core has none: all sync, core is thread-agnostic.
- Screenshot: crates/kelp/src/dev_screenshot.rs: `DevScreenshot::from_env()` reads KELP_SCREENSHOT=<png path> (+ KELP_SCREENSHOT_WAIT secs); `tick(ctx, ready)` swallows key/text input, waits SETTLE_FRAMES=3 + ready flag, sends `ViewportCommand::Screenshot`, saves the PNG from `Event::Screenshot`, then quits. settings.rs:83 disables settings/tab persistence when KELP_SCREENSHOT or KELP_BENCH_SCROLL set. Bench: dev_bench.rs (KELP_BENCH_SCROLL=1).

## 8. Tests / commands (.agents/rules/project.md)
- `cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all --check`; `cargo bench -p kelp-core --bench layout`; `./scripts/bundle-macos.sh`; `KELP_SCREENSHOT=/tmp/x.png cargo run -p kelp -- <repo>`; `KELP_BENCH_SCROLL=1 cargo run --release -p kelp -- <repo>`.
- Layout: kelp-core/tests/*_roundtrip.rs (13 files: history, compare, conflict, console, modules, ops, rebase, reflog, sidebar, staging, undo, view, watch) + tests/common (builds real temp repos); inline unit tests in src. App has ~220 #[test] matches under crates/kelp/src (inline, e.g. worktrees_view.rs:583, updater.rs:276). Dev env switches: KELP_OPEN_*, KELP_SEARCH, KELP_SELECT_WIP, KELP_FAKE_UPDATE, KELP_OFFLINE, KELP_FAKE_AGENTS.
- Conventions: Conventional Commits, no Co-Authored-By, read LEDGER.md + PLAN.md first.

## Gaps (not read)
- Release workflow tail (publish + tap push), packaging/README.md.
- Exact `Worktree` struct fields, `Summary`/`Details` fields, `Selection` type, `ViewFilter` use.
- Whether `crates/kelp/src/app/` dir and repo_view.rs hold more diff/thread glue (repo_view.rs 2829 lines unread).
- widgets.rs/theme.rs internals (how global theme state is stored).
- Not checked: whether kelp-core builds without macOS (uses ps/lsof/brew; likely fine on unix).
- PLAN.md/LEDGER.md perf targets.
