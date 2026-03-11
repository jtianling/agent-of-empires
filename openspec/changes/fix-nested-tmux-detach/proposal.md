## Why

When aoe itself runs inside a tmux session, opening a managed agent/terminal session calls `switch-client` to move the tmux client from the outer (aoe TUI) session to the managed session. Pressing `Ctrl+b d` (the default tmux detach key) in the managed session then fully detaches the client from all sessions, closing the terminal entirely instead of returning to the aoe TUI session.

## What Changes

- When inside tmux and switching to a managed session, rebind `Ctrl+b d` so that pressing it in any `aoe_`-prefixed session switches back to the previous session (`switch-client -l`) instead of fully detaching the tmux client.
- The binding falls back to normal `detach-client` behavior when the current session is not an aoe-managed session, preserving existing behavior elsewhere.

## Capabilities

### New Capabilities

- `nested-tmux-detach`: Graceful detach behavior for managed sessions when aoe runs inside tmux -- `Ctrl+b d` in a managed session returns to the parent session instead of closing the terminal.

### Modified Capabilities

- `session-management`: Session attach behavior changes: when using `switch-client`, a global tmux key binding is set to intercept detach in aoe-prefixed sessions.

## Impact

- `src/tmux/session.rs`: `Session::attach()` -- set binding after `switch-client`
- `src/tmux/terminal_session.rs`: `TerminalSession::attach()` and `ContainerTerminalSession::attach()` -- same
- No config schema changes, no breaking changes to CLI or storage
