## Why

When AoE sessions have left/right tmux panes (created via right-pane tool or manual `ctrl+b %`), narrow terminals like iPhone portrait (~40 cols) split both panes to unusable widths (~20 cols each). Users need a way to view one pane at a time on narrow screens while preserving the ability to switch between them.

## What Changes

- Make the `C-;` pane-switch keybinding zoom-aware: when the current pane is zoomed, `C-;` switches to the next pane and re-zooms it, providing seamless full-screen pane cycling
- Auto-zoom the left pane when attaching to a session from a narrow terminal, so iPhone users see a usable full-screen pane immediately
- No behavior change on wide terminals -- `C-;` works exactly as before when panes are not zoomed

## Capabilities

### New Capabilities
- `zoom-pane-switch`: Zoom-aware pane switching and auto-zoom on narrow attach

### Modified Capabilities

## Impact

- `src/tmux/utils.rs`: Modify `C-;` binding in `setup_session_cycle_bindings()` to handle zoomed state
- `src/tui/app.rs`: Add auto-zoom before `tmux_session.attach()` when terminal is narrow
