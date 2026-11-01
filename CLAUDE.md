Before implementing anything, read `LEDGER.md` (current state) and the current phase in
`PLAN.md`, then follow `.agents/rules/workflow.md`. Identify which area you are working in
and read the matching rule file from `.agents/rules/` (only the core rules auto-load via
AGENTS.md; domain rules load on demand):

- UI: render loop, tabs, panes, graph/diff panes, dialogs, settings (`crates/tidewatch/src/`):
  read ui.md
- Performance: repaint scheduling, many-agent load, allocation on the hot path: read
  performance.md
- Quake window, global hotkey, monitor sizing, macOS keybinds: read platform.md
- Terminal engine internals: they live in `../stdusk` (the `stdusk-core` and `stdusk-egui`
  crates); read `../stdusk/.agents/rules/terminal.md` and change them there
- Graph, diff and review internals: they live in `../kelp` (`kelp-core`, `kelp-ui`); read
  `../kelp/.agents/rules/project.md` and change them there
- Tests: testing.md (already loaded as core)
- What a module owns: grep `docs/layout.md` for its file
- Dev switches, screenshot and live checks: read `docs/dev-switches.md`
- All other code: project.md, workflow.md, code-principles.md, implementation.md (always on)

@AGENTS.md
