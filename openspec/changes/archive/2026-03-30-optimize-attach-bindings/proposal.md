## Why

When the TUI attaches to a tmux session, `with_raw_mode_disabled()` leaves the alternate screen before running `tmux attach-session`. The `session.attach()` method calls `setup_session_cycle_bindings()` inside this window, executing 120+ individual tmux subprocess calls (108 from number-jump key tables alone) while the normal buffer (command line) is visible. This creates a noticeable flash, especially over SSH.

## What Changes

- Move `setup_session_cycle_bindings()` out of `Session::attach()` and call it before `with_raw_mode_disabled()` in `App::attach_session()`, so all tmux binding commands execute while the TUI is still visible
- Batch all tmux bind-key commands in `setup_session_cycle_bindings()` into a single `tmux source-file` invocation using a temporary config file, reducing 120+ subprocess spawns to 1

## Capabilities

### New Capabilities

_None_

### Modified Capabilities

- `session-management`: The attach requirement changes -- `setup_session_cycle_bindings()` is no longer called inside `Session::attach()` but before the raw-mode-disabled window. The `Session::attach()` method becomes a thin wrapper around `tmux attach-session`.
- `number-jump`: The keybinding lifecycle requirement changes -- bindings are still set up before attach, but via batched `tmux source-file` instead of individual subprocess calls.

## Impact

- `src/tmux/session.rs`: `attach()` simplified to only run `tmux attach-session`
- `src/tui/app.rs`: `setup_session_cycle_bindings()` called before `with_raw_mode_disabled()`
- `src/tmux/utils.rs`: `setup_session_cycle_bindings()` and helpers refactored to write a temp file and call `tmux source-file` once
