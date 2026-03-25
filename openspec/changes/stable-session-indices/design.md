## Context

Session indices (`@aoe_index`) are currently computed from `ordered_profile_session_names()`, which builds a `GroupTree` with the real group collapse state. When a group is collapsed, `flatten_tree()` skips all sessions in that group, shifting every subsequent session's index. This makes `Ctrl+b N` jumps unreliable because the mapping between number and session changes with every collapse/expand.

The notification monitor already solves this for its own ordering via `expanded_groups()` in `notification_monitor.rs`, which clones all groups with `collapsed: false`. However, `format_notification_text()` then uses `.enumerate()` to assign sequential `[1]`, `[2]`, `[3]` labels, which don't match the real `@aoe_index`.

## Goals / Non-Goals

**Goals:**
- Session indices are stable across collapse/expand -- the same session always gets the same number regardless of group state.
- Notification bar `[N]` entries show the session's real `@aoe_index`, so pressing `Ctrl+b N` jumps to the session displayed as `[N]`.
- The `expanded_groups()` helper is shared, not duplicated.

**Non-Goals:**
- Changing how `flatten_tree()` works for TUI rendering. The TUI home screen must still respect collapsed state for display purposes.
- Changing the TUI number-jump digit key behavior (pending state, two-digit support). Only the index source changes.
- Changing notification bar visibility rules (which statuses are shown, collapsed-group filtering for Idle/Running).

## Decisions

### Decision 1: Use expanded groups in `ordered_profile_session_names()`

`ordered_profile_session_names()` in `src/tmux/utils.rs` will call `expanded_groups()` on the input groups before passing them to `GroupTree::new_with_groups()` and `flatten_tree()`. This ensures the session order and index assignment always reflects the full tree.

**Rationale**: This is the minimal change that fixes index stability. The notification monitor already does this for its ordering, so the pattern is proven. All callers of `session_index_in_order()` and `setup_session_cycle_bindings()` will automatically benefit.

**Alternative considered**: Adding a separate `stable_session_index()` function. Rejected because it would create two index-computation paths that could diverge.

### Decision 2: Move `expanded_groups()` to a shared location

The `expanded_groups()` function currently lives in `notification_monitor.rs`. It will be moved to `src/session/groups.rs` (on the `Group` type or as a free function) since it operates on `Group` data and is now needed by both `utils.rs` and `notification_monitor.rs`.

**Rationale**: `src/session/groups.rs` already owns the `Group` type and tree operations. Placing the helper there avoids cross-module dependency issues.

### Decision 3: Add `real_index` field to `NotificationEntry`

`NotificationEntry` gains a `real_index: usize` field (1-based). This index is computed in `ordered_existing_notification_entries()` from the position in the fully-expanded flattened tree, matching what `session_index_in_order()` would return for that session.

**Rationale**: The index must be computed at the point where the full ordered list is available. Carrying it on the entry avoids recomputing it downstream. Using 1-based indexing matches `@aoe_index` convention.

### Decision 4: `format_notification_text()` uses `entry.real_index`

Replace `.enumerate()` in `format_notification_text()` with `entry.real_index`. The output changes from sequential `[1] [2] [3]` to stable `[2] [5] [7]` (matching real indices, with gaps where sessions are filtered out).

**Rationale**: Direct use of the precomputed index. No additional lookup needed.

## Risks / Trade-offs

- **[Gap numbers in notification bar]** Users may see `[2] [5] [7]` instead of `[1] [2] [3]`. This is intentional -- the numbers now match `Ctrl+b N` keybindings, which is the whole point. Mitigation: the behavior is self-explanatory once users press the number and land on the right session.

- **[Gap numbers in TUI session list]** When groups are collapsed, the TUI will show indices with gaps (e.g., 1, 2, 5, 6 skipping 3-4 in a collapsed group). This is the correct trade-off: stable indices are more useful than contiguous indices. Mitigation: this matches how most editors/IDEs handle numbered tabs.

- **[Test updates required]** Existing tests for `format_notification_text` and `session_index_in_order` assume the old behavior (sequential numbering, collapse-sensitive indices). These must be updated. Mitigation: the test changes are straightforward since we're changing expected values, not test structure.

- **[Expanded groups function relocation]** Moving `expanded_groups()` from `notification_monitor.rs` to `groups.rs` changes import paths. Mitigation: simple find-and-replace; the function signature is unchanged.
