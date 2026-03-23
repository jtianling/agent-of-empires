## Context

AoE manages tmux sessions and provides session navigation via keybindings (Ctrl+b n/p/N/P for cycling, Ctrl+b b for back-toggle). When switching between sessions at the tmux level, the system records `@aoe_from_title` (session-scoped, shows source session name in the status bar) and `@aoe_prev_session_{client}` (global, enables Ctrl+b b to jump back).

When a user returns to the AoE TUI via Ctrl+q and then enters a session from the home screen, these options are not cleared. This creates two bugs: (1) the status bar shows a stale "from:" label, and (2) Ctrl+b b jumps to a session the user did not come from in their current navigation flow.

The relevant code paths are:
- `src/tmux/utils.rs`: contains `set_target_from_title()` and `get_previous_session_for_client()` which manage these options using private helpers `unset_tmux_session_option()`, `set_global_option()`, and `client_context_option_key()`.
- `src/tui/app.rs`: the attach path (around line 644) where the TUI transitions to a tmux session. This is where cleanup should happen.

## Goals / Non-Goals

**Goals:**
- Clear `@aoe_from_title` on the target session when entering from the TUI, so the status bar starts clean.
- Clear `@aoe_prev_session_{client}` for the current client when entering from the TUI, so Ctrl+b b has no stale target.
- Expose reusable public helpers for the cleanup operations.

**Non-Goals:**
- Changing the behavior of tmux-level session switches (Ctrl+b n/p/N/P/b). Those should continue to set from-title and prev-session as they do today.
- Clearing these options when detaching (Ctrl+q). The cleanup happens on re-entry, not on exit.
- Clearing options for other clients. Only the current client's prev-session is affected.

## Decisions

### Decision 1: Clear on TUI entry, not on TUI return

The cleanup happens in the TUI attach path (when the user selects a session and enters it), not when the user returns to the TUI via Ctrl+q.

**Rationale**: Clearing on return would be premature -- the user might return to the TUI and then re-enter the same session, where preserving context could be useful in future features. Clearing on entry is the semantically correct moment: the user is starting a fresh navigation context from the TUI.

**Alternative considered**: Clear on Ctrl+q return. Rejected because the user has not yet chosen their next destination, so we cannot know which session to clear.

### Decision 2: Add public helper functions rather than inline tmux commands

Two new public functions (`clear_from_title` and `clear_previous_session_for_client`) in `src/tmux/utils.rs` wrap the existing private `unset_tmux_session_option` and a new `unset_global_option` helper.

**Rationale**: The TUI code in `app.rs` should not construct tmux option keys or call raw tmux commands. The existing pattern in `utils.rs` is to expose semantic functions (like `set_last_detached_session_for_client`) that hide the option key formatting.

### Decision 3: Add a private `unset_global_option` helper

Currently `src/tmux/utils.rs` has `set_global_option` and `get_global_option` but no `unset_global_option`. A new private function will mirror `unset_tmux_session_option` but use `-gq -u` flags.

**Rationale**: Consistent with the existing pattern of having set/get/unset helpers for both session and global options.

### Decision 4: Call cleanup before `update_session_index`

In the TUI attach path, the cleanup calls should go before the existing `update_session_index` call (line 645) but after `refresh_agent_tmux_options` (line 642). This ordering ensures the session is in a clean state before any index or status bar updates happen.

**Rationale**: `update_session_index` may trigger status bar refreshes. Clearing stale options first avoids a brief flash of stale "from:" text.

## Risks / Trade-offs

- [Risk] Clearing `@aoe_prev_session` on TUI entry means the user cannot use Ctrl+b b immediately after entering from the TUI to "go back" to whatever they were in before. -> This is the intended behavior: TUI entry is a fresh start, not a continuation of previous navigation. Users who want to toggle between two sessions should use tmux-level navigation without returning to the TUI.
- [Risk] Race condition if the client name is unavailable when entering a session. -> Mitigation: `clear_previous_session_for_client` is only called when `attach_client_name` is `Some`. This matches the existing pattern for `set_last_detached_session_for_client`.
