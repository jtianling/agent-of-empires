## Context

AoE binds `C-;` to `select-pane -t :.+` in `setup_session_cycle_bindings()` at `src/tmux/utils.rs:341`. This cycles between panes in the current tmux window. When a user splits panes on Mac and then accesses the same session from a narrow terminal (iPhone), both panes are squeezed to unusable widths.

tmux's zoom feature (`resize-pane -Z`) makes a single pane fill the entire window. When `select-pane` is called in a zoomed state, tmux automatically unzooms first. By re-zooming after the switch, we get seamless full-screen pane cycling.

## Goals / Non-Goals

**Goals:**
- Make `C-;` work correctly in zoomed state (switch + re-zoom)
- Auto-zoom when attaching to a multi-pane session from a narrow terminal
- Zero behavior change on wide terminals or when panes are not zoomed

**Non-Goals:**
- Per-client zoom state (tmux zoom is per-window; acceptable since user typically uses one device at a time)
- Auto-unzoom when terminal becomes wider (user can `C-b z` manually)
- New keybindings -- reuse existing `C-;`

## Decisions

### 1. Zoom-aware `C-;` via `if-shell`

Use tmux's `if-shell` with `#{window_zoomed_flag}` to branch behavior:

```
if-shell -F "#{window_zoomed_flag}" \
  "select-pane -t :.+ ; resize-pane -Z" \
  "select-pane -t :.+"
```

When zoomed: `select-pane` auto-unzooms, then `resize-pane -Z` re-zooms the newly selected pane. When not zoomed: behaves exactly as before.

Alternative considered: separate keybinding for zoom-switch. Rejected because `C-;` already has the right semantics (cycle panes) and making it zoom-aware is transparent.

### 2. Auto-zoom threshold reuses `is_narrow_layout()`

Use the same narrow-screen detection from the single-panel change (`available_width < list_width + 20`) applied to the terminal width at attach time. This provides consistency -- if the TUI is in single-panel mode, the attached session should also be zoomed.

Alternative considered: fixed column threshold. Rejected for same reason as the single-panel change -- dynamic threshold adapts to user's list_width setting.

### 3. Auto-zoom targets pane 0 (left pane)

When auto-zooming, zoom `{session}:.0` (the leftmost/first pane). This is the agent pane, which is the primary content. The right pane (shell/tool) is secondary.

If only one pane exists, `resize-pane -Z` is a no-op (zoom on a single pane has no visible effect), so no special-casing needed.

### 4. Auto-zoom runs before attach

Insert the zoom command between keybinding setup and `tmux_session.attach()` in `app.rs`. This ensures the user sees the zoomed state immediately upon entering the session.

## Risks / Trade-offs

- [Zoom is per-window, not per-client] -> If both Mac and iPhone are connected, zoom on one affects the other. Acceptable because the user typically uses one device at a time. Mac user can unzoom with `C-b z`.
- [Auto-zoom on single-pane sessions] -> No visible effect, no harm. `resize-pane -Z` on a single pane just toggles the zoom flag without changing layout.
