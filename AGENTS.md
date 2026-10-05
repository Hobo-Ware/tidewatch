# Always-loaded core (small, project-wide)

@.agents/rules/project.md

@.agents/rules/workflow.md

@.agents/rules/code-principles.md

@.agents/rules/implementation.md

@.agents/rules/testing.md

# Domain rules - load on demand

Not auto-imported, to keep the baseline context small. Read them with the Read tool when the
work touches the matching area. `CLAUDE.md` routes the mapping; the files live in
`.agents/rules/`:

- `ui.md` - egui/eframe render loop, tabs, panes, dialogs, design-system primitives
- `performance.md` - frame budget, repaint scheduling, allocation discipline
- `platform.md` - quake window, global hotkey, monitor sizing, macOS keybinds

Re-read after long gaps if context was compacted.
