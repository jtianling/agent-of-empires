## Why

Several TUI dialogs use fixed narrow widths (Rename 50, Fork 56, New Session sub-dialogs 72). Nested group paths like `work/frontend/xxx` and filesystem paths get truncated, making the displayed value unreadable. Current fixed widths do not use available terminal space on the wide terminals where users normally operate `aoe`.

## What Changes

- Add a `responsive_width(area, max)` helper in `src/tui/dialogs/mod.rs` that returns `min(area.width.saturating_sub(4), max)`, so dialogs scale up to a cap but never overflow a small terminal.
- Switch 7 dialog render sites to compute width via `responsive_width(area, 120)`:
  - New Session main dialog (was 80)
  - New Session Sandbox Config sub-dialog (was 72)
  - New Session Tool Config sub-dialog (was 72)
  - New Session Worktree Config sub-dialog (was 72)
  - Edit Session dialog (was 50)
  - Edit Group dialog (was 50)
  - Fork Session dialog (was 56)
- Update the New Session error-line wrap calculation so the wrap width tracks the new responsive dialog width.
- **BREAKING**: None. Only container width changes; field layout, input behavior, and keybindings are unchanged.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `tui`: add a requirement that dialogs rendering user-editable paths use a responsive width with a 120-column cap, so long group paths and filesystem paths display in full on normal terminals.

## Impact

- Affected code: `src/tui/dialogs/mod.rs`, `src/tui/dialogs/new_session/render.rs`, `src/tui/dialogs/rename.rs`, `src/tui/dialogs/fork_session.rs`.
- No API / CLI / data format changes.
- No dependency changes.
- Narrow terminals (<124 columns) fall back to the `centered_rect` clamp behavior, which already handles under-sized areas. The user has accepted degraded behavior on phone-class widths.
