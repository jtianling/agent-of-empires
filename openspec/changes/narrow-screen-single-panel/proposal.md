## Why

The AoE TUI uses a fixed two-panel layout (session list + preview) that requires ~85 columns minimum (45 list + 40 preview). On narrow terminals -- iPhone portrait (~40 cols), Mac split-screen windows, or narrow tmux panes -- both panels get crushed to the point of being unusable.

## What Changes

- Add narrow-screen detection: when `available_width < list_width + 20`, switch to single-panel mode
- In single-panel mode, render only the session list at full width; hide the preview panel entirely
- Skip `update_caches` (tmux capture-pane) when preview is not rendered, reducing overhead
- No new keybindings or toggle mechanism needed -- users attach directly to see session details

## Capabilities

### New Capabilities
- `narrow-layout`: Responsive single-panel layout mode for narrow terminals

### Modified Capabilities

## Impact

- `src/tui/home/render.rs`: Layout branching in `render()` based on terminal width
- `src/tui/app.rs`: Conditional `update_caches` skip when in single-panel mode
- No config changes, no new dependencies, no breaking changes
