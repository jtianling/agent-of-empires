## Context

The notification bar in the tmux status-left area shows other sessions' statuses (Waiting, Running, Idle) with bracketed indices like `[1] ○ main [2] ○ file`. Currently these indices are sequential -- assigned by `.enumerate()` after filtering out the current session and collapsed-group sessions. Meanwhile, the `@aoe_index` tmux option (used by `Ctrl+b N` number-jump) reflects a session's 1-based position in the full `flatten_tree` ordered list of existing sessions.

This mismatch means the number shown in the notification bar does not correspond to the key the user should press. For example, if the user is attached to session #1 and sees `[1] ○ main`, pressing `Ctrl+b 1` would re-select the current session rather than jumping to "main" (which is actually `@aoe_index` 2).

## Goals / Non-Goals

**Goals:**

- Notification bar `[N]` values SHALL match the session's real `@aoe_index` so `Ctrl+b N` works as expected
- The ordering of entries in the notification bar remains the same (flatten_tree order)
- Indices are stable regardless of which session the user is currently viewing or which groups are collapsed

**Non-Goals:**

- Changing how `@aoe_index` is computed (that stays in `session_index_in_order` / `ordered_profile_session_names`)
- Modifying the number-jump keybinding system
- Changing the visibility/filtering rules for which sessions appear in the notification bar

## Decisions

### 1. Add `real_index: usize` to `NotificationEntry`

The `NotificationEntry` struct gains a `real_index` field representing the 1-based position in the global ordered list. This field is computed once during `ordered_existing_notification_entries()` and preserved through all downstream filtering.

**Rationale**: The index is a property of the session's position in the global list, not of its position in the filtered notification list. Computing it at the source (where we already have the full ordered list) is the cleanest approach.

**Alternative considered**: Computing the index separately in `format_notification_text` by looking up each entry's position. Rejected because it would duplicate the ordering logic and require passing additional parameters through the chain.

### 2. Compute index from the filtered flatten_tree iteration

In `ordered_existing_notification_entries`, the code already iterates through `flatten_tree` output, filters to sessions only, and filters to existing tmux sessions. The 1-based index is simply the position in this filtered iteration. We use `.enumerate()` at the point after filtering out group items and non-existing sessions, matching exactly how `session_index_in_order` computes its index.

**Rationale**: This produces the same index as `session_index_in_order` because both use the same ordering (`flatten_tree` with same sort_order) and the same existence filter (`existing_sessions` set vs `tmux_session_exists`). Using the same pipeline avoids any possibility of drift.

### 3. Use `real_index` in `format_notification_text` instead of `.enumerate()`

Replace the current `enumerate()` + `index + 1` pattern with direct use of `entry.real_index`.

**Rationale**: Direct substitution, minimal change. The filtering in `visible_notification_entries` (excluding current session) and `build_notification_entries` (excluding collapsed groups for non-Waiting) no longer affects the displayed index.

### 4. Use expanded_groups (not user-collapsed groups) for index computation

`ordered_existing_notification_entries` already uses `expanded_groups()` which forces all groups to `collapsed: false`. This means the flatten_tree produces all sessions in order regardless of collapse state, matching how `@aoe_index` is computed. No change needed here.

**Rationale**: The index must be stable across collapse/expand toggling. A session's `@aoe_index` does not change when a group is collapsed, so the notification bar index must not either.

## Risks / Trade-offs

- **[Index gaps in notification bar]** When sessions are filtered out (current session, collapsed groups), the displayed indices will have gaps (e.g., `[2] ○ main [4] ○ file`). This is intentional and correct -- it tells the user exactly which `Ctrl+b N` key to press. Users familiar with the number-jump feature will find this intuitive. Mitigation: the gaps are self-documenting since users already see `@aoe_index` in the TUI.

- **[Test updates required]** Existing unit tests for `format_notification_text` assert sequential `[1] [2] [3]` numbering. These must be rewritten to use real indices. Risk is low since the tests are straightforward.
