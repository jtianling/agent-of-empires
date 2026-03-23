## Context

The back-toggle feature (Ctrl+b b) lets users quickly return to their previous session. Currently, when a user returns to the AoE TUI from any session, the `attach_to_session` method unconditionally clears `@aoe_prev_session_{client}` and `@aoe_from_title` on the target session. This means Ctrl+b b never works after entering a session from TUI -- the navigation chain is broken every time the user passes through the home screen.

The existing `last_detached_session` mechanism already tracks which session the user came from (used for TUI selection restoration). This information is available but discarded before the next attach.

## Goals / Non-Goals

**Goals:**
- Make Ctrl+b b work after TUI-mediated session transitions (A -> TUI -> B, then Ctrl+b b returns to A)
- Show "from: <title>" in status bar when entering a session from TUI with a known source
- Preserve existing behavior when no source session is available (first launch, direct TUI open)

**Non-Goals:**
- Multi-level back stack (only the immediate previous session is tracked, same as current)
- Changing how tmux-level navigation (n/p/N/P/number/b) records previous sessions -- that already works correctly

## Decisions

### Decision 1: Store source session as TUI field, not as a tmux option

Add `session_before_tui: Option<String>` to `App` struct. Populate it in `try_restore_selection_from_client_context` (which already reads `last_detached_session`). Consume it in `attach_to_session`.

**Alternative considered**: Add a new tmux global option `@aoe_tui_source_{client}`. Rejected because the lifecycle is purely within one TUI session -- no need to persist outside the process.

### Decision 2: Conditional set-or-clear in attach_to_session

Replace the unconditional `clear_from_title` + `clear_previous_session_for_client` with:
- If `session_before_tui` is `Some(source)` AND source != target session name: call `set_previous_session_for_client(client, source)` and `set_target_from_title(source, target)`
- Otherwise: call existing clear functions (unchanged behavior)

This keeps the clearing path as fallback for first-launch and same-session-reentry.

### Decision 3: Make set_previous_session_for_client and set_target_from_title pub

These are currently private in `utils.rs`. The TUI attach path needs to call them. Expose as `pub` with the same signatures.

### Decision 4: Consume source on use

`attach_to_session` takes `self.session_before_tui.take()` so the source is only used once. If the user returns to TUI without detaching from a session (e.g., startup), the field stays `None` and the clear path runs.

## Risks / Trade-offs

- [Risk] Source session no longer exists when user enters target session. -> Mitigation: `switch_aoe_session_back` already checks `tmux_session_exists` before switching. Status bar shows a stale "from:" title but it is harmless and will be overwritten on next navigation.
- [Risk] User re-enters the same session they came from (A -> TUI -> A). -> Mitigation: source == target guard triggers clear path, so Ctrl+b b is a no-op (same as current behavior).
- [Trade-off] `set_previous_session_for_client` and `set_target_from_title` become public API surface. Acceptable since they are simple tmux option setters and follow the existing pattern of `clear_*` being public.
