## Why

Session indices (`@aoe_index`) change when groups are collapsed or expanded, making `Ctrl+b N` unreliable. The notification bar compounds this by using sequential renumbering (`[1]`, `[2]`, `[3]`) that doesn't match the real `@aoe_index`, so users can't jump to a notified session by pressing its displayed number.

## What Changes

- `ordered_profile_session_names()` will always compute indices from a fully-expanded group tree, so collapsing/expanding groups never renumbers sessions.
- The notification bar will display each session's real `@aoe_index` (from the stable expanded ordering) instead of sequential enumerate-based numbers.
- `NotificationEntry` gains a `real_index` field computed from the expanded tree position.
- The `expanded_groups()` helper is shared between notification monitor and utils to avoid duplication.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `number-jump`: Index assignment must always use the fully-expanded group tree, so collapsed groups no longer remove sessions from the numbering. The requirement "Sessions inside collapsed groups SHALL NOT receive indices" changes to "Sessions inside collapsed groups SHALL still receive stable indices computed from the expanded tree."
- `notification-bar`: Notification entries must display the session's real `@aoe_index` instead of sequential renumbered positions.

## Impact

- `src/tmux/utils.rs`: `ordered_profile_session_names()` must use expanded groups for index computation.
- `src/tmux/notification_monitor.rs`: `NotificationEntry` struct, `ordered_existing_notification_entries()`, `format_notification_text()`, and `build_notification_entries()` updated to track and display real indices.
- `src/session/groups.rs`: `expanded_groups()` helper extracted to a shared location (or duplicated in utils).
- Existing unit tests for `format_notification_text`, `session_index_in_order`, and notification entry functions need updating.
- **BREAKING**: TUI number-jump display will show stable indices even when groups are collapsed, meaning visible session numbers may have gaps (e.g., 1, 2, 5, 6 if sessions 3-4 are in a collapsed group).
