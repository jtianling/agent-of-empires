## Why

Resume restart and cold-start recovery represent the same user intent, but the TUI exposes them on separate `R` and `V` keys while fresh restart differs from resume only by letter case (`r` versus `R`). Cold recovery also reconstructs every additional pane as a horizontal split because the durable session store does not retain the tmux layout, so nested layouts such as a right column split vertically are lost after reboot.

## What Changes

- **BREAKING** Replace the lowercase `r` fresh-restart binding with `Shift+C`.
- **BREAKING** Remove the separate `Shift+V` recovery binding and make `Shift+R` state-aware: resume panes in an existing tmux session, or rebuild and recover a recoverable session.
- Update home status hints and help text to describe the state-aware `R` action and the new `C` clean restart action.
- Persist a recent tmux window-layout snapshot for each tracked instance while its session is alive.
- During cold recovery, recreate the panes, restore the saved nested pane geometry, then resume each persisted agent slot and update pane identifiers.
- Degrade safely when no valid layout snapshot exists or when a stored layout cannot be applied, while preserving per-pane recovery isolation.
- Add deterministic coverage for both key-routing states and a three-pane nested layout (left pane plus vertically split right column) surviving cold recovery.

## Capabilities

### New Capabilities

- `session-layout-recovery`: Persist and restore the tmux pane layout used by cold-start recovery, including nested horizontal and vertical splits.

### Modified Capabilities

- `tui`: Unify resume and cold recovery on `Shift+R`, move fresh restart to `Shift+C`, and update contextual hints and help.

## Impact

- `src/tui/home/input.rs`, `src/tui/home/render.rs`, `src/tui/components/help.rs`, and `src/tui/app.rs` for state-aware action routing and labels.
- `src/db/` and `src/migrations/` for durable layout snapshot storage.
- tmux query/reconcile code for capturing the active window layout without mutating global tmux state.
- `src/session/instance.rs` and `src/tmux/` for applying the stored layout during cold recovery while preserving slot-to-pane mapping.
- Unit and `tests/e2e/` coverage for keybindings, recovery fallback, and nested layout geometry.
- User-visible keyboard behavior changes; no external API dependency is introduced.
