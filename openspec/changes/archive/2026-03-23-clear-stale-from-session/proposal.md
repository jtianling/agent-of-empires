## Why

When a user navigates between sessions using tmux keybindings (Ctrl+b n/p/N/P/b) and then returns to the AoE TUI via Ctrl+q, the target session's `@aoe_from_title` and the client's `@aoe_prev_session_{client}` options are left intact. If the user then enters a different session from the TUI, those stale values cause the status bar to incorrectly show a "from:" label and allow Ctrl+b b to jump to a session the user did not actually come from. Entering a session from the TUI is a fresh navigation -- there is no "previous session" context.

## What Changes

- When the TUI attach path enters a session, clear the `@aoe_from_title` session option on the target session so the status bar does not display a stale "from:" label.
- When the TUI attach path enters a session, clear the `@aoe_prev_session_{client}` global option for the current client so Ctrl+b b does not jump to a stale previous session.
- Add two public helper functions in `src/tmux/utils.rs`: `clear_from_title(session_name)` and `clear_previous_session_for_client(client_name)` to encapsulate the cleanup logic.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `session-back-toggle`: Add a requirement that entering a session from the TUI clears stale from-title and previous-session state, so the back toggle and status bar reflect fresh navigation context.

## Impact

- `src/tmux/utils.rs`: Two new public functions (`clear_from_title`, `clear_previous_session_for_client`) using existing internal helpers (`unset_tmux_session_option`, `unset_global_option`).
- `src/tui/app.rs`: Call the new helpers in the attach path (around line 644-650) before attaching to the tmux session.
- No breaking changes. No new dependencies.
