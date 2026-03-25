## Why

Quick-switch navigation (Ctrl+b <num> Space, Ctrl+./Ctrl+,, Ctrl+b b) already takes the user to a target session, but does not acknowledge Waiting status on that session. Users must separately use Ctrl+b N to acknowledge notifications, which is redundant when the switch itself proves the user has seen the session. Integrating ack into quick-switch removes this friction and makes the dedicated Ctrl+b N shortcut unnecessary, simplifying the keybinding surface.

## What Changes

- Quick-switch functions (`switch_aoe_session_by_index`, `switch_aoe_session`, `switch_aoe_session_back` in `src/tmux/utils.rs`) will write the target session's instance_id to the ack signal file after a successful switch, so the notification monitor downgrades Waiting to Idle for that session.
- **BREAKING**: The `Ctrl+b N` notification keybinding and its sub-table (`aoe_notify`) are removed entirely. This includes `setup_notification_key_bindings()`, `cleanup_notification_key_bindings()`, `notification_binding_hint()`, `NOTIFICATION_TRIGGER_KEY`, `NOTIFICATION_KEY_TABLE`, `NOTIFICATION_HINT_OPTION`, and the `@aoe_notify_target_*` / `@aoe_notify_instance_*` tmux session options.
- The notification hint text (e.g., "Ctrl+b N 1 notify") is removed from the status bar. The notification bar entries themselves (session status icons and titles) remain unchanged.

## Capabilities

### New Capabilities
- `quick-switch-ack`: Auto-acknowledge Waiting status when switching to a session via any quick-switch path (number jump, Ctrl+./Ctrl+,, back toggle).

### Modified Capabilities
- `notification-bar`: Remove the Ctrl+b N keybinding hint from the status bar and remove all notification-specific keybinding setup/cleanup/option writes. The notification bar display itself (session entries with status icons) is unchanged.

## Impact

- **`src/tmux/utils.rs`**: The three switch functions gain ack signal file writes. Requires resolving the target session name to its instance_id (via session storage or tmux option lookup).
- **`src/tmux/notification_monitor.rs`**: Significant code removal -- notification keybinding setup/cleanup, hint generation, and related constants. The `ack_signal_path()` and `take_ack_signal()` helpers remain as the signal consumer. `setup_notification_key_bindings()` call sites in the monitor loop also removed.
- **`src/tmux/status_bar.rs`** (or wherever `build_notification_session_updates` lives): The hint insertion into status-left is removed.
- **Keybinding lifecycle**: `setup_session_cycle_bindings()` and `cleanup_session_cycle_bindings()` no longer need to set up or tear down notification-specific bindings (if they delegate to the removed functions).
- **No data migration needed**: No persisted data format changes. The ack signal file mechanism is unchanged.
