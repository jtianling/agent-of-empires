## Why

The notification bar currently uses sequential numbering ([1], [2], [3]...) that does not match the session's real `@aoe_index`. When a user sees `[2] ○ main` in the notification bar, they expect to press `Ctrl+b 2` to jump there, but the real index may be different. This mismatch makes the number-jump feature unreliable when used from the notification bar context.

## What Changes

- Add a `real_index` field to `NotificationEntry` so each entry carries its position from the global ordered session list
- Compute the real index during `ordered_existing_notification_entries()` using the same `flatten_tree` order that determines `@aoe_index`
- Change `format_notification_text()` to use the entry's real index instead of sequential `.enumerate()` numbering
- Preserve real indices through filtering steps (`build_notification_entries`, `visible_notification_entries`) so collapsed-group filtering and current-session exclusion do not affect displayed indices
- Update existing unit tests for `format_notification_text` to reflect real-index numbering

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `notification-bar`: The notification format requirement changes from sequential indices to real `@aoe_index` values, so displayed `[N]` matches the tmux `Ctrl+b N` jump target.

## Impact

- `src/tmux/notification_monitor.rs`: `NotificationEntry` struct gains a field; `ordered_existing_notification_entries`, `build_notification_entries`, `visible_notification_entries`, and `format_notification_text` all change
- `src/tmux/utils.rs`: May be referenced for `session_index_in_order` / `ordered_profile_session_names` logic, but likely not modified
- Existing unit tests in `notification_monitor.rs` need updating for the new index behavior
- No breaking changes to stored data, CLI interface, or tmux option schema
